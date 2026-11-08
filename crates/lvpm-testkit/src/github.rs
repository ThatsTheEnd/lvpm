//! A loopback HTTP server standing in for GitHub's releases API.
//!
//! It serves `GET /repos/{owner}/{repo}` (the repo itself, when the caller may
//! see it), `GET /repos/{owner}/{repo}/releases` (paged by `per_page`, with a
//! `Link: rel="next"`), each release's draft and pre-release flags, and its
//! assets, and `GET /repos/{owner}/{repo}/contents/{path}` for the files a test
//! registers with [`GitHubFake::add_file`] (raw, for `Accept:
//! application/vnd.github.raw`; any other file is a 404). An asset's API URL answers JSON, unless the request asks for
//! `Accept: application/octet-stream`: then it redirects to
//! `http://localhost:{port}/download/{id}`, the same server under another host
//! *name*, so a client's redirect policy sees a cross-host hop. `/download/{id}`
//! serves the bytes. `GET /{owner}/{repo}/releases/download/{tag}/{file}` serves
//! a release file the way the web site does, for a public repo.
//! `GET /orgs/{org}/public_members/{login}` answers 204 for a member a test
//! registers with [`GitHubFake::add_public_member`] and 404 for anyone else.
//!
//! Each failure GitHub can answer with is a mode a test switches on, and every
//! request is logged, so a test can say what was asked and with which token.

use crate::http::{Request, accept, accept_loop, response, shut_down};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// The one token a private repo accepts.
pub const PRIVATE_TOKEN: &str = "s3cret-token";
/// The host name an asset's download redirect points at.
pub const DOWNLOAD_HOST: &str = "localhost";
const DEFAULT_PER_PAGE: usize = 30;
const MAX_PER_PAGE: usize = 100;
const OCTET_STREAM: &str = "application/octet-stream";
const RAW: &str = "application/vnd.github.raw";
const FIRST_ASSET_ID: u64 = 1000;

/// A release as a test describes it: ids are the fake's to give.
#[derive(Debug, Clone)]
pub struct FakeRelease {
    tag: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<(String, Vec<u8>)>,
}

impl FakeRelease {
    pub fn new(tag: &str) -> FakeRelease {
        FakeRelease {
            tag: tag.to_string(),
            draft: false,
            prerelease: false,
            assets: Vec::new(),
        }
    }

    /// Attach a file called `name` holding `bytes`.
    pub fn asset(mut self, name: &str, bytes: &[u8]) -> FakeRelease {
        self.assets.push((name.to_string(), bytes.to_vec()));
        self
    }

    pub fn draft(mut self) -> FakeRelease {
        self.draft = true;
        self
    }

    pub fn prerelease(mut self) -> FakeRelease {
        self.prerelease = true;
        self
    }
}

/// One request the fake received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedRequest {
    /// The path, without its query.
    pub path: String,
    pub host: Option<String>,
    pub accept: Option<String>,
    pub authorization: Option<String>,
}

#[derive(Debug)]
struct Asset {
    id: u64,
    name: String,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct Release {
    tag: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Debug, Default)]
struct Repo {
    /// Only [`PRIVATE_TOKEN`] sees it.
    private: bool,
    releases: Vec<Release>,
    /// The files `contents` serves, by path in the repo.
    files: BTreeMap<String, Vec<u8>>,
    /// The SHA-256 (hex) of each file the attestations API lists one for.
    attested: BTreeSet<String>,
}

/// What every API request (not a download) answers instead of its own reply.
#[derive(Debug, Clone)]
enum Fault {
    /// `status` is 403 or 429; the limit resets at `reset`, Unix seconds.
    RateLimit {
        status: u16,
        reset: u64,
    },
    /// A 403 that is not a rate limit: requests remain.
    Forbidden {
        message: String,
    },
    Server {
        status: u16,
    },
    /// A 200 whose body is a web page, as a captive portal's is.
    NotJson,
}

impl Fault {
    fn reply(&self) -> Vec<u8> {
        match self {
            Fault::RateLimit { status, reset } => json_reply(
                &status_line(*status),
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset.to_string()),
                ],
                &message("API rate limit exceeded"),
            ),
            Fault::Forbidden { message } => json_reply(
                "403 Forbidden",
                &[("x-ratelimit-remaining", "4999")],
                &self::message(message),
            ),
            Fault::Server { status } => {
                json_reply(&status_line(*status), &[], &message("Server Error"))
            }
            Fault::NotJson => response(
                "200 OK",
                &[("Content-Type", "text/html")],
                b"<html>sign in to the network</html>",
            ),
        }
    }
}

fn status_line(status: u16) -> String {
    let reason = match status {
        403 => "Forbidden",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    };
    format!("{status} {reason}")
}

#[derive(Debug, Default)]
struct State {
    fault: Option<Fault>,
    /// The fault is answered to requests with no `Authorization` only.
    fault_for_anonymous_only: bool,
    repos: BTreeMap<String, Repo>,
    /// The public members of each organisation, by login (lower case).
    orgs: BTreeMap<String, BTreeSet<String>>,
    assets_issued: u64,
    log: Vec<LoggedRequest>,
}

