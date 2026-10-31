//! A repo's releases, as GitHub's API lists them, and the package file in each.
//!
//! One request per page, never retried: a failure is the answer. The token goes
//! to the API host only: a `Link` to a next page elsewhere is refused.

use super::auth::GitHubAuth;
use super::release_file;
use crate::cache::{Cache, SourceState};
use crate::errors::{GitHubError, GitHubFault};
use crate::fetch::Rejected;
use crate::version::Version;
use anyhow::Result;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, HeaderMap, LINK, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

/// Releases per page: GitHub's maximum.
const PER_PAGE: usize = 100;
/// Pages read before giving up: a repo with this many releases is not one a
/// package lives in.
const MAX_PAGES: usize = 50;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// A page of releases larger than this is not one GitHub sent.
const MAX_BODY_BYTES: u64 = 32 * 1024 * 1024;
/// The API version the requests ask for.
const API_VERSION: &str = "2022-11-28";
/// The cache's `kind` for a repo's listing.
const CACHE_KIND: &str = "github";

/// The package file of one release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFile {
    /// The version the file name carries, not the tag's.
    pub version: Version,
    /// The asset's API URL: JSON about the asset, or, asked for
    /// `application/octet-stream`, a redirect to its bytes.
    pub url: String,
    pub tag: String,
    pub file: String,
}

/// What a repo's releases hold of one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// One per release that has a file for the package, in the API's order.
    pub files: Vec<ReleaseFile>,
    /// How many drafts and pre-releases were left out.
    pub skipped: usize,
    /// The releases came from the cache, not from GitHub.
    pub from_cache: bool,
}

/// A release as the API sends it, and as the cache keeps it.
#[derive(Debug, Serialize, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Asset {
    name: String,
    url: String,
}

/// List the releases of `repo` (`owner/repo`) on the API at `api`, from
/// GitHub, and pick `name`'s file in each.
pub fn releases(
    client: &Client,
    api: &str,
    repo: &str,
    name: &str,
    auth: &GitHubAuth,
) -> Result<Listing, GitHubError> {
    let all = fetch(client, api, repo, name, auth)?;
    package_files(name, repo, api, &all, false)
}

/// [`releases`], from the cache when it holds this repo's listing and
/// `refresh` is not set; a listing read from GitHub is cached for next time.
/// The error is a [`GitHubError`] when GitHub or the releases are at fault.
pub fn releases_cached(
    cache: &Cache,
    client: &Client,
    api: &str,
    repo: &str,
    name: &str,
    auth: &GitHubAuth,
    refresh: bool,
) -> Result<Listing> {
    let key = releases_url(api, repo);
    if !refresh && let Some(cached) = cached_releases(cache, &key)? {
        // A cached listing that fails (two releases hold a version) may be what
        // the user has just fixed: ask GitHub again before giving up.
        if let Ok(found) = package_files(name, repo, api, &cached, true) {
            return Ok(found);
        }
    }
    let all = fetch(client, api, repo, name, auth)?;
    let sha256 = cache.store(&serde_json::to_vec(&all)?)?;
    let state = SourceState {
        sha256: Some(sha256),
        ..SourceState::default()
    };
    cache.record(&key, CACHE_KIND, api, &state)?;
    Ok(package_files(name, repo, api, &all, false)?)
}

/// `error`, the failure of a download of a release file of `repo`, as the
/// [`GitHubError`] the user reads when a response or a failed connection is
/// what it came from; any other error (a cancel, a bad MD5) is returned as it
/// is.
pub fn download_error(
    error: anyhow::Error,
    name: &str,
    repo: &str,
    auth: &GitHubAuth,
) -> anyhow::Error {
    let fault = if let Some(rejected) = error.downcast_ref::<Rejected>() {
        failure(rejected.status, &rejected.headers, b"", auth, unix_now())
    } else if let Some(sent) = error.downcast_ref::<reqwest::Error>() {
        unreadable(without_query(format!("{error:#}"), sent))
    } else {
        return error;
    };
    github_error(name, repo, fault).into()
}

/// `text` with the query of the URL `error` failed on cut off: after a
/// redirect that URL is the CDN's, and its query is a signed grant to the file.
fn without_query(text: String, error: &reqwest::Error) -> String {
    let Some(url) = error.url().filter(|url| url.query().is_some()) else {
        return text;
    };
    let mut bare = url.clone();
    bare.set_query(None);
    text.replace(url.as_str(), bare.as_str())
}

/// The first page of `repo`'s releases on the API at `api`; also the cache key.
fn releases_url(api: &str, repo: &str) -> String {
    format!("{}/repos/{repo}/releases", api.trim_end_matches('/'))
}

