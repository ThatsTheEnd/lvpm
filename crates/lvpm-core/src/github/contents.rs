//! One file of a repo, as the contents API serves it raw: what a community
//! index is read through, one `packages/<key>.toml` at a time.
//!
//! The answers are cached by URL, a 404 too: asking again is `--refresh`'s job.

use super::api::get_accepting;
use super::auth::GitHubAuth;
use crate::cache::{Cache, SourceState};
use crate::errors::GitHubFault;
use anyhow::{Result, anyhow};
use reqwest::blocking::Client;

/// A file larger than this is not an index entry: 1 MiB.
pub const MAX_ENTRY_BYTES: u64 = 1024 * 1024;
const RAW: &str = "application/vnd.github.raw";
/// The cache's `kind` for a file of a repo.
const CACHE_KIND: &str = "github-file";
/// What a cached 404 records as the resolved URL, in place of a body's digest.
const NOT_LISTED: &str = "not listed";

/// The text of `path` in `repo` (`owner/repo`) on the API at `api`: `None` when
/// the repo has no such file (a 404), an error for any other failure, and for a
/// body over `limit` bytes. From the cache when it holds an answer and
/// `refresh` is not set; an answer read from GitHub is cached for next time.
#[allow(clippy::too_many_arguments)]
pub fn contents_cached(
    cache: &Cache,
    client: &Client,
    api: &str,
    repo: &str,
    path: &str,
    auth: &GitHubAuth,
    refresh: bool,
    limit: u64,
) -> Result<Option<String>> {
    let url = format!("{}/repos/{repo}/contents/{path}", api.trim_end_matches('/'));
    if !refresh && let Some(answer) = cached(cache, &url)? {
        return Ok(answer);
    }
    let answer = match get_accepting(client, &url, auth, RAW, limit) {
        Ok((body, _)) => Some(String::from_utf8(body).map_err(|_| anyhow!("{path} is not text"))?),
        Err(GitHubFault::NotFound { .. }) => None,
        // The fault's own words about "the releases" do not fit a file.
        Err(GitHubFault::Unreadable { cause }) => return Err(anyhow!(cause)),
        Err(fault) => return Err(fault.into()),
    };
    let state = match &answer {
        Some(text) => SourceState {
            sha256: Some(cache.store(text.as_bytes())?),
            ..SourceState::default()
        },
        None => SourceState {
            resolved_url: Some(NOT_LISTED.to_string()),
            ..SourceState::default()
        },
    };
    cache.record(&url, CACHE_KIND, api, &state)?;
    Ok(answer)
}

/// What the cache holds for `url`: `Some(None)` is a recorded 404. A body that
/// cannot be read back is a miss.
fn cached(cache: &Cache, url: &str) -> Result<Option<Option<String>>> {
    let Some(state) = cache.state(url)? else {
        return Ok(None);
    };
    let Some(sha256) = state.sha256 else {
        return Ok((state.resolved_url.as_deref() == Some(NOT_LISTED)).then_some(None));
    };
    let body = cache.load(&sha256)?;
    Ok(body
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(Some))
}
