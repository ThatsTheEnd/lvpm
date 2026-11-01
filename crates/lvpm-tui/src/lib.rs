//! lvpm-tui: a terminal user interface for lvpm, on `lvpm-core` alone.
//!
//! `app` holds the state and the event loop, `ui` draws it, and `keys` is the
//! one table of keys the footer and the dispatch read.
//! `browse` is the package list, `detail` one package's details,
//! `destination` where installs go, `plan` an install's preview,
//! `progress` how far it has got, `log` its record, and
//! `worker` runs core's operations off the UI thread.

pub mod app;
pub mod browse;
pub mod destination;
pub mod detail;
pub mod keys;
pub mod log;
pub mod plan;
pub mod progress;
pub mod ui;
pub mod worker;

use lvpm_core::github::{GitHub, gh_auth_token};
use lvpm_core::{config, target};
use ratatui::crossterm::event;
use std::time::Duration;

/// How long the loop waits for a key before it looks for the worker's result.
const TICK: Duration = Duration::from_millis(100);

/// Run the TUI in this terminal until it quits: what `lvpm tui` does.
// The front end is where process state is read; the workspace-wide
// `clippy.toml` bars it from `lvpm-core` only.
#[allow(clippy::disallowed_methods)]
pub fn run() -> anyhow::Result<()> {
    // Read before the terminal is taken, so an error prints as usual.
    let mut builder =
        destination::EnvBuilder::new(config::load()?, target::detect()?, std::env::current_dir()?);
    builder.github = GitHub::from_lookup(|name| std::env::var(name).ok(), gh_auth_token);
    let mut app = app::App::new(builder);
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    app.log_file = local_app_data.as_deref().map(log::path);
    // Made before the terminal is set up, so a set-up that fails half way is
    // undone too.
    let _restore = Restore(ratatui::restore);
    let mut terminal = ratatui::try_init()?;
    app::run(&mut terminal, &mut app, || {
        event::poll(TICK)?.then(event::read).transpose()
    })?;
    Ok(())
}

/// Puts the terminal back (raw mode off, main screen) when dropped, which
/// also happens while a panic unwinds; ratatui's panic hook does it too.
struct Restore(fn());

impl Drop for Restore {
    fn drop(&mut self) {
        (self.0)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static RESTORED: AtomicUsize = AtomicUsize::new(0);

    fn count_restore() {
        RESTORED.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn the_terminal_is_restored_on_drop_and_on_a_panic() {
        drop(Restore(count_restore));
        assert_eq!(RESTORED.load(Ordering::SeqCst), 1);
        let unwound = std::panic::catch_unwind(|| {
            let _restore = Restore(count_restore);
            panic!("a panic inside the TUI");
        });
        assert!(unwound.is_err());
        assert_eq!(RESTORED.load(Ordering::SeqCst), 2);
    }
}
