//! A small OCI Distribution client over the blocking `reqwest` lvpm already
//! uses: what publishing and installing a package need, and no more.
//!
//! `reference` parses `oci://` URLs, `auth` logs in with the registry's bearer
//! challenge, `manifest` holds the manifest, blob and upload calls. Nothing
//! here reads the process environment.
//!
//! A package version is one OCI image manifest: `artifactType`
//! [`ARTIFACT_TYPE`], an empty config, and one layer, the `.lvpkg` bytes
//! (media type [`LAYER_TYPE`]). Its annotations carry what an index needs.

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::LOCATION;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

mod auth;
mod manifest;
pub mod reference;

pub use manifest::{Manifest, Package};
pub use reference::{FeedUrl, Reference, SCHEME, Target, UrlError, valid_tag};

/// The manifest's `artifactType`: what tells a package from any other artifact.
pub const ARTIFACT_TYPE: &str = "application/vnd.lvpm.package.v1";
/// The config's media type: OCI's empty descriptor, the two bytes `{}`.
pub const EMPTY_CONFIG_TYPE: &str = "application/vnd.oci.empty.v1+json";
/// The config blob of every package.
pub const EMPTY_CONFIG: &[u8] = b"{}";
/// The layer's media type: the `.lvpkg` bytes.
pub const LAYER_TYPE: &str = "application/vnd.lvpm.lvpkg.v1+zip";
/// The media type of an OCI image manifest.
pub const MANIFEST_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

pub const ANNOTATION_TITLE: &str = "org.opencontainers.image.title";
pub const ANNOTATION_VERSION: &str = "org.opencontainers.image.version";
pub const ANNOTATION_CREATED: &str = "org.opencontainers.image.created";
pub const ANNOTATION_REQUIRES: &str = "dev.lvpm.requires";
pub const ANNOTATION_LABVIEW: &str = "dev.lvpm.labview";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long one request may take in all. blocking `reqwest` has no timeout
/// for a stalled read alone, so this bounds the whole exchange.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// A blob is the package itself and may be large, so its transfer gets longer.
const BLOB_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// A registry may refuse a manifest above 4 MiB, so none is larger.
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
/// The most of a blob that is read: the cap of one entry of a package.
const MAX_BLOB_BYTES: u64 = 1024 * 1024 * 1024;

/// `sha256:<hex>` of `bytes`, as an OCI digest.
pub fn digest_of(bytes: &[u8]) -> String {
    let hex: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sha256:{hex}")
}

/// `error`'s text without the ` for url (...)` reqwest appends: the URL can be
/// a signed upload or CDN address whose query is a grant. The registry is
/// named by the message anyway.
fn without_url(error: &dyn std::error::Error) -> String {
    let text = error.to_string();
    match text.find(" for url (") {
        Some(at) => text[..at].to_string(),
        None => text,
    }
}

/// What a call to a registry can come to. No variant holds a token or a
/// credential.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The registry answered with a status the call does not accept.
    #[error("registry {registry} answered {status} to {request}")]
    Status {
        registry: String,
        status: u16,
        request: String,
    },
    /// No answer: it could not be reached, or it stopped answering in time.
    #[error("registry {registry} is unreachable: {cause}")]
    Unreachable { registry: String, cause: String },
    /// What the registry sent does not hash to the digest it was asked for.
    #[error("registry {registry} served bytes that do not match {digest}")]
    DigestMismatch { registry: String, digest: String },
    /// An answer that is not what the distribution API says.
    #[error("registry {registry} {problem}")]
    Protocol { registry: String, problem: String },
}

impl Error {
    fn status(registry: &str, status: u16, request: &str) -> Error {
        Error::Status {
            registry: registry.to_string(),
            status,
            request: request.to_string(),
        }
    }

    fn unreachable(registry: &str, cause: &dyn std::error::Error) -> Error {
        let mut text = without_url(cause);
        let mut source = cause.source();
        while let Some(next) = source {
            text.push_str(": ");
            text.push_str(&without_url(next));
            source = next.source();
        }
        Error::Unreachable {
            registry: registry.to_string(),
            cause: text,
        }
    }

    fn protocol(registry: &str, problem: &str) -> Error {
        Error::Protocol {
            registry: registry.to_string(),
            problem: problem.to_string(),
        }
    }

    /// The registry that failed.
    pub fn registry(&self) -> &str {
        match self {
            Error::Status { registry, .. }
            | Error::Unreachable { registry, .. }
            | Error::DigestMismatch { registry, .. }
            | Error::Protocol { registry, .. } => registry,
        }
    }