impl State {
    fn add_release(&mut self, repo: &str, release: FakeRelease) {
        let assets = release
            .assets
            .into_iter()
            .map(|(name, bytes)| Asset {
                id: self.take_id(),
                name,
                bytes,
            })
            .collect();
        self.repos
            .entry(repo.to_string())
            .or_default()
            .releases
            .push(Release {
                tag: release.tag,
                draft: release.draft,
                prerelease: release.prerelease,
                assets,
            });
    }

    fn take_id(&mut self) -> u64 {
        let id = FIRST_ASSET_ID + self.assets_issued;
        self.assets_issued += 1;
        id
    }
}

#[derive(Debug)]
pub struct GitHubFake {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl GitHubFake {
    /// Start serving `repos` (`"owner/repo"` and its releases, newest first) on
    /// an ephemeral loopback port.
    pub fn start(repos: &[(&str, Vec<FakeRelease>)]) -> GitHubFake {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("bound address");
        let mut state = State::default();
        for (repo, releases) in repos {
            state.repos.entry(repo.to_string()).or_default();
            for release in releases {
                state.add_release(repo, release.clone());
            }
        }
        let state = Arc::new(Mutex::new(state));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (state, stop) = (state.clone(), stop.clone());
            std::thread::spawn(move || {
                accept_loop(listener, &stop, |stream| {
                    // One bad connection must not take the server down for the rest.
                    handle(stream, &state, addr).ok();
                })
            })
        };
        GitHubFake {
            addr,
            state,
            stop,
            thread: Some(thread),
        }
    }

    /// From now on `repo` answers only a caller with [`PRIVATE_TOKEN`]: 404
    /// without a token, 401 with another.
    pub fn make_private(&self, repo: &str) {
        let mut state = lock(&self.state);
        state.repos.entry(repo.to_string()).or_default().private = true;
    }

    /// Serve `body` as the file `path` of `repo`, replacing what it held.
    pub fn add_file(&self, repo: &str, path: &str, body: &str) {
        let mut state = lock(&self.state);
        let repo = state.repos.entry(repo.to_string()).or_default();
        repo.files
            .insert(path.to_string(), body.as_bytes().to_vec());
    }

    /// Stop serving the file `path` of `repo`: a 404 from now on.
    pub fn remove_file(&self, repo: &str, path: &str) {
        let mut state = lock(&self.state);
        let removed = state
            .repos
            .entry(repo.to_string())
            .or_default()
            .files
            .remove(path);
        assert!(removed.is_some(), "{repo} serves no file {path}");
    }

    /// From now on `GET /repos/{repo}/attestations/sha256:{sha256}` lists an
    /// attestation for that file, as GitHub does for one a workflow attested;
    /// any other digest is a 404, which GitHub answers for an unattested file.
    pub fn add_attestation(&self, repo: &str, sha256: &str) {
        let mut state = lock(&self.state);
        let repo = state.repos.entry(repo.to_string()).or_default();
        repo.attested.insert(sha256.to_ascii_lowercase());
    }

    /// From now on the attestations API lists none for the file with this
    /// digest in `repo`: undoes [`add_attestation`](GitHubFake::add_attestation).
    pub fn remove_attestation(&self, repo: &str, sha256: &str) {
        let mut state = lock(&self.state);
        let repo = state.repos.entry(repo.to_string()).or_default();
        assert!(
            repo.attested.remove(&sha256.to_ascii_lowercase()),
            "{sha256} has no attestation"
        );
    }

    /// From now on `GET /orgs/{org}/public_members/{login}` answers 204: `login`
    /// is a public member of the organisation `org`, as GitHub says it of one
    /// whose membership is public. Any other login is a 404, which is also what a
    /// user account that is no organisation answers.
    pub fn add_public_member(&self, org: &str, login: &str) {
        let mut state = lock(&self.state);
        let members = state.orgs.entry(org.to_ascii_lowercase()).or_default();
        members.insert(login.to_ascii_lowercase());
    }

    /// Whether the attestations API lists one for the file with this digest in
    /// `repo`.
    pub fn lists_attestation(&self, repo: &str, sha256: &str) -> bool {
        let state = lock(&self.state);
        let repo = state.repos.get(repo);
        repo.is_some_and(|repo| repo.attested.contains(&sha256.to_ascii_lowercase()))
    }

    /// Add `release` to `repo`, after the releases it has (newest first).
    pub fn add_release(&self, repo: &str, release: FakeRelease) {
        lock(&self.state).add_release(repo, release);
    }

    /// Remove the file `name` from every release of `repo`: its download is a
    /// 404 from now on, and the releases stay.
    pub fn remove_asset(&self, repo: &str, name: &str) {
        let mut state = lock(&self.state);
        let releases = &mut state.repos.get_mut(repo).expect("a known repo").releases;
        let before: usize = releases.iter().map(|r| r.assets.len()).sum();
        releases
            .iter_mut()
            .for_each(|release| release.assets.retain(|asset| asset.name != name));
        let after: usize = releases.iter().map(|r| r.assets.len()).sum();
        assert!(after < before, "{repo} has no file {name}");
    }

