//! The OCI client of `lvpm-core` (login, manifests, blobs, digests) against
//! `Registry`. Rust tests, not harness scenarios. They live here because
//! `lvpm-testkit` depends on `lvpm-core`: a dev-dependency back would be a cycle.

use lvpm_core::oci::{Access, Error, Oci, Repo, digest_of};
use lvpm_testkit::oci::{LoggedRequest, Registry};

const NAME: &str = "team/alpha";

/// `run` on `NAME` in the registry: a `Repo` borrows the registry's host.
fn with<T>(registry: &Registry, access: Access, run: impl FnOnce(&Repo) -> T) -> T {
    let host = registry.host();
    run(&Repo {
        registry: &host,
        name: NAME,
        access,
    })
}

fn paths(requests: &[LoggedRequest]) -> Vec<String> {
    requests
        .iter()
        .map(|r| {
            let auth = r.authorization.as_deref().map_or("no login", |_| "login");
            format!("{} {} ({auth})", r.method, r.path)
        })
        .collect()
}

#[test]
fn a_challenged_call_asks_the_realm_for_a_token_and_repeats_with_it() {
    let registry = Registry::start();
    let oci = Oci::default();
    let found = with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0")
    });
    assert_eq!(found, Ok(false));
    let service = "service=fake-registry";
    let scope = "scope=repository%3Ateam%2Falpha%3Apull";
    assert_eq!(
        paths(&registry.requests()),
        [
            "HEAD /v2/team/alpha/manifests/1.0.0 (no login)".to_string(),
            format!("GET /token?{service}&{scope} (no login)"),
            "HEAD /v2/team/alpha/manifests/1.0.0 (login)".to_string(),
        ]
    );
}

#[test]
fn a_token_is_kept_for_its_repository_and_access_so_a_run_logs_in_once_for_each() {
    let registry = Registry::start();
    let oci = Oci::default();
    with(&registry, Access::Push, |repo| {
        oci.manifest_exists(repo, "1.0.0").unwrap();
        oci.manifest_exists(repo, "2.0.0").unwrap();
    });
    let tokens = |registry: &Registry| {
        let all = registry.requests();
        all.iter().filter(|r| r.path.starts_with("/token")).count()
    };
    assert_eq!(tokens(&registry), 1);
    with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0").unwrap()
    });
    assert_eq!(tokens(&registry), 2, "pull is another scope than push");
}

#[test]
fn a_push_asks_for_the_push_scope() {
    let registry = Registry::start();
    let oci = Oci::default();
    with(&registry, Access::Push, |repo| {
        oci.blob_exists(repo, "sha256:00").unwrap()
    });
    let token = registry
        .requests()
        .into_iter()
        .find(|r| r.path.starts_with("/token"))
        .expect("a token request");
    assert!(token.path.contains("pull%2Cpush"), "{}", token.path);
}

#[test]
fn a_manifest_put_by_tag_is_read_back_by_tag_and_by_digest_and_a_tag_is_then_taken() {
    let registry = Registry::start();
    let oci = Oci::default();
    let digest = with(&registry, Access::Push, |repo| {
        let config = oci.upload_blob(repo, b"{}").unwrap();
        let layer = oci.upload_blob(repo, b"package bytes").unwrap();
        let manifest = format!(
            r#"{{"schemaVersion":2,"config":{{"digest":"{config}","size":2}},"layers":[{{"digest":"{layer}","size":13}}]}}"#
        );
        let digest = oci
            .put_manifest(repo, "1.0.0", manifest.as_bytes())
            .unwrap();
        assert_eq!(digest, digest_of(manifest.as_bytes()));
        assert_eq!(oci.manifest_exists(repo, "1.0.0"), Ok(true));
        for reference in ["1.0.0", digest.as_str()] {
            let got = oci.get_manifest(repo, reference).unwrap();
            assert_eq!(
                (got.digest.as_str(), got.bytes.as_slice()),
                (digest.as_str(), manifest.as_bytes())
            );
        }
        assert_eq!(oci.get_blob(repo, &layer).unwrap(), b"package bytes");
        assert_eq!(oci.blob_exists(repo, &layer), Ok(true));
        digest
    });
    assert_eq!(registry.tag_digest(NAME, "1.0.0"), Some(digest));
}

#[test]
fn a_blob_is_uploaded_with_one_post_and_one_put_naming_its_digest() {
    let registry = Registry::start();
    let oci = Oci::default();
    let digest = with(&registry, Access::Push, |repo| {
        oci.upload_blob(repo, b"bytes").unwrap()
    });
    assert_eq!(digest, digest_of(b"bytes"));
    let writes: Vec<_> = registry.writes().iter().map(|r| r.method.clone()).collect();
    assert_eq!(writes, ["POST", "PUT"]);
    let put = &registry.writes()[1];
    assert!(
        put.path
            .ends_with(&format!("?digest={}", digest.replace(':', "%3A"))),
        "{}",
        put.path
    );
    assert_eq!(registry.blob(NAME, &digest), Some(b"bytes".to_vec()));
}