    /// The status the registry answered, when it did.
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Error::Status { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Whether the registry said no to who asked: `401` or `403`.
    pub fn is_refused(&self) -> bool {
        matches!(self.status_code(), Some(401 | 403))
    }
}

/// Why a package pinned to an OCI source could not be listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingFault {
    /// The registry has no repository for it.
    NotListed,
    /// `401` or `403`: the package is missing, or the login cannot read it. A
    /// registry that hides what a login cannot read answers this for both.
    Refused,
    /// No answer, or one that is no listing; the cause in words.
    Unreachable(String),
}

impl ListingFault {
    /// What a failed tags list or manifest read comes to.
    pub fn of(error: &Error) -> ListingFault {
        match error {
            _ if error.status_code() == Some(404) => ListingFault::NotListed,
            _ if error.is_refused() => ListingFault::Refused,
            Error::Unreachable { cause, .. } => ListingFault::Unreachable(cause.clone()),
            other => ListingFault::Unreachable(other.to_string()),
        }
    }
}

/// One package pinned to an OCI source that the source could not list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingFailure {
    pub package: String,
    /// The `[sources]` name.
    pub source: String,
    /// The source's URL, `oci://registry/prefix`.
    pub feed: FeedUrl,
    pub fault: ListingFault,
}

impl fmt::Display for ListingFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            package,
            source,
            feed,
            fault,
        } = self;
        let registry = feed.registry();
        match fault {
            ListingFault::NotListed => write!(
                f,
                "{package}: source \"{source}\" ({feed}) does not list it; check the name, or \
                 publish it with lvpm publish --to {feed}"
            ),
            ListingFault::Refused => write!(
                f,
                "{package}: registry {registry} refused the lookup for source \"{source}\"; the \
                 package is missing or your login cannot read it; run docker login {registry}"
            ),
            ListingFault::Unreachable(cause) => {
                write!(
                    f,
                    "{package}: source \"{source}\" ({feed}) is unreachable: {cause}"
                )
            }
        }
    }
}

/// Every pinned package the OCI sources could not list, in package order: one
/// error, so that a run reports them all and none is read as "not listed" by
/// a source that is asked next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingFailures(Vec<ListingFailure>);

impl ListingFailures {
    pub fn new(mut failures: Vec<ListingFailure>) -> ListingFailures {
        failures.sort_by_cached_key(|f| crate::project::name_key(&f.package));
        ListingFailures(failures)
    }

    pub fn failures(&self) -> &[ListingFailure] {
        &self.0
    }
}

/// One line for each package.
impl fmt::Display for ListingFailures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lines: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        f.write_str(&lines.join("\n"))
    }
}

impl std::error::Error for ListingFailures {}

/// The words for a package that came from an OCI source by digest (`manifest`)
/// and was not fetched: `None` when `why` is not a registry's doing.
pub fn fetch_failure(
    why: &anyhow::Error,
    package: &str,
    version: &str,
    source: &str,
    manifest: &str,
) -> Option<String> {
    let error = why.downcast_ref::<Error>()?;
    let registry = error.registry();
    let by_digest = format!("cannot fetch {package} {version} by digest from source \"{source}\"");
    Some(match error {
        Error::DigestMismatch { digest, .. } => format!(
            "{package} {version} from source \"{source}\": registry {registry} served bytes that \
             do not match {digest}; nothing was installed"
        ),
        _ if error.is_refused() => {
            format!("{by_digest}: registry {registry} refused it; run docker login {registry}")
        }
        _ if error.status_code() == Some(404) => {
            format!("{by_digest}: registry {registry} no longer holds {manifest}; run lvpm lock")
        }
        other => format!("{by_digest}: {other}"),
    })
}

/// What a call may do in a repository: a push may read as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Pull,
    Push,
}

/// A repository on a registry, and what the caller wants to do there.
#[derive(Debug, Clone, Copy)]
pub struct Repo<'a> {
    pub registry: &'a str,
    pub name: &'a str,
    pub access: Access,
}

impl Repo<'_> {
    /// The token scope that covers `access`.
    fn scope(&self) -> String {
        let actions = match self.access {
            Access::Pull => "pull",
            Access::Push => "pull,push",
        };
        format!("repository:{}:{actions}", self.name)
    }
}

