//! Tells an interactive user when a newer stable lvpm is out. The check runs
//! on a thread beside the command, at most once a day, and is cached in the
//! `cache` folder. A check still running when the command ends gets a short
//! grace period, then is abandoned rather than delaying the exit.

use anyhow::{Context, Result};
use lvpm_core::version::Version;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// shortcut: points at upstream's releases, which are numbered below this fork's, so it
// stays quiet until upstream passes our version; point it at the fork's own once it publishes.
const RELEASES_API: &str =
    "https://api.github.com/repos/labview-package-manager/lvpm/releases?per_page=30";
const RELEASES_PAGE: &str = "https://github.com/labview-package-manager/lvpm/releases";
const CACHE_FILE: &str = "update-check.json";
/// How long a completed check, successful or not, is reused.
const INTERVAL: u64 = 24 * 60 * 60;
/// How long a check that was started but never completed (the command exited
/// first) blocks the next attempt.
const RETRY: u64 = 60 * 60;
const TIMEOUT: Duration = Duration::from_secs(2);
/// How long a finished command waits for a check that is still running.
const GRACE: Duration = Duration::from_millis(150);

#[derive(Clone, Default, Deserialize, Serialize)]
struct Cached {
    checked_at: u64,
    latest: Option<String>,
}

impl Cached {
    fn fresh(&self, now: u64) -> bool {
        self.checked_at <= now && now - self.checked_at < INTERVAL
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

pub struct UpdateNotice {
    current: String,
    latest: Option<String>,
    worker: Option<JoinHandle<Option<Cached>>>,
}

/// Start a check, unless the run is not interactive or the user opted out.
pub fn start(cache: &Path) -> Option<UpdateNotice> {
    let forced = test_override("LVPM_TEST_TERMINAL").is_some();
    if !enabled(
        forced || std::io::stdout().is_terminal(),
        forced || std::io::stderr().is_terminal(),
        |key| std::env::var_os(key).is_some(),
    ) {
        return None;
    }
    prepare(cache.join(CACHE_FILE)).ok()
}

fn enabled(stdout_terminal: bool, stderr_terminal: bool, env_set: impl Fn(&str) -> bool) -> bool {
    stdout_terminal
        && stderr_terminal
        && !["LVPM_NO_UPDATE_CHECK", "CI", "LV_RTE_HEADLESS"]
            .iter()
            .any(|key| env_set(key))
}

/// Tests only, and only in a debug build (what `cargo test` runs):
/// `LVPM_TEST_TERMINAL` makes lvpm treat its output as a terminal, and
/// `LVPM_TEST_RELEASES_URL` replaces the GitHub releases API (and makes the
/// command wait for the check), so the notice can be driven end to end
/// without a pseudoterminal or GitHub. A release build never reads them.
#[cfg(debug_assertions)]
fn test_override(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

#[cfg(not(debug_assertions))]
fn test_override(_key: &str) -> Option<String> {
    None
}

fn prepare(path: PathBuf) -> Result<UpdateNotice> {
    let cached = read_cache(&path);
    let checked_at = now()?;
    let fresh = cached.as_ref().is_some_and(|c| c.fresh(checked_at));
    let mut notice = UpdateNotice {
        current: env!("CARGO_PKG_VERSION").to_string(),
        latest: cached.and_then(|c| c.latest),
        worker: None,
    };
    if !fresh {
        let url = test_override("LVPM_TEST_RELEASES_URL").unwrap_or_else(|| RELEASES_API.into());
        notice.worker = Some(
            std::thread::Builder::new()
                .name("update-check".into())
                .spawn(move || {
                    now()
                        .and_then(|now| refresh(&path, &url, now, TIMEOUT))
                        .ok()
                        .flatten()
                })?,
        );
    }
    Ok(notice)
}

impl UpdateNotice {
    /// Print the notice, if any. Call it only after a successful command.
    pub fn finish(self) {
        let current = self.current.clone();
        let grace = match test_override("LVPM_TEST_RELEASES_URL") {
            Some(_) => TIMEOUT + Duration::from_secs(1),
            None => GRACE,
        };
        if let Some(latest) = self.available(grace) {
            eprintln!(
                "\nA newer lvpm is available: {current} -> {latest}\n\
                 Run the installation script again, or download the new release, to upgrade.\n\
                 {RELEASES_PAGE}"
            );
        }
    }

    fn available(mut self, grace: Duration) -> Option<String> {
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + grace;
            while !worker.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if worker.is_finished()
                && let Ok(Some(cached)) = worker.join()
            {
                self.latest = cached.latest;
            }
        }
        let current = stable(&self.current);
        self.latest
            .filter(|v| stable(v).is_release() && stable(v) > current)
    }
}

/// A version without its `+build` part, which never decides which is newer.
fn stable(v: &str) -> Version {
    Version::parse(v.split('+').next().unwrap_or(v))
}

fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn read_cache(path: &Path) -> Option<Cached> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Only called under the lock, so one fixed temp name is enough, and a temp
/// file left by an interrupted write is overwritten next time.
fn write_cache(path: &Path, cached: &Cached) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(cached)?;
    std::fs::write(&tmp, bytes)
        .and_then(|_| std::fs::rename(&tmp, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
        .with_context(|| format!("writing update cache {}", path.display()))
}

fn refresh(path: &Path, url: &str, now: u64, timeout: Duration) -> Result<Option<Cached>> {
    let dir = path
        .parent()
        .context("update cache has no parent directory")?;
    std::fs::create_dir_all(dir)?;
    let lock: File = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let mut cached = read_cache(path).unwrap_or_default();
    if cached.fresh(now) {
        return Ok(Some(cached));
    }
    // Record the attempt first: most commands exit before the request
    // completes, and without this every such run would send another one.
    // The back-dated time makes an abandoned check retry after RETRY, not
    // after a full INTERVAL.
    write_cache(
        path,
        &Cached {
            checked_at: now.saturating_sub(INTERVAL - RETRY),
            latest: cached.latest.clone(),
        },
    )?;
    if let Ok(latest) = fetch_latest(url, timeout) {
        cached.latest = latest;
    }
    // Cache completed failures for a full interval too, so offline runs and
    // rate limits do not trigger a request on every command.
    cached.checked_at = now;
    write_cache(path, &cached)?;
    Ok(Some(cached))
}

fn fetch_latest(url: &str, timeout: Duration) -> Result<Option<String>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("lvpm/", env!("CARGO_PKG_VERSION")))
        .timeout(timeout)
        .build()?;
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()?
        .error_for_status()?;
    let releases: Vec<Release> =
        serde_json::from_slice(&response.bytes()?).context("reading GitHub releases")?;
    Ok(latest_release(releases))
}

/// The newest stable release: drafts, releases marked as prereleases and
/// versions with a prerelease part (`-rc.1`) do not count.
fn latest_release(releases: Vec<Release>) -> Option<String> {
    releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .map(|r| {
            r.tag_name
                .strip_prefix('v')
                .unwrap_or(&r.tag_name)
                .to_string()
        })
        .filter(|tag| tag.starts_with(|c: char| c.is_ascii_digit()) && stable(tag).is_release())
        .max_by_key(|tag| stable(tag))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn cache() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CACHE_FILE);
        (dir, path)
    }

    fn server(status: &str, body: &str) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/releases", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

    fn notice(
        current: &str,
        latest: Option<&str>,
        worker: Option<JoinHandle<Option<Cached>>>,
    ) -> UpdateNotice {
        UpdateNotice {
            current: current.into(),
            latest: latest.map(String::from),
            worker,
        }
    }

    #[test]
    fn only_stable_releases_count_and_versions_compare_numerically() {
        let releases = serde_json::from_str(
            r#"[
            {"tag_name":"v0.9.9","draft":false},
            {"tag_name":"v0.10.0","draft":false},
            {"tag_name":"v0.11.0-rc.1","draft":false},
            {"tag_name":"v0.12.0","draft":false,"prerelease":true},
            {"tag_name":"v99.0.0","draft":true},
            {"tag_name":"not-a-version","draft":false}
        ]"#,
        )
        .unwrap();
        assert_eq!(latest_release(releases).as_deref(), Some("0.10.0"));
        let releases =
            serde_json::from_str(r#"[{"tag_name":"v0.10.0-rc.1","draft":false}]"#).unwrap();
        assert!(latest_release(releases).is_none());
    }

    #[test]
    fn only_interactive_runs_without_an_opt_out_are_enabled() {
        assert!(enabled(true, true, |_| false));
        assert!(!enabled(false, true, |_| false));
        assert!(!enabled(true, false, |_| false));
        for key in ["LVPM_NO_UPDATE_CHECK", "CI", "LV_RTE_HEADLESS"] {
            assert!(!enabled(true, true, |env| env == key), "{key}");
        }
    }

    #[test]
    fn checks_are_persisted_and_reused_for_exactly_one_day() {
        let (_dir, path) = cache();
        let (url, server) = server("200 OK", r#"[{"tag_name":"v0.4.0","draft":false}]"#);
        let now = INTERVAL * 10;
        let first = refresh(&path, &url, now, TIMEOUT).unwrap().unwrap();
        server.join().unwrap();
        assert_eq!(first.latest.as_deref(), Some("0.4.0"));
        // The server is gone: a fresh cache must not attempt a request.
        let reused = refresh(&path, &url, now + INTERVAL - 1, TIMEOUT)
            .unwrap()
            .unwrap();
        assert_eq!(reused.checked_at, now);
        // A failed check keeps the previous notice and is cached a day too.
        let retried = refresh(&path, &url, now + INTERVAL, TIMEOUT)
            .unwrap()
            .unwrap();
        assert_eq!(retried.checked_at, now + INTERVAL);
        assert_eq!(retried.latest.as_deref(), Some("0.4.0"));
    }

    #[test]
    fn an_abandoned_check_is_retried_after_an_hour_not_on_every_run() {
        let (_dir, path) = cache();
        // A server that accepts but never answers: the check is in flight,
        // as it is when a short command exits.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/releases", listener.local_addr().unwrap());
        let (release, wait) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(10));
        });
        let now = INTERVAL * 10;
        let p = path.clone();
        let check = std::thread::spawn(move || refresh(&p, &url, now, Duration::from_secs(10)));
        let deadline = Instant::now() + Duration::from_secs(5);
        let claimed = loop {
            if let Some(c) = read_cache(&path) {
                break c;
            }
            assert!(
                Instant::now() < deadline,
                "the check was not recorded first"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(claimed.fresh(now + RETRY - 1));
        assert!(!claimed.fresh(now + RETRY));
        release.send(()).unwrap();
        server.join().unwrap();
        check.join().unwrap().unwrap();
    }

    #[test]
    fn a_running_check_delays_the_cached_notice_by_at_most_the_grace_period() {
        let (release, wait) = mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            let _ = wait.recv_timeout(Duration::from_secs(5));
            None
        });
        let started = Instant::now();
        let latest = notice("0.3.0", Some("0.4.0"), Some(worker)).available(GRACE);
        let elapsed = started.elapsed();
        release.send(()).unwrap();
        assert_eq!(latest.as_deref(), Some("0.4.0"));
        assert!(elapsed >= GRACE, "{elapsed:?}");
        // Well short of the worker's 5 s, with room for a loaded CI machine.
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    }

    #[test]
    fn installed_older_and_prerelease_versions_do_not_notify() {
        for latest in [
            "0.3.0",
            "0.2.9",
            "0.3.0-rc.1",
            "0.3.0+new-build",
            "0.4.0-rc.1",
        ] {
            assert!(
                notice("0.3.0+installed-build", Some(latest), None)
                    .available(GRACE)
                    .is_none(),
                "{latest}"
            );
        }
        // A prerelease user is told about the stable release it led to.
        assert_eq!(
            notice("0.3.0-rc.1", Some("0.3.0"), None)
                .available(GRACE)
                .as_deref(),
            Some("0.3.0")
        );
    }
}