/// Whether `url` is on the API at `api`: the token rides every request, so it
/// must not follow a link or an asset off it.
fn under_api(api: &str, url: &str) -> bool {
    url.starts_with(&format!("{}/", api.trim_end_matches('/')))
}

/// What the cache holds for `key`; a listing that cannot be read is a miss.
fn cached_releases(cache: &Cache, key: &str) -> Result<Option<Vec<Release>>> {
    let Some(sha256) = cache.state(key)?.and_then(|state| state.sha256) else {
        return Ok(None);
    };
    let Some(bytes) = cache.load(&sha256)? else {
        return Ok(None);
    };
    Ok(serde_json::from_slice(&bytes).ok())
}

fn github_error(name: &str, repo: &str, fault: GitHubFault) -> GitHubError {
    GitHubError {
        name: name.to_string(),
        repo: repo.to_string(),
        fault,
    }
}

/// [`fetch_all`] with its fault named for `name` and `repo`.
fn fetch(
    client: &Client,
    api: &str,
    repo: &str,
    name: &str,
    auth: &GitHubAuth,
) -> Result<Vec<Release>, GitHubError> {
    fetch_all(client, api, repo, auth).map_err(|fault| github_error(name, repo, fault))
}

/// Every release of `repo`, page after page.
fn fetch_all(
    client: &Client,
    api: &str,
    repo: &str,
    auth: &GitHubAuth,
) -> Result<Vec<Release>, GitHubFault> {
    let api = api.trim_end_matches('/');
    let mut url = format!("{}?per_page={PER_PAGE}", releases_url(api, repo));
    let mut all = Vec::new();
    for _ in 0..MAX_PAGES {
        let (body, link) = get(client, &url, auth)?;
        let page: Vec<Release> = serde_json::from_slice(&body).map_err(unreadable)?;
        all.extend(page);
        let Some(next) = link.as_deref().and_then(next_link) else {
            return Ok(all);
        };
        // The token rides every request: it must not follow a link off the API.
        if !under_api(api, next) {
            return Err(unreadable(format!(
                "a next page outside {api}: {}",
                printable(next)
            )));
        }
        url = next.to_string();
    }
    Err(unreadable(format!(
        "more than {MAX_PAGES} pages of releases"
    )))
}

/// One API call: the body, and the `Link` header.
fn get(
    client: &Client,
    url: &str,
    auth: &GitHubAuth,
) -> Result<(Vec<u8>, Option<String>), GitHubFault> {
    let mut request = client
        .get(url)
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header(USER_AGENT, concat!("lvpm/", env!("CARGO_PKG_VERSION")))
        .timeout(REQUEST_TIMEOUT);
    if let Some(token) = auth.token() {
        request = request.bearer_auth(token);
    }
    let response = request.send().map_err(|error| transport(&error))?;
    let status = response.status();
    let headers = response.headers().clone();
    let mut body = Vec::new();
    response
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| transport(&error))?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(unreadable(format!(
            "a response of more than {MAX_BODY_BYTES} bytes"
        )));
    }
    if !status.is_success() {
        return Err(failure(status, &headers, &body, auth, unix_now()));
    }
    let link = headers
        .get(LINK)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    Ok((body, link))
}

/// The fault for a status that is not a success, at `now` (Unix seconds).
/// `auth` says whether a token was sent, and from where.
fn failure(
    status: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
    auth: &GitHubAuth,
    now: i64,
) -> GitHubFault {
    let token_source = auth.source();
    let header = |name: &str| headers.get(name)?.to_str().ok();
    let none_left = header("x-ratelimit-remaining") == Some("0");
    // The reset time of the primary limit; a secondary limit says how long to
    // wait instead.
    let reset = if none_left {
        header("x-ratelimit-reset").and_then(|value| value.parse().ok())
    } else {
        header("retry-after")
            .and_then(|value| value.parse::<i64>().ok())
            .map(|wait| now.saturating_add(wait))
    };
    match (status.as_u16(), token_source) {
        (429, _) => GitHubFault::RateLimited {
            token_source,
            reset,
            now,
        },
        (403, _) if none_left || header("retry-after").is_some() => GitHubFault::RateLimited {
            token_source,
            reset,
            now,
        },
        (401, Some(token_source)) => GitHubFault::TokenRejected { token_source },
        (401 | 403, _) => GitHubFault::Refused {
            message: message_of(body).unwrap_or_else(|| status.to_string()),
        },
        (404, _) => GitHubFault::NotFound { token_source },
        (other, _) => unreadable(format!("HTTP {other}")),
    }
}

/// The `message` of GitHub's JSON error body.
fn message_of(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Body {
        message: String,
    }
    Some(printable(
        &serde_json::from_slice::<Body>(body).ok()?.message,
    ))
}

