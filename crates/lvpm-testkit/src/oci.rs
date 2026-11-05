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

use crate::http::{Request, accept, accept_loop, read_body, response, shut_down};
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
        !matches!(self.method.as_str(), "GET" | "HEAD")
    }
}

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
    /// After the login, a request under a repository prefix answers a status.
    refused_under: Vec<(String, u16)>,
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
            .push((prefix.to_string(), status));
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
            None => json_reply("200 OK", &[], &json!({ "token": TOKEN })),
        };
    }
    let name = match &route {
        Route::Tags(name)
        | Route::Manifest(name, _)
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
    let refused = state.refused_under.iter().find_map(|(prefix, status)| {
        name.strip_prefix(prefix.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
            .then_some(*status)
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
    response(
        "201 Created",
        &[("Docker-Content-Digest", &digest), ("Location", &location)],
        b"",
    )
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

    #[test]
    fn dropping_the_registry_stops_its_thread() {
        let registry = Registry::start();
        let addr = registry.addr();
        drop(registry);
        assert!(TcpStream::connect_timeout(&addr, SOCKET_TIMEOUT).is_err());
    }
}
