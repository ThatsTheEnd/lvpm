//! What an OCI source lists of the packages pinned to it, read with
//! `index::load` against `Registry`. Rust tests, not harness scenarios: they
//! live here because `lvpm-testkit` depends on `lvpm-core`.

use lvpm_core::cancel::CancelToken;
use lvpm_core::errors::{ListingFailures, ListingFault};
use lvpm_core::index::{self, Index, OciFeed, SourceSet};
use lvpm_core::oci::{FeedUrl, Oci};
use lvpm_core::released;
use lvpm_testkit::events::RecordingSink;
use lvpm_testkit::oci::Registry;

const CREATED: &str = "2026-10-31T14:05:00Z";

fn feed(registry: &Registry, pinned: &[&str]) -> OciFeed {
    OciFeed {
        name: "team".to_string(),
        url: FeedUrl::parse(&format!("oci://{}/team", registry.host())).unwrap(),
        pinned: pinned.iter().map(|name| name.to_string()).collect(),
    }
}

fn load(feeds: &[OciFeed]) -> anyhow::Result<Index> {
    let cache = tempfile::tempdir().unwrap();
    index::load(
        cache.path(),
        false,
        &SourceSet {
            extra: &[],
            local: &[],
            defaults: false,
            oci: feeds,
        },
        &Oci::default(),
        &RecordingSink::default(),
        &CancelToken::new(),
    )
}

/// A package version as `lvpm publish` writes it.
fn seed(registry: &Registry, name: &str, version: &str) -> String {
    registry
        .seed_package(
            &format!("team/{name}"),
            version,
            &[
                ("org.opencontainers.image.title", name),
                ("org.opencontainers.image.version", version),
                ("org.opencontainers.image.created", CREATED),
                ("dev.lvpm.requires", "beta >=1.0, gamma"),
                ("dev.lvpm.labview", "21.0"),
            ],
            format!("{name} {version}").as_bytes(),
        )
        .manifest
}

/// The failures `load` gave, as core says them: the hint a front end adds to a
/// package its source does not list is not in them.
fn failure_lines(error: &anyhow::Error) -> Vec<String> {
    let failures = error
        .downcast_ref::<ListingFailures>()
        .unwrap_or_else(|| panic!("not a listing failure: {error:#}"));
    failures.to_string().lines().map(str::to_string).collect()
}

#[test]
fn a_pinned_package_is_listed_by_its_tags_with_what_the_annotations_say() {
    let registry = Registry::start();
    let one = seed(&registry, "alpha", "1.0.0");
    let two = seed(&registry, "alpha", "1.1.0");
    let loaded = load(&[feed(&registry, &["alpha"])]).unwrap();

    let mut entries = loaded.entries;
    entries.sort_by(|a, b| a.version.cmp(&b.version));
    let host = registry.host();
    let versions: Vec<_> = entries.iter().map(|e| e.version.to_string()).collect();
    assert_eq!(versions, ["1.0.0", "1.1.0"]);
    let entry = &entries[1];
    assert_eq!(entry.name, "alpha");
    assert_eq!(entry.url, format!("oci://{host}/team/alpha@{two}"));
    assert_eq!(entries[0].url, format!("oci://{host}/team/alpha@{one}"));
    assert_eq!(entry.source, format!("oci://{host}/team"));
    assert_eq!(entry.md5, None);
    assert_eq!(entry.lv_min, Some(21.0));
    assert_eq!(entry.released, released::parse_rfc3339(CREATED));
    let requires: Vec<_> = entry
        .requires
        .iter()
        .map(|r| (r.name.as_str(), r.min.as_ref().map(ToString::to_string)))
        .collect();
    assert_eq!(
        requires,
        [("beta", Some("1.0".to_string())), ("gamma", None)]
    );
}

