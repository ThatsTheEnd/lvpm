//! The token for GitHub's API: `GH_TOKEN`, else `GITHUB_TOKEN`, else what
//! `gh auth token` prints.
//!
//! Core reads no environment variable (`clippy.toml`): the front end hands
//! [`GitHubAuth`] the variable lookup and the `gh` runner. The chain runs on the
//! first use, so a run with no GitHub dependency never starts `gh`. The token
//! is shown nowhere: `Debug` prints `***`.

use std::fmt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How long `gh auth token` may take: it reads a local file or the keyring.
const GH_TIMEOUT: Duration = Duration::from_secs(10);
/// What `gh auth token` may print before it is not a token.
const MAX_OUTPUT_BYTES: u64 = 4096;

/// Where the token came from, as an error text names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    GhToken,
    GitHubToken,
    GhAuthToken,
}

impl TokenSource {
    /// The environment variable the token is read from; `None` for `gh`.
    fn variable(self) -> Option<&'static str> {
        match self {
            TokenSource::GhToken => Some("GH_TOKEN"),
            TokenSource::GitHubToken => Some("GITHUB_TOKEN"),
            TokenSource::GhAuthToken => None,
        }
    }
}

impl fmt::Display for TokenSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.variable().unwrap_or("gh auth token"))
    }
}

type Lookup = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
type GhRunner = Box<dyn Fn() -> Option<String> + Send + Sync>;

/// The token GitHub calls are made with, if there is one. A clone shares the
/// chain, so the token is looked for once whoever asks first.
#[derive(Clone)]
pub struct GitHubAuth(Arc<Chain>);

struct Chain {
    lookup: Lookup,
    gh: GhRunner,
    resolved: OnceLock<Option<(String, TokenSource)>>,
}

impl GitHubAuth {
    /// `lookup` gives the value of an environment variable; `gh` gives what
    /// `gh auth token` printed, or `None` when `gh` is missing or failed (see
    /// [`gh_auth_token`]).
    pub fn new(
        lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
        gh: impl Fn() -> Option<String> + Send + Sync + 'static,
    ) -> GitHubAuth {
        GitHubAuth(Arc::new(Chain {
            lookup: Box::new(lookup),
            gh: Box::new(gh),
            resolved: OnceLock::new(),
        }))
    }

    /// No token, and nothing to ask for one: every call is anonymous.
    pub fn anonymous() -> GitHubAuth {
        GitHubAuth::new(|_| None, || None)
    }

    /// The token, found on the first call. A blank (empty or whitespace-only)
    /// variable counts as unset.
    pub fn token(&self) -> Option<&str> {
        self.resolve().as_ref().map(|(token, _)| token.as_str())
    }

    /// Where [`GitHubAuth::token`] found the token.
    pub fn source(&self) -> Option<TokenSource> {
        self.resolve().as_ref().map(|(_, source)| *source)
    }

    fn resolve(&self) -> &Option<(String, TokenSource)> {
        let chain = &*self.0;
        chain.resolved.get_or_init(|| {
            [TokenSource::GhToken, TokenSource::GitHubToken]
                .into_iter()
                .find_map(|source| {
                    let name = source.variable()?;
                    Some((filled((chain.lookup)(name))?, source))
                })
                .or_else(|| Some((filled((chain.gh)())?, TokenSource::GhAuthToken)))
        })
    }
}

