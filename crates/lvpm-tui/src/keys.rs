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

/// Ctrl-C alone: it still quits while the search box takes `q` as a letter.
pub const INTERRUPT: Binding = Binding {
    key: "Ctrl-C",
    action: "quit",
    presses: &[(KeyCode::Char('c'), KeyModifiers::CONTROL)],
};

pub const SEARCH: Binding = Binding {
    key: "/",
    action: "search",
    presses: &[(KeyCode::Char('/'), KeyModifiers::NONE)],
};

pub const FILTER: Binding = Binding {
    key: "f",
    action: "filter",
    presses: &[(KeyCode::Char('f'), KeyModifiers::NONE)],
};

/// Shift-r: terminals report the capital with Shift, some without.
pub const RELOAD: Binding = Binding {
    key: "R",
    action: "reload feeds",
    presses: &[
        (KeyCode::Char('R'), KeyModifiers::SHIFT),
        (KeyCode::Char('R'), KeyModifiers::NONE),
    ],
};

pub const INSTALL: Binding = Binding {
    key: "i",
    action: "install",
    presses: &[(KeyCode::Char('i'), KeyModifiers::NONE)],
};

pub const UP: Binding = Binding {
    key: "↑",
    action: "up",
    presses: &[(KeyCode::Up, KeyModifiers::NONE)],
};

pub const DOWN: Binding = Binding {
    key: "↓",
    action: "down",
    presses: &[(KeyCode::Down, KeyModifiers::NONE)],
};

pub const PAGE_UP: Binding = Binding {
    key: "PgUp",
    action: "page up",
    presses: &[(KeyCode::PageUp, KeyModifiers::NONE)],
};

pub const PAGE_DOWN: Binding = Binding {
    key: "PgDn",
    action: "page down",
    presses: &[(KeyCode::PageDown, KeyModifiers::NONE)],
};

pub const HOME: Binding = Binding {
    key: "Home",
    action: "first",
    presses: &[(KeyCode::Home, KeyModifiers::NONE)],
};

pub const END: Binding = Binding {
    key: "End",
    action: "last",
    presses: &[(KeyCode::End, KeyModifiers::NONE)],
};

/// The keys the footer shows, in order.
pub const FOOTER: &[&Binding] = &[&SEARCH, &FILTER, &RELOAD, &QUIT];
