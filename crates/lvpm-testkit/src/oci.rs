//! A loopback HTTP server standing in for an OCI registry.
//!
//! It serves the subset of the distribution API lvpm uses: a bearer challenge
//! with a `/token` realm, the tags list, manifest GET/HEAD/PUT by tag and by
//! digest, blob HEAD/GET, and the monolithic blob upload (POST, then PUT with
//! `?digest=`). Repositories live in memory.
//!
//! Every request is logged (method, path, `Authorization`), so a test can say
//! what was asked and what was written. Each failure a registry can show is
//! something a test switches on: `go_down`, a status for one method and path,
//! a tag that appears while a package is uploading.
//!
//! Referrers: a manifest PUT with a `subject` is listed by the referrers API
//! (`/v2/<name>/referrers/<digest>`, with `artifactType` filtering) and
//! answered with `OCI-Subject`, unless [`Registry::offer_referrers_api`] turned
//! the API off: then the route is a `404` and no `OCI-Subject` is sent, as with a
//! registry that predates it, and a client keeps the list under the tag
//! `sha256-<hex>` itself.
//!
//! The `/token` realm gives its one token to anyone, unless a login is
//! required for a repository prefix ([`Registry::require_login`]): then Basic
//! auth, or a refresh-token grant, is asked for, per repository and access, and
//! a user can be refused. What it was asked is [`Registry::token_requests`].

use crate::http::{Request, accept, accept_loop, read_body, response, shut_down};
use base64::Engine as _;
use lvpm_core::oci;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// The one token the fake accepts; the `/token` realm gives it to anyone.
pub const TOKEN: &str = "fake-registry-token";
const SERVICE: &str = "fake-registry";
/// What a manifest PUT without a `Content-Type` is taken to be.
const IMAGE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";

/// `sha256:<hex>` of `bytes`, as an OCI digest.
pub fn digest_of(bytes: &[u8]) -> String {
    let hex: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sha256:{hex}")
}

/// The digests of a package [`Registry::seed_package`] stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seeded {
    /// Its OCI manifest, which a lock records.
    pub manifest: String,
    /// Its one layer, the archive's sha256.
    pub layer: String,
}

/// One request the fake received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedRequest {
    pub method: String,
    /// The target as sent, query included.
    pub path: String,
    pub authorization: Option<String>,
}

impl LoggedRequest {
    /// Whether it changes what the registry holds.
    pub fn is_write(&self) -> bool {
        !matches!(self.method.as_str(), "GET" | "HEAD") && !self.path.starts_with("/token")
    }
}

/// One ask of the token realm: a GET with its query, or a POST with a form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRequest {
    pub method: String,
    /// The `Authorization` header as sent.
    pub authorization: Option<String>,
    /// The query of a GET, or the form of a POST: `service`, `scope`, and for
    /// a refresh-token grant `grant_type` and `refresh_token`.
    pub form: BTreeMap<String, String>,
}

impl TokenRequest {
    /// The user name and password of its Basic login.
    pub fn basic_login(&self) -> Option<(String, String)> {
        let sent = self.authorization.as_deref()?.strip_prefix("Basic ")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(sent)
            .ok()?;
        let text = String::from_utf8(bytes).ok()?;
        let (user, secret) = text.split_once(':')?;
        Some((user.to_string(), secret.to_string()))
    }

    /// The refresh token of a `refresh_token` grant.
    pub fn refresh_token(&self) -> Option<&str> {
        (self.form.get("grant_type")? == "refresh_token")
            .then(|| self.form.get("refresh_token").map(String::as_str))?
    }

    /// The scope asked for, as `repository:<name>:<actions>`.
    pub fn scope(&self) -> &str {
        self.form.get("scope").map_or("", String::as_str)
    }

    /// Who logged in: the Basic user name, or `<refresh-token>`.
    fn login_user(&self) -> Option<String> {
        let basic = self.basic_login().map(|(user, _)| user);
        basic.or_else(|| self.refresh_token().map(|_| REFRESH_USER.to_string()))
    }
}

/// What a login by refresh token counts as the user of.
const REFRESH_USER: &str = "<refresh-token>";

#[derive(Debug, Default)]
struct Repo {
    blobs: HashMap<String, Vec<u8>>,
    manifests: HashMap<String, Stored>,
    tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct Stored {
    media_type: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Default)]
struct State {
    repos: BTreeMap<String, Repo>,
    log: Vec<LoggedRequest>,
    down: bool,
    /// Connections a registry that is down accepted and closed unanswered.
    turned_away: usize,
    /// The registry goes down once this many blobs have been uploaded.
    down_after_blobs: Option<usize>,
    blobs_uploaded: usize,
    /// Answered, after the login, in place of the request's own reply.
    statuses: Vec<(String, String, u16)>,
    /// A tag created the moment the first blob is uploaded: (repository, tag).
    tag_after_first_blob: Option<(String, String)>,
    uploads_started: usize,
    /// The host a blob upload's `Location` names, in place of the registry's own.
    upload_host: Option<String>,
    /// A repository the registry does not hold answers `403 DENIED`, as a
    /// registry does that hides what a login cannot read.
    deny_missing: bool,
    /// After the login, a request under a repository prefix answers a status:
    /// (prefix, status, only for a write).
    refused_under: Vec<(String, u16, bool)>,
    /// A token for a repository under the prefix needs a login: (prefix, only
    /// for a token that may push).
    login_required: Vec<(String, bool)>,
    /// A login by this user name is refused at the token realm.
    refused_users: Vec<String>,
    token_requests: Vec<TokenRequest>,
    /// Set by `offer_referrers_api(false)`.
    referrers_api_off: bool,
    /// Repositories that refuse a manifest with a `subject` and any `sha256-`
    /// tag: the writes of a referrer.
    refused_referrer_writes: Vec<String>,
}