#[test]
fn what_the_registry_does_not_hold_is_not_found_and_not_an_error() {
    let registry = Registry::start();
    let oci = Oci::default();
    with(&registry, Access::Pull, |repo| {
        assert_eq!(oci.manifest_exists(repo, "9.9.9"), Ok(false));
        assert_eq!(oci.blob_exists(repo, &digest_of(b"x")), Ok(false));
        let missing = oci.get_manifest(repo, "9.9.9").unwrap_err();
        assert_eq!(missing.status_code(), Some(404));
        assert!(
            missing.to_string().contains("GET team/alpha:9.9.9"),
            "{missing}"
        );
    });
}

#[test]
fn bytes_that_miss_their_digest_are_refused_for_a_blob_a_digest_and_a_tag() {
    let registry = Registry::start();
    let oci = Oci::default();
    let layer = with(&registry, Access::Push, |repo| {
        oci.upload_blob(repo, b"layer").unwrap()
    });
    let manifest = registry.seed_manifest(NAME, "1.0.0", b"{}");
    registry.corrupt(NAME, &layer, b"other layer");
    registry.corrupt(NAME, &manifest, b"[]");
    with(&registry, Access::Pull, |repo| {
        for refused in [
            oci.get_blob(repo, &layer).unwrap_err(),
            oci.get_manifest(repo, &manifest).unwrap_err(),
            oci.get_manifest(repo, "1.0.0").unwrap_err(),
        ] {
            let digest = match &refused {
                Error::DigestMismatch { digest, .. } => digest.clone(),
                other => panic!("not a digest mismatch: {other}"),
            };
            assert!(digest == layer || digest == manifest, "{refused}");
            assert!(
                refused
                    .to_string()
                    .contains("served bytes that do not match"),
                "{refused}"
            );
            assert!(refused.to_string().contains(&registry.host()), "{refused}");
        }
    });
}

#[test]
fn a_status_is_an_error_that_says_whether_the_registry_refused_the_login() {
    let registry = Registry::start();
    let oci = Oci::default();
    registry.answer("HEAD", "/v2/team/alpha/manifests/500", 500);
    registry.answer("HEAD", "/v2/team/alpha/manifests/403", 403);
    registry.answer("HEAD", "/v2/team/alpha/manifests/401", 401);
    with(&registry, Access::Pull, |repo| {
        let server = oci.manifest_exists(repo, "500").unwrap_err();
        assert_eq!(server.status_code(), Some(500));
        assert!(!server.is_refused());
        assert!(server.to_string().contains(&registry.host()), "{server}");
        // A 403 after a valid token, and a 401 that offers no way to log in.
        for status in ["403", "401"] {
            let refused = oci.manifest_exists(repo, status).unwrap_err();
            assert!(refused.is_refused(), "{status}: {refused}");
        }
    });
}

#[test]
fn a_token_realm_that_fails_is_an_error_naming_the_registry() {
    let registry = Registry::start();
    registry.answer("GET", "/token", 500);
    let oci = Oci::default();
    let why = with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0")
    })
    .unwrap_err();
    assert_eq!(why.status_code(), Some(500));
    assert!(why.to_string().contains("the token request"), "{why}");
}

#[test]
fn a_registry_that_is_down_is_unreachable_and_named() {
    let registry = Registry::start();
    registry.go_down();
    let oci = Oci::default();
    let why = with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0")
    })
    .unwrap_err();
    assert!(matches!(why, Error::Unreachable { .. }), "{why}");
    assert_eq!(why.registry(), registry.host());
    assert!(!why.is_refused());
}

#[test]
fn an_upload_location_on_another_host_is_refused_before_any_bytes_are_sent() {
    let registry = Registry::start();
    registry.upload_location_host("elsewhere.example:9");
    let oci = Oci::default();
    let why = with(&registry, Access::Push, |repo| {
        oci.upload_blob(repo, b"bytes")
    })
    .unwrap_err();
    assert!(why.to_string().contains("another host"), "{why}");
    let writes: Vec<_> = registry.writes().iter().map(|r| r.method.clone()).collect();
    assert_eq!(writes, ["POST"]);
}

#[test]
fn a_manifest_larger_than_a_registry_allows_is_refused_not_read_to_the_end() {
    let registry = Registry::start();
    registry.seed_manifest(NAME, "big", &vec![b'x'; 4 * 1024 * 1024 + 1]);
    let oci = Oci::default();
    let why = with(&registry, Access::Pull, |repo| {
        oci.get_manifest(repo, "big")
    })
    .unwrap_err();
    assert!(matches!(why, Error::Protocol { .. }), "{why}");
    assert!(
        why.to_string().contains("sent more than 4194304 bytes"),
        "{why}"
    );
}
