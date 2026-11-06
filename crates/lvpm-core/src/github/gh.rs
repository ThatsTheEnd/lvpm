//! Running `gh attestation verify` on a downloaded archive, or on the OCI
//! manifest of a package with the bundle that signs it.
//!
//! Core reads no environment: the front end installs [`real_runner`] on
//! [`GitHub`](super::GitHub). What the run came to is a [`GhRun`]; what that
//! means for the release is `attestation`'s to say.

use crate::process::{self, Ran};
use crate::verify::NotChecked;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

/// How long `gh attestation verify` may take: it asks GitHub and sigstore.
const GH_TIMEOUT: Duration = Duration::from_secs(60);
/// How much of gh's standard output, and of its standard error, is kept.
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

/// How a run of gh came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhRun {
    /// gh ran to its end. `status` is its exit code; `None` when a signal ended it.
    Spawned {
        status: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    /// gh could not give an answer, for this reason.
    NotRunnable(NotChecked),
}

/// What one run of `gh attestation verify` is asked. It holds the token, so it
/// has no `Debug`.
#[derive(Clone, Copy)]
pub struct GhVerify<'a> {
    /// The file whose SHA-256 the attestation names: a release archive, or the
    /// bytes of an OCI manifest.
    pub artifact: &'a Path,
    /// `owner/repo` that built it.
    pub repo: &'a str,
    pub token: &'a str,
    /// The Sigstore bundle to verify against, for an artifact whose
    /// attestations are not looked up on GitHub.
    pub bundle: Option<&'a Path>,
}

/// What runs gh.
pub type Runner = Arc<dyn for<'a> Fn(GhVerify<'a>) -> GhRun + Send + Sync>;

/// The runner a [`GitHub`](super::GitHub) starts with: gh is never run.
pub(super) fn missing() -> Runner {
    Arc::new(|_| GhRun::NotRunnable(NotChecked::GhMissing))
}

/// `gh attestation verify <archive> [--bundle <file>] --repo <owner/repo>
/// --format json`, the token handed over as `GH_TOKEN`.
pub fn real_runner() -> Runner {
    Arc::new(|ask| {
        process::run(&mut verify_command(ask), None, GH_TIMEOUT, MAX_OUTPUT_BYTES).into()
    })
}

fn verify_command(ask: GhVerify) -> Command {
    let mut command = Command::new("gh");
    command.args(["attestation", "verify"]).arg(ask.artifact);
    if let Some(bundle) = ask.bundle {
        command.arg("--bundle").arg(bundle);
    }
    command
        .args(["--repo", ask.repo, "--format", "json"])
        .env("GH_TOKEN", ask.token);
    command
}

impl From<Ran> for GhRun {
    fn from(ran: Ran) -> GhRun {
        match ran {
            Ran::Exited {
                status,
                stdout,
                stderr,
            } => GhRun::Spawned {
                status,
                stdout,
                stderr,
            },
            Ran::NotFound => GhRun::NotRunnable(NotChecked::GhMissing),
            Ran::TimedOut => GhRun::NotRunnable(NotChecked::GhTimedOut),
            Ran::Interrupted => GhRun::NotRunnable(NotChecked::GhInterrupted),
            Ran::Failed(error) => GhRun::NotRunnable(NotChecked::GhFailed {
                error: error.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_run_came_to_is_what_gh_did_or_why_it_could_not_say() {
        let exited = GhRun::from(Ran::Exited {
            status: Some(1),
            stdout: b"out".to_vec(),
            stderr: b"err".to_vec(),
        });
        assert_eq!(
            exited,
            GhRun::Spawned {
                status: Some(1),
                stdout: b"out".to_vec(),
                stderr: b"err".to_vec()
            }
        );
        assert_eq!(
            GhRun::from(Ran::NotFound),
            GhRun::NotRunnable(NotChecked::GhMissing)
        );
        assert_eq!(
            GhRun::from(Ran::TimedOut),
            GhRun::NotRunnable(NotChecked::GhTimedOut)
        );
        assert_eq!(
            GhRun::from(Ran::Interrupted),
            GhRun::NotRunnable(NotChecked::GhInterrupted)
        );
        assert_eq!(
            GhRun::from(Ran::Failed(std::io::Error::other("denied"))),
            GhRun::NotRunnable(NotChecked::GhFailed {
                error: "denied".into()
            })
        );
    }

    #[test]
    fn the_missing_runner_never_runs_gh() {
        assert_eq!(
            missing()(ask(None)),
            GhRun::NotRunnable(NotChecked::GhMissing)
        );
    }

    fn ask(bundle: Option<&Path>) -> GhVerify<'_> {
        GhVerify {
            artifact: Path::new("blob"),
            repo: "o/r",
            token: "tok",
            bundle,
        }
    }

    #[test]
    fn gh_is_asked_to_verify_the_archive_for_the_repo_with_the_token_in_its_environment() {
        let command = verify_command(ask(None));
        assert_eq!(command.get_program(), "gh");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "attestation",
                "verify",
                "blob",
                "--repo",
                "o/r",
                "--format",
                "json"
            ]
        );
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(envs, [("GH_TOKEN".as_ref(), Some("tok".as_ref()))]);
    }

    #[test]
    fn a_bundle_is_handed_to_gh_after_the_artifact() {
        let command = verify_command(ask(Some(Path::new("sig"))));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "attestation",
                "verify",
                "blob",
                "--bundle",
                "sig",
                "--repo",
                "o/r",
                "--format",
                "json"
            ]
        );
    }
}
