//! Logins for a registry from where `docker login` keeps them: the Docker
//! config's `credHelpers` entry for the host, else its `credsStore`, else its
//! `auths` entry. With none, a call is anonymous.
//!
//! Core reads no environment: the front end finds the config directory and
//! hands it to [`docker_store`]. A helper is `docker-credential-<name> get`
//! with the host on its standard input; what it holds comes back as JSON.
//!
//! A secret reaches nothing that prints: [`Credential`]'s `Debug` omits it, no
//! [`Error`] holds one, and a helper's standard error is dropped unread, so a
//! secret it printed there cannot leak.

use crate::process::{self, Ran};
use base64::Engine as _;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

/// How long a credential helper may take: it may unlock a keyring.
pub const DEFAULT_HELPER_TIMEOUT: Duration = Duration::from_secs(30);

const HELPER_PREFIX: &str = "docker-credential-";
/// What a helper that holds no login for the host prints, and exits 1 on.
const NOT_FOUND: &[u8] = b"credentials not found in native keychain";
/// A helper's answer is a few hundred bytes: more is not one.
const MAX_HELPER_OUTPUT: u64 = 64 * 1024;
/// The user name with which a helper (or `docker login --identitytoken`) says
/// its secret is an identity token, not a password.
const IDENTITY_USER: &str = "<token>";

/// Where a [`Credential`] was found, as a message names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// `credHelpers`: the helper `docker-credential-<name>`.
    CredHelpers(String),
    /// `credsStore`: the helper `docker-credential-<name>`.
    CredsStore(String),
    /// `auths`: the config file itself.
    Auths,
}

impl Origin {
    /// The program that holds the login, when one does.
    fn helper(&self) -> Option<String> {
        match self {
            Origin::CredHelpers(name) | Origin::CredsStore(name) => {
                Some(format!("{HELPER_PREFIX}{name}"))
            }
            Origin::Auths => None,
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self, self.helper()) {
            (Origin::CredHelpers(_), Some(helper)) => write!(f, "credHelpers {helper}"),
            (Origin::CredsStore(_), Some(helper)) => write!(f, "credsStore {helper}"),
            _ => f.write_str("auths"),
        }
    }
}

/// What to log in with.
#[derive(Clone, PartialEq, Eq)]
enum Login {
    /// A user name and password, sent as Basic auth to the token service.
    Basic { user: String, secret: String },
    /// A refresh token, exchanged at the token service for an access token.
    Identity(String),
}

/// A login for one registry, and where it came from.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential {
    origin: Origin,
    login: Login,
}

/// The secret is left out.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl Credential {
    /// A user name and password.
    pub fn basic(origin: Origin, user: &str, secret: &str) -> Credential {
        Credential {
            origin,
            login: Login::Basic {
                user: user.to_string(),
                secret: secret.to_string(),
            },
        }
    }

    /// An identity token.
    pub fn identity(origin: Origin, token: &str) -> Credential {
        Credential {
            origin,
            login: Login::Identity(token.to_string()),
        }
    }

    pub fn origin(&self) -> &Origin {
        &self.origin
    }
}

/// Why no login could be had. No variant holds a secret, or anything a helper
/// printed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error(
        "credential helper {helper} for {registry} failed{}; run it by hand to see why",
        exit_text(*code)
    )]
    HelperFailed {
        helper: String,
        registry: String,
        code: Option<i32>,
    },
    #[error(
        "credential helper {helper} for {registry} was not found; check that it is installed and \
         on PATH"
    )]
    HelperNotFound { helper: String, registry: String },
    #[error("credential helper {helper} for {registry} did not answer within {secs}s")]
    HelperTimedOut {
        helper: String,
        registry: String,
        secs: u64,
    },
    #[error(
        "credential helper {helper} for {registry} gave an answer lvpm cannot read; run it by \
         hand to see why"
    )]
    HelperUnreadable { helper: String, registry: String },
    #[error("the Docker config {path} cannot be read for {registry}: {problem}")]
    Config {
        path: String,
        registry: String,
        problem: String,
    },
}

fn exit_text(code: Option<i32>) -> String {
    code.map_or_else(String::new, |code| format!(" with exit code {code}"))
}

impl Error {
    /// The registry the login was wanted for.
    pub fn registry(&self) -> &str {
        match self {
            Error::HelperFailed { registry, .. }
            | Error::HelperNotFound { registry, .. }
            | Error::HelperTimedOut { registry, .. }
            | Error::HelperUnreadable { registry, .. }
            | Error::Config { registry, .. } => registry,
        }
    }
}