    /// Replace the bytes of the file `name` in `repo`, keeping its asset id,
    /// as a re-upload over the same asset does.
    pub fn replace_bytes(&self, repo: &str, name: &str, bytes: &[u8]) {
        let mut state = lock(&self.state);
        asset_named(&mut state, repo, name).bytes = bytes.to_vec();
    }

    /// Replace the file `name` in `repo` by one holding `bytes` under a new
    /// asset id, as `gh release upload --clobber` does. The old id is gone.
    pub fn replace_with_new_id(&self, repo: &str, name: &str, bytes: &[u8]) {
        let mut state = lock(&self.state);
        let id = state.take_id();
        let asset = asset_named(&mut state, repo, name);
        asset.id = id;
        asset.bytes = bytes.to_vec();
    }

    /// Remove the release tagged `tag` from `repo`, assets and all.
    pub fn delete_release(&self, repo: &str, tag: &str) {
        let mut state = lock(&self.state);
        let releases = &mut state.repos.get_mut(repo).expect("a known repo").releases;
        let before = releases.len();
        releases.retain(|release| release.tag != tag);
        assert!(releases.len() < before, "{repo} has no release {tag}");
    }

    /// From now on every API request answers `status` (403 or 429) with no
    /// requests remaining until `reset`, in Unix seconds. Downloads are served.
    pub fn limit_rate(&self, status: u16, reset: u64) {
        self.fail(Fault::RateLimit { status, reset });
    }

    /// From now on every API request answers 403 with `message`, though
    /// requests remain.
    pub fn forbid(&self, message: &str) {
        self.fail(Fault::Forbidden {
            message: message.to_string(),
        });
    }

    /// From now on every API request answers `status`, a 5xx.
    pub fn fail_with(&self, status: u16) {
        self.fail(Fault::Server { status });
    }

    /// From now on every API request answers 200 with a body that is not JSON.
    pub fn answer_not_json(&self) {
        self.fail(Fault::NotJson);
    }

    /// [`limit_rate`](GitHubFake::limit_rate), for requests that carry no
    /// `Authorization`: a caller with a token is served, as GitHub's limits
    /// are per caller and a token's is larger.
    pub fn limit_rate_for_anonymous(&self, status: u16, reset: u64) {
        let mut state = lock(&self.state);
        state.fault = Some(Fault::RateLimit { status, reset });
        state.fault_for_anonymous_only = true;
    }

    fn fail(&self, fault: Fault) {
        let mut state = lock(&self.state);
        state.fault = Some(fault);
        state.fault_for_anonymous_only = false;
    }

    /// The value for `GITHUB_API_URL`: `http://127.0.0.1:port`.
    pub fn api_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Every request received since the start or the last [`clear_requests`],
    /// oldest first.
    ///
    /// [`clear_requests`]: GitHubFake::clear_requests
    pub fn requests(&self) -> Vec<LoggedRequest> {
        lock(&self.state).log.clone()
    }

    pub fn clear_requests(&self) {
        lock(&self.state).log.clear();
    }
}

/// The state, even after a test thread panicked holding it: the server must
/// keep answering, so the test's own failure is the one that shows.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn asset_named<'a>(state: &'a mut State, repo: &str, name: &str) -> &'a mut Asset {
    state
        .repos
        .get_mut(repo)
        .expect("a known repo")
        .releases
        .iter_mut()
        .flat_map(|release| &mut release.assets)
        .find(|asset| asset.name == name)
        .unwrap_or_else(|| panic!("{repo} has no file {name}"))
}

impl Drop for GitHubFake {
    fn drop(&mut self) {
        shut_down(self.addr, &self.stop, self.thread.take());
    }
}

fn handle(mut stream: TcpStream, state: &Mutex<State>, addr: SocketAddr) -> std::io::Result<()> {
    let request = accept(&mut stream)?;
    let (path, query) = split_query(&request.path);
    let mut state = lock(state);
    state.log.push(LoggedRequest {
        path: path.to_string(),
        host: request.header("host").map(str::to_string),
        accept: request.header("accept").map(str::to_string),
        authorization: request.header("authorization").map(str::to_string),
    });
    let reply = answer(&state, &request, path, &query, addr);
    drop(state);
    stream.write_all(&reply)
}

fn split_query(target: &str) -> (&str, HashMap<&str, &str>) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let pairs = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .collect();
    (path, pairs)
}

