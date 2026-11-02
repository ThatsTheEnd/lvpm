//! Running `gh attestation verify` on a downloaded archive.
//!
//! Core reads no environment: the front end installs [`real_runner`] on
//! [`GitHub`](super::GitHub). What the run came to is a [`GhRun`]; what that
//! means for the release is `attestation`'s to say.

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
    NotRunnable(String),
}

/// What runs gh: given the archive, `owner/repo` and the token.
pub type Runner = Arc<dyn Fn(&Path, &str, &str) -> GhRun + Send + Sync>;

/// The runner a [`GitHub`](super::GitHub) starts with: gh is never run.
pub(super) fn missing() -> Runner {
    Arc::new(|_, _, _| GhRun::NotRunnable("gh is not installed".to_string()))
}

/// `gh attestation verify <archive> --repo <owner/repo> --format json`, the
/// token handed over as `GH_TOKEN`.
pub fn real_runner() -> Runner {
    Arc::new(|archive, repo, token| {
        let mut command = Command::new("gh");
        command
            .args(["attestation", "verify"])
            .arg(archive)
            .args(["--repo", repo, "--format", "json"])
            .env("GH_TOKEN", token);
        run(&mut command, GH_TIMEOUT, MAX_OUTPUT_BYTES)
    })
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
            return GhRun::NotRunnable("gh is not installed".to_string());
        }
        Err(error) => return GhRun::NotRunnable(format!("gh could not be started: {error}")),
    };
    let stdout = read_capped(child.stdout.take().expect("piped above"), cap);
    let stderr = read_capped(child.stderr.take().expect("piped above"), cap);
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => {
                // The grace covers the readers seeing EOF, and stops a grandchild
                // that kept a pipe open from holding us.
                let grace = Duration::from_secs(1);
                return GhRun::Spawned {
                    status: status.code(),
                    stdout: stdout.recv_timeout(grace).unwrap_or_default(),
                    stderr: stderr.recv_timeout(grace).unwrap_or_default(),
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break,
        }
    }
    child.kill().ok();
    child.wait().ok();
    GhRun::NotRunnable("gh timed out".to_string())
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
    fn a_run_gives_status_stdout_and_stderr_unless_gh_is_missing_or_too_slow() {
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

        let missing = run(
            &mut Command::new("no-such-program-lvpm"),
            LONG,
            MAX_OUTPUT_BYTES,
        );
        assert_eq!(missing, GhRun::NotRunnable("gh is not installed".into()));

        let slow = run(&mut cargo(&["--version"]), Duration::ZERO, MAX_OUTPUT_BYTES);
        assert_eq!(slow, GhRun::NotRunnable("gh timed out".into()));
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
            GhRun::NotRunnable("gh is not installed".into())
        );
    }
}
