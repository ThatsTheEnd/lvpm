//! Referrers in the OCI client of `lvpm-core` against `Registry`: attached and
//! found through the referrers API, and through the fallback index of a
//! registry that has none. Rust tests, not harness scenarios.

use lvpm_core::oci::{Access, Error, Manifest, Oci, Referrer, Repo, SBOM_TYPE, digest_of};
use lvpm_testkit::oci::Registry;
use serde_json::Value;

const NAME: &str = "team/alpha";
const BUNDLE_TYPE: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";

/// A registry that holds the package `team/alpha:1.0.0`, and that manifest.
fn with_package(api: bool) -> (Registry, Manifest) {
    let registry = Registry::start();
    registry.offer_referrers_api(api);
    let seeded = registry.seed_package(NAME, "1.0.0", &[], b"the package");
    let bytes = registry.manifest(NAME, &seeded.manifest).unwrap();
    let subject = Manifest {
        digest: seeded.manifest,
        bytes,
    };
    (registry, subject)
}

fn on<T>(registry: &Registry, run: impl FnOnce(&Repo) -> T) -> T {
    let host = registry.host();
    run(&Repo {
        registry: &host,
        name: NAME,
        access: Access::Push,
    })
}

fn fallback_tag(subject: &Manifest) -> String {
    subject.digest.replacen(':', "-", 1)
}

#[test]
fn an_attached_sbom_is_listed_by_the_api_and_holds_the_payload_under_the_subject() {
    let (registry, subject) = with_package(true);
    let oci = Oci::default();
    let (digest, listed) = on(&registry, |repo| {
        let digest = oci
            .push_referrer(repo, &subject, SBOM_TYPE, b"{\"bomFormat\":\"CycloneDX\"}")
            .unwrap();
        (digest, oci.referrers(repo, &subject.digest, SBOM_TYPE))
    });
    let listed = listed.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].digest, digest);
    assert_eq!(listed[0].artifact_type, SBOM_TYPE);

    let referrer: Value =
        serde_json::from_slice(&registry.manifest(NAME, &digest).unwrap()).unwrap();
    assert_eq!(referrer["artifactType"], SBOM_TYPE);
    assert_eq!(referrer["subject"]["digest"], subject.digest.as_str());
    assert_eq!(referrer["subject"]["size"], subject.bytes.len());
    assert_eq!(
        referrer["config"]["mediaType"],
        "application/vnd.oci.empty.v1+json"
    );
    assert_eq!(referrer["layers"][0]["mediaType"], SBOM_TYPE);
    let layer = referrer["layers"][0]["digest"].as_str().unwrap();
    assert_eq!(
        registry.blob(NAME, layer),
        Some(b"{\"bomFormat\":\"CycloneDX\"}".to_vec())
    );
    assert_eq!(
        registry.tag_digest(NAME, &fallback_tag(&subject)),
        None,
        "a registry that answers OCI-Subject needs no fallback index"
    );
}

#[test]
fn without_the_api_the_referrer_is_listed_in_the_fallback_index_and_found_there() {
    let (registry, subject) = with_package(false);
    let oci = Oci::default();
    let (digest, listed) = on(&registry, |repo| {
        let digest = oci
            .push_referrer(repo, &subject, SBOM_TYPE, b"sbom")
            .unwrap();
        (digest, oci.referrers(repo, &subject.digest, SBOM_TYPE))
    });
    assert_eq!(
        listed.unwrap(),
        [Referrer {
            digest: digest.clone(),
            size: registry.manifest(NAME, &digest).unwrap().len() as u64,
            artifact_type: SBOM_TYPE.into(),
        }]
    );
    let index_digest = registry
        .tag_digest(NAME, &fallback_tag(&subject))
        .expect("the fallback tag");
    let index: Value =
        serde_json::from_slice(&registry.manifest(NAME, &index_digest).unwrap()).unwrap();
    assert_eq!(
        index["mediaType"],
        "application/vnd.oci.image.index.v1+json"
    );
    assert_eq!(index["manifests"][0]["digest"], digest.as_str());
    assert_eq!(index["manifests"][0]["artifactType"], SBOM_TYPE);
}