fn answer(
    state: &State,
    request: &Request,
    path: &str,
    query: &HashMap<&str, &str>,
    addr: SocketAddr,
) -> Vec<u8> {
    if request.method != "GET" {
        return json_reply(
            "405 Method Not Allowed",
            &[],
            &message("Method Not Allowed"),
        );
    }
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segments.as_slice() {
        ["repos", owner, repo, rest @ ..] => {
            let anonymous = request.header("authorization").is_none();
            if let Some(fault) = &state.fault
                && (anonymous || !state.fault_for_anonymous_only)
            {
                return fault.reply();
            }
            let slug = format!("{owner}/{repo}");
            let found = match visible(state, &slug, request) {
                Ok(found) => found,
                Err(refusal) => return refusal,
            };
            match rest {
                [] => json_reply("200 OK", &[], &json!({ "full_name": slug })),
                ["releases"] => listing(found, &slug, query, addr),
                ["releases", "assets", id] => match asset_in(found, id) {
                    Some(asset) => asset_reply(asset, &slug, request.header("accept"), addr),
                    None => not_found(),
                },
                ["contents", path @ ..] => file_reply(found, &path.join("/"), request),
                ["attestations", subject] => attestations_reply(found, subject),
                _ => not_found(),
            }
        }
        ["orgs", org, "public_members", login] => {
            if let Some(fault) = &state.fault
                && (request.header("authorization").is_none() || !state.fault_for_anonymous_only)
            {
                return fault.reply();
            }
            let member = state
                .orgs
                .get(&org.to_ascii_lowercase())
                .is_some_and(|members| members.contains(&login.to_ascii_lowercase()));
            match member {
                true => response("204 No Content", &[], b""),
                false => not_found(),
            }
        }
        [owner, repo, "releases", "download", tag, file] => {
            // The web site's address for a release file: public, no token read.
            let slug = format!("{owner}/{repo}");
            let asset = state
                .repos
                .get(&slug)
                .filter(|repo| !repo.private)
                .and_then(|repo| repo.releases.iter().find(|release| release.tag == *tag))
                .and_then(|release| release.assets.iter().find(|asset| asset.name == *file));
            match asset {
                Some(asset) => response("200 OK", &[("Content-Type", OCTET_STREAM)], &asset.bytes),
                None => not_found(),
            }
        }
        ["download", id] => {
            let found = state.repos.values().find_map(|repo| asset_in(repo, id));
            match found {
                Some(asset) => response("200 OK", &[("Content-Type", OCTET_STREAM)], &asset.bytes),
                None => not_found(),
            }
        }
        _ => not_found(),
    }
}

/// The repo, when this request may see it; else the answer GitHub gives.
/// A private repo is a 404 to a caller with no token and a 401 to one with
/// the wrong token, as GitHub answers both.
fn visible<'a>(state: &'a State, slug: &str, request: &Request) -> Result<&'a Repo, Vec<u8>> {
    let repo = state.repos.get(slug).ok_or_else(not_found)?;
    if !repo.private {
        return Ok(repo);
    }
    match request.header("authorization") {
        None => Err(not_found()),
        Some(value) if value == format!("Bearer {PRIVATE_TOKEN}") => Ok(repo),
        Some(_) => Err(json_reply(
            "401 Unauthorized",
            &[],
            &message("Bad credentials"),
        )),
    }
}

/// The file at `path`, raw as the client asks for it; JSON about it is not
/// something the fake can say.
fn file_reply(repo: &Repo, path: &str, request: &Request) -> Vec<u8> {
    let Some(body) = repo.files.get(path) else {
        return not_found();
    };
    match request.header("accept") {
        Some(RAW) => response("200 OK", &[("Content-Type", RAW)], body),
        _ => json_reply(
            "415 Unsupported Media Type",
            &[],
            &message("ask for application/vnd.github.raw"),
        ),
    }
}

/// The attestations of the file with this digest: one when it was added, else
/// the 404 of a file nothing attested. What the bundle holds is gh's to read.
fn attestations_reply(repo: &Repo, subject: &str) -> Vec<u8> {
    let attested = subject
        .strip_prefix("sha256:")
        .is_some_and(|sha| repo.attested.contains(&sha.to_ascii_lowercase()));
    match attested {
        true => json_reply(
            "200 OK",
            &[],
            &json!({ "attestations": [{ "bundle": {} }] }),
        ),
        false => not_found(),
    }
}

fn asset_in<'a>(repo: &'a Repo, id: &str) -> Option<&'a Asset> {
    let id: u64 = id.parse().ok()?;
    repo.releases
        .iter()
        .flat_map(|release| &release.assets)
        .find(|asset| asset.id == id)
}

/// JSON about the asset, or, for a client that asks for the bytes, the
/// redirect GitHub gives to its CDN.
fn asset_reply(asset: &Asset, slug: &str, accept: Option<&str>, addr: SocketAddr) -> Vec<u8> {
    if accept == Some(OCTET_STREAM) {
        let location = format!(
            "http://{DOWNLOAD_HOST}:{}/download/{}",
            addr.port(),
            asset.id
        );
        return response("302 Found", &[("Location", &location)], b"");
    }
    json_reply("200 OK", &[], &asset_json(asset, slug, addr))
}

fn asset_json(asset: &Asset, slug: &str, addr: SocketAddr) -> Value {
    json!({
        "id": asset.id,
        "name": asset.name,
        "size": asset.bytes.len(),
        "url": format!("http://{addr}/repos/{slug}/releases/assets/{}", asset.id),
    })
}

