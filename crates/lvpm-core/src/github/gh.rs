//! Running `gh attestation verify` on a downloaded archive.
//!
//! Core reads no environment: the front end installs [`real_runner`] on
//! [`GitHub`](super::GitHub). What the run came to is a [`GhRun`]; what that
//! means for the release is `attestation`'s to say.

use crate::verify::NotChecked;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

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
        run(
            &mut verify_command(archive, repo, token),
            GH_TIMEOUT,
            MAX_OUTPUT_BYTES,
        )
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

/// `command` run to its end within `limit`, keeping at most `cap` bytes of each
/// output. Not `auth.rs`'s `run`: that one drops stderr, and a failed
/// verification is told in stderr.
fn run(command: &mut Command, limit: Duration, cap: u64) -> GhRun {
    let mut child = match command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return GhRun::NotRunnable(NotChecked::GhMissing);
        }
        Err(error) => return failed(&error),
    };
    let stdout = read_capped(child.stdout.take().expect("piped above"), cap);
    let stderr = read_capped(child.stderr.take().expect("piped above"), cap);
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => {
                // One grace covers both readers seeing EOF. Output that has not
                // come by then (a grandchild holding a pipe open) is not gh's
                // answer; shortcut: that reader thread is left behind, kill
                // gh's process tree if it is ever seen.
                let grace = Instant::now() + Duration::from_secs(1);
                let left = || grace.saturating_duration_since(Instant::now());
                let (Ok(stdout), Ok(stderr)) =
                    (stdout.recv_timeout(left()), stderr.recv_timeout(left()))
                else {
                    return GhRun::NotRunnable(NotChecked::GhInterrupted);
                };
                return GhRun::Spawned {
                    status: status.code(),
                    stdout,
                    stderr,
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                child.kill().ok();
                child.wait().ok();
                return failed(&error);
            }
        }
    }
    child.kill().ok();
    child.wait().ok();
    GhRun::NotRunnable(NotChecked::GhTimedOut)
}

fn failed(error: &std::io::Error) -> GhRun {
    GhRun::NotRunnable(NotChecked::GhFailed {
        error: error.to_string(),
    })
}

/// What `pipe` yields, the first `cap` bytes of it, once it ends. Read as it is
/// written and drained past the cap, so output past the pipe's buffer cannot
/// stall the child.
fn read_capped(mut pipe: impl Read + Send + 'static, cap: u64) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.by_ref().take(cap).read_to_end(&mut bytes).ok();
        std::io::copy(&mut pipe, &mut std::io::sink()).ok();
        sender.send(bytes).ok();
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(60);

    fn cargo(args: &[&str]) -> Command {
        let mut command = Command::new("cargo");
        command.args(args);
        command
    }

    #[test]
    fn a_run_keeps_stdout_and_the_status() {
        let GhRun::Spawned {
            status,
            stdout,
            stderr,
        } = run(&mut cargo(&["--version"]), LONG, MAX_OUTPUT_BYTES)
        else {
            panic!("cargo runs");
        };
        assert_eq!(status, Some(0));
        assert!(stdout.starts_with(b"cargo "));
        assert!(stderr.is_empty());
    }

    #[test]
    fn a_failing_run_keeps_stderr() {
        let failed = run(
            &mut cargo(&["no-such-subcommand-lvpm"]),
            LONG,
            MAX_OUTPUT_BYTES,
        );
        let GhRun::Spawned { status, stderr, .. } = failed else {
            panic!("cargo runs");
        };
        assert!(status.is_some_and(|code| code != 0));
        assert!(!stderr.is_empty(), "stderr is kept");
    }

    #[test]
    fn a_missing_program_is_gh_missing_and_a_slow_one_timed_out() {
        let missing = run(
            &mut Command::new("no-such-program-lvpm"),
            LONG,
            MAX_OUTPUT_BYTES,
        );
        assert_eq!(missing, GhRun::NotRunnable(NotChecked::GhMissing));

        let slow = run(&mut cargo(&["--version"]), Duration::ZERO, MAX_OUTPUT_BYTES);
        assert_eq!(slow, GhRun::NotRunnable(NotChecked::GhTimedOut));
    }

    #[test]
    fn output_past_the_cap_is_dropped_not_stalled_on() {
        let GhRun::Spawned { stdout, .. } = run(&mut cargo(&["--version"]), LONG, 5) else {
            panic!("cargo runs");
        };
        assert_eq!(stdout, b"cargo");
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