#[derive(Debug)]
pub struct Registry {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Registry {
    /// Start serving an empty registry on an ephemeral loopback port.
    pub fn start() -> Registry {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("bound address");
        let state = Arc::new(Mutex::new(State::default()));
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
        Registry {
            addr,
            state,
            stop,
            thread: Some(thread),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `127.0.0.1:port`, as an `oci://` URL names the registry.
    pub fn host(&self) -> String {
        self.addr.to_string()
    }

    /// From now on a connection is accepted and closed unanswered, as a
    /// registry that went away does. Nothing it was asked is logged, but the
    /// connections are counted: [`Registry::turned_away`].
    pub fn go_down(&self) {
        lock(&self.state).down = true;
    }

    pub fn come_back(&self) {
        let mut state = lock(&self.state);
        state.down = false;
        state.down_after_blobs = None;
    }

    /// The registry goes down once `count` blobs have been uploaded in all.
    pub fn go_down_after_blob_uploads(&self, count: usize) {
        lock(&self.state).down_after_blobs = Some(count);
    }

    /// From now on `method` on `path` (no query) answers `status`, a failure,
    /// after the login has passed.
    pub fn answer(&self, method: &str, path: &str, status: u16) {
        lock(&self.state)
            .statuses
            .push((method.to_string(), path.to_string(), status));
    }

    /// From now on a repository the registry does not hold answers `403
    /// DENIED` instead of `404`, as ghcr.io does for what a login cannot see.
    pub fn deny_missing(&self) {
        lock(&self.state).deny_missing = true;
    }

    /// From now on, after the login, any request for a repository under
    /// `prefix` (`prefix/...`) answers `status`, as a registry does that has
    /// started to ask for a login the client does not have.
    pub fn refuse_under(&self, prefix: &str, status: u16) {
        lock(&self.state)
            .refused_under
            .push((prefix.to_string(), status, false));
    }

    /// Like [`Registry::refuse_under`], for a write only: a registry that lets
    /// anyone read and lets no one push.
    pub fn refuse_writes_under(&self, prefix: &str, status: u16) {
        lock(&self.state)
            .refused_under
            .push((prefix.to_string(), status, true));
    }

    /// From now on the token realm gives no token for a repository under
    /// `prefix` (`prefix/...`) without a login.
    pub fn require_login(&self, prefix: &str) {
        lock(&self.state)
            .login_required
            .push((prefix.to_string(), false));
    }

    /// Like [`Registry::require_login`], for a token that may push.
    pub fn require_login_to_write(&self, prefix: &str) {
        lock(&self.state)
            .login_required
            .push((prefix.to_string(), true));
    }

    /// From now on a login by `user` is refused at the token realm, `401`.
    pub fn refuse_user(&self, user: &str) {
        lock(&self.state).refused_users.push(user.to_string());
    }

    /// Every ask of the token realm since the start or the last
    /// [`clear_requests`](Registry::clear_requests), oldest first.
    pub fn token_requests(&self) -> Vec<TokenRequest> {
        lock(&self.state).token_requests.clone()
    }

    /// Whether the registry serves the referrers API (it does until this says
    /// it does not). Without it, `/referrers/` is a `404` and a manifest with a
    /// `subject` is answered with no `OCI-Subject`.
    pub fn offer_referrers_api(&self, offered: bool) {
        lock(&self.state).referrers_api_off = !offered;
    }

    /// From now on a write of a referrer to `repo` (a manifest with a `subject`,
    /// or a `sha256-<hex>` tag) answers `403`, as a registry does that lets
    /// a login push packages and nothing else.
    pub fn refuse_referrer_writes(&self, repo: &str) {
        lock(&self.state)
            .refused_referrer_writes
            .push(repo.to_string());
    }

    /// `tag` appears in `repo` when the first blob is uploaded, as another
    /// publisher's would.
    pub fn create_tag_after_first_blob(&self, repo: &str, tag: &str) {
        lock(&self.state).tag_after_first_blob = Some((repo.to_string(), tag.to_string()));
    }

    /// From now on a blob upload's `Location` is on `host`, not the registry.
    pub fn upload_location_host(&self, host: &str) {
        lock(&self.state).upload_host = Some(host.to_string());
    }

    /// From now on the manifest or blob of `repo` that has this digest holds
    /// `bytes`, which do not hash to it.
    pub fn corrupt(&self, repo: &str, digest: &str, bytes: &[u8]) {
        let mut state = lock(&self.state);
        let repo = state.repos.get_mut(repo).expect("a known repository");
        match (repo.blobs.get_mut(digest), repo.manifests.get_mut(digest)) {
            (Some(blob), _) => *blob = bytes.to_vec(),
            (None, Some(manifest)) => manifest.bytes = bytes.to_vec(),
            (None, None) => panic!("{digest} is not in the repository"),
        }
    }

    /// Put `bytes` in `repo` as the manifest of `tag`; its digest.
    pub fn seed_manifest(&self, repo: &str, tag: &str, bytes: &[u8]) -> String {
        let mut state = lock(&self.state);
        let stored = Stored {
            media_type: IMAGE_MANIFEST.to_string(),
            bytes: bytes.to_vec(),
        };
        store_tag(
            state.repos.entry(repo.to_string()).or_default(),
            tag,
            stored,
        )
    }

    /// Put `bytes` in `repo` as a blob; its digest.
    pub fn seed_blob(&self, repo: &str, bytes: &[u8]) -> String {
        let digest = digest_of(bytes);
        let mut state = lock(&self.state);
        let repo = state.repos.entry(repo.to_string()).or_default();
        repo.blobs.insert(digest.clone(), bytes.to_vec());
        digest
    }

    /// An lvpm package as `lvpm publish` writes it: `layer` as the one blob,
    /// an empty config, `annotations`, tagged `tag`. What it is made of.
    pub fn seed_package(
        &self,
        repo: &str,
        tag: &str,
        annotations: &[(&str, &str)],
        layer: &[u8],
    ) -> Seeded {
        let config = self.seed_blob(repo, oci::EMPTY_CONFIG);
        let layer_digest = self.seed_blob(repo, layer);
        let manifest = json!({
            "schemaVersion": 2,
            "mediaType": oci::MANIFEST_TYPE,
            "artifactType": oci::ARTIFACT_TYPE,
            "config": {
                "mediaType": oci::EMPTY_CONFIG_TYPE,
                "digest": config,
                "size": oci::EMPTY_CONFIG.len(),
            },
            "layers": [{
                "mediaType": oci::LAYER_TYPE,
                "digest": layer_digest,
                "size": layer.len(),
            }],
            "annotations": annotations.iter().cloned().collect::<BTreeMap<_, _>>(),
        });
        let manifest = self.seed_manifest(repo, tag, manifest.to_string().as_bytes());
        Seeded {
            manifest,
            layer: layer_digest,
        }
    }

    /// A referrer of `subject` in `repo`: `payload` as its one layer, an empty
    /// config and `artifact_type`; its digest. With the referrers API off it is
    /// listed in the fallback index too, as the client that attached it would
    /// have done.
    pub fn seed_referrer(
        &self,
        repo: &str,
        subject: &str,
        artifact_type: &str,
        payload: &[u8],
    ) -> String {
        let config = self.seed_blob(repo, oci::EMPTY_CONFIG);
        let layer = self.seed_blob(repo, payload);
        let manifest = json!({
            "schemaVersion": 2,
            "mediaType": oci::MANIFEST_TYPE,
            "artifactType": artifact_type,
            "config": oci::descriptor(oci::EMPTY_CONFIG_TYPE, &config, oci::EMPTY_CONFIG.len()),
            "layers": [oci::descriptor(artifact_type, &layer, payload.len())],
            "subject": { "mediaType": oci::MANIFEST_TYPE, "digest": subject, "size": 0 },
        })
        .to_string()
        .into_bytes();
        let digest = digest_of(&manifest);
        let mut state = lock(&self.state);
        let off = state.referrers_api_off;
        let repo = state.repos.entry(repo.to_string()).or_default();
        let stored = Stored {
            media_type: IMAGE_MANIFEST.to_string(),
            bytes: manifest,
        };
        repo.manifests.insert(digest.clone(), stored);
        if off {
            let mut listed = fallback_manifests(repo, subject);
            listed.push(descriptor_of(&digest, repo));
            let stored = Stored {
                media_type: oci::INDEX_TYPE.to_string(),
                bytes: fallback_index(&listed),
            };
            store_tag(repo, &fallback_tag(subject), stored);
        }
        digest
    }

    /// What a client sees as referrers of `subject` in `repo`, of any
    /// artifact type: `(digest, artifact type)`. From the referrers API, or
    /// from the fallback index when the API is off.
    pub fn referrers(&self, repo: &str, subject: &str) -> Vec<(String, String)> {
        let state = lock(&self.state);
        let Some(repo) = state.repos.get(repo) else {
            return Vec::new();
        };
        let listed = match state.referrers_api_off {
            true => fallback_manifests(repo, subject),
            false => referrer_descriptors(repo, subject),
        };
        let text = |d: &Value, key: &str| d[key].as_str().unwrap_or_default().to_string();
        let all = listed.iter();
        all.map(|d| (text(d, "digest"), text(d, "artifactType")))
            .collect()
    }

    /// A container image under `tag`, which is no lvpm package.
    pub fn seed_image(&self, repo: &str, tag: &str) -> String {
        let config = self.seed_blob(repo, b"{}");
        let manifest = json!({
            "schemaVersion": 2,
            "mediaType": oci::MANIFEST_TYPE,
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": config,
                "size": 2,
            },
            "layers": [],
        });
        self.seed_manifest(repo, tag, manifest.to_string().as_bytes())
    }

    /// The manifest of `repo` with this digest is gone, and the tags that
    /// pointed at it.
    pub fn delete_manifest(&self, repo: &str, digest: &str) {
        let mut state = lock(&self.state);
        let repo = state.repos.get_mut(repo).expect("a known repository");
        repo.manifests.remove(digest);
        repo.tags.retain(|_, tagged| tagged != digest);
    }

    /// The digest `tag` of `repo` points at.
    pub fn tag_digest(&self, repo: &str, tag: &str) -> Option<String> {
        lock(&self.state).repos.get(repo)?.tags.get(tag).cloned()
    }

    /// The manifest of `repo` with this digest.
    pub fn manifest(&self, repo: &str, digest: &str) -> Option<Vec<u8>> {
        let state = lock(&self.state);
        Some(state.repos.get(repo)?.manifests.get(digest)?.bytes.clone())
    }

    /// The blob of `repo` with this digest.
    pub fn blob(&self, repo: &str, digest: &str) -> Option<Vec<u8>> {
        lock(&self.state)
            .repos
            .get(repo)?
            .blobs
            .get(digest)
            .cloned()
    }

    /// Every request received since the start or the last [`clear_requests`],
    /// oldest first.
    ///
    /// [`clear_requests`]: Registry::clear_requests
    pub fn requests(&self) -> Vec<LoggedRequest> {
        lock(&self.state).log.clone()
    }

    /// The requests that changed, or tried to change, what it holds. A write
    /// turned away at the door for want of a login is not one.
    pub fn writes(&self) -> Vec<LoggedRequest> {
        let mut all = self.requests();
        all.retain(|request| request.is_write() && request.authorization.is_some());
        all
    }

    pub fn clear_requests(&self) {
        let mut state = lock(&self.state);
        state.log.clear();
        state.token_requests.clear();
        state.turned_away = 0;
    }

    /// How many connections the registry accepted and closed unanswered while
    /// it was down: a client that tried it, whatever it made of the silence.
    pub fn turned_away(&self) -> usize {
        lock(&self.state).turned_away
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        shut_down(self.addr, &self.stop, self.thread.take());
    }
}

/// The state, even after a test thread panicked holding it: the server must
/// keep answering, so the test's own failure is the one that shows.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn store_tag(repo: &mut Repo, tag: &str, stored: Stored) -> String {
    let digest = digest_of(&stored.bytes);
    repo.manifests.insert(digest.clone(), stored);
    repo.tags.insert(tag.to_string(), digest.clone());
    digest
}

fn handle(mut stream: TcpStream, state: &Mutex<State>, addr: SocketAddr) -> std::io::Result<()> {
    {
        let mut state = lock(state);
        if state.down {
            state.turned_away += 1;
            return Ok(());
        }
    }
    let request = accept(&mut stream)?;
    let body = read_body(&mut stream, &request)?;
    let mut state = lock(state);
    state.log.push(LoggedRequest {
        method: request.method.clone(),
        path: request.path.clone(),
        authorization: request.header("authorization").map(str::to_string),
    });
    let mut reply = answer(&mut state, &request, &body, addr);
    drop(state);
    if request.method == "HEAD" {
        reply = head_only(reply);
    }
    stream.write_all(&reply)
}

/// A HEAD answer is the GET's head: the length stays, the body goes.
fn head_only(reply: Vec<u8>) -> Vec<u8> {
    let end = reply
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map_or(reply.len(), |at| at + 4);
    reply[..end].to_vec()
}

/// What a path asks for.
enum Route<'a> {
    Token,
    Tags(&'a str),
    Manifest(&'a str, &'a str),
    Referrers(&'a str, &'a str),
    Blob(&'a str, &'a str),
    /// Where a blob upload starts (`""`) or goes on (its id).
    Upload(&'a str, &'a str),
    Other,
}

fn route(path: &str) -> Route<'_> {
    if path == "/token" {
        return Route::Token;
    }
    let Some(rest) = path.strip_prefix("/v2/") else {
        return Route::Other;
    };
    if let Some(name) = rest.strip_suffix("/tags/list") {
        Route::Tags(name)
    } else if let Some((name, id)) = rest.split_once("/blobs/uploads/") {
        Route::Upload(name, id)
    } else if let Some((name, digest)) = rest.split_once("/blobs/") {
        Route::Blob(name, digest)
    } else if let Some((name, digest)) = rest.split_once("/referrers/") {
        Route::Referrers(name, digest)
    } else if let Some((name, reference)) = rest.split_once("/manifests/") {
        Route::Manifest(name, reference)
    } else {
        Route::Other
    }
}

