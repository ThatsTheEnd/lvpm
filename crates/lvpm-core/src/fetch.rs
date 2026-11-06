//! Acquire archive bytes for an already-resolved package, before installation.
//!
//! Remote requests reuse an unambiguous name/version/MD5 cache match; misses
//! download, verify, and populate the cache. Explicit `cache:<sha256>` entries
//! require a valid stored blob, while ordinary local paths remain live reads.
//! Feed fetching and resolution stay in `index`; storage stays in `cache`.

use crate::cache::{Cache, LocalEntry, sha256_hex};
use crate::cancel::CancelToken;
use crate::errors::{Cancelled, IndexFileTooLarge, IndexHashMismatch, IndexUrlRefused};
use crate::index::{self, Entry};
use crate::oci::{Access, Oci};
use anyhow::{Context, Result, ensure};
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{ACCEPT, HeaderMap};
use std::io::Read;

/// The most an index entry's file may be, in bytes: a community index is not
/// trusted to bound its own entries.
pub const MAX_INDEX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// A GitHub download that came back with a status that is not a success. Its
/// headers are kept (`error_for_status` drops them), so that a rate limit can
/// say when it resets. The URL is not kept: after a redirect it is the CDN's,
/// and its query is a signed grant to the file.
#[derive(Debug)]
pub struct Rejected {
    pub status: StatusCode,
    pub headers: HeaderMap,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}", self.status)
    }
}

impl std::error::Error for Rejected {}

/// Refuse a community index entry whose `url` is not a file of its publisher's
/// releases on the site at `web` (see [`crate::github::GitHub::web_base`]); any
/// other entry passes. The url is the index's, so it is checked before anything
/// is read from it: no UNC path, `cache:` or local path, `oci://` reference or
/// other host is ever opened for an index entry.
pub fn check_index_url(entry: &Entry, web: &str) -> Result<(), IndexUrlRefused> {
    let Some(pin) = &entry.index_pin else {
        return Ok(());
    };
    if is_release_file(&entry.url, web, &pin.publisher) {
        return Ok(());
    }
    Err(IndexUrlRefused {
        name: entry.name.clone(),
        version: entry.version.to_string(),
        index: entry.source.clone(),
        publisher: pin.publisher.clone(),
    })
}

/// Whether `url` is `<web>/<publisher>/releases/download/<tag>/<file>`, the
/// owner and repo compared ignoring ASCII case: exactly one tag and one file
/// segment, neither empty, `.` or `..`. A `%` after the prefix refuses the url,
/// since `%2f`, `%5c` and `%2e` would decode to a separator or a dot segment. A
/// url with userinfo, a query or a fragment, or one with a control character
/// (which a parser would strip), is no release file.
fn is_release_file(url: &str, web: &str, publisher: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let prefix = format!("{web}/{publisher}/releases/download/");
    let parsed_text = parsed.as_str();
    let (Some(head), Some(rest)) = (
        parsed_text.get(..prefix.len()),
        parsed_text.get(prefix.len()..),
    ) else {
        return false;
    };
    let segments: Vec<&str> = rest.split('/').collect();
    !url.chars().any(char::is_control)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && head.eq_ignore_ascii_case(&prefix)
        && !rest.contains('%')
        && segments.len() == 2
        && segments.iter().all(|s| !matches!(*s, "" | "." | ".."))
}

