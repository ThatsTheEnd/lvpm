//! Test-only helpers shared by the root test suites and lvpm-tui's: the
//! fixture packages, the local feed that serves them, and the event sink that
//! records what core emits. Nothing here ships.

// Test code reads the test process's environment; `clippy.toml` bars that
// from `lvpm-core` only.
#![allow(clippy::disallowed_methods)]

pub mod events;
pub mod feed;
pub mod fixtures;

/// A scenario with a step that starts with this runs in lvpm-tui's harness;
/// the root harness runs every other scenario of a shared feature file.
pub const TUI_STEP_PREFIX: &str = "the TUI ";
