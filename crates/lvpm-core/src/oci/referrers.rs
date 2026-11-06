//! Referrers: manifests whose `subject` is another manifest, such as the SBOM
//! of a package. A registry with the referrers API lists them at
//! `/v2/<name>/referrers/<digest>`. One without it answers `404` there, and a
//! client keeps the list itself, as an image index under the tag
//! `sha256-<hex>` (the OCI "tag schema").
//!
//! A lookup that fails with anything but `404` is an error: it is never read as
//! "no referrers", so a registry that is failing cannot pass for an unsigned
//! package.

use super::manifest::Manifest;
use super::reference::valid_digest;
use super::{
    EMPTY_CONFIG, EMPTY_CONFIG_TYPE, Error, INDEX_TYPE, MANIFEST_TYPE, MAX_MANIFEST_BYTES, Oci,
    Repo, base_url, descriptor, digest_of, expect, read_limited,
};
use reqwest::header::ACCEPT;
use serde_json::{Value, json};

/// A manifest that names another as its subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referrer {
    /// The digest of the referrer's own manifest.
    pub digest: String,
    pub size: u64,
    pub artifact_type: String,
}

impl Oci {
    /// The manifests of `repo` that have `subject` (a digest) as their subject
    /// and are of `artifact_type`. From the referrers API; on `404`, from the
    /// fallback index, where none is the same as no index.
    pub fn referrers(
        &self,
        repo: &Repo,
        subject: &str,
        artifact_type: &str,
    ) -> Result<Vec<Referrer>, Error> {
        check_subject(repo, subject)?;
        let mut url = reqwest::Url::parse(&format!(
            "{}/v2/{}/referrers/{subject}",
            base_url(repo.registry),
            repo.name
        ))
        .map_err(|_| Error::protocol(repo.registry, "has a name that makes no URL"))?;
        url.query_pairs_mut()
            .append_pair("artifactType", artifact_type);
        let request = format!("GET {}/referrers/{subject}", repo.name);
        let response = self.send(repo, |client| {
            client.get(url.clone()).header(ACCEPT, INDEX_TYPE)
        })?;
        let listed = match response.status().as_u16() {
            404 => self.fallback_index(repo, subject)?,
            _ => {
                let response = expect(repo, response, &request, &[200])?;
                let body = read_limited(repo.registry, response, MAX_MANIFEST_BYTES)?;
                manifests_of(repo, &body)?
            }
        };
        // A registry may ignore `artifactType`, and the fallback index never
        // filters, so the answer is checked here.
        Ok(listed
            .iter()
            .filter_map(Referrer::of)
            .filter(|referrer| referrer.artifact_type == artifact_type)
            .collect())
    }

    /// Attach `payload` to `subject` as a referrer of `artifact_type`: an image
    /// manifest with an empty config, `payload` as its one layer (of media type
    /// `artifact_type`) and `subject`. Its digest.
    ///
    /// A registry that answers the manifest PUT without the subject's digest in
    /// `OCI-Subject` does not index referrers, so the fallback index of the
    /// subject is updated to list this one.
    ///
    /// shortcut: the fallback index is read, then written, with no lock: two
    /// publishers that attach to one subject at once may lose one entry. Only
    /// a registry without the API has it, and a version is published once.
    pub fn push_referrer(
        &self,
        repo: &Repo,
        subject: &Manifest,
        artifact_type: &str,
        payload: &[u8],
    ) -> Result<String, Error> {
        check_subject(repo, &subject.digest)?;
        let config = self.ensure_blob(repo, EMPTY_CONFIG)?;
        let layer = self.ensure_blob(repo, payload)?;
        let manifest = json!({
            "schemaVersion": 2,
            "mediaType": MANIFEST_TYPE,
            "artifactType": artifact_type,
            "config": descriptor(EMPTY_CONFIG_TYPE, &config, EMPTY_CONFIG.len()),
            "layers": [descriptor(artifact_type, &layer, payload.len())],
            "subject": descriptor(MANIFEST_TYPE, &subject.digest, subject.bytes.len()),
        })
        .to_string()
        .into_bytes();
        let digest = digest_of(&manifest);
        let put = self.put(repo, &digest, MANIFEST_TYPE, &manifest)?;
        if put.subject.as_deref() != Some(subject.digest.as_str()) {
            let mut entry = descriptor(MANIFEST_TYPE, &digest, manifest.len());
            entry["artifactType"] = artifact_type.into();
            self.list_in_fallback(repo, &subject.digest, entry)?;
        }
        Ok(digest)
    }