/// Return archive bytes, checking SHA-256 on cache reads and MD5 when advertised.
///
/// Ambiguous, missing, corrupt, or MD5-mismatched remote cache candidates trigger
/// a download. Without an advertised MD5, remote bytes are stored but are not
/// automatically reused on later requests. Local paths are not imported here.
/// Cache population also occurs during dry runs; target writes are the caller's
/// responsibility. Missing explicit cache entries fail instead of downloading.
///
/// A GitHub entry (see [`index::github_repo`]) is asked for its bytes
/// (`Accept: application/octet-stream`); `auth` gives the token sent along with
/// it, as `Authorization: Bearer`. It is called only when a download is due, so
/// finding the token (which may run `gh`) costs nothing on a cache hit. Nothing
/// of the token is kept or shown.
///
/// An entry a community index lists (`entry.index_pin`) is downloaded with no
/// token and no more than [`MAX_INDEX_DOWNLOAD_BYTES`], and its SHA-256 must be
/// the pinned one: bytes that are not fail with [`IndexHashMismatch`] before
/// anything is cached. The cache answers for it by the pinned SHA-256.
///
/// A cancelled `cancel` ends the fetch with [`Cancelled`] before it starts and
/// between the chunks of a download, so nothing of a cancelled download is
/// cached.
///
/// It handles no `oci://` entry: the dispatch is in `Downloads::fetch`
/// (`ops/session.rs`), which sends one to [`oci_package`].
pub fn package<'a>(
    cache: &Cache,
    client: &Client,
    entry: &Entry,
    auth: impl FnOnce() -> Option<&'a str>,
    cancel: &CancelToken,
) -> Result<Vec<u8>> {
    ensure_running(cancel)?;
    if let Some(bytes) = on_hand(cache, entry)? {
        return Ok(bytes);
    }
    download(
        cache,
        client,
        entry,
        auth(),
        cancel,
        MAX_INDEX_DOWNLOAD_BYTES,
    )
}

/// [`package`] past the cache: the download, with `index_limit` the most an
/// index entry's file may be.
fn download(
    cache: &Cache,
    client: &Client,
    entry: &Entry,
    auth: Option<&str>,
    cancel: &CancelToken,
    index_limit: u64,
) -> Result<Vec<u8>> {
    // An index's url is its own to write: name the package, not the url.
    let what = match entry.index_pin {
        Some(_) => format!("{} {}", entry.name, entry.version),
        None => entry.url.clone(),
    };
    let response = request(client, entry, auth)
        .send()
        .with_context(|| format!("downloading {what}"))?;
    if index::github_repo(entry).is_some() && !response.status().is_success() {
        return Err(Rejected {
            status: response.status(),
            headers: response.headers().clone(),
        }
        .into());
    }
    let response = response.error_for_status()?;
    let bytes = match entry.index_pin {
        Some(_) => read_within(response, cancel, index_limit, entry)?,
        None => read_body(response, cancel)?,
    };
    let md5 = verify_md5(entry, &bytes)?;
    verify_sha256(entry, &bytes)?;
    let sha256 = cache.store(&bytes)?;
    cache.record_download(&LocalEntry::from(entry), &entry.url, &sha256, &md5)?;
    Ok(bytes)
}

/// The archive of a package an OCI source lists, `entry.url` being the OCI
/// manifest's reference by digest: the manifest is read by that digest first, and
/// its one layer is the archive. Then the cache answers, by the layer's SHA-256;
/// otherwise the layer is fetched, its digest checked, and kept. Nothing is
/// read by tag, so a tag that moved changes nothing here.
///
/// The failures are [`crate::oci::Error`], for the caller to put in words.
pub fn oci_package(
    cache: &Cache,
    oci: &Oci,
    entry: &Entry,
    cancel: &CancelToken,
) -> Result<Vec<u8>> {
    ensure_running(cancel)?;
    let manifest_ref = index::manifest_reference(entry)?;
    let repo = manifest_ref.repo(Access::Pull);
    let manifest = oci.get_manifest(&repo, manifest_ref.name())?;
    let package = manifest
        .package()
        .with_context(|| format!("{} is not an lvpm package", entry.url))?;
    let sha256 = package.layer.trim_start_matches("sha256:");
    if let Some(bytes) = cache.load(sha256)? {
        return Ok(bytes);
    }
    ensure_running(cancel)?;
    let bytes = oci.get_blob(&repo, &package.layer)?;
    cache.store(&bytes)?;
    Ok(bytes)
}

