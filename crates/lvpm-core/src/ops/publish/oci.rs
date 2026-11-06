//! `publish --to oci://...`: the package as one tagged OCI manifest, never
//! over an existing tag.
//!
//! In order: read the package and validate its tag, check the tag is free,
//! upload the config and the layer (skipping a blob the registry holds), check
//! the tag again, then PUT the manifest under the tag. Every failure before
//! the PUT leaves nothing visible: blobs no manifest names are not a version.
//! The race between the second check and the PUT stays open: OCI has no
//! create-only tag, so the docs tell teams to turn on tag immutability.

use super::Published;
use crate::events::Event;
use crate::lvpkg;
use crate::oci::{
    self, ANNOTATION_CREATED, ANNOTATION_LABVIEW, ANNOTATION_REQUIRES, ANNOTATION_TITLE,
    ANNOTATION_VERSION, ARTIFACT_TYPE, Access, EMPTY_CONFIG, EMPTY_CONFIG_TYPE, FeedUrl,
    LAYER_TYPE, MANIFEST_TYPE, Reference, Repo, digest_of,
};
use crate::ops::OpEnv;
use crate::spec::Spec;
use crate::{cache, timefmt};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Map, Value, json};
use std::io::Cursor;
use std::path::Path;

/// What every failure before the manifest is written ends in.
const SAFE_TO_RERUN: &str = "nothing was published, it is safe to rerun";
/// What a spec's LabVIEW gate starts with; `lvpkg` always writes it so.
const LABVIEW_GATE: &str = "LabVIEW>=";

pub(super) fn publish(env: &OpEnv, feed: &FeedUrl, package: &Path) -> Result<Published> {
    let (bytes, spec) = read_package(package).map_err(|why| anyhow!("{why:#}; {SAFE_TO_RERUN}"))?;
    let tag = spec.version.as_str();
    if !oci::valid_tag(tag) {
        bail!("{tag} is not a valid OCI tag; publish a version without \"+\" build metadata");
    }
    let Some(name) = feed.repository(&spec.name) else {
        bail!(
            "{} is not a valid OCI repository name; name the package with lowercase letters and              digits, separated by . _ - (nothing was published)",
            spec.name
        );
    };
    let repo = Repo {
        registry: feed.registry(),
        name: &name,
        access: Access::Push,
    };
    let reference = Reference::tagged(feed.registry(), &name, tag);

    let client = &env.oci;
    let free = || -> Result<()> {
        match client
            .manifest_exists(&repo, tag)
            .map_err(not_published(&name))?
        {
            true => bail!(taken(&name, tag, feed)),
            false => Ok(()),
        }
    };
    free()?;
    env.sink.emit(Event::OciUploading {
        repository: name.clone(),
        tag: tag.to_string(),
        registry: feed.registry().to_string(),
    });
    let config = blob(env, &repo, EMPTY_CONFIG)?;
    let layer = blob(env, &repo, &bytes)?;
    free()?;
    let manifest = manifest(&spec, &config, &layer, bytes.len());
    let digest = client
        .put_manifest(&repo, tag, &manifest)
        .map_err(|why| at_the_put(why, &name, &reference))?;
    Ok(Published::Oci {
        name: spec.name,
        version: spec.version,
        reference,
        digest,
    })
}

/// The package's bytes and the spec inside them.
fn read_package(package: &Path) -> Result<(Vec<u8>, Spec)> {
    let bytes = std::fs::read(package).with_context(|| format!("reading {}", package.display()))?;
    let spec = lvpkg::read(&mut zip::ZipArchive::new(Cursor::new(bytes.as_slice()))?)
        .with_context(|| format!("reading {}", package.display()))?;
    Ok((bytes, spec))
}