/// `value` trimmed, unless nothing is left.
fn filled(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

impl fmt::Debug for GitHubAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// What `gh auth token` prints: `None` when `gh` is not installed, exits
/// non-zero or takes longer than [`GH_TIMEOUT`].
pub fn gh_auth_token() -> Option<String> {
    run(Command::new("gh").args(["auth", "token"]), GH_TIMEOUT)
}

/// The standard output of `command` when it succeeds within `limit`.
fn run(command: &mut Command, limit: Duration) -> Option<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Read as it is written, so output past the pipe's buffer cannot stall the
    // child; a token is a few dozen bytes, and nothing longer is let through.
    let stdout = child.stdout.take()?;
    let (sender, output) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        // One byte over the cap tells a too-long output from one that fits.
        let read = stdout.take(MAX_OUTPUT_BYTES + 1).read_to_string(&mut text);
        sender
            .send(
                read.ok()
                    .filter(|_| text.len() as u64 <= MAX_OUTPUT_BYTES)
                    .map(|_| text),
            )
            .ok();
    });
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                // gh has exited; the grace covers the reader seeing EOF, and stops a
                // grandchild that kept the pipe open from holding us.
                return output.recv_timeout(Duration::from_secs(1)).ok().flatten();
            }
            Ok(Some(_)) => return None,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break,
        }
    }
    child.kill().ok();
    child.wait().ok();
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// What `gh auth token` does in a row of the outline.
    enum Gh {
        Answering(&'static str),
        Failing,
        Missing,
    }

    fn auth(gh_token: Option<&str>, github_token: Option<&str>, gh: Gh) -> GitHubAuth {
        let variables = [
            ("GH_TOKEN", gh_token.map(str::to_string)),
            ("GITHUB_TOKEN", github_token.map(str::to_string)),
        ];
        GitHubAuth::new(
            move |name| {
                variables
                    .iter()
                    .find(|(have, _)| *have == name)
                    .and_then(|(_, value)| value.clone())
            },
            move || match gh {
                Gh::Answering(text) => Some(text.to_string()),
                Gh::Failing | Gh::Missing => None,
            },
        )
    }

    #[test]
    fn the_token_is_the_first_of_gh_token_github_token_and_gh_auth_token() {
        use Gh::{Answering, Failing, Missing};
        use TokenSource::{GhAuthToken, GhToken, GitHubToken};
        let table = [
            (Some("a"), Some("b"), Answering("c"), Some(("a", GhToken))),
            (None, Some("b"), Answering("c"), Some(("b", GitHubToken))),
            (
                Some(""),
                Some("b"),
                Answering("c"),
                Some(("b", GitHubToken)),
            ),
            (
                Some("  "),
                Some("b"),
                Answering("c"),
                Some(("b", GitHubToken)),
            ),
            (None, None, Answering("c\n"), Some(("c", GhAuthToken))),
            (None, None, Answering(""), None),
            (None, None, Failing, None),
            (None, None, Missing, None),
        ];
        for (gh_token, github_token, gh, want) in table {
            let auth = auth(gh_token, github_token, gh);
            let got = auth.token().zip(auth.source());
            assert_eq!(
                got, want,
                "GH_TOKEN {gh_token:?}, GITHUB_TOKEN {github_token:?}"
            );
        }
    }

    #[test]
    fn the_chain_runs_on_first_use_once_and_gh_only_when_no_variable_has_a_token() {
        let (lookups, runs) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let auth = {
            let (lookups, runs) = (lookups.clone(), runs.clone());
            GitHubAuth::new(
                move |_| {
                    lookups.fetch_add(1, Ordering::SeqCst);
                    None
                },
                move || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    Some("c".to_string())
                },
            )
        };
        assert_eq!(
            (lookups.load(Ordering::SeqCst), runs.load(Ordering::SeqCst)),
            (0, 0)
        );
        assert_eq!(auth.token(), Some("c"));
        assert_eq!(auth.token(), Some("c"));
        assert_eq!(
            (lookups.load(Ordering::SeqCst), runs.load(Ordering::SeqCst)),
            (2, 1)
        );

        runs.store(0, Ordering::SeqCst);
        let from_variable = {
            let runs = runs.clone();
            GitHubAuth::new(
                |name| (name == "GH_TOKEN").then(|| "a".to_string()),
                move || {
                    runs.fetch_add(1, Ordering::SeqCst);
                    None
                },
            )
        };
        assert_eq!(from_variable.token(), Some("a"));
        assert_eq!(
            runs.load(Ordering::SeqCst),
            0,
            "gh is not asked when a variable has it"
        );
    }

    #[test]
    fn debug_shows_no_token_and_the_source_names_read_as_the_error_texts_need() {
        let auth = auth(Some("s3cret"), None, Gh::Missing);
        assert_eq!(auth.token(), Some("s3cret"));
        assert_eq!(format!("{auth:?}"), "***");
        assert_eq!(format!("{auth:#?}"), "***");
        let names: Vec<String> = [
            TokenSource::GhToken,
            TokenSource::GitHubToken,
            TokenSource::GhAuthToken,
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(names, ["GH_TOKEN", "GITHUB_TOKEN", "gh auth token"]);
    }

    #[test]
    fn a_command_gives_its_output_unless_it_is_missing_fails_or_is_too_slow() {
        let cargo = || Command::new("cargo");
        let ok = run(cargo().arg("--version"), Duration::from_secs(60));
        assert!(ok.is_some_and(|text| text.starts_with("cargo ")));
        assert_eq!(
            run(
                cargo().arg("no-such-subcommand-lvpm"),
                Duration::from_secs(60)
            ),
            None
        );
        assert_eq!(
            run(
                cargo().args(["metadata", "--offline", "--format-version", "1"]),
                Duration::from_secs(60)
            ),
            None,
            "more output than a token can be"
        );
        assert_eq!(
            run(
                &mut Command::new("no-such-program-lvpm"),
                Duration::from_secs(60)
            ),
            None
        );
        assert_eq!(
            run(cargo().arg("--version"), Duration::ZERO),
            None,
            "killed at the limit"
        );
    }
}