/// The OCI side of an operation's environment: one HTTP client for every
/// registry call, and the bearer tokens it has been given.
///
/// The front end may replace it; an `OpEnv` starts with the default.
#[derive(Default)]
pub struct Oci {
    /// Built on the first call: most commands never talk to a registry.
    client: OnceLock<Client>,
    /// A token per registry and scope, so a run logs in once for each.
    tokens: Mutex<HashMap<(String, String), String>>,
}

impl fmt::Debug for Oci {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Oci").finish_non_exhaustive()
    }
}

impl Oci {
    fn client(&self, registry: &str) -> Result<&Client, Error> {
        if let Some(client) = self.client.get() {
            return Ok(client);
        }
        let built = Client::builder()
            .user_agent(concat!("lvpm/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(redirect_policy())
            .build()
            .map_err(|e| Error::unreachable(registry, &e))?;
        Ok(self.client.get_or_init(|| built))
    }

    /// `build`'s request, logged in: with the token held for this repository
    /// and access, or after the registry's challenge for one. A second `401`
    /// is returned for the caller to read.
    fn send(
        &self,
        repo: &Repo,
        build: impl Fn(&Client) -> RequestBuilder,
    ) -> Result<Response, Error> {
        let client = self.client(repo.registry)?;
        let key = (repo.registry.to_string(), repo.scope());
        let attempt = |token: Option<String>| {
            let request = build(client);
            let request = match token {
                Some(token) => request.bearer_auth(token),
                None => request,
            };
            request
                .send()
                .map_err(|e| Error::unreachable(repo.registry, &e))
        };
        let held = self.tokens().get(&key).cloned();
        let first = attempt(held)?;
        if first.status().as_u16() != 401 {
            return Ok(first);
        }
        let Some(challenge) = auth::challenge(first.headers()) else {
            return Ok(first);
        };
        let token = auth::fetch_token(client, repo.registry, &challenge, &key.1)?;
        self.tokens().insert(key, token.clone());
        attempt(Some(token))
    }

    fn tokens(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), String>> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Redirects are followed up to reqwest's own limit, but never to plain http
/// beyond loopback: the package or a token would cross the wire in the clear.
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let url = attempt.url();
        match scheme_allowed(url.scheme(), url.host_str().unwrap_or_default())
            && attempt.previous().len() < 10
        {
            true => attempt.follow(),
            false => attempt.error("a redirect to plain http or too many redirects"),
        }
    })
}

/// `https://registry`, or `http://registry` for the loopback addresses a test
/// registry listens on.
fn base_url(registry: &str) -> String {
    let host = registry.split(':').next().unwrap_or_default();
    let scheme = match is_loopback(host) {
        true => "http",
        false => "https",
    };
    format!("{scheme}://{registry}")
}

fn is_loopback(host: &str) -> bool {
    host == "127.0.0.1" || host == "localhost"
}

/// Whether a URL of `scheme` to `host` may be called: `https`, and plain
/// `http` only to loopback.
fn scheme_allowed(scheme: &str, host: &str) -> bool {
    scheme == "https" || (scheme == "http" && is_loopback(host))
}

/// `response` when its status is one of `accepted`; else the error naming
/// `request`.
fn expect(
    repo: &Repo,
    response: Response,
    request: &str,
    accepted: &[u16],
) -> Result<Response, Error> {
    let status = response.status().as_u16();
    match accepted.contains(&status) {
        true => Ok(response),
        false => Err(Error::status(repo.registry, status, request)),
    }
}

/// All of `response`'s body, up to `max` bytes.
fn read_limited(registry: &str, response: Response, max: u64) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    response
        .take(max + 1)
        .read_to_end(&mut body)
        .map_err(|e| Error::unreachable(registry, &e))?;
    match body.len() as u64 > max {
        true => Err(Error::protocol(
            registry,
            &format!("sent more than {max} bytes"),
        )),
        false => Ok(body),
    }
}