/// The digest of `bytes` once the repository holds them.
fn blob(env: &OpEnv, repo: &Repo, bytes: &[u8]) -> Result<String> {
    let digest = digest_of(bytes);
    let failed = not_published(repo.name);
    if !env.oci.blob_exists(repo, &digest).map_err(&failed)? {
        env.oci.upload_blob(repo, bytes).map_err(&failed)?;
    }
    Ok(digest)
}

/// The image manifest of a package: an empty config, the archive as the one
/// layer, and the annotations an index reads without the layer.
fn manifest(spec: &Spec, config: &str, layer: &str, layer_size: usize) -> Vec<u8> {
    let mut annotations = Map::new();
    let mut note =
        |key: &str, value: String| annotations.insert(key.to_string(), Value::from(value));
    note(ANNOTATION_TITLE, spec.name.clone());
    note(ANNOTATION_VERSION, spec.version.clone());
    note(ANNOTATION_CREATED, timefmt::rfc3339(cache::now()));
    if let Some(requires) = &spec.requires {
        note(ANNOTATION_REQUIRES, spaced(requires));
    }
    if let Some(labview) = spec
        .lv_gate
        .as_deref()
        .and_then(|g| g.strip_prefix(LABVIEW_GATE))
    {
        note(ANNOTATION_LABVIEW, labview.to_string());
    }
    json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST_TYPE,
        "artifactType": ARTIFACT_TYPE,
        "config": {
            "mediaType": EMPTY_CONFIG_TYPE,
            "digest": config,
            "size": EMPTY_CONFIG.len(),
        },
        "layers": [{
            "mediaType": LAYER_TYPE,
            "digest": layer,
            "size": layer_size,
        }],
        "annotations": annotations,
    })
    .to_string()
    .into_bytes()
}

/// `beta>=1.0,gamma` as `beta >=1.0, gamma`: each name, a space, its floor.
fn spaced(requires: &str) -> String {
    let item = |item: &str| match item.find(['<', '>', '=']) {
        Some(at) => format!("{} {}", &item[..at], &item[at..]),
        None => item.to_string(),
    };
    requires.split(',').map(item).collect::<Vec<_>>().join(", ")
}

/// The tag-exists message, in the shape the folder publish uses.
fn taken(repository: &str, tag: &str, feed: &FeedUrl) -> String {
    let said = super::already_published(&format!("{repository}:{tag}"), &format!(" at {feed}"));
    format!("{said}; bump the version to publish again")
}

/// What turns a failure before the manifest is written, in `repository`, into
/// the error to show.
fn not_published(repository: &str) -> impl Fn(oci::Error) -> anyhow::Error {
    move |why| match why.is_refused() {
        true => anyhow!(
            "registry {reg} refused the upload to {repository}; run docker login {reg} with a \
             token that can write packages; {SAFE_TO_RERUN}",
            reg = why.registry()
        ),
        false => anyhow!("{why}; {SAFE_TO_RERUN}"),
    }
}

