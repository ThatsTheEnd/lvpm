//! Running `gh attestation verify` on a downloaded archive.
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

/// What runs gh: given the archive, `owner/repo` and the token.
pub type Runner = Arc<dyn Fn(&Path, &str, &str) -> GhRun + Send + Sync>;

/// The runner a [`GitHub`](super::GitHub) starts with: gh is never run.
pub(super) fn missing() -> Runner {
    Arc::new(|_, _, _| GhRun::NotRunnable(NotChecked::GhMissing))
}

/// `gh attestation verify <archive> --repo <owner/repo> --format json`, the
/// token handed over as `GH_TOKEN`.
pub fn real_runner() -> Runner {
    Arc::new(|archive, repo, token| {
        process::run(
            &mut verify_command(archive, repo, token),
            None,
            GH_TIMEOUT,
            MAX_OUTPUT_BYTES,
        )
        .into()
    })
}

fn verify_command(archive: &Path, repo: &str, token: &str) -> Command {
    let mut command = Command::new("gh");
    command
        .args(["attestation", "verify"])
        .arg(archive)
        .args(["--repo", repo, "--format", "json"])
        .env("GH_TOKEN", token);
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
            missing()(Path::new("a.lvpkg"), "o/r", "tok"),
            GhRun::NotRunnable(NotChecked::GhMissing)
        );
    }

    #[test]
    fn gh_is_asked_to_verify_the_archive_for_the_repo_with_the_token_in_its_environment() {
        let command = verify_command(Path::new("blob"), "o/r", "tok");
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
}