#[test]
fn the_fallback_index_keeps_what_it_listed_and_lists_a_referrer_once() {
    let (registry, subject) = with_package(false);
    let bundle = registry.seed_referrer(NAME, &subject.digest, BUNDLE_TYPE, b"a bundle");
    let oci = Oci::default();
    on(&registry, |repo| {
        let first = oci
            .push_referrer(repo, &subject, SBOM_TYPE, b"sbom")
            .unwrap();
        let again = oci
            .push_referrer(repo, &subject, SBOM_TYPE, b"sbom")
            .unwrap();
        assert_eq!(first, again, "the same payload is the same referrer");
        let kept = oci.referrers(repo, &subject.digest, BUNDLE_TYPE).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].digest, bundle);
    });
    let listed = registry.referrers(NAME, &subject.digest);
    let types: Vec<&str> = listed.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(types, [BUNDLE_TYPE, SBOM_TYPE]);
}

#[test]
fn a_lookup_returns_only_the_artifact_type_asked_for_with_the_api_and_without() {
    for api in [true, false] {
        let (registry, subject) = with_package(api);
        registry.seed_referrer(NAME, &subject.digest, BUNDLE_TYPE, b"a bundle");
        let oci = Oci::default();
        on(&registry, |repo| {
            let sboms = oci.referrers(repo, &subject.digest, SBOM_TYPE).unwrap();
            assert_eq!(sboms, [], "api {api}");
            oci.push_referrer(repo, &subject, SBOM_TYPE, b"sbom")
                .unwrap();
            let sboms = oci.referrers(repo, &subject.digest, SBOM_TYPE).unwrap();
            assert_eq!(sboms.len(), 1, "api {api}");
            let bundles = oci.referrers(repo, &subject.digest, BUNDLE_TYPE).unwrap();
            assert_eq!(bundles.len(), 1, "api {api}");
        });
    }
}

#[test]
fn no_referrers_and_no_fallback_index_is_an_empty_list_not_an_error() {
    for api in [true, false] {
        let (registry, subject) = with_package(api);
        let found = on(&registry, |repo| {
            Oci::default().referrers(repo, &subject.digest, SBOM_TYPE)
        });
        assert_eq!(found, Ok(vec![]), "api {api}");
    }
}

#[test]
fn a_lookup_that_fails_for_any_reason_but_404_is_an_error_never_an_empty_list() {
    for status in [403, 500] {
        let (registry, subject) = with_package(true);
        registry.answer(
            "GET",
            &format!("/v2/{NAME}/referrers/{}", subject.digest),
            status,
        );
        let found = on(&registry, |repo| {
            Oci::default().referrers(repo, &subject.digest, SBOM_TYPE)
        });
        assert_eq!(found.unwrap_err().status_code(), Some(status));
    }

    let (registry, subject) = with_package(false);
    let tag_path = format!("/v2/{NAME}/manifests/{}", fallback_tag(&subject));
    registry.answer("GET", &tag_path, 500);
    let found = on(&registry, |repo| {
        Oci::default().referrers(repo, &subject.digest, SBOM_TYPE)
    });
    assert_eq!(found.unwrap_err().status_code(), Some(500));
}

#[test]
fn a_registry_that_refuses_the_write_fails_the_push_whichever_way_it_lists() {
    for api in [true, false] {
        let (registry, subject) = with_package(api);
        registry.refuse_referrer_writes(NAME);
        let pushed = on(&registry, |repo| {
            Oci::default().push_referrer(repo, &subject, SBOM_TYPE, b"sbom")
        });
        let error = pushed.unwrap_err();
        assert!(error.is_refused(), "api {api}: {error}");
    }
}

