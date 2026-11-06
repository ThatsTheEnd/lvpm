//! Running a child program to its end: the one runner `gh` and a Docker
//! credential helper share.
//!
//! It knows nothing of either. What a run came to is a [`Ran`]; what that
//! means (gh could not check, a helper holds no login) is the caller's to say.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// How a run of a program came out.
#[derive(Debug)]
pub enum Ran {
    /// It ran to its end. `status` is its exit code; `None` when a signal ended it.
    Exited {
        status: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
    /// There is no such program.
    NotFound,
    /// It was still running at the limit, and was killed.
    TimedOut,
    /// It ended, but its output did not: a grandchild holds a pipe open.
    Interrupted,
    /// It could not be started or waited for, for this reason.
    Failed(std::io::Error),
}

/// `command` run to its end within `limit`, with `stdin` on its standard
/// input (none when `None`), keeping at most `cap` bytes of each output.
pub fn run(command: &mut Command, stdin: Option<&[u8]>, limit: Duration, cap: u64) -> Ran {
    let mut child = match command
        .stdin(match stdin {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ran::NotFound,
        Err(error) => return Ran::Failed(error),
    };
    // Written on its own thread, so a child that never reads cannot stall us;
    // dropping the pipe at the end is the child's end of input.
    if let (Some(bytes), Some(mut pipe)) = (stdin.map(<[u8]>::to_vec), child.stdin.take()) {
        std::thread::spawn(move || pipe.write_all(&bytes).ok());
    }
    let stdout = read_capped(child.stdout.take().expect("piped above"), cap);
    let stderr = read_capped(child.stderr.take().expect("piped above"), cap);
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => {
                // One grace covers both readers seeing EOF. Output that has not
                // come by then (a grandchild holding a pipe open) is not the
                // program's answer; shortcut: that reader thread is left
                // behind, kill the program's process tree if it is ever seen.
                let grace = Instant::now() + Duration::from_secs(1);
                let left = || grace.saturating_duration_since(Instant::now());
                let (Ok(stdout), Ok(stderr)) =
                    (stdout.recv_timeout(left()), stderr.recv_timeout(left()))
                else {
                    return Ran::Interrupted;
                };
                return Ran::Exited {
                    status: status.code(),
                    stdout,
                    stderr,
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                child.kill().ok();
                child.wait().ok();
                return Ran::Failed(error);
            }
        }
    }
    child.kill().ok();
    child.wait().ok();
    Ran::TimedOut
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
    const CAP: u64 = 1024 * 1024;
    /// Set in the copy of the test program that `echoes_stdin` starts.
    const ECHO_VARIABLE: &str = "LVPM_PROCESS_TEST_ECHO";

    fn cargo(args: &[&str]) -> Command {
        let mut command = Command::new("cargo");
        command.args(args);
        command
    }

    #[test]
    fn a_run_keeps_stdout_and_the_status() {
        let Ran::Exited {
            status,
            stdout,
            stderr,
        } = run(&mut cargo(&["--version"]), None, LONG, CAP)
        else {
            panic!("cargo runs");
        };
        assert_eq!(status, Some(0));
        assert!(stdout.starts_with(b"cargo "));
        assert!(stderr.is_empty());
    }

    #[test]
    fn a_failing_run_keeps_stderr() {
        let failed = run(&mut cargo(&["no-such-subcommand-lvpm"]), None, LONG, CAP);
        let Ran::Exited { status, stderr, .. } = failed else {
            panic!("cargo runs");
        };
        assert!(status.is_some_and(|code| code != 0));
        assert!(!stderr.is_empty(), "stderr is kept");
    }

    #[test]
    fn a_missing_program_is_not_found_and_a_slow_one_timed_out() {
        let missing = run(&mut Command::new("no-such-program-lvpm"), None, LONG, CAP);
        assert!(matches!(missing, Ran::NotFound), "{missing:?}");

        let slow = run(&mut cargo(&["--version"]), None, Duration::ZERO, CAP);
        assert!(matches!(slow, Ran::TimedOut), "{slow:?}");
    }

    #[test]
    fn output_past_the_cap_is_dropped_not_stalled_on() {
        let Ran::Exited { stdout, .. } = run(&mut cargo(&["--version"]), None, LONG, 5) else {
            panic!("cargo runs");
        };
        assert_eq!(stdout, b"cargo");
    }

    /// The child half of `a_program_reads_the_stdin_it_is_given`: when the
    /// parent runs this test program with the variable set, say what stdin held.
    #[test]
    #[allow(clippy::disallowed_methods)] // a test, and the variable is its own
    fn echoes_stdin() {
        if std::env::var_os(ECHO_VARIABLE).is_none() {
            return;
        }
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).ok();
        std::io::stdout()
            .write_all(format!("echo:{text}:end").as_bytes())
            .ok();
    }

    #[test]
    fn a_program_reads_the_stdin_it_is_given() {
        let me = std::env::current_exe().expect("this test program");
        let mut command = Command::new(me);
        command
            .args(["--exact", "process::tests::echoes_stdin", "--nocapture"])
            .env(ECHO_VARIABLE, "1");
        let Ran::Exited { stdout, .. } = run(&mut command, Some(b"registry:5000"), LONG, CAP)
        else {
            panic!("the test program runs");
        };
        let said = String::from_utf8_lossy(&stdout);
        assert!(said.contains("echo:registry:5000:end"), "{said}");
    }
}