/// The GET for an entry's archive. A token goes only to a GitHub entry's own URL:
/// a feed entry never gets one, and reqwest drops it when a redirect leaves the host.
fn request(client: &Client, entry: &Entry, auth: Option<&str>) -> RequestBuilder {
    let request = client.get(&entry.url);
    if index::github_repo(entry).is_none() {
        return request;
    }
    let request = request.header(ACCEPT, "application/octet-stream");
    match auth {
        Some(token) => request.bearer_auth(token),
        None => request,
    }
}

/// The archive's bytes when [`package`] would not download them: from the
/// cache or a local file. `None` when only the feed has them.
pub fn on_hand(cache: &Cache, entry: &Entry) -> Result<Option<Vec<u8>>> {
    // An index entry is on hand only by the SHA-256 it pins: its url never
    // names a cache blob or a local file.
    if let Some(pin) = &entry.index_pin {
        return cache.load(&pin.sha256.to_ascii_lowercase());
    }
    if let Some(sha256) = entry.url.strip_prefix("cache:") {
        let bytes = cache.load(sha256)?.with_context(|| format!(
            "cached archive for {} is missing or corrupt; run cache add again with the original package", entry.name,
        ))?;
        verify_md5(entry, &bytes)?;
        return Ok(Some(bytes));
    }
    if index::is_local_url(&entry.url) {
        let bytes = std::fs::read(&entry.url).with_context(|| format!("reading {}", entry.url))?;
        verify_md5(entry, &bytes)?;
        return Ok(Some(bytes));
    }
    if let Some(md5) = &entry.md5
        && let Some(sha256) =
            cache.package_sha256(&entry.name, &entry.version.to_string(), &md5.to_lowercase())?
        && let Some(bytes) = cache.load(&sha256)?
        && verify_md5(entry, &bytes).is_ok()
    {
        return Ok(Some(bytes));
    }
    Ok(None)
}

/// How much of a download is read between two looks at the cancel token.
const CHUNK_BYTES: usize = 64 * 1024;

/// All of `body`, a chunk at a time, looking at `cancel` between the chunks. A
/// read that fails once `cancel` was cancelled is the cancel: a stalled
/// download ends in a timeout, and the user asked for it to stop.
fn read_body(mut body: impl Read, cancel: &CancelToken) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; CHUNK_BYTES];
    loop {
        ensure_running(cancel)?;
        match body.read(&mut chunk) {
            Ok(0) => return Ok(bytes),
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            Err(_) if cancel.is_cancelled() => return Err(Cancelled.into()),
            Err(why) => return Err(why.into()),
        }
    }
}

/// [`read_body`] of at most `limit` bytes of `entry`'s file: a longer body fails
/// naming the limit, and is not kept.
fn read_within(
    body: impl Read,
    cancel: &CancelToken,
    limit: u64,
    entry: &Entry,
) -> Result<Vec<u8>> {
    let bytes = read_body(body.take(limit + 1), cancel)?;
    if bytes.len() as u64 > limit {
        return Err(IndexFileTooLarge {
            name: entry.name.clone(),
            version: entry.version.to_string(),
            limit,
        }
        .into());
    }
    Ok(bytes)
}

fn ensure_running(cancel: &CancelToken) -> Result<()> {
    match cancel.is_cancelled() {
        true => Err(Cancelled.into()),
        false => Ok(()),
    }
}

/// Compute lowercase MD5, rejecting bytes that differ from the advertised hash.
fn verify_md5(entry: &Entry, bytes: &[u8]) -> Result<String> {
    let got = index::md5_hex(bytes);
    if let Some(want) = &entry.md5 {
        ensure!(
            got.eq_ignore_ascii_case(want),
            "MD5 mismatch for {}: expected {want}, got {got}",
            entry.name
        );
    }
    Ok(got)
}