/// `text` from the server as a short line: control characters (a terminal
/// would act on them) are dropped and anything past 200 characters is cut.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(200).collect()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

/// A failure to read the releases, for `cause`.
fn unreadable(cause: impl std::fmt::Display) -> GitHubFault {
    GitHubFault::Unreadable {
        cause: cause.to_string(),
    }
}

/// [`unreadable`] for a transport error: its whole chain, since its own text
/// is only "error sending request".
fn transport(error: &dyn std::error::Error) -> GitHubFault {
    let mut cause = error.to_string();
    let mut next = error.source();
    while let Some(source) = next {
        cause.push_str(&format!(": {source}"));
        next = source.source();
    }
    unreadable(cause)
}

/// The target of the `rel="next"` entry of a `Link` header.
fn next_link(header: &str) -> Option<&str> {
    header.split(',').find_map(|entry| {
        let (target, params) = entry.split_once(';')?;
        let is_next = params
            .split(';')
            .any(|param| param.trim().eq_ignore_ascii_case("rel=\"next\""));
        is_next.then(|| target.trim().trim_start_matches('<').trim_end_matches('>'))
    })
}

/// The package files of `name` in `releases`, drafts and pre-releases left out;
/// a file whose URL is not under `api` is a failure.
fn package_files(
    name: &str,
    repo: &str,
    api: &str,
    releases: &[Release],
    from_cache: bool,
) -> Result<Listing, GitHubError> {
    let mut files: Vec<ReleaseFile> = Vec::new();
    let mut skipped = 0;
    for release in releases {
        if release.draft || release.prerelease {
            skipped += 1;
            continue;
        }
        let names: Vec<&str> = release
            .assets
            .iter()
            .map(|asset| asset.name.as_str())
            .collect();
        if let Some((at, version)) = release_file::choose(&names, name) {
            // The token goes to this URL when the file is downloaded.
            let url = &release.assets[at].url;
            if !under_api(api, url) {
                return Err(github_error(
                    name,
                    repo,
                    unreadable(format!("an asset URL outside {api}: {}", printable(url))),
                ));
            }
            files.push(ReleaseFile {
                version: Version::parse(version),
                url: url.clone(),
                tag: release.tag_name.clone(),
                file: release.assets[at].name.clone(),
            });
        }
    }
    let mut seen: BTreeMap<&Version, &str> = BTreeMap::new();
    for file in &files {
        if let Some(first_tag) = seen.insert(&file.version, &file.tag) {
            return Err(github_error(
                name,
                repo,
                GitHubFault::TwoReleases {
                    version: file.version.raw.clone(),
                    first_tag: printable(first_tag),
                    second_tag: printable(&file.tag),
                },
            ));
        }
    }
    Ok(Listing {
        files,
        skipped,
        from_cache,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::TokenSource;
    use reqwest::header::HeaderValue;

    /// A status, the headers and body that came with it, the token sent, the fault.
    type Row<'a> = (
        u16,
        &'a [(&'a str, &'a str)],
        &'a [u8],
        Option<TokenSource>,
        GitHubFault,
    );

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    /// An auth whose token, `t`, comes from `source`.
    fn with_token(source: Option<TokenSource>) -> GitHubAuth {
        match source {
            None => GitHubAuth::anonymous(),
            Some(TokenSource::GhAuthToken) => GitHubAuth::new(|_| None, || Some("t".into())),
            Some(source) => {
                let variable = source.to_string();
                GitHubAuth::new(move |var| (var == variable).then(|| "t".into()), || None)
            }
        }
    }

    #[test]
    fn a_status_maps_to_one_fault_by_what_the_headers_and_body_say() {
        let spent = [
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "1000"),
        ];
        let left = [("x-ratelimit-remaining", "12")];
        let json = br#"{"message":"Resource not accessible"}"#;
        let (gh, anon) = (Some(TokenSource::GhToken), None);
        let limited = |token_source, reset| GitHubFault::RateLimited {
            token_source,
            reset,
            now: 7,
        };
        let refused = |message: &str| GitHubFault::Refused {
            message: message.into(),
        };
        let table: Vec<Row> = vec![
            (403, &spent, b"", anon, limited(anon, Some(1000))),
            (403, &spent, b"", gh, limited(gh, Some(1000))),
            (429, &[], b"", anon, limited(anon, None)),
            (429, &spent, b"", gh, limited(gh, Some(1000))),
            (403, &left, json, anon, refused("Resource not accessible")),
            (403, &[], b"<html>", anon, refused("403 Forbidden")),
            (403, &left, json, gh, refused("Resource not accessible")),
            // A secondary limit says how long to wait, not when the limit resets.
            (
                429,
                &[("retry-after", "30"), ("x-ratelimit-remaining", "12")],
                b"",
                anon,
                limited(anon, Some(37)),
            ),
            (
                403,
                &[("retry-after", "30"), ("x-ratelimit-remaining", "12")],
                b"",
                anon,
                limited(anon, Some(37)),
            ),
            // The server's words are shown without what a terminal would act on.
            (
                403,
                &left,
                br#"{"message":"a\u001b[31mb"}"#,
                anon,
                refused("a[31mb"),
            ),
            // A reset that is not a number is no reset.
            (
                429,
                &[("x-ratelimit-reset", "soon")],
                b"",
                anon,
                limited(anon, None),
            ),
            (
                401,
                &[],
                b"",
                gh,
                GitHubFault::TokenRejected {
                    token_source: TokenSource::GhToken,
                },
            ),
            (
                401,
                &[],
                br#"{"message":"Requires authentication"}"#,
                anon,
                refused("Requires authentication"),
            ),
            (
                404,
                &[],
                b"",
                anon,
                GitHubFault::NotFound { token_source: anon },
            ),
            (
                404,
                &[],
                b"",
                gh,
                GitHubFault::NotFound { token_source: gh },
            ),
            (500, &[], b"", anon, unreadable("HTTP 500")),
            (503, &[], b"", gh, unreadable("HTTP 503")),
            (410, &[], b"", anon, unreadable("HTTP 410")),
        ];
        for (status, header_pairs, body, token, want) in table {
            let status = StatusCode::from_u16(status).unwrap();
            let got = failure(status, &headers(header_pairs), body, &with_token(token), 7);
            assert_eq!(got, want, "{status} with {header_pairs:?}");
        }
    }

    #[test]
    fn a_url_is_under_the_api_only_at_its_host_and_scheme_and_port() {
        for url in ["http://api/repos/o/r/releases?page=2", "http://api/x"] {
            assert!(under_api("http://api", url), "{url}");
            assert!(under_api("http://api/", url), "{url}");
        }
        for url in [
            "http://api",
            "http://api.evil/x",
            "https://api/x",
            "http://api:1/x",
            "/x",
        ] {
            assert!(!under_api("http://api", url), "{url}");
        }
    }

    #[test]
    fn a_cached_listing_that_cannot_be_read_is_a_miss() {
        let root = std::env::temp_dir().join(format!("lvpm-github-cache-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let cache = Cache::open(&root).unwrap();
        let key = releases_url("http://api", "o/r");
        let record = |sha256: Option<String>| {
            let state = SourceState {
                sha256,
                ..SourceState::default()
            };
            cache
                .record(&key, CACHE_KIND, "http://api", &state)
                .unwrap();
        };
        let miss = || cached_releases(&cache, &key).unwrap().is_none();

        assert!(miss(), "nothing recorded");
        record(Some("0".repeat(64)));
        assert!(miss(), "a digest whose blob is gone");
        record(Some(cache.store(b"<html>").unwrap()));
        assert!(miss(), "a blob that is not JSON");
        record(Some(cache.store(br#"{"not":"a list"}"#).unwrap()));
        assert!(miss(), "JSON of another shape");
        record(Some(cache.store(b"[]").unwrap()));
        assert!(!miss(), "an empty list is a listing");
    }

    #[test]
    fn the_next_page_is_the_link_marked_next() {
        let header = r#"<https://api.github.com/r?page=2>; rel="next", <https://api.github.com/r?page=9>; rel="last""#;
        assert_eq!(next_link(header), Some("https://api.github.com/r?page=2"));
        let last_only = r#"<https://api.github.com/r?page=1>; rel="prev", <https://api.github.com/r?page=9>; rel="last""#;
        assert_eq!(next_link(last_only), None);
        assert_eq!(next_link(""), None);
        assert_eq!(next_link("garbage"), None);
    }

    #[test]
    fn a_failed_download_never_shows_the_signed_query_of_the_url_it_failed_on() {
        // Nothing listens on port 1: the send fails, naming the URL it tried.
        let failed = Client::new()
            .get("http://127.0.0.1:1/file.vip?sig=SIGNED")
            .send()
            .unwrap_err();
        let shown = download_error(failed.into(), "pkg", "o/r", &GitHubAuth::anonymous());
        let text = shown.to_string();
        assert!(
            text.starts_with("pkg (github.com/o/r): cannot read the releases"),
            "{text}"
        );
        assert!(text.contains("127.0.0.1:1/file.vip"), "{text}");
        assert!(!text.contains("SIGNED"), "{text}");
    }
}
