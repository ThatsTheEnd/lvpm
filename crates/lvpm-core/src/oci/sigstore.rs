//! The Sigstore bundle that signs a package: a referrer of the package's OCI
//! manifest (`actions/attest-build-provenance` with `push-to-registry`), and
//! the repository its certificate names.
//!
//! lvpm does not verify a bundle: `gh attestation verify` does, against the
//! OCI manifest's own bytes. What lvpm needs of the bundle is the repository to
//! hand gh as `--repo`.

use super::manifest::Manifest;
use super::reference::valid_digest;
use super::{BUNDLE_TYPE, Error, Oci, Repo};
use base64::Engine as _;
use serde_json::Value;

/// The Fulcio extension `SourceRepositoryURI` (1.3.6.1.4.1.57264.1.12) as DER:
/// tag, length and the OID's content.
const SOURCE_REPOSITORY_OID: [u8; 12] = [
    0x06, 0x0a, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x83, 0xbf, 0x30, 0x01, 0x0c,
];
/// A bundle is a few KiB; more is not one.
const MAX_BUNDLE_BYTES: u64 = 1024 * 1024;
const GITHUB: &str = "https://github.com/";
const TAG_BOOLEAN: u8 = 0x01;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_UTF8_STRING: u8 = 0x0c;

impl Oci {
    /// The Sigstore bundle attached to `subject`, the OCI manifest at `digest`
    /// of `repo`: `None` when it has none. A lookup that fails is an error,
    /// never `None`, so a failing registry cannot pass for an unsigned
    /// package.
    ///
    /// shortcut: the first bundle only. A package that carries several
    /// attestations (an SBOM attestation too) may list another first, and
    /// then fails to verify; try each bundle when that bites.
    pub fn sigstore_bundle(&self, repo: &Repo, subject: &str) -> Result<Option<Vec<u8>>, Error> {
        let Some(referrer) = self
            .referrers(repo, subject, BUNDLE_TYPE)?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let manifest = self.get_manifest(repo, &referrer.digest)?;
        match bundle_layer(&manifest) {
            Some(layer) => self
                .get_blob_up_to(repo, &layer, MAX_BUNDLE_BYTES)
                .map(Some),
            None => Err(Error::protocol(
                repo.registry,
                "listed a signature that has no bundle in it",
            )),
        }
    }
}

/// The digest of the one layer of a bundle's manifest.
fn bundle_layer(manifest: &Manifest) -> Option<String> {
    let json: Value = serde_json::from_slice(&manifest.bytes).ok()?;
    let [layer] = json["layers"].as_array()?.as_slice() else {
        return None;
    };
    let digest = layer["digest"].as_str().filter(|d| valid_digest(d))?;
    (layer["mediaType"] == BUNDLE_TYPE).then(|| digest.to_string())
}

/// `owner/repo` of the GitHub repository the bundle's certificate says built
/// the artifact: its `SourceRepositoryURI`. `None` when the bundle has no
/// certificate, the certificate has no such claim, or it is no GitHub
/// repository.
///
/// shortcut: a scan of the DER for the extension, not a full X.509 parse. gh
/// verifies the certificate and the repo this returns, so a wrong scan fails
/// closed; upgrade if a non-Fulcio signer must be supported.
pub fn source_repository(bundle: &[u8]) -> Option<String> {
    let bundle: Value = serde_json::from_slice(bundle).ok()?;
    let raw = bundle["verificationMaterial"]["certificate"]["rawBytes"].as_str()?;
    let der = base64::engine::general_purpose::STANDARD.decode(raw).ok()?;
    let at = der
        .windows(SOURCE_REPOSITORY_OID.len())
        .position(|window| window == SOURCE_REPOSITORY_OID)?;
    let mut rest = &der[at + SOURCE_REPOSITORY_OID.len()..];
    // The extension's `critical` flag is there only when set.
    let (tag, _, after) = read_tlv(rest)?;
    if tag == TAG_BOOLEAN {
        rest = after;
    }
    let (tag, content, _) = read_tlv(rest)?;
    if tag != TAG_OCTET_STRING {
        return None;
    }
    // The value is a DER UTF8String inside the octet string.
    let (tag, uri, _) = read_tlv(content)?;
    if tag != TAG_UTF8_STRING {
        return None;
    }
    let repo = std::str::from_utf8(uri).ok()?.strip_prefix(GITHUB)?;
    is_owner_and_repo(repo).then(|| repo.to_string())
}

/// One DER element at the start of `bytes`: its tag, its content and what
/// follows. Only lengths up to 65535 are read.
fn read_tlv(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = bytes.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = match first {
        0..=0x7f => (usize::from(first), rest),
        0x81 => (usize::from(*rest.first()?), rest.get(1..)?),
        0x82 => (
            usize::from(u16::from_be_bytes([*rest.first()?, *rest.get(1)?])),
            rest.get(2..)?,
        ),
        _ => return None,
    };
    Some((tag, rest.get(..length)?, rest.get(length..)?))
}