/// Reject bytes whose SHA-256 is not the one a community index pins for the
/// entry; any other entry pins none.
fn verify_sha256(entry: &Entry, bytes: &[u8]) -> Result<()> {
    let Some(pin) = &entry.index_pin else {
        return Ok(());
    };
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(&pin.sha256) {
        return Ok(());
    }
    Err(IndexHashMismatch {
        name: entry.name.clone(),
        version: entry.version.to_string(),
        index: entry.source.clone(),
        publisher: pin.publisher.clone(),
        pinned: pin.sha256.clone(),
        actual,
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Version;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;

    /// The fetch of a token nobody cancels, which is what most tests are about.
    fn package(cache: &Cache, client: &Client, entry: &Entry) -> Result<Vec<u8>> {
        super::package(cache, client, entry, || None, &CancelToken::new())
    }

    fn test_cache(name: &str) -> (PathBuf, Cache) {
        let root = std::env::temp_dir().join(format!("lvpm-fetch-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let cache = Cache::open(&root).unwrap();
        (root, cache)
    }

    fn entry(url: String, md5: Option<String>) -> Entry {
        Entry {
            name: "test_package".into(),
            version: Version::parse("1.0.0"),
            url,
            md5,
            display_name: None,
            requires: Vec::new(),
            lv_min: None,
            source: String::new(),
            released: None,
            index_pin: None,
        }
    }

    fn server(bytes: &'static [u8]) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/package.vip", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(bytes).unwrap();
        });
        (url, thread)
    }

    #[test]
    fn a_download_is_cached_and_reused_with_the_server_offline() {
        let (root, cache) = test_cache("reuse");
        let (url, thread) = server(b"package bytes");
        let entry = entry(url, Some(index::md5_hex(b"package bytes")));
        let client = Client::new();
        // The token is looked for when a download is due, not on a cache hit.
        let asked = std::cell::Cell::new(0);
        let fetch = |entry: &Entry| {
            let auth = || {
                asked.set(asked.get() + 1);
                None
            };
            super::package(&cache, &client, entry, auth, &CancelToken::new()).unwrap()
        };
        assert_eq!(fetch(&entry), b"package bytes");
        thread.join().unwrap();
        assert_eq!(fetch(&entry), b"package bytes");
        assert_eq!(asked.get(), 1);
        assert!(cache.imported_packages().unwrap().is_empty());
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    /// Serves `first`, says so on the first channel, and serves `rest` only
    /// once the second channel is signalled.
    fn halting_server(
        first: &'static [u8],
        rest: &'static [u8],
    ) -> (
        String,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/package.vip", listener.local_addr().unwrap());
        let (sent, first_sent) = std::sync::mpsc::channel();
        let (go, wait_for_go) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                first.len() + rest.len()
            )
            .unwrap();
            stream.write_all(first).unwrap();
            stream.flush().unwrap();
            sent.send(()).unwrap();
            wait_for_go
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            // The client may have given up and closed the connection.
            stream.write_all(rest).ok();
        });
        (url, first_sent, go, thread)
    }

    /// What a community index pins for `bytes`, as `entry` carries it.
    fn pinned(url: String, bytes: &[u8]) -> Entry {
        let mut pinned = entry(url, None);
        pinned.source = "github-index:o/r".into();
        pinned.index_pin = Some(index::IndexPin {
            sha256: sha256_hex(bytes),
            publisher: "someone/Serializer".into(),
        });
        pinned
    }

    /// Serves `bytes` once, and returns the head of the request it got.
    fn head_server(bytes: &'static [u8]) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/pkg.lvpkg", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
            stream.write_all(bytes).unwrap();
            String::from_utf8_lossy(&head).to_lowercase()
        });
        (url, thread)
    }

    #[test]
    fn an_index_entrys_file_is_asked_for_with_no_token_even_when_one_is_configured() {
        let (root, cache) = test_cache("index-no-token");
        let (url, server) = head_server(b"package bytes");
        let wanted = pinned(url, b"package bytes");
        let token = || Some("tok");
        let fetched = super::package(&cache, &Client::new(), &wanted, token, &CancelToken::new());
        assert_eq!(fetched.unwrap(), b"package bytes");
        let head = server.join().unwrap();
        assert!(!head.contains("authorization"), "{head}");
        assert!(!head.contains("octet-stream"), "{head}");
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn bytes_that_are_not_the_pinned_sha256_fail_with_both_hashes_and_cache_nothing() {
        let (root, cache) = test_cache("index-mismatch");
        let (url, thread) = server(b"other bytes");
        let wanted = pinned(url, b"package bytes");
        let why = package(&cache, &Client::new(), &wanted).unwrap_err();
        thread.join().unwrap();
        let mismatch = why.downcast_ref::<IndexHashMismatch>().expect("typed");
        assert_eq!(mismatch.pinned, sha256_hex(b"package bytes"));
        assert_eq!(mismatch.actual, sha256_hex(b"other bytes"));
        assert_eq!(
            (&*mismatch.name, &*mismatch.index, &*mismatch.publisher),
            ("test_package", "github-index:o/r", "someone/Serializer")
        );
        assert!(!root.join("content").exists());
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_pinned_file_is_cached_and_then_reused_by_its_sha256_with_the_server_gone() {
        let (root, cache) = test_cache("index-reuse");
        let (url, thread) = server(b"package bytes");
        let wanted = pinned(url, b"package bytes");
        assert_eq!(
            package(&cache, &Client::new(), &wanted).unwrap(),
            b"package bytes"
        );
        thread.join().unwrap();
        assert_eq!(
            package(&cache, &Client::new(), &wanted).unwrap(),
            b"package bytes"
        );
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn an_index_file_over_the_limit_fails_naming_the_limit_and_caches_nothing() {
        let (root, cache) = test_cache("index-limit");
        let client = Client::new();
        let download = |bytes: &'static [u8], limit| {
            let (url, thread) = server(bytes);
            let wanted = pinned(url, bytes);
            let fetched =
                super::download(&cache, &client, &wanted, None, &CancelToken::new(), limit);
            thread.join().unwrap();
            fetched
        };
        let why = download(b"12345", 4).unwrap_err();
        assert_eq!(
            why.to_string(),
            "the file of test_package 1.0.0 is larger than the download limit of 4 bytes"
        );
        assert!(!root.join("content").exists());
        assert_eq!(download(b"1234", 4).unwrap(), b"1234");
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_file_no_index_lists_is_not_limited_or_hash_checked() {
        let (root, cache) = test_cache("no-index-limit");
        let (url, thread) = server(b"12345");
        let unpinned = entry(url, None);
        let fetched = super::download(
            &cache,
            &Client::new(),
            &unpinned,
            None,
            &CancelToken::new(),
            4,
        );
        thread.join().unwrap();
        assert_eq!(fetched.unwrap(), b"12345");
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn an_oci_reference_is_never_read_as_a_file_and_is_not_on_hand() {
        let (root, cache) = test_cache("oci-on-hand");
        let reference = format!("oci://r:5000/team/alpha@sha256:{}", "ab".repeat(32));
        assert_eq!(on_hand(&cache, &entry(reference, None)).unwrap(), None);
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_cancelled_token_fetches_nothing() {
        let (root, cache) = test_cache("cancelled-before");
        let token = CancelToken::new();
        token.cancel();
        let wanted = entry("http://127.0.0.1:1/unavailable".into(), None);
        let why = super::package(&cache, &Client::new(), &wanted, || None, &token).unwrap_err();
        assert!(why.is::<Cancelled>(), "{why:#}");
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    /// A body that gives out the way a stalled download does, once it was
    /// cancelled, or without anyone having cancelled it.
    struct GivesOut<'a>(Option<&'a CancelToken>);

    impl Read for GivesOut<'_> {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            if let Some(token) = self.0 {
                assert!(token.cancel());
            }
            Err(std::io::ErrorKind::TimedOut.into())
        }
    }

    #[test]
    fn a_read_that_fails_after_a_cancel_is_the_cancel_and_any_other_is_the_failure() {
        let token = CancelToken::new();
        let why = read_body(GivesOut(Some(&token)), &token).unwrap_err();
        assert!(why.is::<Cancelled>(), "{why:#}");

        let why = read_body(GivesOut(None), &CancelToken::new()).unwrap_err();
        assert!(!why.is::<Cancelled>(), "{why:#}");
    }

    #[test]
    fn a_download_cancelled_between_chunks_is_not_cached() {
        let (root, cache) = test_cache("cancelled-between");
        let (url, first_sent, go, thread) = halting_server(b"package ", b"bytes");
        let wanted = entry(url, Some(index::md5_hex(b"package bytes")));
        let token = CancelToken::new();
        // The cache is not shared between threads: the fetch takes it along.
        let (why, cache) = std::thread::scope(|scope| {
            let fetching = scope.spawn(|| {
                let fetched = super::package(&cache, &Client::new(), &wanted, || None, &token);
                (fetched, cache)
            });
            first_sent
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert!(token.cancel());
            go.send(()).unwrap();
            let (fetched, cache) = fetching.join().unwrap();
            (fetched.unwrap_err(), cache)
        });
        thread.join().unwrap();
        assert!(why.is::<Cancelled>(), "{why:#}");
        assert!(
            cache
                .package_sha256(&wanted.name, "1.0.0", wanted.md5.as_ref().unwrap())
                .unwrap()
                .is_none()
        );
        assert!(!root.join("content").exists());
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_bad_download_is_not_cached() {
        let (root, cache) = test_cache("bad-md5");
        let (url, thread) = server(b"wrong");
        let entry = entry(url, Some(index::md5_hex(b"expected")));
        assert!(
            package(&cache, &Client::new(), &entry)
                .unwrap_err()
                .to_string()
                .contains("MD5 mismatch")
        );
        thread.join().unwrap();
        assert!(
            cache
                .package_sha256(&entry.name, "1.0.0", entry.md5.as_ref().unwrap())
                .unwrap()
                .is_none()
        );
        assert!(!root.join("content").exists());
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn imported_content_is_reused_across_sources_and_local_files_remain_live() {
        let (root, cache) = test_cache("imported");
        let source = root.join("original.vip");
        std::fs::write(&source, b"imported bytes").unwrap();
        let metadata = std::fs::metadata(&source).unwrap();
        let mut wanted = entry(
            "http://127.0.0.1:1/unavailable".into(),
            Some(index::md5_hex(b"imported bytes")),
        );
        let sha256 = cache.store(b"imported bytes").unwrap();
        cache
            .record_import(
                &source,
                Some(&metadata),
                &LocalEntry::from(&wanted),
                &sha256,
                wanted.md5.as_ref().unwrap(),
            )
            .unwrap();
        cache
            .record_download(
                &LocalEntry::from(&wanted),
                "http://another-source/",
                &sha256,
                wanted.md5.as_ref().unwrap(),
            )
            .unwrap();
        assert_eq!(
            cache
                .package_sha256("TEST_PACKAGE", "1.0.0", wanted.md5.as_ref().unwrap())
                .unwrap(),
            Some(sha256.clone())
        );
        std::fs::remove_file(&source).unwrap();
        assert_eq!(
            package(&cache, &Client::new(), &wanted).unwrap(),
            b"imported bytes"
        );
        wanted.url = format!("cache:{sha256}");
        assert_eq!(
            package(&cache, &Client::new(), &wanted).unwrap(),
            b"imported bytes"
        );
        wanted.md5 = Some(index::md5_hex(b"different"));
        assert!(package(&cache, &Client::new(), &wanted).is_err());
        wanted.url = source.to_string_lossy().into_owned();
        wanted.md5 = None;
        std::fs::write(&source, b"current local bytes").unwrap();
        assert_eq!(
            package(&cache, &Client::new(), &wanted).unwrap(),
            b"current local bytes"
        );
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn missing_or_corrupt_cached_bytes_are_downloaded_again() {
        let (root, cache) = test_cache("repair");
        let bytes = b"package bytes";
        let sha256 = cache.store(bytes).unwrap();
        let blob = root.join("content/sha256").join(&sha256[..2]).join(&sha256);
        for corrupt in [false, true] {
            if corrupt {
                std::fs::write(&blob, b"corrupt").unwrap()
            } else {
                std::fs::remove_file(&blob).unwrap()
            }
            let (url, thread) = server(bytes);
            let entry = entry(url, Some(index::md5_hex(bytes)));
            cache
                .record_download(
                    &LocalEntry::from(&entry),
                    &entry.url,
                    &sha256,
                    entry.md5.as_ref().unwrap(),
                )
                .unwrap();
            assert_eq!(package(&cache, &Client::new(), &entry).unwrap(), bytes);
            thread.join().unwrap();
            assert_eq!(cache.load(&sha256).unwrap().unwrap(), bytes);
        }
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn ambiguity_metadata_mismatches_and_missing_md5_do_not_reuse_bytes() {
        let (root, cache) = test_cache("ambiguous");
        let bytes = b"selected package";
        let md5 = index::md5_hex(bytes);
        let wanted = entry("http://127.0.0.1:1/unavailable".into(), Some(md5.clone()));
        let first = cache.store(b"first").unwrap();
        let second = cache.store(b"second").unwrap();
        cache
            .record_download(&LocalEntry::from(&wanted), "http://first/", &first, &md5)
            .unwrap();
        cache
            .record_download(&LocalEntry::from(&wanted), "http://second/", &second, &md5)
            .unwrap();
        assert!(
            cache
                .package_sha256("test_package", "1.0.0", &md5)
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .package_sha256("other_package", "1.0.0", &md5)
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .package_sha256("test_package", "2.0.0", &md5)
                .unwrap()
                .is_none()
        );
        let (url, thread) = server(bytes);
        assert_eq!(
            package(&cache, &Client::new(), &entry(url, Some(md5))).unwrap(),
            bytes
        );
        thread.join().unwrap();
        let (url, thread) = server(bytes);
        let no_md5 = entry(url, None);
        assert_eq!(package(&cache, &Client::new(), &no_md5).unwrap(), bytes);
        thread.join().unwrap();
        assert!(package(&cache, &Client::new(), &no_md5).is_err());
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    /// An entry a community index lists at `url`, its publisher `someone/Serializer`.
    fn listed_at(url: &str) -> Entry {
        let mut listed = pinned(url.into(), b"package bytes");
        listed.source = "github-index:o/r".into();
        listed
    }

    #[test]
    fn an_index_url_is_accepted_only_under_its_publishers_releases_on_the_web_base() {
        const WEB: &str = "https://github.com";
        let under = "https://github.com/someone/Serializer/releases/download/1.0.0/pkg.lvpkg";
        assert_eq!(check_index_url(&listed_at(under), WEB), Ok(()));
        // The owner and the repo are compared ignoring ASCII case.
        let cased = "https://github.com/SOMEONE/serializer/releases/download/1.0.0/pkg.lvpkg";
        assert_eq!(check_index_url(&listed_at(cased), WEB), Ok(()));
        // A test's web base is its loopback server.
        let loopback = "http://127.0.0.1:9/someone/Serializer/releases/download/1/p.lvpkg";
        assert_eq!(
            check_index_url(&listed_at(loopback), "http://127.0.0.1:9"),
            Ok(())
        );

        let refused = [
            r"\\attacker\share\p.lvpkg",
            "//attacker/share/p.lvpkg",
            "cache:abcd",
            "C:/packages/p.lvpkg",
            "relative/p.lvpkg",
            "oci://x",
            "http://evil.example/someone/Serializer/releases/download/1.0.0/p.lvpkg",
            "https://github.com/other/Serializer/releases/download/1.0.0/p.lvpkg",
            "https://github.com/someone/Other/releases/download/1.0.0/p.lvpkg",
            "https://github.com/someone/Serializer/releases/download/",
            "https://github.com/someone/Serializer/releases/download/../../../evil/p.lvpkg",
            "https://github.com/someone/Serializer/releases/download/%2e%2e/%2e%2e/p.lvpkg",
            "https://github.com/someone/Serializer/releases/download/1.1.0/sub/foo.lvpkg",
            "https://github.com/someone/Serializer/releases/download/1.1.0%2f..%2f..%2fb/foo.lvpkg",
            "https://github.com/someone/Serializer/releases/download/1.1.0%5c..%5cfoo.lvpkg",
            "https://github.com/someone/Serializer/releases/download/1.1.0/",
            "https://github.com/someone/Serializer/releases/download/1.1.0",
            "https://github.com/someone/Serializer/releases/download//foo.lvpkg",
            "https://github.com/someone/Serializer/releases/download/1.1.0//foo.lvpkg",
            "https://github.com/someone/Serializer/releases/download/1/p.lvpkg?x=1",
            "https://github.com/someone/Serializer/releases/download/1/p.lvpkg#frag",
            "https://user@github.com/someone/Serializer/releases/download/1/p.lvpkg",
            "https://github.com@evil.example/someone/Serializer/releases/download/1/p.lvpkg",
            "https://github.com/someone/Serializer/releases/download/1/p\n.lvpkg",
            "https://github.com/someone/Serializer/releases/download/1/p\t.lvpkg",
        ];
        for url in refused {
            let why = check_index_url(&listed_at(url), WEB).expect_err(url);
            let text = why.to_string();
            assert_eq!(
                (&*why.name, &*why.version, &*why.index, &*why.publisher),
                (
                    "test_package",
                    "1.0.0",
                    "github-index:o/r",
                    "someone/Serializer"
                )
            );
            assert!(
                text.contains("test_package 1.0.0") && text.contains("fix:"),
                "{text}"
            );
            assert!(
                !text.contains("attacker") && !text.contains("evil"),
                "{text}"
            );
        }
    }

    #[test]
    fn an_entry_no_index_lists_is_never_refused_for_its_url() {
        assert_eq!(
            check_index_url(&entry("C:/p.vip".into(), None), "https://github.com"),
            Ok(())
        );
    }

    #[test]
    fn an_index_entry_is_never_read_from_the_cache_by_url_or_from_a_local_file() {
        let (root, cache) = test_cache("index-not-local");
        let file = root.join("local.lvpkg");
        std::fs::write(&file, b"package bytes").unwrap();
        let sha256 = cache.store(b"other bytes").unwrap();
        for url in [
            file.to_string_lossy().into_owned(),
            format!("cache:{sha256}"),
        ] {
            let wanted = listed_at(&url);
            assert_eq!(on_hand(&cache, &wanted).unwrap(), None, "{url}");
        }
        // What the index pins is on hand by that SHA-256, whatever its url says.
        let pinned_sha = cache.store(b"package bytes").unwrap();
        let wanted = listed_at("cache:0000");
        assert_eq!(wanted.index_pin.as_ref().unwrap().sha256, pinned_sha);
        assert_eq!(on_hand(&cache, &wanted).unwrap().unwrap(), b"package bytes");
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_failed_index_download_names_the_package_and_not_the_url() {
        let (root, cache) = test_cache("index-download-words");
        let url = "http://127.0.0.1:1/someone/Serializer/releases/download/1.0.0/p.lvpkg";
        let fetch = |entry: &Entry| {
            super::download(&cache, &Client::new(), entry, None, &CancelToken::new(), 4)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(fetch(&listed_at(url)), "downloading test_package 1.0.0");
        assert_eq!(
            fetch(&entry(url.into(), None)),
            format!("downloading {url}")
        );
        drop(cache);
        std::fs::remove_dir_all(root).ok();
    }
}
