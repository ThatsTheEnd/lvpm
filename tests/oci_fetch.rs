//! `fetch::oci_package` against `Registry`: a package comes by its manifest
//! digest; the manifest is read first, then the cache answers by the layer's
//! SHA-256. Rust tests, not harness
//! scenarios: they live here because `lvpm-testkit` depends on `lvpm-core`.

use lvpm_core::cache::Cache;
use lvpm_core::cancel::CancelToken;
use lvpm_core::fetch;
use lvpm_core::index::Entry;
use lvpm_core::oci::{Error, Oci};
use lvpm_core::version::Version;
use lvpm_testkit::oci::Registry;

const REPO: &str = "team/alpha";

fn entry(registry: &Registry, manifest: &str) -> Entry {
    Entry {
        name: "alpha".to_string(),
        version: Version::parse("1.1.0"),
        url: format!("oci://{}/{REPO}@{manifest}", registry.host()),
        md5: None,
        display_name: None,
        requires: Vec::new(),
        lv_min: None,
        source: format!("oci://{}/team", registry.host()),
        released: None,
    }
}

fn fetch(cache: &Cache, entry: &Entry) -> anyhow::Result<Vec<u8>> {
    fetch::oci_package(cache, &Oci::default(), entry, &CancelToken::new())
}

#[test]
fn a_package_is_fetched_by_its_manifest_digest() {
    let registry = Registry::start();
    let seeded = registry.seed_package(REPO, "1.1.0", &[], b"the archive");
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();

    let bytes = fetch(&cache, &entry(&registry, &seeded.manifest)).unwrap();
    assert_eq!(bytes, b"the archive");
}

#[test]
fn a_package_fetched_twice_downloads_its_layer_once() {
    let registry = Registry::start();
    let seeded = registry.seed_package(REPO, "1.1.0", &[], b"the archive");
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();
    let entry = entry(&registry, &seeded.manifest);

    assert_eq!(fetch(&cache, &entry).unwrap(), b"the archive");
    registry.clear_requests();
    assert_eq!(fetch(&cache, &entry).unwrap(), b"the archive");
    let asked: Vec<String> = registry.requests().into_iter().map(|r| r.path).collect();
    assert!(
        asked.iter().all(|path| !path.contains("/blobs/")),
        "{asked:?}"
    );
}

#[test]
fn nothing_is_read_by_tag_so_a_tag_that_moved_changes_nothing() {
    let registry = Registry::start();
    let seeded = registry.seed_package(REPO, "1.1.0", &[], b"the archive");
    registry.seed_package(REPO, "1.1.0", &[], b"other bytes");
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();
    registry.clear_requests();

    let bytes = fetch(&cache, &entry(&registry, &seeded.manifest)).unwrap();
    assert_eq!(bytes, b"the archive");
    let asked: Vec<String> = registry.requests().into_iter().map(|r| r.path).collect();
    let by_tag = asked
        .iter()
        .filter(|path| path.contains("1.1.0") || path.contains("tags"));
    assert_eq!(by_tag.count(), 0, "{asked:?}");
}

#[test]
fn a_layer_whose_bytes_do_not_match_its_digest_is_refused_and_not_kept() {
    let registry = Registry::start();
    let seeded = registry.seed_package(REPO, "1.1.0", &[], b"the archive");
    registry.corrupt(REPO, &seeded.layer, b"not the archive");
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();

    let why = fetch(&cache, &entry(&registry, &seeded.manifest)).unwrap_err();
    assert!(matches!(
        why.downcast_ref::<Error>(),
        Some(Error::DigestMismatch { .. })
    ));
    // Put the right bytes back: the refused ones were not kept in the meantime.
    registry.corrupt(REPO, &seeded.layer, b"the archive");
    assert_eq!(
        fetch(&cache, &entry(&registry, &seeded.manifest)).unwrap(),
        b"the archive"
    );
}

#[test]
fn a_manifest_the_registry_no_longer_holds_is_a_404_and_one_that_is_no_package_is_refused() {
    let registry = Registry::start();
    let seeded = registry.seed_package(REPO, "1.1.0", &[], b"the archive");
    let image = registry.seed_image(REPO, "1.2.0");
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();

    let not_a_package = fetch(&cache, &entry(&registry, &image)).unwrap_err();
    assert!(
        not_a_package.to_string().contains("is not an lvpm package"),
        "{not_a_package:#}"
    );

    registry.delete_manifest(REPO, &seeded.manifest);
    let gone = fetch(&cache, &entry(&registry, &seeded.manifest)).unwrap_err();
    let gone = gone.downcast_ref::<Error>().unwrap();
    assert_eq!(gone.status_code(), Some(404));
}

#[test]
fn a_cancelled_fetch_asks_the_registry_nothing() {
    let registry = Registry::start();
    let seeded = registry.seed_package(REPO, "1.1.0", &[], b"the archive");
    let cache_dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(cache_dir.path()).unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    registry.clear_requests();
    let why = fetch::oci_package(
        &cache,
        &Oci::default(),
        &entry(&registry, &seeded.manifest),
        &cancel,
    )
    .unwrap_err();
    assert!(why.is::<lvpm_core::errors::Cancelled>());
    assert!(registry.requests().is_empty());
}
