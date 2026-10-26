//! lvpm-tui: a terminal user interface for lvpm, on `lvpm-core` alone.
//!
//! `app` holds the state and the event loop, `ui` draws it, and `keys` is the
//! one table of keys the footer, the help screen and the dispatch read.

pub mod app;
pub mod keys;
pub mod ui;