    /// The descriptors the fallback index of `subject` lists; none when it has
    /// no index.
    fn fallback_index(&self, repo: &Repo, subject: &str) -> Result<Vec<Value>, Error> {
        match self.read_manifest(repo, &fallback_tag(subject), INDEX_TYPE) {
            Ok(index) => manifests_of(repo, &index.bytes),
            Err(why) if why.status_code() == Some(404) => Ok(Vec::new()),
            Err(why) => Err(why),
        }
    }

    /// Add `entry` to the fallback index of `subject`, keeping what it lists.
    fn list_in_fallback(&self, repo: &Repo, subject: &str, entry: Value) -> Result<(), Error> {
        let mut manifests = self.fallback_index(repo, subject)?;
        if !manifests.iter().any(|m| m["digest"] == entry["digest"]) {
            manifests.push(entry);
        }
        let index = json!({
            "schemaVersion": 2,
            "mediaType": INDEX_TYPE,
            "manifests": manifests,
        })
        .to_string()
        .into_bytes();
        self.put(repo, &fallback_tag(subject), INDEX_TYPE, &index)?;
        Ok(())
    }
}

impl Referrer {
    /// The referrer a descriptor of an index describes, when it describes one.
    fn of(descriptor: &Value) -> Option<Referrer> {
        Some(Referrer {
            digest: descriptor["digest"]
                .as_str()
                .filter(|d| valid_digest(d))?
                .to_string(),
            size: descriptor["size"].as_u64()?,
            artifact_type: descriptor["artifactType"].as_str()?.to_string(),
        })
    }
}

/// The tag a registry without the referrers API keeps the list under:
/// `sha256-<hex>`.
fn fallback_tag(subject: &str) -> String {
    subject.replacen(':', "-", 1)
}

/// `subject` is a digest, so no path or tag is made from anything else.
fn check_subject(repo: &Repo, subject: &str) -> Result<(), Error> {
    match valid_digest(subject) {
        true => Ok(()),
        false => Err(Error::protocol(
            repo.registry,
            "was asked for referrers of something that is no digest",
        )),
    }
}

/// The descriptors of the index in `body`.
fn manifests_of(repo: &Repo, body: &[u8]) -> Result<Vec<Value>, Error> {
    let index: Value = serde_json::from_slice(body)
        .map_err(|_| Error::protocol(repo.registry, "sent a referrers list that is not JSON"))?;
    Ok(index["manifests"].as_array().cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBJECT: &str = "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";

    #[test]
    fn the_fallback_tag_is_the_digest_with_a_dash_for_the_colon() {
        assert_eq!(
            fallback_tag(SUBJECT),
            "sha256-44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }

    #[test]
    fn a_descriptor_is_a_referrer_only_with_a_digest_a_size_and_an_artifact_type() {
        let whole =
            json!({ "digest": SUBJECT, "size": 7, "artifactType": "a/b", "annotations": {} });
        assert_eq!(
            Referrer::of(&whole),
            Some(Referrer {
                digest: SUBJECT.into(),
                size: 7,
                artifact_type: "a/b".into()
            })
        );
        for broken in [
            json!({ "digest": "sha256:ab", "size": 7, "artifactType": "a/b" }),
            json!({ "digest": SUBJECT, "artifactType": "a/b" }),
            json!({ "digest": SUBJECT, "size": 7 }),
            json!("text"),
        ] {
            assert_eq!(Referrer::of(&broken), None, "{broken}");
        }
    }
}