fn answer(state: &mut State, request: &Request, body: &[u8], addr: SocketAddr) -> Vec<u8> {
    let (path, query) = request.path.split_once('?').unwrap_or((&request.path, ""));
    let route = route(path);
    let wanted = state.statuses.iter().find_map(|(method, on, status)| {
        (method == &request.method && on == path).then_some(*status)
    });
    if matches!(route, Route::Token) {
        return match wanted {
            Some(status) => error_reply(status, "UNAVAILABLE", "answered on purpose"),
            None => token(state, request, query, body),
        };
    }
    let name = match &route {
        Route::Tags(name)
        | Route::Manifest(name, _)
        | Route::Referrers(name, _)
        | Route::Blob(name, _)
        | Route::Upload(name, _) => *name,
        Route::Token | Route::Other => return not_found("UNSUPPORTED", "no such route"),
    };
    if request.header("authorization") != Some(&format!("Bearer {TOKEN}")) {
        return challenge(name, &request.method, addr);
    }
    if let Some(status) = wanted {
        return error_reply(status, "UNAVAILABLE", "answered on purpose");
    }
    let write = !matches!(request.method.as_str(), "GET" | "HEAD");
    let refused = state
        .refused_under
        .iter()
        .find_map(|(prefix, status, writes_only)| {
            (is_below(name, prefix) && (write || !writes_only)).then_some(*status)
        });
    if let Some(status) = refused {
        return error_reply(
            status,
            "DENIED",
            "requested access to the resource is denied",
        );
    }
    let read = matches!(request.method.as_str(), "GET" | "HEAD");
    if state.deny_missing && read && !state.repos.contains_key(name) {
        return error_reply(403, "DENIED", "requested access to the resource is denied");
    }
    match route {
        Route::Tags(name) => tags(state, name),
        Route::Manifest(name, reference) => match request.method.as_str() {
            "GET" | "HEAD" => get_manifest(state, name, reference),
            "PUT" => put_manifest(state, request, name, reference, body),
            _ => method_not_allowed(),
        },
        Route::Referrers(name, digest) => {
            match (request.method.as_str(), state.referrers_api_off) {
                ("GET", false) => referrers(state, name, digest, query),
                ("GET", true) => not_found("UNSUPPORTED", "no such route"),
                _ => method_not_allowed(),
            }
        }
        Route::Blob(name, digest) => match request.method.as_str() {
            "GET" | "HEAD" => get_blob(state, name, digest),
            _ => method_not_allowed(),
        },
        Route::Upload(name, id) => match request.method.as_str() {
            "POST" => start_upload(state, name),
            "PUT" => finish_upload(state, name, id, query, body),
            _ => method_not_allowed(),
        },
        Route::Token | Route::Other => unreachable!("answered above"),
    }
}