/// A failure of the PUT itself. A 4xx or a failed login refused it; anything
/// else (no answer, a 5xx a proxy may have sent after the registry committed,
/// a digest that does not match) may have stored it.
fn at_the_put(why: oci::Error, repository: &str, reference: &Reference) -> anyhow::Error {
    let refused = why.status_code().is_some_and(|status| status < 500)
        || matches!(why, oci::Error::Protocol { .. })
        || why.is_host_level();
    match refused {
        true => not_published(repository)(why),
        false => anyhow!(
            "{why}; the manifest upload did not finish, check whether {reference} exists before \
             rerunning"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_are_a_name_a_space_and_the_floor() {
        assert_eq!(spaced("beta>=1.0"), "beta >=1.0");
        assert_eq!(
            spaced("beta>=1.0,gamma,delta>2"),
            "beta >=1.0, gamma, delta >2"
        );
    }

    fn status(status: u16) -> oci::Error {
        oci::Error::Status {
            registry: "r:1".into(),
            status,
            request: "PUT t/a:1".into(),
        }
    }

    fn unreachable() -> oci::Error {
        oci::Error::Unreachable {
            registry: "r:1".into(),
            cause: "closed".into(),
        }
    }

    #[test]
    fn a_refusal_says_how_to_log_in_and_that_nothing_was_published() {
        for refused in [status(401), status(403)] {
            assert_eq!(
                not_published("t/a")(refused).to_string(),
                "registry r:1 refused the upload to t/a; run docker login r:1 with a token that \
                 can write packages; nothing was published, it is safe to rerun"
            );
        }
        assert_eq!(
            not_published("t/a")(status(500)).to_string(),
            "registry r:1 answered 500 to PUT t/a:1; nothing was published, it is safe to rerun"
        );
    }

    #[test]
    fn a_put_that_was_refused_wrote_nothing_and_one_that_may_have_been_stored_says_to_look() {
        let reference = Reference::tagged("r:1", "t/a", "1");
        assert_eq!(
            at_the_put(status(409), "t/a", &reference).to_string(),
            "registry r:1 answered 409 to PUT t/a:1; nothing was published, it is safe to rerun"
        );
        assert_eq!(
            at_the_put(status(504), "t/a", &reference).to_string(),
            "registry r:1 answered 504 to PUT t/a:1; the manifest upload did not finish, check whether oci://r:1/t/a:1 exists before rerunning"
        );
        let mismatch = oci::Error::DigestMismatch {
            registry: "r:1".into(),
            digest: "sha256:ab".into(),
        };
        assert!(
            at_the_put(mismatch, "t/a", &reference)
                .to_string()
                .contains("check whether")
        );
        let login = oci::Error::LoginRefused {
            registry: "r:1".into(),
            origin: "auths".into(),
        };
        assert_eq!(
            at_the_put(login, "t/a", &reference).to_string(),
            "registry r:1 refused the login from auths; run docker login r:1; nothing was \
             published, it is safe to rerun"
        );
        assert_eq!(
            at_the_put(unreachable(), "t/a", &reference).to_string(),
            "registry r:1 is unreachable: closed; the manifest upload did not finish, check \
             whether oci://r:1/t/a:1 exists before rerunning"
        );
    }

    fn spec(requires: Option<&str>, lv_gate: Option<&str>) -> Spec {
        Spec {
            name: "alpha".into(),
            version: "1.0.0".into(),
            display_name: None,
            file_groups: Vec::new(),
            script_vis: Vec::new(),
            requires: requires.map(str::to_string),
            lv_gate: lv_gate.map(str::to_string),
            description: Default::default(),
        }
    }

    #[test]
    fn the_manifest_names_the_config_and_the_layer_and_carries_what_an_index_reads() {
        let bytes = manifest(
            &spec(Some("beta>=1.0"), Some("LabVIEW>=21.0")),
            "sha256:c",
            "sha256:l",
            7,
        );
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["artifactType"], ARTIFACT_TYPE);
        assert_eq!(json["config"]["digest"], "sha256:c");
        assert_eq!(json["layers"][0]["digest"], "sha256:l");
        assert_eq!(json["layers"][0]["size"], 7);
        assert_eq!(json["annotations"][ANNOTATION_REQUIRES], "beta >=1.0");
        assert_eq!(json["annotations"][ANNOTATION_LABVIEW], "21.0");
        assert_eq!(json["annotations"][ANNOTATION_TITLE], "alpha");
    }

    #[test]
    fn a_package_with_no_dependencies_and_no_gate_has_neither_annotation() {
        let bytes = manifest(&spec(None, None), "sha256:c", "sha256:l", 7);
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        let annotations = json["annotations"].as_object().unwrap();
        assert!(!annotations.contains_key(ANNOTATION_REQUIRES));
        assert!(!annotations.contains_key(ANNOTATION_LABVIEW));
        assert!(annotations.contains_key(ANNOTATION_CREATED));
    }
}
