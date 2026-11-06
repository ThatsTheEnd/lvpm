//! The OCI client of `lvpm-core` (login, manifests, blobs, digests) against
//! `Registry`. Rust tests, not harness scenarios. They live here because
//! `lvpm-testkit` depends on `lvpm-core`: a dev-dependency back would be a cycle.

use lvpm_core::oci::logins::{self, Origin};
use lvpm_core::oci::{Access, Credential, Credentials, Error, Oci, Repo, digest_of};
use lvpm_testkit::oci::{LoggedRequest, Registry};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
fn an_expired_token_is_asked_for_again_and_the_call_goes_through() {
    let registry = Registry::start();
    let oci = Oci::default();
    with(&registry, Access::Push, |repo| {
        oci.manifest_exists(repo, "1.0.0").unwrap();
        assert_eq!(registry.token_requests().len(), 1);
        registry.expire_tokens();
        let digest = oci.upload_blob(repo, b"{}").unwrap();
        assert_eq!(digest, digest_of(b"{}"));
    });
    assert_eq!(registry.token_requests().len(), 2);
    assert_eq!(registry.blob(NAME, &digest_of(b"{}")), Some(b"{}".to_vec()));
}

#[test]
fn a_401_that_offers_no_challenge_is_returned_without_asking_the_realm_again() {
    let registry = Registry::start();
    let oci = Oci::default();
    registry.answer("HEAD", "/v2/team/alpha/manifests/2.0.0", 401);
    with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0").unwrap();
        let refused = oci.manifest_exists(repo, "2.0.0").unwrap_err();
        assert_eq!(refused.status_code(), Some(401));
    });
    assert_eq!(registry.token_requests().len(), 1);
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
fn a_tag_that_holds_an_index_or_a_docker_manifest_is_taken() {
    let registry = Registry::start();
    let oci = Oci::default();
    for (tag, media_type) in [
        ("index", "application/vnd.oci.image.index.v1+json"),
        (
            "docker",
            "application/vnd.docker.distribution.manifest.v2+json",
        ),
        (
            "list",
            "application/vnd.docker.distribution.manifest.list.v2+json",
        ),
    ] {
        registry.seed_manifest_of_type(NAME, tag, media_type, b"{}");
        let found = with(&registry, Access::Push, |repo| {
            oci.manifest_exists(repo, tag)
        });
        assert_eq!(found, Ok(true), "{media_type}");
    }
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
        for (refused, expected) in [
            (oci.get_blob(repo, &layer).unwrap_err(), &layer),
            (oci.get_manifest(repo, &manifest).unwrap_err(), &manifest),
            (oci.get_manifest(repo, "1.0.0").unwrap_err(), &manifest),
        ] {
            let digest = match &refused {
                Error::DigestMismatch { digest, .. } => digest.clone(),
                other => panic!("not a digest mismatch: {other}"),
            };
            assert_eq!(&digest, expected, "{refused}");
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
    assert!(why.to_string().contains("the token request"), "{why}");
    assert!(why.to_string().contains(&registry.host()), "{why}");
}

#[test]
fn a_token_realm_status_is_not_the_status_of_the_call_it_served() {
    // A 404 from the token service is no word on the package: a lock must not
    // read it as "not listed", nor a fetch as "no longer holds".
    for status in [404, 500] {
        let registry = Registry::start();
        registry.answer("GET", "/token", status);
        let oci = Oci::default();
        let why = with(&registry, Access::Pull, |repo| {
            oci.manifest_exists(repo, "1.0.0")
        })
        .unwrap_err();
        assert_eq!(why.status_code(), None, "{status}: {why}");
        assert!(!why.is_refused(), "{status}: {why}");
        assert!(why.to_string().contains(&status.to_string()), "{why}");
    }
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

// ---- logins from the Docker config ----

const SECRET: &str = "s1-7f3a9c-secret";

/// An `Oci` whose Docker config gives `credential` for every registry, and how
/// many times it was asked.
fn logged_in_with(credential: Credential) -> (Oci, Arc<AtomicUsize>) {
    let asked = Arc::new(AtomicUsize::new(0));
    let credentials: Credentials = {
        let asked = asked.clone();
        Arc::new(move |_| {
            asked.fetch_add(1, Ordering::SeqCst);
            Ok(Some(credential.clone()))
        })
    };
    (Oci::with_credentials(credentials), asked)
}

fn user_and_helper() -> Credential {
    Credential::basic(Origin::CredsStore("lvpmtest".into()), "u1", SECRET)
}

#[test]
fn a_registry_that_wants_a_login_is_given_the_basic_login_of_the_docker_config() {
    let registry = Registry::start();
    registry.require_login("team");
    let (oci, _) = logged_in_with(user_and_helper());
    let found = with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0")
    });
    assert_eq!(found, Ok(false));
    let asked = registry.token_requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].basic_login(),
        Some(("u1".to_string(), SECRET.to_string()))
    );
    assert_eq!(asked[0].scope(), "repository:team/alpha:pull");
}