/// Whether repository `name` is under `prefix`: `prefix/...`.
fn is_below(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// The token realm's answer: the one token, unless the scope is for a
/// repository that needs a login the ask does not carry, or the login is by a
/// refused user.
fn token(state: &mut State, request: &Request, query: &str, body: &[u8]) -> Vec<u8> {
    let form = match request.method.as_str() {
        "POST" => form_pairs(&String::from_utf8_lossy(body)),
        _ => form_pairs(query),
    };
    let asked = TokenRequest {
        method: request.method.clone(),
        authorization: request.header("authorization").map(str::to_string),
        form,
    };
    let user = asked.login_user();
    let scope = asked.scope().to_string();
    state.token_requests.push(asked);
    let wanted = scope.strip_prefix("repository:").unwrap_or_default();
    let (name, actions) = wanted.rsplit_once(':').unwrap_or_default();
    let may_push = actions.split(',').any(|action| action == "push");
    let refused = user
        .as_ref()
        .is_some_and(|u| state.refused_users.contains(u));
    let needs_login = (state.login_required.iter())
        .any(|(prefix, push_only)| is_below(name, prefix) && (may_push || !push_only));
    if refused || (needs_login && user.is_none()) {
        return error_reply(401, "UNAUTHORIZED", "authentication required");
    }
    json_reply("200 OK", &[], &json!({ "token": TOKEN }))
}

/// `a=b&c=d`, each value with its `+` and `%XX` decoded.
fn form_pairs(text: &str) -> BTreeMap<String, String> {
    let decoded = |part: &str| percent_decoded(&part.replace('+', " "));
    let pairs = text.split('&').filter_map(|pair| pair.split_once('='));
    pairs
        .map(|(key, value)| (decoded(key), decoded(value)))
        .collect()
}

/// The 401 a registry gives a request with no valid token: where to get one.
fn challenge(name: &str, method: &str, addr: SocketAddr) -> Vec<u8> {
    let actions = match method {
        "GET" | "HEAD" => "pull",
        _ => "pull,push",
    };
    let header = format!(
        "Bearer realm=\"http://{addr}/token\",service=\"{SERVICE}\",scope=\"repository:{name}:{actions}\""
    );
    let body = errors("UNAUTHORIZED", "authentication required");
    json_reply("401 Unauthorized", &[("WWW-Authenticate", &header)], &body)
}

fn tags(state: &State, name: &str) -> Vec<u8> {
    match state.repos.get(name) {
        Some(repo) => {
            let tags: Vec<&String> = repo.tags.keys().collect();
            json_reply("200 OK", &[], &json!({ "name": name, "tags": tags }))
        }
        None => not_found("NAME_UNKNOWN", "repository name not known to registry"),
    }
}

fn get_manifest(state: &State, name: &str, reference: &str) -> Vec<u8> {
    let Some(repo) = state.repos.get(name) else {
        return not_found("NAME_UNKNOWN", "repository name not known to registry");
    };
    let digest = repo.tags.get(reference).map_or(reference, String::as_str);
    match repo.manifests.get(digest) {
        Some(stored) => response(
            "200 OK",
            &[
                ("Content-Type", &stored.media_type),
                ("Docker-Content-Digest", digest),
            ],
            &stored.bytes,
        ),
        None => not_found("MANIFEST_UNKNOWN", "manifest unknown"),
    }
}

fn put_manifest(
    state: &mut State,
    request: &Request,
    name: &str,
    reference: &str,
    body: &[u8],
) -> Vec<u8> {
    let digest = digest_of(body);
    if reference.starts_with("sha256:") && reference != digest {
        return error_reply(400, "DIGEST_INVALID", "the digest does not match the body");
    }
    let subject = subject_of(body);
    let referrer_write = subject.is_some() || reference.starts_with("sha256-");
    if referrer_write && state.refused_referrer_writes.iter().any(|r| r == name) {
        return error_reply(403, "DENIED", "requested access to the resource is denied");
    }
    let indexed = subject.is_some() && !state.referrers_api_off;
    let repo = state.repos.entry(name.to_string()).or_default();
    if let Some(missing) = unknown_blob(repo, body) {
        return error_reply(
            400,
            "MANIFEST_BLOB_UNKNOWN",
            &format!("blob unknown to registry: {missing}"),
        );
    }
    let stored = Stored {
        media_type: request
            .header("content-type")
            .unwrap_or(IMAGE_MANIFEST)
            .to_string(),
        bytes: body.to_vec(),
    };
    match reference.starts_with("sha256:") {
        true => drop(repo.manifests.insert(digest.clone(), stored)),
        false => drop(store_tag(repo, reference, stored)),
    }
    let location = format!("/v2/{name}/manifests/{digest}");
    let mut headers = vec![
        ("Docker-Content-Digest", digest.as_str()),
        ("Location", &location),
    ];
    if let (true, Some(subject)) = (indexed, subject.as_deref()) {
        headers.push(("OCI-Subject", subject));
    }
    response("201 Created", &headers, b"")
}

/// The digest a manifest names as its `subject`.
fn subject_of(manifest: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(manifest).ok()?;
    Some(json["subject"]["digest"].as_str()?.to_string())
}

/// `sha256-<hex>`: where a client keeps the referrers of a registry that has
/// no API.
fn fallback_tag(subject: &str) -> String {
    subject.replacen(':', "-", 1)
}

/// The descriptor of the stored manifest `digest`, as an index lists it.
fn descriptor_of(digest: &str, repo: &Repo) -> Value {
    let stored = &repo.manifests[digest];
    let json: Value = serde_json::from_slice(&stored.bytes).unwrap_or_default();
    // A registry gives a manifest with no `artifactType` its config's type.
    let artifact_type = (json["artifactType"].as_str())
        .or_else(|| json["config"]["mediaType"].as_str())
        .unwrap_or_default();
    json!({
        "mediaType": stored.media_type,
        "digest": digest,
        "size": stored.bytes.len(),
        "artifactType": artifact_type,
    })
}

/// The manifests of `repo` whose subject is `subject`, as descriptors.
fn referrer_descriptors(repo: &Repo, subject: &str) -> Vec<Value> {
    let mut found: Vec<&String> = (repo.manifests.iter())
        .filter(|(_, stored)| subject_of(&stored.bytes).as_deref() == Some(subject))
        .map(|(digest, _)| digest)
        .collect();
    found.sort();
    found.into_iter().map(|d| descriptor_of(d, repo)).collect()
}

/// What the fallback index of `subject` lists.
fn fallback_manifests(repo: &Repo, subject: &str) -> Vec<Value> {
    let tagged = repo.tags.get(&fallback_tag(subject));
    let index = tagged.and_then(|digest| repo.manifests.get(digest));
    let json: Value = index
        .and_then(|stored| serde_json::from_slice(&stored.bytes).ok())
        .unwrap_or_default();
    json["manifests"].as_array().cloned().unwrap_or_default()
}

fn fallback_index(manifests: &[Value]) -> Vec<u8> {
    json!({ "schemaVersion": 2, "mediaType": oci::INDEX_TYPE, "manifests": manifests })
        .to_string()
        .into_bytes()
}

/// The referrers API's answer: an index of the manifests that have `digest` as
/// their subject, those of the `artifactType` in `query` when it has one.
fn referrers(state: &State, name: &str, digest: &str, query: &str) -> Vec<u8> {
    let Some(repo) = state.repos.get(name) else {
        return not_found("NAME_UNKNOWN", "repository name not known to registry");
    };
    let asked = form_pairs(query).remove("artifactType");
    let mut manifests = referrer_descriptors(repo, digest);
    let mut headers = vec![("Content-Type", oci::INDEX_TYPE)];
    if let Some(asked) = &asked {
        manifests.retain(|d| d["artifactType"].as_str() == Some(asked.as_str()));
        headers.push(("OCI-Filters-Applied", "artifactType"));
    }
    response("200 OK", &headers, &fallback_index(&manifests))
}

/// The first blob a manifest's config or layers name that `repo` lacks.
fn unknown_blob(repo: &Repo, manifest: &[u8]) -> Option<String> {
    let manifest: Value = serde_json::from_slice(manifest).ok()?;
    let layers = manifest["layers"].as_array().into_iter().flatten();
    let named = std::iter::once(&manifest["config"]).chain(layers);
    named
        .filter_map(|blob| blob["digest"].as_str())
        .find(|digest| !repo.blobs.contains_key(*digest))
        .map(str::to_string)
}

fn get_blob(state: &State, name: &str, digest: &str) -> Vec<u8> {
    let blob = state
        .repos
        .get(name)
        .and_then(|repo| repo.blobs.get(digest));
    match blob {
        Some(bytes) => response(
            "200 OK",
            &[
                ("Content-Type", "application/octet-stream"),
                ("Docker-Content-Digest", digest),
            ],
            bytes,
        ),
        None => not_found("BLOB_UNKNOWN", "blob unknown to registry"),
    }
}

fn start_upload(state: &mut State, name: &str) -> Vec<u8> {
    state.uploads_started += 1;
    let id = state.uploads_started;
    let origin = state
        .upload_host
        .as_ref()
        .map_or(String::new(), |host| format!("http://{host}"));
    let location = format!("{origin}/v2/{name}/blobs/uploads/{id}");
    response(
        "202 Accepted",
        &[
            ("Location", &location),
            ("Docker-Upload-UUID", &id.to_string()),
        ],
        b"",
    )
}

fn finish_upload(state: &mut State, name: &str, id: &str, query: &str, body: &[u8]) -> Vec<u8> {
    let digest = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("digest="))
        .map(percent_decoded)
        .unwrap_or_default();
    let digest = digest.as_str();
    if id.is_empty() || digest != digest_of(body) {
        return error_reply(400, "DIGEST_INVALID", "the digest does not match the body");
    }
    let repo = state.repos.entry(name.to_string()).or_default();
    repo.blobs.insert(digest.to_string(), body.to_vec());
    state.blobs_uploaded += 1;
    if let Some((repo, tag)) = state.tag_after_first_blob.take() {
        let stored = Stored {
            media_type: IMAGE_MANIFEST.to_string(),
            bytes: b"{\"schemaVersion\":2}".to_vec(),
        };
        store_tag(state.repos.entry(repo).or_default(), &tag, stored);
    }
    if state
        .down_after_blobs
        .is_some_and(|n| state.blobs_uploaded >= n)
    {
        state.down = true;
    }
    let location = format!("/v2/{name}/blobs/{digest}");
    response(
        "201 Created",
        &[("Docker-Content-Digest", digest), ("Location", &location)],
        b"",
    )
}

