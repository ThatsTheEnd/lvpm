//! Dev-only probe of what VIPM needs from a `.vip`: installs variants of
//! lvpm's build of pilot A through `vipm.exe` and records what VIPM does.
//! Only `probe_vipm` touches VIPM or LabVIEW, and it is `#[ignore]`; every
//! other test here runs on synthetic data.

// Test code reads the test process's environment; the workspace-wide
// `clippy.toml` bars that from `lvpm-core` only.
#![allow(clippy::disallowed_methods)]
// Removed with step 1.5, when the run uses everything the earlier steps built.
#![allow(dead_code)]

#[path = "vipm_probe/run.rs"]
mod run;
mod support;
#[path = "vipm_probe/table.rs"]
mod table;
#[path = "vipm_probe/variants.rs"]
mod variants;
#[path = "vipm_probe/vipm.rs"]
mod vipm;