/// `owner/repo` with nothing else in it, and no leading `-` that could pass for
/// an option of gh.
fn is_owner_and_repo(text: &str) -> bool {
    let name = |part: &str| {
        !part.is_empty()
            && !part.starts_with('-')
            && !part.chars().all(|c| c == '.')
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    text.split_once('/')
        .is_some_and(|(owner, repo)| name(owner) && name(repo))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The bundle `gh attestation download oci://ghcr.io/astral-sh/uv:latest
    /// --repo astral-sh/uv` wrote (gh 2.100.0), with a real Fulcio certificate.
    const REAL: &[u8] = include_bytes!("../../tests/fixtures/sigstore-bundle-uv.json");

    /// A bundle whose certificate is `der`.
    fn bundle_of(der: &[u8]) -> Vec<u8> {
        let raw = base64::engine::general_purpose::STANDARD.encode(der);
        json!({ "verificationMaterial": { "certificate": { "rawBytes": raw } } })
            .to_string()
            .into_bytes()
    }

    /// `tag`, the DER length of `content` and `content`.
    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let length = match content.len() {
            n @ 0..=0x7f => vec![n as u8],
            n @ 0x80..=0xff => vec![0x81, n as u8],
            n => [&[0x82][..], &(n as u16).to_be_bytes()].concat(),
        };
        [&[tag][..], &length, content].concat()
    }

    /// The extension as Fulcio writes it: the OID, then the URI as a DER
    /// UTF8String inside an octet string.
    fn extension(uri: &str, critical: bool) -> Vec<u8> {
        let mut der = SOURCE_REPOSITORY_OID.to_vec();
        if critical {
            der.extend([TAG_BOOLEAN, 1, 0xff]);
        }
        der.extend(tlv(TAG_OCTET_STRING, &tlv(TAG_UTF8_STRING, uri.as_bytes())));
        der
    }

    #[test]
    fn a_real_fulcio_certificate_names_its_source_repository() {
        assert_eq!(source_repository(REAL).as_deref(), Some("astral-sh/uv"));
    }

    #[test]
    fn the_claim_is_read_with_or_without_the_critical_flag() {
        for critical in [false, true] {
            let der = extension("https://github.com/o/r", critical);
            assert_eq!(
                source_repository(&bundle_of(&der)).as_deref(),
                Some("o/r"),
                "critical {critical}"
            );
        }
    }

    #[test]
    fn a_claim_longer_than_127_bytes_is_read_with_its_long_form_length() {
        let repo = format!("o/{}", "r".repeat(150));
        let der = extension(&format!("https://github.com/{repo}"), false);
        assert_eq!(source_repository(&bundle_of(&der)), Some(repo));
    }

    #[test]
    fn a_bundle_without_a_usable_claim_has_no_source_repository() {
        let table = [
            ("not JSON", b"not JSON".to_vec()),
            ("no certificate", br#"{"verificationMaterial":{}}"#.to_vec()),
            (
                "a certificate that is not base64",
                json!({"verificationMaterial": {"certificate": {"rawBytes": "%%%"}}})
                    .to_string()
                    .into_bytes(),
            ),
            ("a certificate without the claim", bundle_of(&[0x30, 0x00])),
            (
                "a claim of another host",
                bundle_of(&extension("https://gitlab.com/o/r", false)),
            ),
            (
                "a claim that is no repository",
                bundle_of(&extension("https://github.com/o", false)),
            ),
            (
                "a repository that is an option",
                bundle_of(&extension("https://github.com/o/-x", false)),
            ),
            (
                "a claim with a path",
                bundle_of(&extension("https://github.com/o/r/x", false)),
            ),
            (
                "a claim that is cut short",
                bundle_of(&SOURCE_REPOSITORY_OID),
            ),
            (
                "a length that runs past the end",
                bundle_of(&[&SOURCE_REPOSITORY_OID[..], &[TAG_OCTET_STRING, 0x20, 0x0c]].concat()),
            ),
            (
                "an indefinite length",
                bundle_of(&[&SOURCE_REPOSITORY_OID[..], &[TAG_OCTET_STRING, 0x80, 0x00]].concat()),
            ),
            (
                "a length of three bytes",
                bundle_of(
                    &[
                        &SOURCE_REPOSITORY_OID[..],
                        &[TAG_OCTET_STRING, 0x83, 0, 0, 4, 1, 2, 3, 4],
                    ]
                    .concat(),
                ),
            ),
            (
                "a value that is no octet string",
                bundle_of(
                    &[
                        &SOURCE_REPOSITORY_OID[..],
                        &tlv(0x13, b"https://github.com/o/r"),
                    ]
                    .concat(),
                ),
            ),
            (
                "a flag with nothing after it",
                bundle_of(&[&SOURCE_REPOSITORY_OID[..], &[TAG_BOOLEAN, 1, 0xff]].concat()),
            ),
        ];
        for (what, bundle) in table {
            assert_eq!(source_repository(&bundle), None, "{what}");
        }
    }

    #[test]
    fn a_bundles_manifest_has_one_layer_of_the_bundle_type() {
        let digest = "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
        let layer = json!({ "mediaType": BUNDLE_TYPE, "digest": digest, "size": 1 });
        let of = |value: Value| Manifest {
            digest: digest.into(),
            bytes: value.to_string().into_bytes(),
        };
        assert_eq!(
            bundle_layer(&of(json!({ "layers": [layer] }))).as_deref(),
            Some(digest)
        );
        let other = json!({ "mediaType": "text/plain", "digest": digest, "size": 1 });
        for broken in [
            json!({ "layers": [] }),
            json!({ "layers": [layer, layer] }),
            json!({ "layers": [other] }),
            json!({ "layers": [{ "mediaType": BUNDLE_TYPE, "digest": "sha256:ab" }] }),
        ] {
            assert_eq!(bundle_layer(&of(broken.clone())), None, "{broken}");
        }
    }
}