#[test]
fn a_name_nothing_is_pinned_for_is_never_asked_and_a_source_with_no_pins_is_not_asked_at_all() {
    let registry = Registry::start();
    seed(&registry, "alpha", "1.0.0");
    seed(&registry, "beta", "1.0.0");
    let loaded = load(&[feed(&registry, &["alpha"])]).unwrap();
    assert_eq!(loaded.entries.len(), 1);
    let asked: Vec<String> = registry.requests().into_iter().map(|r| r.path).collect();
    assert!(
        asked.iter().all(|path| !path.contains("team/beta")),
        "{asked:?}"
    );

    registry.clear_requests();
    let none = load(&[feed(&registry, &[])]).unwrap();
    assert!(none.entries.is_empty());
    assert!(registry.requests().is_empty());
}

#[test]
fn a_manifest_that_names_another_package_is_not_an_entry_of_the_pinned_one() {
    let registry = Registry::start();
    seed(&registry, "alpha", "1.0.0");
    // Tagged in alpha's repository, and claiming to be beta.
    registry.seed_package(
        "team/alpha",
        "2.0.0",
        &[
            ("org.opencontainers.image.title", "beta"),
            ("org.opencontainers.image.version", "2.0.0"),
        ],
        b"beta 2.0.0",
    );
    let loaded = load(&[feed(&registry, &["alpha"])]).unwrap();
    let listed: Vec<_> = loaded.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(listed, ["alpha"]);
}

#[test]
fn a_tag_that_is_no_tag_is_never_asked_for() {
    let registry = Registry::start();
    seed(&registry, "alpha", "1.0.0");
    registry.seed_manifest("team/alpha", "x?y#z", b"{}");
    let loaded = load(&[feed(&registry, &["alpha"])]).unwrap();
    assert_eq!(loaded.entries.len(), 1);
    let asked: Vec<String> = registry.requests().into_iter().map(|r| r.path).collect();
    assert!(asked.iter().all(|path| !path.contains('#')), "{asked:?}");
}

#[test]
fn a_manifest_that_is_not_a_package_is_skipped() {
    let registry = Registry::start();
    registry.seed_image("team/alpha", "1.2.0");
    // A package manifest that says neither what it is nor which version.
    registry.seed_package("team/alpha", "1.3.0", &[], b"nameless");
    seed(&registry, "alpha", "1.1.0");
    let loaded = load(&[feed(&registry, &["alpha"])]).unwrap();
    let versions: Vec<_> = loaded
        .entries
        .iter()
        .map(|e| e.version.to_string())
        .collect();
    assert_eq!(versions, ["1.1.0"]);
}

#[test]
fn a_repository_with_an_sbom_fallback_tag_still_lists_its_package() {
    let registry = Registry::start();
    let manifest = seed(&registry, "alpha", "1.0.0");
    // No referrers API: the SBOM is listed under the `sha256-<hex>` tag.
    registry.offer_referrers_api(false);
    registry.seed_referrer(
        "team/alpha",
        &manifest,
        "application/vnd.cyclonedx+json",
        b"{}",
    );
    let loaded = load(&[feed(&registry, &["alpha"])]).unwrap();
    let versions: Vec<_> = loaded
        .entries
        .iter()
        .map(|e| e.version.to_string())
        .collect();
    assert_eq!(versions, ["1.0.0"]);
    let asked: Vec<String> = registry.requests().into_iter().map(|r| r.path).collect();
    assert!(
        asked
            .iter()
            .all(|path| !path.contains("/manifests/sha256-")),
        "{asked:?}"
    );
}

#[test]
fn a_tag_of_another_type_or_one_that_answers_not_found_is_skipped() {
    let registry = Registry::start();
    seed(&registry, "alpha", "1.0.0");
    let index = "application/vnd.oci.image.index.v1+json";
    registry.seed_manifest_of_type("team/alpha", "multi", index, b"{\"manifests\":[]}");
    registry.seed_image("team/alpha", "gone");
    registry.answer("GET", "/v2/team/alpha/manifests/gone", 404);
    let loaded = load(&[feed(&registry, &["alpha"])]).unwrap();
    let versions: Vec<_> = loaded
        .entries
        .iter()
        .map(|e| e.version.to_string())
        .collect();
    assert_eq!(versions, ["1.0.0"]);
}