fn listing(repo: &Repo, slug: &str, query: &HashMap<&str, &str>, addr: SocketAddr) -> Vec<u8> {
    let per_page = number(query, "per_page")
        .unwrap_or(DEFAULT_PER_PAGE)
        .clamp(1, MAX_PER_PAGE);
    let page = number(query, "page").unwrap_or(1).max(1);
    let first = (page - 1) * per_page;
    let releases: Vec<Value> = repo
        .releases
        .iter()
        .skip(first)
        .take(per_page)
        .map(|release| release_json(release, slug, addr))
        .collect();
    let link = format!(
        "<http://{addr}/repos/{slug}/releases?per_page={per_page}&page={}>; rel=\"next\"",
        page + 1
    );
    let more = first + per_page < repo.releases.len();
    let headers: &[(&str, &str)] = if more { &[("Link", &link)] } else { &[] };
    json_reply("200 OK", headers, &Value::Array(releases))
}

fn number(query: &HashMap<&str, &str>, key: &str) -> Option<usize> {
    query.get(key)?.parse().ok()
}

fn release_json(release: &Release, slug: &str, addr: SocketAddr) -> Value {
    let assets: Vec<Value> = release
        .assets
        .iter()
        .map(|asset| asset_json(asset, slug, addr))
        .collect();
    json!({
        "tag_name": release.tag,
        "draft": release.draft,
        "prerelease": release.prerelease,
        "assets": assets,
    })
}

fn not_found() -> Vec<u8> {
    json_reply("404 Not Found", &[], &message("Not Found"))
}

/// The body of GitHub's JSON errors.
fn message(text: &str) -> Value {
    json!({ "message": text })
}