/// Where the response says to go on (`Location`), which may be relative, as a
/// full URL on the registry's own host: a token is never sent anywhere else.
fn location(repo: &Repo, from: &str, response: &Response) -> Result<reqwest::Url, Error> {
    let header = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| Error::protocol(repo.registry, "gave no upload location"))?;
    let base = reqwest::Url::parse(from).expect("a URL lvpm built");
    let url = base
        .join(header)
        .map_err(|_| Error::protocol(repo.registry, "gave an upload location that is no URL"))?;
    // The origin, not the authority: `http://` on the registry's own host
    // would carry the token in the clear.
    match url.origin() == base.origin() {
        true => Ok(url),
        false => Err(Error::protocol(
            repo.registry,
            "gave an upload location on another host or scheme",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_http_is_for_loopback_only() {
        assert_eq!(base_url("127.0.0.1:5000"), "http://127.0.0.1:5000");
        assert_eq!(base_url("localhost:5000"), "http://localhost:5000");
        assert_eq!(base_url("ghcr.io"), "https://ghcr.io");
        assert_eq!(
            base_url("127.0.0.1.evil.example"),
            "https://127.0.0.1.evil.example"
        );
        assert!(scheme_allowed("https", "ghcr.io"));
        assert!(scheme_allowed("http", "localhost"));
        assert!(!scheme_allowed("http", "ghcr.io"));
        assert!(!scheme_allowed("ftp", "127.0.0.1"));
    }

    #[test]
    fn an_unreachable_cause_drops_the_url_reqwest_appends() {
        let signed = std::io::Error::other(
            "error sending request for url (https://cdn.example/b?X-Amz-Signature=s3cret-7f3a9c)",
        );
        let shown = Error::unreachable("r:1", &signed).to_string();
        assert_eq!(shown, "registry r:1 is unreachable: error sending request");
    }

    #[test]
    fn a_digest_is_sha256_of_the_bytes() {
        assert_eq!(
            digest_of(EMPTY_CONFIG),
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }

    #[test]
    fn an_error_names_its_registry_and_says_whether_the_login_was_refused() {
        let refused = Error::status("r:1", 403, "HEAD t/a:1");
        assert_eq!(
            refused.to_string(),
            "registry r:1 answered 403 to HEAD t/a:1"
        );
        assert!(refused.is_refused());
        assert_eq!(refused.registry(), "r:1");
        assert!(!Error::status("r", 500, "x").is_refused());
        let down = Error::unreachable("r:1", &std::io::Error::other("closed"));
        assert_eq!(down.to_string(), "registry r:1 is unreachable: closed");
        assert!(!down.is_refused());
        assert_eq!(down.status_code(), None);
    }
    #[test]
    fn a_fetch_failure_by_digest_says_what_the_registry_did_and_what_to_run() {
        let say = |error: Error| {
            let why = anyhow::Error::new(error);
            fetch_failure(&why, "alpha", "1.1.0", "team", "sha256:ab")
        };
        let refused = say(Error::status("r:1", 403, "GET"));
        assert_eq!(
            refused.as_deref(),
            Some(
                "cannot fetch alpha 1.1.0 by digest from source \"team\": registry r:1 refused \
                 it; run docker login r:1"
            )
        );
        let gone = say(Error::status("r:1", 404, "GET"));
        assert_eq!(
            gone.as_deref(),
            Some(
                "cannot fetch alpha 1.1.0 by digest from source \"team\": registry r:1 no longer \
                 holds sha256:ab; run lvpm lock"
            )
        );
        let wrong = say(Error::DigestMismatch {
            registry: "r:1".into(),
            digest: "sha256:ab".into(),
        });
        assert_eq!(
            wrong.as_deref(),
            Some(
                "alpha 1.1.0 from source \"team\": registry r:1 served bytes that do not match \
                 sha256:ab; nothing was installed"
            )
        );
        assert_eq!(
            fetch_failure(&anyhow::anyhow!("disk full"), "a", "1", "t", "sha256:ab"),
            None
        );
    }

    #[test]
    fn pinned_failures_are_one_line_each_in_package_order() {
        let feed = FeedUrl::parse("oci://r:1/org").unwrap();
        let failure = |package: &str, fault| ListingFailure {
            package: package.to_string(),
            source: "team".to_string(),
            feed: feed.clone(),
            fault,
        };
        let failures = ListingFailures::new(vec![
            failure("zeta", ListingFault::Unreachable("closed".into())),
            failure("alpha", ListingFault::NotListed),
        ]);
        assert_eq!(
            failures.to_string(),
            "alpha: source \"team\" (oci://r:1/org) does not list it; check the name, or \
             publish it with lvpm publish --to oci://r:1/org\n\
             zeta: source \"team\" (oci://r:1/org) is unreachable: closed"
        );
        assert_eq!(
            ListingFault::of(&Error::status("r:1", 404, "GET")),
            ListingFault::NotListed
        );
        assert_eq!(
            ListingFault::of(&Error::status("r:1", 401, "GET")),
            ListingFault::Refused
        );
    }
}
