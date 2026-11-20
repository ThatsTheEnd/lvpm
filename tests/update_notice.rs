//! The update notice, end to end, through the lvpm binary. A debug build of
//! lvpm takes `LVPM_TEST_TERMINAL` as "the output is a terminal" and
//! `LVPM_TEST_RELEASES_URL` in place of the GitHub releases API, so no
//! pseudoterminal and no GitHub access is needed: each test has its own cache
//! folder and, where it checks for releases, its own local server.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const NOTICE: &str = "A newer lvpm is available";

struct Lab {
    root: PathBuf,
}

impl Lab {
    fn new(name: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "lvpm-update-notice-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cache")).unwrap();
        Self { root }
    }

    fn cache_file(&self) -> PathBuf {
        self.root.join("cache").join("update-check.json")
    }

    fn write_cache(&self, checked_at: u64, latest: &str) -> String {
        let state = serde_json::json!({ "checked_at": checked_at, "latest": latest }).to_string();
        std::fs::write(self.cache_file(), &state).unwrap();
        state
    }

    fn cache(&self) -> String {
        std::fs::read_to_string(self.cache_file()).unwrap()
    }

    fn lock_exists(&self) -> bool {
        self.root.join("cache").join("update-check.lock").exists()
    }

    /// Run lvpm with every opt-out cleared, plus `env`.
    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lvpm"));
        command
            .args(args)
            .current_dir(&self.root)
            .env("LVPM_CONFIG_CACHE", self.root.join("cache"))
            .env("LVPM_CONFIG_LOG_DIR", self.root.join("logs"))
            .env("LOCALAPPDATA", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home"))
            .env("HOME", self.root.join("home"))
            .stdin(Stdio::null());
        for key in [
            "CI",
            "LV_RTE_HEADLESS",
            "LVPM_NO_UPDATE_CHECK",
            "LVPM_LOG",
            "LVPM_TEST_TERMINAL",
            "LVPM_TEST_RELEASES_URL",
        ] {
            command.env_remove(key);
        }
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().unwrap()
    }

    /// Run as if in a terminal; it must succeed. Returns stderr.
    fn interactive(&self, args: &[&str], env: &[(&str, &str)]) -> String {
        let mut env = env.to_vec();
        env.push(("LVPM_TEST_TERMINAL", "1"));
        let out = self.run(args, &env);
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(out.status.success(), "lvpm {args:?}: {out:?}\n{stderr}");
        stderr
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Answers one request with `body`, then closes, so a later request fails.
fn releases_server(body: &str) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/releases", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    );
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        stream.write_all(response.as_bytes()).unwrap();
    });
    (url, thread)
}

#[test]
fn a_cached_newer_release_is_shown_in_a_terminal_and_respects_opt_outs() {
    let lab = Lab::new("cached");
    let state = lab.write_cache(now(), "9999.0.0");

    let stderr = lab.interactive(&["config", "get", "cache"], &[]);
    let notice = format!("{NOTICE}: {} -> 9999.0.0", env!("CARGO_PKG_VERSION"));
    assert_eq!(stderr.matches(&notice).count(), 1, "{stderr}");
    has(&stderr, "Run the installation script again");
    has(
        &stderr,
        "https://github.com/labview-package-manager/lvpm/releases",
    );

    for env in [
        ("LVPM_NO_UPDATE_CHECK", "1"),
        ("LVPM_NO_UPDATE_CHECK", ""),
        ("CI", "1"),
        ("LV_RTE_HEADLESS", "1"),
    ] {
        lacks(
            &lab.interactive(&["config", "get", "cache"], &[env]),
            NOTICE,
        );
    }
    for arg in ["--help", "--version"] {
        lacks(&lab.interactive(&[arg], &[]), NOTICE);
    }
    assert_eq!(lab.cache(), state);
    assert!(!lab.lock_exists());
}

#[test]
fn a_failed_command_shows_no_notice() {
    let lab = Lab::new("failed");
    lab.write_cache(now(), "9999.0.0");
    let out = lab.run(
        &["config", "get", "no-such-key"],
        &[("LVPM_TEST_TERMINAL", "1")],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{out:?}");
    has(&stderr, "unknown config key");
    lacks(&stderr, NOTICE);
}

#[test]
fn redirected_commands_neither_check_nor_notify() {
    let lab = Lab::new("redirected");
    let state = lab.write_cache(0, "9999.0.0");
    for args in [
        vec!["config", "get", "cache"],
        vec!["--help"],
        vec!["--version"],
    ] {
        let out = lab.run(&args, &[]);
        assert!(out.status.success(), "{args:?}: {out:?}");
        assert!(out.stderr.is_empty(), "{args:?}: {out:?}");
        let stdout = String::from_utf8(out.stdout).unwrap();
        lacks(&stdout, NOTICE);
        if args[0] == "config" {
            assert_eq!(stdout.trim(), lab.root.join("cache").display().to_string());
        }
        assert_eq!(lab.cache(), state, "{args:?}");
        assert!(!lab.lock_exists(), "{args:?}");
    }
}

#[test]
fn a_check_finds_the_newest_stable_release_and_is_reused_for_a_day() {
    let lab = Lab::new("check");
    let (url, server) = releases_server(
        r#"[
            {"tag_name":"v99999.0.0-rc.1","draft":false,"prerelease":true},
            {"tag_name":"v99998.0.0","draft":false,"prerelease":true},
            {"tag_name":"v99997.0.0","draft":true,"prerelease":false},
            {"tag_name":"v9999.0.0","draft":false,"prerelease":false},
            {"tag_name":"v0.0.1","draft":false,"prerelease":false}
        ]"#,
    );
    let env = [("LVPM_TEST_RELEASES_URL", url.as_str())];

    let stderr = lab.interactive(&["config", "get", "cache"], &env);
    server.join().unwrap();
    let notice = format!("{NOTICE}: {} -> 9999.0.0", env!("CARGO_PKG_VERSION"));
    assert_eq!(stderr.matches(&notice).count(), 1, "{stderr}");
    let state = lab.cache();
    let cached: serde_json::Value = serde_json::from_str(&state).unwrap();
    assert_eq!(cached["latest"], "9999.0.0", "{state}");

    // The server is gone. The cache is fresh, so there is no request, and
    // the notice comes from the cache.
    let stderr = lab.interactive(&["config", "get", "cache"], &env);
    assert_eq!(stderr.matches(&notice).count(), 1, "{stderr}");
    assert_eq!(lab.cache(), state);
}

#[track_caller]
fn has(text: &str, part: &str) {
    assert!(text.contains(part), "expected {part:?} in:\n{text}");
}

#[track_caller]
fn lacks(text: &str, part: &str) {
    assert!(!text.contains(part), "did not expect {part:?} in:\n{text}");
}
