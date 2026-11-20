//! lvpm's run log: every command appends what it did to a file in the
//! `log.dir` folder (`%LOCALAPPDATA%\lvpm\logs` by default). A command on a
//! venv writes the venv's own file, named after its project folder (see
//! [`venv_file_name`]); any other command writes `lvpm.log`.
//!
//! Each run writes a header (version, arguments, folder), every event core
//! emitted as JSON, which holds more than the console shows (each file linkfix
//! touched, for one), and how it ended. A line is appended as it happens, so a
//! run cut off by Ctrl+C keeps what it logged. Logging never fails a command:
//! a log that cannot be written is reported once and dropped.

use lvpm_core::events::{Event, EventSink};
use lvpm_core::venv::Venv;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// The log of commands not run on a venv.
pub const FILE_NAME: &str = "lvpm.log";
/// Longest readable part of a venv's log name, before its hash.
const NAME_MAX: usize = 80;

/// Passes each event on to `inner` and appends it to the log file.
pub struct LogSink {
    inner: Box<dyn EventSink>,
    /// `None` until [`LogSink::attach`], and again after a write failed.
    path: Mutex<Option<PathBuf>>,
}

impl LogSink {
    pub fn new(inner: Box<dyn EventSink>) -> Self {
        LogSink {
            inner,
            path: Mutex::new(None),
        }
    }

    /// Write this run's log to `path` from now on, starting with its header.
    pub fn attach(&self, path: PathBuf) {
        *self.lock() = Some(path);
        let args: Vec<String> = std::env::args().skip(1).map(|a| quote(&a)).collect();
        let cwd = std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        self.write(&format!(
            "---- lvpm {} {} (in {cwd})",
            env!("CARGO_PKG_VERSION"),
            args.join(" ")
        ));
    }

    /// Log how the run ended.
    pub fn finish<T>(&self, result: &anyhow::Result<T>) {
        match result {
            Ok(_) => self.write("---- done"),
            Err(e) => self.write(&format!("---- failed: {e:#}")),
        }
    }

    fn write(&self, text: &str) {
        let mut path = self.lock();
        let Some(p) = path.as_ref() else { return };
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let head = format!(
            "{} {:>6}",
            lvpm_tui::log::timestamp(secs),
            std::process::id()
        );
        let lines: Vec<String> = text.lines().map(|l| format!("{head} {l}")).collect();
        if let Err(e) = lvpm_tui::log::append(p, &lines) {
            eprintln!("note: cannot write the lvpm log {}: {e}", p.display());
            *path = None;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<PathBuf>> {
        self.path.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl EventSink for LogSink {
    fn emit(&self, event: Event) {
        if let Ok(json) = serde_json::to_string(&event) {
            self.write(&json);
        }
        self.inner.emit(event);
    }
}

/// The log file of a command: the venv's own, or `lvpm.log`.
pub fn path(log_dir: &Path, venv: Option<&Venv>) -> PathBuf {
    match venv {
        Some(v) => {
            let key = v.dir.to_string_lossy().to_lowercase().replace('/', "\\");
            log_dir.join(venv_file_name(
                &v.repo,
                lvpm_core::venv_icon::fnv1a(key.trim_end_matches('\\')),
            ))
        }
        None => log_dir.join(FILE_NAME),
    }
}

fn quote(a: &str) -> String {
    if a.is_empty() || a.contains(char::is_whitespace) {
        format!("\"{a}\"")
    } else {
        a.to_string()
    }
}

/// A venv's log name: its project's path, lowercased, with `:`, `\` and `/`
/// as `-`, cut from the left to [`NAME_MAX`] characters so the project's own
/// folder name survives, then a hash of the venv's path, which keeps two
/// paths apart that read the same (`C:\a-b\c`, `C:\a\b-c`) or were cut.
///
/// `C:\Git\lvpm` → `c--git-lvpm-3fa2c1d0.log`
fn venv_file_name(repo: &Path, hash: u64) -> String {
    let readable: String = repo
        .to_string_lossy()
        .to_lowercase()
        .chars()
        .map(|c| {
            if matches!(c, ':' | '\\' | '/') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let chars: Vec<char> = readable.chars().collect();
    let mut tail: String = chars[chars.len().saturating_sub(NAME_MAX)..]
        .iter()
        .collect();
    // Cut: start at a whole folder name, not the end of one.
    if chars.len() > NAME_MAX
        && let Some(i) = tail.find('-')
    {
        tail.drain(..=i);
    }
    format!("{}-{:08x}.log", tail.trim_start_matches('-'), hash as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venv_log_names_read_as_the_project_path() {
        let name = |p: &str| venv_file_name(Path::new(p), 0x1234_5678_3fa2_c1d0);
        assert_eq!(name(r"C:\Git\lvpm"), "c--git-lvpm-3fa2c1d0.log");
        assert_eq!(
            name(r"D:\Projects\Test Rig\Station 2"),
            "d--projects-test rig-station 2-3fa2c1d0.log"
        );
        let deep = format!(r"C:\{}my-app", "nested\\".repeat(20));
        let cut = name(&deep);
        assert!(
            cut.starts_with("nested-nested-") && cut.ends_with("nested-my-app-3fa2c1d0.log"),
            "{cut}"
        );
        assert!(
            cut.trim_end_matches("-3fa2c1d0.log").chars().count() <= NAME_MAX,
            "{cut}"
        );
    }

    #[test]
    fn a_run_logs_its_header_events_and_end() {
        struct Quiet;
        impl EventSink for Quiet {
            fn emit(&self, _: Event) {}
        }
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("sub").join(FILE_NAME);
        let sink = LogSink::new(Box::new(Quiet));
        sink.emit(Event::LinkfixPreparing); // before attach: passed on, not logged
        sink.attach(log.clone());
        sink.emit(Event::LinkfixPreparing);
        sink.finish::<()>(&Err(anyhow::anyhow!("boom")));
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[0].contains("---- lvpm "), "{text}");
        assert!(
            lines[1].ends_with(r#"{"event":"linkfix_preparing"}"#),
            "{text}"
        );
        assert!(lines[2].ends_with("---- failed: boom"), "{text}");
    }
}
