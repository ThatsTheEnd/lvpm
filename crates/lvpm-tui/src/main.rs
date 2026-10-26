//! `lvpm-tui`: lvpm in a terminal user interface.

use clap::Parser;
use lvpm_tui::app::{self, App};
use ratatui::crossterm::event;

#[derive(Parser)]
#[command(version, about)]
struct Cli {}

fn main() -> std::io::Result<()> {
    Cli::parse();
    // Made before the terminal is set up, so a set-up that fails half way is
    // undone too.
    let _restore = Restore(ratatui::restore);
    let mut terminal = ratatui::try_init()?;
    app::run(&mut terminal, &mut App::new(), event::read)
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