/// `text` with each `%XX` as the byte it stands for.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let hex = bytes
            .get(at + 1..at + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn method_not_allowed() -> Vec<u8> {
    error_reply(405, "UNSUPPORTED", "the operation is unsupported")
}

fn not_found(code: &str, message: &str) -> Vec<u8> {
    error_reply(404, code, message)
}

fn error_reply(status: u16, code: &str, message: &str) -> Vec<u8> {
    json_reply(&status_line(status), &[], &errors(code, message))
}

/// The body of a registry's errors.
fn errors(code: &str, message: &str) -> Value {
    json!({ "errors": [{ "code": code, "message": message }] })
}

fn status_line(status: u16) -> String {
    let reason = match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    };
    format!("{status} {reason}")
}

fn json_reply(status: &str, headers: &[(&str, &str)], body: &Value) -> Vec<u8> {
    let mut all = vec![("Content-Type", "application/json")];
    all.extend_from_slice(headers);
    response(status, &all, body.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Reply, SOCKET_TIMEOUT, get_from, send_to};

    const AUTH: &str = "Authorization: Bearer fake-registry-token\r\n";

    fn json_of(reply: &Reply) -> Value {
        serde_json::from_slice(&reply.body).expect("a JSON body")
    }

    fn upload(registry: &Registry, name: &str, bytes: &[u8]) -> Reply {
        let started = send_to(
            registry.addr(),
            "POST",
            &format!("/v2/{name}/blobs/uploads/"),
            AUTH,
            b"",
        );
        assert_eq!(started.status_line, "HTTP/1.1 202 Accepted");
        let location = started
            .head
            .lines()
            .find_map(|line| line.strip_prefix("Location: "))
            .expect("a Location");
        let target = format!("{location}?digest={}", digest_of(bytes));
        send_to(registry.addr(), "PUT", &target, AUTH, bytes)
    }

    fn put_manifest(registry: &Registry, name: &str, tag: &str, body: &[u8]) -> Reply {
        let headers = format!("{AUTH}Content-Type: {IMAGE_MANIFEST}\r\n");
        send_to(
            registry.addr(),
            "PUT",
            &format!("/v2/{name}/manifests/{tag}"),
            &headers,
            body,
        )
    }

    /// Whether a plain request to the registry gets no answer at all, as when it is down.
    fn unanswered(registry: &Registry) -> bool {
        let mut stream = TcpStream::connect(registry.addr()).unwrap();
        stream.set_read_timeout(Some(SOCKET_TIMEOUT)).unwrap();
        stream
            .write_all(b"GET /v2/ HTTP/1.1\r\nHost: t\r\n\r\n")
            .ok();
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut stream, &mut raw).ok();
        raw.is_empty()
    }

    /// A manifest whose config and one layer are blobs of `name`.
    fn manifest_of(config: &[u8], layer: &[u8]) -> Vec<u8> {
        json!({
            "schemaVersion": 2,
            "config": { "digest": digest_of(config), "size": config.len() },
            "layers": [{ "digest": digest_of(layer), "size": layer.len() }],
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_package_uploaded_and_tagged_is_read_back_by_tag_and_by_digest() {
        let registry = Registry::start();
        assert_eq!(
            upload(&registry, "t/a", b"{}").status_line,
            "HTTP/1.1 201 Created"
        );
        assert_eq!(
            upload(&registry, "t/a", b"layer").status_line,
            "HTTP/1.1 201 Created"
        );
        let manifest = manifest_of(b"{}", b"layer");
        let put = put_manifest(&registry, "t/a", "1.0.0", &manifest);
        assert_eq!(put.status_line, "HTTP/1.1 201 Created");
        let digest = digest_of(&manifest);
        assert!(
            put.head
                .contains(&format!("Docker-Content-Digest: {digest}")),
            "{}",
            put.head
        );

        for reference in ["1.0.0", digest.as_str()] {
            let got = get_from(
                registry.addr(),
                &format!("/v2/t/a/manifests/{reference}"),
                AUTH,
            );
            assert_eq!(got.status_line, "HTTP/1.1 200 OK");
            assert_eq!(got.body, manifest);
        }
        let blob = get_from(
            registry.addr(),
            &format!("/v2/t/a/blobs/{}", digest_of(b"layer")),
            AUTH,
        );
        assert_eq!(blob.body, b"layer");
        assert_eq!(registry.tag_digest("t/a", "1.0.0"), Some(digest.clone()));
        assert_eq!(registry.manifest("t/a", &digest), Some(manifest));
        assert_eq!(
            registry.blob("t/a", &digest_of(b"layer")),
            Some(b"layer".to_vec())
        );
        let listed = get_from(registry.addr(), "/v2/t/a/tags/list", AUTH);
        assert_eq!(json_of(&listed)["tags"], json!(["1.0.0"]));
    }

    #[test]
    fn a_head_answers_the_length_and_no_body() {
        let registry = Registry::start();
        let digest = registry.seed_manifest("t/a", "1.0.0", b"{\"schemaVersion\":2}");
        let head = send_to(
            registry.addr(),
            "HEAD",
            "/v2/t/a/manifests/1.0.0",
            AUTH,
            b"",
        );
        assert_eq!(head.status_line, "HTTP/1.1 200 OK");
        assert!(head.head.contains("Content-Length: 19"), "{}", head.head);
        assert!(
            head.head
                .contains(&format!("Docker-Content-Digest: {digest}"))
        );
        assert!(head.body.is_empty());
    }

    #[test]
    fn what_is_not_there_is_404_with_the_code_a_registry_gives() {
        let registry = Registry::start();
        let unknown = get_from(registry.addr(), "/v2/t/none/tags/list", AUTH);
        assert_eq!(unknown.status_line, "HTTP/1.1 404 Not Found");
        assert_eq!(json_of(&unknown)["errors"][0]["code"], "NAME_UNKNOWN");

        registry.seed_manifest("t/a", "1.0.0", b"{}");
        let tag = get_from(registry.addr(), "/v2/t/a/manifests/2.0.0", AUTH);
        assert_eq!(json_of(&tag)["errors"][0]["code"], "MANIFEST_UNKNOWN");
        let blob = get_from(registry.addr(), "/v2/t/a/blobs/sha256:00", AUTH);
        assert_eq!(json_of(&blob)["errors"][0]["code"], "BLOB_UNKNOWN");
    }

    #[test]
    fn a_request_without_the_token_gets_the_challenge_and_the_realm_gives_the_token() {
        let registry = Registry::start();
        let refused = get_from(registry.addr(), "/v2/t/a/manifests/1.0.0", "");
        assert_eq!(refused.status_line, "HTTP/1.1 401 Unauthorized");
        let expected = format!(
            "WWW-Authenticate: Bearer realm=\"http://{}/token\",service=\"fake-registry\",scope=\"repository:t/a:pull\"",
            registry.addr()
        );
        assert!(refused.head.contains(&expected), "{}", refused.head);

        let token = get_from(registry.addr(), "/token?service=fake-registry&scope=x", "");
        assert_eq!(json_of(&token)["token"], TOKEN);
        let write = send_to(registry.addr(), "POST", "/v2/t/a/blobs/uploads/", "", b"");
        assert!(
            write.head.contains("scope=\"repository:t/a:pull,push\""),
            "{}",
            write.head
        );
    }

    #[test]
    fn a_blob_whose_bytes_miss_its_digest_and_a_manifest_naming_a_missing_blob_are_refused() {
        let registry = Registry::start();
        let started = send_to(registry.addr(), "POST", "/v2/t/a/blobs/uploads/", AUTH, b"");
        let location = started
            .head
            .lines()
            .find_map(|l| l.strip_prefix("Location: "))
            .unwrap();
        let wrong = format!("{location}?digest={}", digest_of(b"other"));
        let refused = send_to(registry.addr(), "PUT", &wrong, AUTH, b"bytes");
        assert_eq!(refused.status_line, "HTTP/1.1 400 Bad Request");
        assert_eq!(registry.blob("t/a", &digest_of(b"bytes")), None);

        let manifest = put_manifest(&registry, "t/a", "1.0.0", &manifest_of(b"{}", b"layer"));
        assert_eq!(
            json_of(&manifest)["errors"][0]["code"],
            "MANIFEST_BLOB_UNKNOWN"
        );
        assert_eq!(registry.tag_digest("t/a", "1.0.0"), None);
    }

    #[test]
    fn requests_are_logged_with_their_login_and_writes_are_told_apart_until_cleared() {
        let registry = Registry::start();
        get_from(registry.addr(), "/v2/t/a/tags/list?n=5", "");
        send_to(registry.addr(), "POST", "/v2/t/a/blobs/uploads/", AUTH, b"");
        assert_eq!(
            registry.requests(),
            [
                LoggedRequest {
                    method: "GET".into(),
                    path: "/v2/t/a/tags/list?n=5".into(),
                    authorization: None,
                },
                LoggedRequest {
                    method: "POST".into(),
                    path: "/v2/t/a/blobs/uploads/".into(),
                    authorization: Some("Bearer fake-registry-token".into()),
                },
            ]
        );
        assert_eq!(registry.writes().len(), 1);
        registry.clear_requests();
        assert!(registry.requests().is_empty());
    }

    #[test]
    fn a_status_set_for_a_method_and_path_answers_after_the_login_and_only_there() {
        let registry = Registry::start();
        registry.seed_manifest("t/a", "1.0.0", b"{}");
        registry.answer("HEAD", "/v2/t/a/manifests/1.0.0", 500);
        let anonymous = send_to(registry.addr(), "HEAD", "/v2/t/a/manifests/1.0.0", "", b"");
        assert_eq!(anonymous.status_line, "HTTP/1.1 401 Unauthorized");
        let failed = send_to(
            registry.addr(),
            "HEAD",
            "/v2/t/a/manifests/1.0.0",
            AUTH,
            b"",
        );
        assert_eq!(failed.status_line, "HTTP/1.1 500 Internal Server Error");
        let other = get_from(registry.addr(), "/v2/t/a/manifests/1.0.0", AUTH);
        assert_eq!(other.status_line, "HTTP/1.1 200 OK");
    }

    #[test]
    fn a_tag_created_after_the_first_blob_appears_once_a_blob_is_uploaded() {
        let registry = Registry::start();
        registry.create_tag_after_first_blob("t/a", "1.0.0");
        assert_eq!(registry.tag_digest("t/a", "1.0.0"), None);
        upload(&registry, "t/a", b"{}");
        assert!(registry.tag_digest("t/a", "1.0.0").is_some());
        registry.seed_manifest("t/a", "1.0.0", b"mine");
        upload(&registry, "t/a", b"again");
        assert_eq!(
            registry.manifest("t/a", &registry.tag_digest("t/a", "1.0.0").unwrap()),
            Some(b"mine".to_vec()),
            "the hook fires once"
        );
    }

    #[test]
    fn a_registry_that_is_down_closes_connections_unanswered_and_logs_nothing_until_it_is_back() {
        let registry = Registry::start();
        registry.go_down();
        assert!(unanswered(&registry));
        assert!(registry.requests().is_empty());
        assert_eq!(registry.turned_away(), 1);
        registry.come_back();
        let tags = get_from(registry.addr(), "/v2/t/a/tags/list", AUTH);
        assert_eq!(tags.status_line, "HTTP/1.1 404 Not Found");
    }

    #[test]
    fn a_registry_that_comes_back_stays_up_through_later_uploads() {
        let registry = Registry::start();
        registry.go_down_after_blob_uploads(1);
        upload(&registry, "t/a", b"{}");
        assert!(unanswered(&registry));
        registry.come_back();
        assert_eq!(
            upload(&registry, "t/a", b"layer").status_line,
            "HTTP/1.1 201 Created"
        );
        assert!(!unanswered(&registry));
    }

    #[test]
    fn a_registry_set_to_go_down_after_blobs_goes_down_once_they_are_uploaded() {
        let registry = Registry::start();
        registry.go_down_after_blob_uploads(2);
        upload(&registry, "t/a", b"{}");
        assert_eq!(
            upload(&registry, "t/a", b"layer").status_line,
            "HTTP/1.1 201 Created"
        );
        assert!(unanswered(&registry));
    }

    #[test]
    fn a_query_value_is_read_with_its_percent_escapes_decoded() {
        assert_eq!(percent_decoded("sha256%3Aab%2Fc"), "sha256:ab/c");
        assert_eq!(percent_decoded("100%"), "100%");
        assert_eq!(percent_decoded("%zz%4"), "%zz%4");
    }

    #[test]
    fn the_token_realm_answers_a_status_set_for_it() {
        let registry = Registry::start();
        registry.answer("GET", "/token", 500);
        let token = get_from(registry.addr(), "/token?scope=x", "");
        assert_eq!(token.status_line, "HTTP/1.1 500 Internal Server Error");
    }

    #[test]
    fn a_corrupted_blob_and_manifest_keep_their_digest_and_serve_other_bytes() {
        let registry = Registry::start();
        upload(&registry, "t/a", b"layer");
        let digest = registry.seed_manifest("t/a", "1.0.0", b"{}");
        registry.corrupt("t/a", &digest_of(b"layer"), b"other");
        registry.corrupt("t/a", &digest, b"[]");
        assert_eq!(
            registry.blob("t/a", &digest_of(b"layer")),
            Some(b"other".to_vec())
        );
        assert_eq!(registry.manifest("t/a", &digest), Some(b"[]".to_vec()));
        let served = get_from(
            registry.addr(),
            &format!("/v2/t/a/manifests/{digest}"),
            AUTH,
        );
        assert_eq!(served.body, b"[]");
    }

    #[test]
    fn an_upload_location_names_the_host_it_is_told_to() {
        let registry = Registry::start();
        let relative = send_to(registry.addr(), "POST", "/v2/t/a/blobs/uploads/", AUTH, b"");
        assert!(
            relative.head.contains("Location: /v2/t/a/blobs/uploads/1"),
            "{}",
            relative.head
        );
        registry.upload_location_host("elsewhere.example:9");
        let moved = send_to(registry.addr(), "POST", "/v2/t/a/blobs/uploads/", AUTH, b"");
        let expected = "Location: http://elsewhere.example:9/v2/t/a/blobs/uploads/2";
        assert!(moved.head.contains(expected), "{}", moved.head);
    }

    #[test]
    fn a_seeded_package_is_what_publish_writes_and_is_read_back_by_tag_and_digest() {
        let registry = Registry::start();
        let seeded = registry.seed_package("t/a", "1.0.0", &[("k", "v")], b"layer");
        assert_eq!(seeded.layer, digest_of(b"layer"));
        assert_eq!(
            registry.tag_digest("t/a", "1.0.0"),
            Some(seeded.manifest.clone())
        );
        assert_eq!(registry.blob("t/a", &seeded.layer), Some(b"layer".to_vec()));
        let json: Value =
            serde_json::from_slice(&registry.manifest("t/a", &seeded.manifest).unwrap()).unwrap();
        assert_eq!(json["artifactType"], oci::ARTIFACT_TYPE);
        assert_eq!(json["layers"][0]["mediaType"], oci::LAYER_TYPE);
        assert_eq!(json["annotations"]["k"], "v");
    }

    #[test]
    fn a_seeded_image_has_no_artifact_type_and_a_deleted_manifest_takes_its_tags() {
        let registry = Registry::start();
        let digest = registry.seed_image("t/a", "1.0.0");
        let json: Value =
            serde_json::from_slice(&registry.manifest("t/a", &digest).unwrap()).unwrap();
        assert!(json.get("artifactType").is_none());
        registry.delete_manifest("t/a", &digest);
        let gone = get_from(registry.addr(), "/v2/t/a/manifests/1.0.0", AUTH);
        assert_eq!(gone.status_line, "HTTP/1.1 404 Not Found");
        assert_eq!(registry.tag_digest("t/a", "1.0.0"), None);
    }

    #[test]
    fn a_registry_that_denies_what_it_lacks_answers_403_for_an_unknown_repository_only() {
        let registry = Registry::start();
        registry.seed_manifest("t/a", "1.0.0", b"{}");
        let before = get_from(registry.addr(), "/v2/t/b/tags/list", AUTH);
        assert_eq!(before.status_line, "HTTP/1.1 404 Not Found");
        registry.deny_missing();
        let denied = get_from(registry.addr(), "/v2/t/b/tags/list", AUTH);
        assert_eq!(denied.status_line, "HTTP/1.1 403 Forbidden");
        let held = get_from(registry.addr(), "/v2/t/a/tags/list", AUTH);
        assert_eq!(held.status_line, "HTTP/1.1 200 OK");
    }

    #[test]
    fn a_prefix_that_requires_a_login_refuses_everything_under_it_after_the_token() {
        let registry = Registry::start();
        registry.seed_manifest("t/a", "1.0.0", b"{}");
        registry.seed_manifest("other/a", "1.0.0", b"{}");
        registry.refuse_under("t", 401);
        let refused = get_from(registry.addr(), "/v2/t/a/manifests/1.0.0", AUTH);
        assert_eq!(refused.status_line, "HTTP/1.1 401 Unauthorized");
        let outside = get_from(registry.addr(), "/v2/other/a/manifests/1.0.0", AUTH);
        assert_eq!(outside.status_line, "HTTP/1.1 200 OK");
    }

    fn basic(user: &str, secret: &str) -> String {
        let sent = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{secret}"));
        format!("Authorization: Basic {sent}\r\n")
    }

    const PUSH: &str = "/token?service=fake-registry&scope=repository%3Ateam%2Fa%3Apull%2Cpush";
    const PULL: &str = "/token?service=fake-registry&scope=repository%3Ateam%2Fa%3Apull";

    #[test]
    fn a_prefix_that_requires_a_login_gives_a_token_only_to_a_login_and_not_to_a_refused_user() {
        let registry = Registry::start();
        registry.require_login("team");
        registry.refuse_user("u9");
        let status =
            |target: &str, headers: &str| get_from(registry.addr(), target, headers).status_line;
        assert_eq!(status(PULL, ""), "HTTP/1.1 401 Unauthorized");
        assert_eq!(status(PULL, &basic("u1", "s1")), "HTTP/1.1 200 OK");
        assert_eq!(
            status(PULL, &basic("u9", "s1")),
            "HTTP/1.1 401 Unauthorized"
        );
        let elsewhere = "/token?scope=repository%3Aother%2Fa%3Apull";
        assert_eq!(status(elsewhere, ""), "HTTP/1.1 200 OK");
        assert_eq!(
            status("/token", ""),
            "HTTP/1.1 200 OK",
            "no scope, no repository"
        );
    }

    #[test]
    fn a_login_required_to_write_leaves_a_pull_anonymous() {
        let registry = Registry::start();
        registry.require_login_to_write("team");
        let status = |target: &str| get_from(registry.addr(), target, "").status_line;
        assert_eq!(status(PULL), "HTTP/1.1 200 OK");
        assert_eq!(status(PUSH), "HTTP/1.1 401 Unauthorized");
    }

    #[test]
    fn a_refresh_token_grant_is_a_login_and_what_the_realm_was_asked_is_kept_until_cleared() {
        let registry = Registry::start();
        registry.require_login("team");
        let form = "grant_type=refresh_token&service=fake-registry&scope=repository%3Ateam%2Fa%3Apull\
                    &client_id=lvpm&refresh_token=r1+x%2By";
        let grant = send_to(registry.addr(), "POST", "/token", "", form.as_bytes());
        assert_eq!(grant.status_line, "HTTP/1.1 200 OK");
        get_from(registry.addr(), PULL, &basic("u1", "s1-secret"));
        let asked = registry.token_requests();
        assert_eq!(asked[0].method, "POST");
        assert_eq!(asked[0].refresh_token(), Some("r1 x+y"));
        assert_eq!(asked[0].scope(), "repository:team/a:pull");
        assert_eq!(asked[0].basic_login(), None);
        assert_eq!(
            asked[1].basic_login(),
            Some(("u1".to_string(), "s1-secret".to_string()))
        );
        assert_eq!(asked[1].refresh_token(), None);
        assert_eq!(
            registry.writes().len(),
            0,
            "a POST to the token realm changes nothing"
        );
        registry.clear_requests();
        assert!(registry.token_requests().is_empty());
    }

    #[test]
    fn writes_refused_under_a_prefix_leave_reads_alone() {
        let registry = Registry::start();
        registry.seed_manifest("t/a", "1.0.0", b"{}");
        registry.refuse_writes_under("t", 403);
        let read = get_from(registry.addr(), "/v2/t/a/manifests/1.0.0", AUTH);
        assert_eq!(read.status_line, "HTTP/1.1 200 OK");
        let write = send_to(registry.addr(), "POST", "/v2/t/a/blobs/uploads/", AUTH, b"");
        assert_eq!(write.status_line, "HTTP/1.1 403 Forbidden");
    }

    /// A manifest with `subject` and the artifact type `artifact`, its blobs uploaded.
    fn referrer_body(registry: &Registry, subject: &str, artifact: &str) -> Vec<u8> {
        upload(registry, "t/a", b"{}");
        upload(registry, "t/a", artifact.as_bytes());
        json!({
            "schemaVersion": 2,
            "artifactType": artifact,
            "config": { "digest": digest_of(b"{}"), "size": 2 },
            "layers": [{ "digest": digest_of(artifact.as_bytes()), "size": artifact.len() }],
            "subject": { "digest": subject, "size": 1 },
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_manifest_with_a_subject_is_listed_by_the_referrers_api_and_filtered_by_type() {
        let registry = Registry::start();
        let subject = registry.seed_manifest("t/a", "1.0.0", b"{}");
        let sbom = referrer_body(&registry, &subject, "a/sbom");
        let put = put_manifest(&registry, "t/a", &digest_of(&sbom), &sbom);
        assert!(
            put.head.contains(&format!("OCI-Subject: {subject}")),
            "{}",
            put.head
        );
        let other = referrer_body(&registry, &subject, "a/bundle");
        put_manifest(&registry, "t/a", &digest_of(&other), &other);

        let all = get_from(
            registry.addr(),
            &format!("/v2/t/a/referrers/{subject}"),
            AUTH,
        );
        assert_eq!(json_of(&all)["manifests"].as_array().unwrap().len(), 2);
        let asked = get_from(
            registry.addr(),
            &format!("/v2/t/a/referrers/{subject}?artifactType=a%2Fsbom"),
            AUTH,
        );
        assert!(asked.head.contains("OCI-Filters-Applied: artifactType"));
        let listed = json_of(&asked);
        assert_eq!(listed["manifests"].as_array().unwrap().len(), 1);
        assert_eq!(listed["manifests"][0]["digest"], digest_of(&sbom).as_str());
        assert_eq!(listed["manifests"][0]["artifactType"], "a/sbom");
        assert_eq!(registry.referrers("t/a", &subject).len(), 2);
    }

    #[test]
    fn without_the_referrers_api_the_route_is_404_and_a_subject_is_not_acknowledged() {
        let registry = Registry::start();
        registry.offer_referrers_api(false);
        let subject = registry.seed_manifest("t/a", "1.0.0", b"{}");
        let body = referrer_body(&registry, &subject, "a/sbom");
        let put = put_manifest(&registry, "t/a", &digest_of(&body), &body);
        assert_eq!(put.status_line, "HTTP/1.1 201 Created");
        assert!(!put.head.contains("OCI-Subject"), "{}", put.head);
        let route = format!("/v2/t/a/referrers/{subject}");
        assert_eq!(
            get_from(registry.addr(), &route, AUTH).status_line,
            "HTTP/1.1 404 Not Found"
        );
        assert_eq!(registry.referrers("t/a", &subject), [], "no index yet");
    }

    #[test]
    fn a_seeded_referrer_is_listed_in_the_fallback_index_when_the_api_is_off() {
        let registry = Registry::start();
        let subject = registry.seed_manifest("t/a", "1.0.0", b"{}");
        let tag = subject.replacen(':', "-", 1);
        let with_api = registry.seed_referrer("t/a", &subject, "a/bundle", b"one");
        assert_eq!(registry.tag_digest("t/a", &tag), None);
        assert_eq!(
            registry.referrers("t/a", &subject),
            [(with_api, "a/bundle".to_string())]
        );
        registry.offer_referrers_api(false);
        let without = registry.seed_referrer("t/a", &subject, "a/other", b"two");
        let index = registry.tag_digest("t/a", &tag).expect("a fallback index");
        let index: Value =
            serde_json::from_slice(&registry.manifest("t/a", &index).unwrap()).unwrap();
        assert_eq!(index["manifests"][0]["digest"], without.as_str());
        assert_eq!(
            registry.referrers("t/a", &subject),
            [(without, "a/other".to_string())]
        );
    }

    #[test]
    fn writes_of_a_referrer_are_refused_for_one_repository_and_other_writes_are_not() {
        let registry = Registry::start();
        let subject = registry.seed_manifest("t/a", "1.0.0", b"{}");
        registry.refuse_referrer_writes("t/a");
        let body = referrer_body(&registry, &subject, "a/sbom");
        let refused = put_manifest(&registry, "t/a", &digest_of(&body), &body);
        assert_eq!(refused.status_line, "HTTP/1.1 403 Forbidden");
        let index = put_manifest(&registry, "t/a", &subject.replacen(':', "-", 1), b"{}");
        assert_eq!(index.status_line, "HTTP/1.1 403 Forbidden");
        let plain = put_manifest(&registry, "t/a", "2.0.0", b"{}");
        assert_eq!(plain.status_line, "HTTP/1.1 201 Created");
    }

    #[test]
    fn dropping_the_registry_stops_its_thread() {
        let registry = Registry::start();
        let addr = registry.addr();
        drop(registry);
        assert!(TcpStream::connect_timeout(&addr, SOCKET_TIMEOUT).is_err());
    }
}
