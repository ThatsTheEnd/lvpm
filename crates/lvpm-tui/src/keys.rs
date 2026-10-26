//! The keys the TUI acts on. The footer, the help screen and the key dispatch
//! all read this table, so a key is named and bound in one place.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A key and what it does.
#[derive(Debug)]
pub struct Binding {
    /// How the footer and the help screen name the key.
    pub key: &'static str,
    /// What the key does, in a word or two.
    pub action: &'static str,
    presses: &'static [(KeyCode, KeyModifiers)],
}

impl Binding {
    pub fn matches(&self, event: KeyEvent) -> bool {
        self.presses
            .iter()
            .any(|&(code, modifiers)| event.code == code && event.modifiers == modifiers)
    }
}

/// `q`, or Ctrl-C: in raw mode Ctrl-C is a key like any other.
pub const QUIT: Binding = Binding {
    key: "q",
    action: "quit",
    presses: &[
        (KeyCode::Char('q'), KeyModifiers::NONE),
        (KeyCode::Char('c'), KeyModifiers::CONTROL),
    ],
};

/// The keys the footer shows, in order.
pub const FOOTER: &[&Binding] = &[&QUIT];