#[test]
fn a_repository_whose_tags_are_none_of_its_package_is_not_listed() {
    let registry = Registry::start();
    registry.seed_image("team/alpha", "latest");
    let error = load(&[feed(&registry, &["alpha"])]).err().unwrap();
    let host = registry.host();
    assert_eq!(
        failure_lines(&error),
        [format!(
            "alpha: source \"team\" (oci://{host}/team) does not list it"
        )]
    );
}

#[test]
fn a_pin_the_registry_has_nothing_for_is_not_listed() {
    let registry = Registry::start();
    let error = load(&[feed(&registry, &["gamma"])]).err().unwrap();
    let host = registry.host();
    assert_eq!(
        failure_lines(&error),
        [format!(
            "gamma: source \"team\" (oci://{host}/team) does not list it"
        )]
    );
}

#[test]
fn a_registry_that_hides_what_a_login_cannot_read_is_a_refusal_not_a_missing_package() {
    let registry = Registry::start();
    registry.deny_missing();
    let error = load(&[feed(&registry, &["gamma"])]).err().unwrap();
    let host = registry.host();
    assert_eq!(
        failure_lines(&error),
        [format!(
            "gamma: registry {host} refused the lookup for source \"team\"; the package is \
             missing or your login cannot read it; run docker login {host}"
        )]
    );
}

#[test]
fn each_status_a_registry_answers_is_told_apart() {
    for (status, refused) in [(401, true), (403, true), (500, false), (503, false)] {
        let registry = Registry::start();
        seed(&registry, "gamma", "1.0.0");
        registry.answer("GET", "/v2/team/gamma/tags/list", status);
        let error = load(&[feed(&registry, &["gamma"])]).err().unwrap();
        let failures = error.downcast_ref::<ListingFailures>().unwrap();
        let [failure] = failures.failures() else {
            panic!("one failure: {failures}");
        };
        match (&failure.fault, refused) {
            (ListingFault::Refused, true) => {}
            (ListingFault::Unreachable(cause), false) => {
                assert!(cause.contains(&status.to_string()), "{cause}");
            }
            (other, _) => panic!("{status}: {other:?}"),
        }
    }
}

#[test]
fn a_token_service_that_answers_not_found_is_no_word_on_the_package() {
    let registry = Registry::start();
    seed(&registry, "alpha", "1.0.0");
    registry.answer("GET", "/token", 404);
    let error = load(&[feed(&registry, &["alpha"])]).err().unwrap();
    let failures = error.downcast_ref::<ListingFailures>().unwrap();
    let [failure] = failures.failures() else {
        panic!("one failure: {failures}");
    };
    assert!(
        matches!(&failure.fault, ListingFault::Unreachable(cause) if cause.contains("404")),
        "{:?}",
        failure.fault
    );
}

#[test]
fn a_registry_that_is_down_is_unreachable() {
    let registry = Registry::start();
    registry.go_down();
    let error = load(&[feed(&registry, &["gamma"])]).err().unwrap();
    let lines = failure_lines(&error);
    let host = registry.host();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with(&format!(
            "gamma: source \"team\" (oci://{host}/team) is unreachable: "
        )),
        "{lines:?}"
    );
}

#[test]
fn every_failing_pin_is_reported_together_in_package_order() {
    let registry = Registry::start();
    seed(&registry, "alpha", "1.0.0");
    registry.answer("GET", "/v2/team/gamma/tags/list", 403);
    let error = load(&[feed(&registry, &["gamma", "alpha", "delta"])])
        .err()
        .unwrap();
    let host = registry.host();
    assert_eq!(
        failure_lines(&error),
        [
            format!("delta: source \"team\" (oci://{host}/team) does not list it"),
            format!(
                "gamma: registry {host} refused the lookup for source \"team\"; the package is \
                 missing or your login cannot read it; run docker login {host}"
            ),
        ]
    );
}

#[test]
fn a_package_name_a_registry_cannot_hold_is_not_listed_and_asks_nothing() {
    let registry = Registry::start();
    let error = load(&[feed(&registry, &["_lib"])]).err().unwrap();
    assert!(failure_lines(&error)[0].starts_with("_lib: source \"team\""));
    assert!(registry.requests().is_empty());
}