#[test]
fn something_that_is_no_digest_is_never_made_into_a_path_or_a_tag() {
    let (registry, subject) = with_package(true);
    let found = on(&registry, |repo| {
        Oci::default().referrers(repo, "../../x", SBOM_TYPE)
    });
    assert!(matches!(found, Err(Error::Protocol { .. })), "{found:?}");
    let wrong = Manifest {
        digest: "latest".into(),
        bytes: subject.bytes.clone(),
    };
    let pushed = on(&registry, |repo| {
        Oci::default().push_referrer(repo, &wrong, SBOM_TYPE, b"sbom")
    });
    assert!(matches!(pushed, Err(Error::Protocol { .. })), "{pushed:?}");
    assert!(registry.writes().is_empty(), "{:?}", registry.writes());
    assert_eq!(digest_of(&subject.bytes), subject.digest);
}

#[test]
fn a_bundle_is_found_with_the_api_and_without_and_its_bytes_come_back_whole() {
    for api in [true, false] {
        let (registry, subject) = with_package(api);
        let oci = Oci::default();
        on(&registry, |repo| {
            let none = oci.sigstore_bundle(repo, &subject.digest);
            assert_eq!(none, Ok(None), "api {api}");
            // Not a bundle, so not what is looked for.
            oci.push_referrer(repo, &subject, SBOM_TYPE, b"sbom")
                .unwrap();
            let none = oci.sigstore_bundle(repo, &subject.digest);
            assert_eq!(none, Ok(None), "api {api}");
        });
        registry.seed_referrer(
            NAME,
            &subject.digest,
            BUNDLE_TYPE,
            b"{\"a\":\"bundle\"}\r\n",
        );
        let found = on(&registry, |repo| oci.sigstore_bundle(repo, &subject.digest));
        assert_eq!(
            found,
            Ok(Some(b"{\"a\":\"bundle\"}\r\n".to_vec())),
            "api {api}"
        );
    }
}

#[test]
fn a_bundle_lookup_that_fails_is_an_error_never_no_bundle() {
    let (registry, subject) = with_package(true);
    let bundle = registry.seed_referrer(NAME, &subject.digest, BUNDLE_TYPE, b"a bundle");
    let oci = Oci::default();
    registry.answer("GET", &format!("/v2/{NAME}/manifests/{bundle}"), 500);
    let found = on(&registry, |repo| oci.sigstore_bundle(repo, &subject.digest));
    assert_eq!(found.unwrap_err().status_code(), Some(500));

    let (registry, subject) = with_package(true);
    registry.answer(
        "GET",
        &format!("/v2/{NAME}/referrers/{}", subject.digest),
        403,
    );
    let found = on(&registry, |repo| oci.sigstore_bundle(repo, &subject.digest));
    assert_eq!(found.unwrap_err().status_code(), Some(403));
}

#[test]
fn a_listed_bundle_that_holds_no_bundle_is_an_error_too() {
    let (registry, subject) = with_package(true);
    // Listed as a bundle, but its layer is of another media type.
    let layer = registry.seed_blob(NAME, b"x");
    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "artifactType": BUNDLE_TYPE,
        "config": { "mediaType": "application/vnd.oci.empty.v1+json", "digest": registry.seed_blob(NAME, b"{}"), "size": 2 },
        "layers": [{ "mediaType": "text/plain", "digest": layer, "size": 1 }],
        "subject": { "mediaType": "application/vnd.oci.image.manifest.v1+json", "digest": subject.digest, "size": 0 },
    });
    registry.seed_manifest(NAME, "not-a-bundle", manifest.to_string().as_bytes());
    let found = on(&registry, |repo| {
        Oci::default().sigstore_bundle(repo, &subject.digest)
    });
    assert!(matches!(found, Err(Error::Protocol { .. })), "{found:?}");
}