/// What gives the login for a registry (`host[:port]`): `None` is anonymous.
pub type Credentials = Arc<dyn Fn(&str) -> Result<Option<Credential>, Error> + Send + Sync>;

/// What runs a credential helper: given the program (`docker-credential-<name>`),
/// the host to put on its standard input, and how long it may take.
pub type HelperRunner = Arc<dyn Fn(&str, &str, Duration) -> Ran + Send + Sync>;

/// No login, whatever the registry.
pub fn anonymous() -> Credentials {
    Arc::new(|_| Ok(None))
}

/// `<helper> get`, which is how Docker asks.
pub fn real_helper_runner() -> HelperRunner {
    Arc::new(|helper, registry, limit| {
        process::run(
            Command::new(helper).arg("get"),
            Some(registry.as_bytes()),
            limit,
            MAX_HELPER_OUTPUT,
        )
    })
}

/// The logins of the Docker config in `config_dir`, asking a helper through
/// `run`, and giving up on one after `helper_timeout`. A directory with no
/// `config.json` holds none.
pub fn docker_store(
    config_dir: PathBuf,
    helper_timeout: Duration,
    run: HelperRunner,
) -> Credentials {
    Arc::new(move |registry| lookup(&config_dir, registry, helper_timeout, &*run))
}

/// The parts of `config.json` that name a login.
#[derive(Deserialize, Default)]
struct Config {
    #[serde(rename = "credHelpers", default)]
    cred_helpers: HashMap<String, String>,
    #[serde(rename = "credsStore", default)]
    creds_store: String,
    #[serde(default)]
    auths: HashMap<String, Auth>,
}

#[derive(Deserialize)]
struct Auth {
    #[serde(default)]
    auth: String,
}

/// What a helper prints for `get`.
#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "Username")]
    username: String,
    #[serde(rename = "Secret")]
    secret: String,
}

type Run<'a> = &'a dyn Fn(&str, &str, Duration) -> Ran;

fn lookup(
    config_dir: &Path,
    registry: &str,
    timeout: Duration,
    run: Run,
) -> Result<Option<Credential>, Error> {
    let config = read_config(config_dir, registry)?;
    let named = config.cred_helpers.get(registry).filter(|n| !n.is_empty());
    let origin = match named {
        Some(name) => Origin::CredHelpers(name.clone()),
        None if !config.creds_store.is_empty() => Origin::CredsStore(config.creds_store),
        None => return Ok(auths_entry(&config.auths, registry)),
    };
    ask_helper(origin, registry, timeout, run)
}

fn read_config(config_dir: &Path, registry: &str) -> Result<Config, Error> {
    let path = config_dir.join("config.json");
    let unreadable = |problem: String| Error::Config {
        path: path.display().to_string(),
        registry: registry.to_string(),
        problem,
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Config::default());
        }
        Err(error) => return Err(unreadable(error.kind().to_string())),
    };
    // A serde error can quote the text it choked on, which may be a secret.
    serde_json::from_slice(&bytes).map_err(|error| {
        unreadable(format!(
            "not a Docker config (line {}, column {})",
            error.line(),
            error.column()
        ))
    })
}

/// The `auths` entry for `registry`: Docker names it `host`, or a URL on that
/// host. Its `auth` is `base64(user:password)`; one that is not, is none.
fn auths_entry(auths: &HashMap<String, Auth>, registry: &str) -> Option<Credential> {
    let host_of = |key: &str| {
        let bare = key.strip_prefix("https://").or(key.strip_prefix("http://"));
        bare.unwrap_or(key)
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string()
    };
    let entry = auths.iter().find(|(key, _)| host_of(key) == registry)?.1;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(entry.auth.trim())
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (user, secret) = text.split_once(':')?;
    Some(Credential::basic(Origin::Auths, user, secret))
}