fn json_reply(status: &str, headers: &[(&str, &str)], body: &Value) -> Vec<u8> {
    let mut all = vec![("Content-Type", "application/json; charset=utf-8")];
    all.extend_from_slice(headers);
    response(status, &all, body.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Reply, SOCKET_TIMEOUT, get_from};

    fn json_of(reply: &Reply) -> Value {
        serde_json::from_slice(&reply.body).expect("a JSON body")
    }

    fn pkg_release(tag: &str) -> FakeRelease {
        FakeRelease::new(tag).asset(&format!("pkg-{tag}.lvpkg"), b"bytes")
    }

    #[test]
    fn a_public_member_is_a_204_and_anyone_else_a_404() {
        let fake = GitHubFake::start(&[]);
        fake.add_public_member("Acme", "Alice");
        let status = |path: &str| get_from(fake.addr, path, "").status_line;

        assert_eq!(
            status("/orgs/acme/public_members/alice"),
            "HTTP/1.1 204 No Content"
        );
        assert_eq!(
            status("/orgs/ACME/public_members/ALICE"),
            "HTTP/1.1 204 No Content"
        );
        assert_eq!(
            status("/orgs/acme/public_members/bob"),
            "HTTP/1.1 404 Not Found"
        );
        assert_eq!(
            status("/orgs/other/public_members/alice"),
            "HTTP/1.1 404 Not Found"
        );
        fake.fail_with(502);
        assert_eq!(
            status("/orgs/acme/public_members/alice"),
            "HTTP/1.1 502 Bad Gateway"
        );
    }

    #[test]
    fn a_listing_round_trips_tags_flags_and_assets() {
        let fake = GitHubFake::start(&[(
            "o/r",
            vec![
                pkg_release("1.0"),
                FakeRelease::new("2.0-rc").prerelease(),
                FakeRelease::new("3.0").draft(),
            ],
        )]);
        let reply = get_from(fake.addr, "/repos/o/r/releases?per_page=100", "");
        assert_eq!(reply.status_line, "HTTP/1.1 200 OK");
        let releases = json_of(&reply);
        let tags: Vec<_> = releases
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["tag_name"].as_str().unwrap(),
                    r["draft"].as_bool().unwrap(),
                    r["prerelease"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            tags,
            [
                ("1.0", false, false),
                ("2.0-rc", false, true),
                ("3.0", true, false)
            ]
        );
        let asset = &releases[0]["assets"][0];
        assert_eq!(asset["name"], "pkg-1.0.lvpkg");
        assert_eq!(
            asset["url"],
            format!(
                "http://{}/repos/o/r/releases/assets/{}",
                fake.addr, asset["id"]
            )
        );
    }

    #[test]
    fn a_listing_is_paged_with_a_link_to_the_next_page_until_the_last() {
        let releases = (0..150)
            .map(|n| FakeRelease::new(&format!("v{n}")))
            .collect();
        let fake = GitHubFake::start(&[("o/r", releases)]);
        let first = get_from(fake.addr, "/repos/o/r/releases?per_page=100", "");
        assert_eq!(json_of(&first).as_array().unwrap().len(), 100);
        let link = first
            .head
            .lines()
            .find_map(|line| line.strip_prefix("Link: "))
            .expect("a Link header");
        let next = link
            .strip_prefix(&format!("<http://{}", fake.addr))
            .and_then(|rest| rest.strip_suffix(">; rel=\"next\""))
            .expect("a next link on this server");
        let second = get_from(fake.addr, next, "");
        let rest = json_of(&second);
        assert_eq!(rest.as_array().unwrap().len(), 50);
        assert_eq!(rest[0]["tag_name"], "v100");
        assert!(!second.head.contains("Link:"), "{}", second.head);
    }

    #[test]
    fn an_asset_url_answers_json_unless_the_bytes_are_asked_for_then_redirects_to_localhost() {
        let fake = GitHubFake::start(&[("o/r", vec![pkg_release("1.0")])]);
        let listed = json_of(&get_from(fake.addr, "/repos/o/r/releases", ""));
        let url = listed[0]["assets"][0]["url"].as_str().unwrap().to_string();
        let target = url.strip_prefix(&format!("http://{}", fake.addr)).unwrap();

        let meta = get_from(fake.addr, target, "Accept: application/vnd.github+json\r\n");
        assert_eq!(meta.status_line, "HTTP/1.1 200 OK");
        assert_eq!(json_of(&meta)["name"], "pkg-1.0.lvpkg");

        let bytes = get_from(fake.addr, target, "Accept: application/octet-stream\r\n");
        assert_eq!(bytes.status_line, "HTTP/1.1 302 Found");
        let id = listed[0]["assets"][0]["id"].as_u64().unwrap();
        let location = format!(
            "Location: http://{DOWNLOAD_HOST}:{}/download/{id}",
            fake.addr.port()
        );
        assert!(bytes.head.contains(&location), "{}", bytes.head);

        let download = get_from(fake.addr, &format!("/download/{id}"), "");
        assert_eq!(download.body, b"bytes");
    }

    #[test]
    fn a_release_file_is_served_at_its_web_address_and_only_when_public() {
        let fake = GitHubFake::start(&[("o/r", vec![pkg_release("1.0")])]);
        let url = "/o/r/releases/download/1.0/pkg-1.0.lvpkg";
        let served = get_from(fake.addr, url, "");
        assert_eq!(served.status_line, "HTTP/1.1 200 OK");
        assert_eq!(served.body, b"bytes");
        for missing in [
            "/o/r/releases/download/2.0/pkg-1.0.lvpkg",
            "/o/r/releases/download/1.0/other.lvpkg",
            "/x/y/releases/download/1.0/pkg-1.0.lvpkg",
        ] {
            let reply = get_from(fake.addr, missing, "");
            assert_eq!(reply.status_line, "HTTP/1.1 404 Not Found", "{missing}");
        }
        fake.make_private("o/r");
        assert_eq!(
            get_from(fake.addr, url, "").status_line,
            "HTTP/1.1 404 Not Found"
        );
    }

    #[test]
    fn an_attestation_is_listed_for_the_digests_added_and_404_for_any_other() {
        let fake = GitHubFake::start(&[("o/r", vec![])]);
        let sha = "ab".repeat(32);
        fake.add_attestation("o/r", &sha.to_ascii_uppercase());
        let asked = |repo: &str, digest: &str| {
            let path = format!("/repos/{repo}/attestations/{digest}");
            get_from(fake.addr, &path, "").status_line
        };
        assert_eq!(asked("o/r", &format!("sha256:{sha}")), "HTTP/1.1 200 OK");
        let other = format!("sha256:{}", "cd".repeat(32));
        assert_eq!(asked("o/r", &other), "HTTP/1.1 404 Not Found");
        assert_eq!(asked("o/r", &sha), "HTTP/1.1 404 Not Found");
        assert_eq!(
            asked("x/y", &format!("sha256:{sha}")),
            "HTTP/1.1 404 Not Found"
        );
        let listed = get_from(
            fake.addr,
            &format!("/repos/o/r/attestations/sha256:{sha}"),
            "",
        );
        assert_eq!(json_of(&listed)["attestations"][0]["bundle"], json!({}));
    }

    #[test]
    fn a_removed_attestation_is_a_404_again() {
        let fake = GitHubFake::start(&[("o/r", vec![])]);
        let sha = "ab".repeat(32);
        let path = format!("/repos/o/r/attestations/sha256:{sha}");
        fake.add_attestation("o/r", &sha);
        assert_eq!(
            get_from(fake.addr, &path, "").status_line,
            "HTTP/1.1 200 OK"
        );

        fake.remove_attestation("o/r", &sha.to_ascii_uppercase());

        assert_eq!(
            get_from(fake.addr, &path, "").status_line,
            "HTTP/1.1 404 Not Found"
        );
    }

    #[test]
    fn a_file_is_served_raw_a_file_not_registered_is_404_and_each_request_is_recorded() {
        let fake = GitHubFake::start(&[("o/r", vec![])]);
        fake.add_file("o/r", "packages/foo.toml", "name = \"foo\"\n");
        let raw = "Accept: application/vnd.github.raw\r\n";

        let served = get_from(fake.addr, "/repos/o/r/contents/packages/foo.toml", raw);
        assert_eq!(served.status_line, "HTTP/1.1 200 OK");
        assert_eq!(served.body, b"name = \"foo\"\n");
        let other = get_from(fake.addr, "/repos/o/r/contents/packages/bar.toml", raw);
        assert_eq!(other.status_line, "HTTP/1.1 404 Not Found");
        let json = get_from(fake.addr, "/repos/o/r/contents/packages/foo.toml", "");
        assert_eq!(json.status_line, "HTTP/1.1 415 Unsupported Media Type");

        let asked: Vec<_> = fake.requests().into_iter().map(|r| r.path).collect();
        assert_eq!(
            asked,
            [
                "/repos/o/r/contents/packages/foo.toml",
                "/repos/o/r/contents/packages/bar.toml",
                "/repos/o/r/contents/packages/foo.toml",
            ]
        );
    }

    #[test]
    fn a_removed_file_is_404() {
        let fake = GitHubFake::start(&[("o/r", vec![])]);
        fake.add_file("o/r", "packages/foo.toml", "x");
        fake.remove_file("o/r", "packages/foo.toml");
        let raw = "Accept: application/vnd.github.raw\r\n";
        let gone = get_from(fake.addr, "/repos/o/r/contents/packages/foo.toml", raw);
        assert_eq!(gone.status_line, "HTTP/1.1 404 Not Found");
    }

    #[test]
    fn a_failing_mode_answers_a_file_request_with_its_status() {
        let fake = GitHubFake::start(&[("o/r", vec![])]);
        fake.add_file("o/r", "packages/foo.toml", "x");
        fake.fail_with(500);
        let raw = "Accept: application/vnd.github.raw\r\n";
        let reply = get_from(fake.addr, "/repos/o/r/contents/packages/foo.toml", raw);
        assert_eq!(reply.status_line, "HTTP/1.1 500 Internal Server Error");
    }

    #[test]
    fn a_private_repo_is_404_without_a_token_401_with_a_wrong_one_and_served_with_the_right_one() {
        let fake = GitHubFake::start(&[("o/private", vec![pkg_release("1.0")])]);
        fake.make_private("o/private");
        let target = "/repos/o/private/releases";
        assert_eq!(
            get_from(fake.addr, target, "").status_line,
            "HTTP/1.1 404 Not Found"
        );
        let wrong = get_from(fake.addr, target, "Authorization: Bearer nope\r\n");
        assert_eq!(wrong.status_line, "HTTP/1.1 401 Unauthorized");
        assert_eq!(json_of(&wrong)["message"], "Bad credentials");
        let right = get_from(
            fake.addr,
            target,
            &format!("Authorization: Bearer {PRIVATE_TOKEN}\r\n"),
        );
        assert_eq!(right.status_line, "HTTP/1.1 200 OK");
        assert_eq!(json_of(&right)[0]["tag_name"], "1.0");
    }

    #[test]
    fn a_limit_for_anonymous_callers_leaves_a_caller_with_a_token_alone_until_another_mode_replaces_it()
     {
        let fake = GitHubFake::start(&[("o/r", vec![pkg_release("1.0")])]);
        let listing = "/repos/o/r/releases";
        fake.limit_rate_for_anonymous(403, 4_070_908_800);
        assert_eq!(
            get_from(fake.addr, listing, "").status_line,
            "HTTP/1.1 403 Forbidden"
        );
        let with_token = get_from(fake.addr, listing, "Authorization: Bearer any\r\n");
        assert_eq!(with_token.status_line, "HTTP/1.1 200 OK");

        fake.limit_rate(403, 4_070_908_800);
        let with_token = get_from(fake.addr, listing, "Authorization: Bearer any\r\n");
        assert_eq!(with_token.status_line, "HTTP/1.1 403 Forbidden");
    }

    #[test]
    fn each_failure_mode_answers_every_api_request_and_leaves_downloads_alone() {
        let fake = GitHubFake::start(&[("o/r", vec![pkg_release("1.0")])]);
        let listing = "/repos/o/r/releases";
        let reset = 4_070_908_800;

        fake.limit_rate(403, reset);
        let limited = get_from(fake.addr, listing, "");
        assert_eq!(limited.status_line, "HTTP/1.1 403 Forbidden");
        assert!(
            limited.head.contains("x-ratelimit-remaining: 0"),
            "{}",
            limited.head
        );
        assert!(
            limited.head.contains("x-ratelimit-reset: 4070908800"),
            "{}",
            limited.head
        );

        fake.limit_rate(429, reset);
        assert_eq!(
            get_from(fake.addr, listing, "").status_line,
            "HTTP/1.1 429 Too Many Requests"
        );

        fake.forbid("Resource not accessible");
        let forbidden = get_from(fake.addr, listing, "");
        assert_eq!(forbidden.status_line, "HTTP/1.1 403 Forbidden");
        assert!(
            forbidden.head.contains("x-ratelimit-remaining: 4999"),
            "{}",
            forbidden.head
        );
        assert_eq!(json_of(&forbidden)["message"], "Resource not accessible");

        fake.fail_with(502);
        assert_eq!(
            get_from(fake.addr, listing, "").status_line,
            "HTTP/1.1 502 Bad Gateway"
        );

        fake.answer_not_json();
        let page = get_from(fake.addr, listing, "");
        assert_eq!(page.status_line, "HTTP/1.1 200 OK");
        assert!(serde_json::from_slice::<Value>(&page.body).is_err());

        let id = FIRST_ASSET_ID;
        assert_eq!(
            get_from(fake.addr, &format!("/download/{id}"), "").body,
            b"bytes"
        );
    }

    fn first_asset(fake: &GitHubFake) -> (u64, String) {
        let listed = json_of(&get_from(fake.addr, "/repos/o/r/releases", ""));
        let asset = &listed[0]["assets"][0];
        (
            asset["id"].as_u64().unwrap(),
            asset["name"].as_str().unwrap().to_string(),
        )
    }

    fn download(fake: &GitHubFake, id: u64) -> Reply {
        get_from(fake.addr, &format!("/download/{id}"), "")
    }

    #[test]
    fn replaced_bytes_keep_the_asset_id() {
        let fake = GitHubFake::start(&[("o/r", vec![pkg_release("1.0")])]);
        let (id, name) = first_asset(&fake);
        fake.replace_bytes("o/r", &name, b"other");
        assert_eq!(first_asset(&fake).0, id);
        assert_eq!(download(&fake, id).body, b"other");
    }

    #[test]
    fn bytes_replaced_under_a_new_id_leave_the_old_id_gone() {
        let fake = GitHubFake::start(&[("o/r", vec![pkg_release("1.0")])]);
        let (id, name) = first_asset(&fake);
        fake.replace_with_new_id("o/r", &name, b"other");
        let (new_id, _) = first_asset(&fake);
        assert_ne!(new_id, id);
        assert_eq!(download(&fake, new_id).body, b"other");
        assert_eq!(download(&fake, id).status_line, "HTTP/1.1 404 Not Found");
    }

    #[test]
    fn a_deleted_release_leaves_the_listing_and_its_assets_are_gone() {
        let fake = GitHubFake::start(&[("o/r", vec![pkg_release("1.0"), pkg_release("0.9")])]);
        let (id, _) = first_asset(&fake);
        fake.delete_release("o/r", "1.0");
        let listed = json_of(&get_from(fake.addr, "/repos/o/r/releases", ""));
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["tag_name"], "0.9");
        assert_eq!(download(&fake, id).status_line, "HTTP/1.1 404 Not Found");
    }

    #[test]
    fn a_removed_asset_is_a_404_for_its_file_and_its_release_stays() {
        let fake = GitHubFake::start(&[("o/r", vec![pkg_release("1.0"), pkg_release("0.9")])]);
        let file = "/o/r/releases/download/1.0/pkg-1.0.lvpkg";
        let other = "/o/r/releases/download/0.9/pkg-0.9.lvpkg";
        assert_eq!(get_from(fake.addr, file, "").status_line, "HTTP/1.1 200 OK");

        fake.remove_asset("o/r", "pkg-1.0.lvpkg");

        assert_eq!(
            get_from(fake.addr, file, "").status_line,
            "HTTP/1.1 404 Not Found"
        );
        assert_eq!(
            get_from(fake.addr, other, "").status_line,
            "HTTP/1.1 200 OK"
        );
        let listed = json_of(&get_from(fake.addr, "/repos/o/r/releases", ""));
        assert_eq!(listed.as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_release_added_later_is_listed_last() {
        let fake = GitHubFake::start(&[("o/r", vec![pkg_release("1.0")])]);
        fake.add_release("o/r", pkg_release("1.1"));
        let listed = json_of(&get_from(fake.addr, "/repos/o/r/releases", ""));
        assert_eq!(listed[1]["tag_name"], "1.1");
    }

    #[test]
    fn an_unknown_repo_is_404_with_a_json_message() {
        let fake = GitHubFake::start(&[]);
        let reply = get_from(fake.addr, "/repos/o/none/releases", "");
        assert_eq!(reply.status_line, "HTTP/1.1 404 Not Found");
        assert_eq!(json_of(&reply)["message"], "Not Found");
    }

    #[test]
    fn requests_are_logged_with_path_host_accept_and_authorization_until_cleared() {
        let fake = GitHubFake::start(&[("o/r", vec![])]);
        get_from(
            fake.addr,
            "/repos/o/r/releases?per_page=100",
            "Accept: application/vnd.github+json\r\nAuthorization: Bearer t\r\n",
        );
        assert_eq!(
            fake.requests(),
            [LoggedRequest {
                path: "/repos/o/r/releases".into(),
                host: Some("test".into()),
                accept: Some("application/vnd.github+json".into()),
                authorization: Some("Bearer t".into()),
            }]
        );
        fake.clear_requests();
        assert!(fake.requests().is_empty());
    }

    #[test]
    fn dropping_the_fake_stops_its_thread() {
        let fake = GitHubFake::start(&[]);
        let addr = fake.addr;
        drop(fake);
        assert!(TcpStream::connect_timeout(&addr, SOCKET_TIMEOUT).is_err());
    }
}
