//! lvpm-tui: a terminal user interface for lvpm, on `lvpm-core` alone.
//!
//! `app` holds the state and the event loop, `ui` draws it, and `keys` is the
//! one table of keys the footer, the help screen and the dispatch read.
//! `browse` is the package list, `detail` one package's details,
//! `destination` where installs go, and
//! `worker` runs core's operations off the UI thread.

pub mod app;
pub mod browse;
pub mod destination;
pub mod detail;
pub mod keys;
pub mod ui;
pub mod worker;