fn ask_helper(
    origin: Origin,
    registry: &str,
    timeout: Duration,
    run: Run,
) -> Result<Option<Credential>, Error> {
    let helper = origin.helper().expect("an origin that is a helper");
    let ran = run(&helper, registry, timeout);
    let registry = registry.to_string();
    match ran {
        // A helper with no login says so on stdout, and exits 1.
        Ran::Exited { stdout, .. } if stdout.trim_ascii() == NOT_FOUND => Ok(None),
        Ran::Exited {
            status: Some(0),
            stdout,
            ..
        } => match serde_json::from_slice::<Answer>(&stdout) {
            Ok(answer) if answer.username == IDENTITY_USER => {
                Ok(Some(Credential::identity(origin, &answer.secret)))
            }
            Ok(answer) => Ok(Some(Credential::basic(
                origin,
                &answer.username,
                &answer.secret,
            ))),
            Err(_) => Err(Error::HelperUnreadable { helper, registry }),
        },
        Ran::Exited { status, .. } => Err(Error::HelperFailed {
            helper,
            registry,
            code: status,
        }),
        Ran::NotFound => Err(Error::HelperNotFound { helper, registry }),
        Ran::TimedOut => Err(Error::HelperTimedOut {
            helper,
            registry,
            secs: timeout.as_secs(),
        }),
        Ran::Interrupted | Ran::Failed(_) => Err(Error::HelperFailed {
            helper,
            registry,
            code: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const HOST: &str = "reg.example:5000";
    const SECRET: &str = "s1-7f3a9c-secret";

    /// A config directory holding `config` (or no file), removed on drop.
    struct Dir(PathBuf);

    impl Dir {
        fn with(config: Option<&str>) -> Dir {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "lvpm-docker-config-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(config) = config {
                std::fs::write(dir.join("config.json"), config).unwrap();
            }
            Dir(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    /// What `run` was asked: program and host.
    type Asked = Arc<Mutex<Vec<(String, String)>>>;

    fn exited(status: i32, stdout: &str) -> Ran {
        Ran::Exited {
            status: Some(status),
            stdout: stdout.as_bytes().to_vec(),
            stderr: format!("token {SECRET} locked").into_bytes(),
        }
    }

    fn gives(user: &str, secret: &str) -> Ran {
        let answer = serde_json::json!({ "ServerURL": HOST, "Username": user, "Secret": secret });
        exited(0, &answer.to_string())
    }

    /// The store of `config`, with helpers answered by `answer`; also what it asked.
    fn store(config: Option<&str>, answer: fn(&str) -> Ran) -> (Credentials, Asked, Dir) {
        let dir = Dir::with(config);
        let asked = Asked::default();
        let run: HelperRunner = {
            let asked = asked.clone();
            Arc::new(move |helper, host, _| {
                asked.lock().unwrap().push((helper.into(), host.into()));
                answer(helper)
            })
        };
        let credentials = docker_store(dir.0.clone(), Duration::from_secs(7), run);
        (credentials, asked, dir)
    }

    fn named(helper: &str) -> Ran {
        gives(&format!("user-of-{helper}"), SECRET)
    }

    fn basic(origin: Origin, user: &str, secret: &str) -> Option<Credential> {
        Some(Credential::basic(origin, user, secret))
    }

    #[test]
    fn credhelpers_for_the_host_wins_over_credsstore_and_that_over_auths() {
        let config = format!(
            r#"{{"credHelpers":{{"{HOST}":"first","other.example":"third"}},
                "credsStore":"second","auths":{{"{HOST}":{{"auth":"dTM6czM="}}}}}}"#
        );
        let (all, asked, _dir) = store(Some(&config), named);
        assert_eq!(
            all(HOST).unwrap(),
            basic(
                Origin::CredHelpers("first".into()),
                "user-of-docker-credential-first",
                SECRET
            )
        );
        assert_eq!(
            asked.lock().unwrap().as_slice(),
            [("docker-credential-first".to_string(), HOST.to_string())],
            "the helper is run with the host on its stdin, and no other is"
        );

        let config = format!(
            r#"{{"credHelpers":{{"other.example":"third"}},"credsStore":"second",
                "auths":{{"{HOST}":{{"auth":"dTM6czM="}}}}}}"#
        );
        let (second, _, _dir) = store(Some(&config), named);
        assert_eq!(
            second(HOST).unwrap(),
            basic(
                Origin::CredsStore("second".into()),
                "user-of-docker-credential-second",
                SECRET
            )
        );

        let config = format!(r#"{{"auths":{{"{HOST}":{{"auth":"dTM6czM="}}}}}}"#);
        let (auths, asked, _dir) = store(Some(&config), named);
        assert_eq!(auths(HOST).unwrap(), basic(Origin::Auths, "u3", "s3"));
        assert!(asked.lock().unwrap().is_empty(), "no helper is configured");
    }

    #[test]
    fn an_auths_key_may_be_a_url_of_the_host_and_a_password_may_hold_a_colon() {
        // base64 of "u:p:q"
        let config = format!(r#"{{"auths":{{"https://{HOST}/v1/":{{"auth":"dTpwOnE="}}}}}}"#);
        let (all, _, _dir) = store(Some(&config), named);
        assert_eq!(all(HOST).unwrap(), basic(Origin::Auths, "u", "p:q"));
        assert_eq!(all("other.example").unwrap(), None);
    }

    #[test]
    fn no_config_or_no_login_in_it_is_anonymous_and_runs_nothing() {
        let not_base64 = format!(r#"{{"auths":{{"{HOST}":{{"auth":"!!"}}}}}}"#);
        let no_colon = format!(r#"{{"auths":{{"{HOST}":{{"auth":"dTM="}}}}}}"#);
        let no_auth = format!(r#"{{"auths":{{"{HOST}":{{}}}}}}"#);
        for config in [
            None,
            Some("{}"),
            Some(r#"{"credHelpers":{"other.example":"x"},"credsStore":""}"#),
            Some(not_base64.as_str()),
            Some(no_colon.as_str()),
            Some(no_auth.as_str()),
        ] {
            let (all, asked, _dir) = store(config, named);
            assert_eq!(all(HOST).unwrap(), None, "{config:?}");
            assert!(asked.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn a_helper_that_holds_nothing_for_the_host_means_anonymous_even_with_auths_beside_it() {
        let config = format!(r#"{{"credsStore":"s","auths":{{"{HOST}":{{"auth":"dTM6czM="}}}}}}"#);
        let not_found = |_: &str| exited(1, "credentials not found in native keychain\r\n");
        let (all, _, _dir) = store(Some(&config), not_found);
        assert_eq!(all(HOST).unwrap(), None);
    }

    #[test]
    fn the_helper_user_name_token_makes_an_identity_token() {
        let (all, _, _dir) = store(Some(r#"{"credsStore":"s"}"#), |_| gives("<token>", SECRET));
        assert_eq!(
            all(HOST).unwrap(),
            Some(Credential::identity(Origin::CredsStore("s".into()), SECRET))
        );
    }

    #[test]
    fn a_helper_that_fails_is_not_found_is_too_slow_or_unreadable_says_so_without_its_output() {
        let config = Some(r#"{"credHelpers":{"reg.example:5000":"h"}}"#);
        let said = |answer: fn(&str) -> Ran| store(config, answer).0(HOST).unwrap_err();
        let h = format!("credential helper docker-credential-h for {HOST}");
        let by_hand = "run it by hand to see why";
        let no_code = |_: &str| Ran::Exited {
            status: None,
            stdout: vec![],
            stderr: vec![],
        };
        let table = [
            (
                said(|_| exited(3, "")),
                format!("{h} failed with exit code 3; {by_hand}"),
            ),
            (said(no_code), format!("{h} failed; {by_hand}")),
            (said(|_| Ran::Interrupted), format!("{h} failed; {by_hand}")),
            (
                said(|_| Ran::NotFound),
                format!("{h} was not found; check that it is installed and on PATH"),
            ),
            (
                said(|_| Ran::TimedOut),
                format!("{h} did not answer within 7s"),
            ),
            (
                said(|_| exited(0, "not json")),
                format!("{h} gave an answer lvpm cannot read; {by_hand}"),
            ),
        ];
        for (error, text) in table {
            assert_eq!(error.to_string(), text);
            assert_eq!(error.registry(), HOST);
            assert!(!format!("{error:?}{error}").contains(SECRET), "{error:?}");
        }
    }

    #[test]
    fn a_config_that_is_not_json_is_named_without_quoting_it() {
        let (all, _, _dir) = store(Some(&format!(r#"{{"auths": "{SECRET}" }}"#)), named);
        let error = all(HOST).unwrap_err().to_string();
        assert!(
            error.contains("not a Docker config (line 1, column"),
            "{error}"
        );
        assert!(!error.contains(SECRET), "{error}");
    }

    #[test]
    fn debug_shows_where_a_login_came_from_and_nothing_else() {
        let found = Credential::basic(Origin::CredsStore("s".into()), "u1-user", SECRET);
        let shown = format!("{found:?} {found:#?}");
        assert!(shown.contains("CredsStore"), "{shown}");
        assert!(
            !shown.contains(SECRET) && !shown.contains("u1-user"),
            "{shown}"
        );
        let token = Credential::identity(Origin::Auths, SECRET);
        assert!(!format!("{token:?}").contains(SECRET));
    }

    #[test]
    fn an_origin_reads_as_the_login_refused_message_names_it() {
        assert_eq!(
            Origin::CredHelpers("n".into()).to_string(),
            "credHelpers docker-credential-n"
        );
        assert_eq!(
            Origin::CredsStore("n".into()).to_string(),
            "credsStore docker-credential-n"
        );
        assert_eq!(Origin::Auths.to_string(), "auths");
    }

    #[test]
    fn a_helper_gets_thirty_seconds_unless_told_otherwise() {
        assert_eq!(DEFAULT_HELPER_TIMEOUT, Duration::from_secs(30));
    }
}