#[test]
fn an_identity_token_is_traded_for_a_token_by_a_refresh_token_grant() {
    let registry = Registry::start();
    registry.require_login("team");
    let token = Credential::identity(Origin::CredHelpers("lvpmtest".into()), "r1-3c8f2b-token");
    let (oci, _) = logged_in_with(token);
    let found = with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0")
    });
    assert_eq!(found, Ok(false));
    let asked = registry.token_requests();
    assert_eq!(asked[0].method, "POST");
    assert_eq!(asked[0].refresh_token(), Some("r1-3c8f2b-token"));
    assert_eq!(asked[0].authorization, None);
    assert_eq!(asked[0].scope(), "repository:team/alpha:pull");
}

#[test]
fn a_token_realm_that_redirects_never_gets_the_identity_token_sent_on() {
    let elsewhere = Registry::start();
    let registry = Registry::start();
    registry.require_login("team");
    registry.redirect_token_realm(&format!("http://{}/token", elsewhere.addr()));
    let token = Credential::identity(Origin::CredHelpers("lvpmtest".into()), "r1-3c8f2b-token");
    let (oci, _) = logged_in_with(token);
    let why = with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0")
    })
    .unwrap_err();
    assert!(why.to_string().contains("307"), "{why}");
    assert!(
        elsewhere.requests().is_empty(),
        "the other host was asked: {:?}",
        elsewhere.requests()
    );
}

#[test]
fn a_login_the_token_realm_refuses_is_named_with_where_it_came_from_and_never_shows_the_secret() {
    let registry = Registry::start();
    registry.refuse_user("u1");
    let (oci, _) = logged_in_with(user_and_helper());
    let found = with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0")
    });
    let host = registry.host();
    let error = found.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "registry {host} refused the login from credsStore docker-credential-lvpmtest; run \
             docker login {host}"
        )
    );
    assert!(error.is_host_level());
    assert!(!format!("{error:?}{error}").contains(SECRET));
}

#[test]
fn an_anonymous_ask_that_the_realm_refuses_is_a_refused_status_not_a_refused_login() {
    let registry = Registry::start();
    registry.require_login("team");
    let oci = Oci::default();
    let error = with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0")
    })
    .unwrap_err();
    assert!(error.is_refused());
    assert!(!error.is_host_level());
}

#[test]
fn a_helper_that_failed_is_a_host_level_error_asked_once_however_many_calls_follow() {
    let registry = Registry::start();
    let asked = Arc::new(AtomicUsize::new(0));
    let credentials: Credentials = {
        let asked = asked.clone();
        Arc::new(move |host| {
            asked.fetch_add(1, Ordering::SeqCst);
            Err(logins::Error::HelperFailed {
                helper: "docker-credential-lvpmtest".into(),
                registry: host.to_string(),
                code: Some(3),
            })
        })
    };
    let oci = Oci::with_credentials(credentials);
    for access in [Access::Pull, Access::Push, Access::Pull] {
        let error = with(&registry, access, |repo| oci.manifest_exists(repo, "1.0.0"));
        let host = registry.host();
        assert_eq!(
            error.unwrap_err().to_string(),
            format!(
                "credential helper docker-credential-lvpmtest for {host} failed with exit code 3; \
                 run it by hand to see why"
            )
        );
    }
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}

#[test]
fn a_push_logs_in_for_the_push_scope_and_the_login_is_asked_once_for_both_scopes() {
    let registry = Registry::start();
    registry.require_login_to_write("team");
    let (oci, asked) = logged_in_with(user_and_helper());
    with(&registry, Access::Pull, |repo| {
        oci.manifest_exists(repo, "1.0.0").unwrap();
    });
    with(&registry, Access::Push, |repo| {
        oci.manifest_exists(repo, "1.0.0").unwrap();
    });
    let tokens = registry.token_requests();
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].basic_login(), Some(("u1".into(), SECRET.into())));
    assert_eq!(tokens[1].scope(), "repository:team/alpha:pull,push");
    assert_eq!(asked.load(Ordering::SeqCst), 1, "one ask for one registry");
}
