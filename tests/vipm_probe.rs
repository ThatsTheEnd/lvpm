//! Dev-only probe of what VIPM needs from a `.vip`: installs variants of
//! lvpm's build of pilot A through `vipm.exe` and records what VIPM does.
//! Only `probe_vipm` touches VIPM or LabVIEW, and it is `#[ignore]`; every
//! other test here runs on synthetic data.
//!
//! `cargo test --test vipm_probe -- --ignored --nocapture`, with
//! `LVPM_TEST_VIPM` (vipm.exe) and `LVPM_TEST_PILOT_A` (pilot A's checkout).
//! The findings go to `target/vipm-probes/<date>T<time>-vipm-<version>/`.

// Test code reads the test process's environment; the workspace-wide
// `clippy.toml` bars that from `lvpm-core` only.
#![allow(clippy::disallowed_methods)]

#[path = "vipm_probe/run.rs"]
mod run;
mod support;
#[path = "vipm_probe/table.rs"]
mod table;
#[path = "vipm_probe/variants.rs"]
mod variants;
#[path = "vipm_probe/vipm.rs"]
mod vipm;

use run::{Guard, Probe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use support::events::RecordingSink;
use support::global;
use support::pilots::{PILOT_A_ARCHIVE, archives, copy_tree, pilots_dir};
use support::project::MANIFEST_FILE;
use support::snapshot::{self, LABVIEW_NOISE};
use variants::Pkg;
use vipm::{Timeouts, Vipm};

const LABVIEW_YEAR: u32 = 2026;
const LABVIEW_EXE: &str = "LabVIEW.exe";

/// Pilot A's `lvpm.toml` as S1's scenario writes it: its repository has none.
const PILOT_A_MANIFEST: &str = r#"[project]
name = "hse_lib_hse_logger"
version = "2.2.2.114"
labview = ">=2016"

[distribution]
top-level-library = "Source/HSE-Logger/hse-logger.lvlib"

[distribution.vip]
vendor = "Hampel Software Engineering"
display-name = "HSE Logger"
"#;

#[test]
#[ignore = "drives the real vipm.exe and the installed LabVIEW 2026: set LVPM_TEST_VIPM and LVPM_TEST_PILOT_A"]
fn probe_vipm() {
    match probe() {
        Ok(summary) => eprintln!("{summary}"),
        Err(why) => panic!("{why}"),
    }
}

/// The run, top to bottom; the summary line when it finishes.
fn probe() -> Result<String, String> {
    let env = vipm::check_env(&|name| std::env::var_os(name))?;
    let labview = labview_tree()?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let probes = repo.join("target").join("vipm-probes");
    let state_file = probes.join(run::STATE_FILE);

    let runner = vipm::real_runner(env.vipm);
    let vipm = Vipm::new(&runner, Timeouts::REAL);
    let me = std::process::id();
    let start = global::process_start(me).ok_or("this run's own start time cannot be read")?;
    let ini = std::fs::read_to_string(labview.join("LabVIEW.ini")).unwrap_or_default();
    let _guard = Guard::take(
        &vipm,
        &state_file,
        &global::claim_path(),
        (me, &start),
        &global::process_start,
        (
            global::vi_server_port(&ini),
            &support::process::port_is_listening,
        ),
    )?;

    let base = build_pilot_a(&env.pilot_a)?;
    let published = published_pilots()?;
    let reference_file = pilots_dir().join(PILOT_A_ARCHIVE);
    let reference = Pkg::read(&read(&reference_file)?)?;
    let (variants, not_varied) = variants::variants(&base, &reference, &published)?;
    let spec =
        lvpm_core::spec::parse(&base.spec()).map_err(|e| format!("pilot A's spec: {e:#}"))?;
    if spec.name != vipm::PILOT_A {
        return Err(format!(
            "pilot A's package is {}, not {}",
            spec.name,
            vipm::PILOT_A
        ));
    }

    let vipm_version = vipm.version()?;
    let header = table::Header {
        vipm: vipm_version.clone(),
        labview: labview_file_version(&labview)?,
    };
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let out = probes.join(table::run_folder(&table::utc_stamp(seconds), &vipm_version));
    eprintln!("{}", vipm::start_line(variants.len()));
    let git_before = git_status(repo)?;
    let tree_before = scan_scope(&labview)?;

    let archive_name = format!("{}-{}.vip", spec.name, spec.version);
    let probe = Probe {
        vipm: &vipm,
        labview: &labview,
        out: &out,
        state_file: &state_file,
        name: &spec.name,
        version: &spec.version,
        archive_name: &archive_name,
    };
    let rows = run::run(&probe, &reference_file, &base, &variants)?;

    run::final_tree_check(&labview, &tree_before, &scan_scope(&labview)?)?;
    run::git_status_unchanged(&git_before, &git_status(repo)?)?;
    let reference_diff = variants::reference_diff(&base, &reference);
    let table_file = out.join("table.md");
    let text = table::render(&header, &rows, &reference_diff, &not_varied);
    run::write_text(&table_file, &text)?;
    Ok(table::summary_line(&rows, &table_file))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))
}

fn scan_scope(labview: &Path) -> Result<snapshot::Snapshot, String> {
    snapshot::scan(labview, Some(&run::SCOPE), &LABVIEW_NOISE)
}

/// The LabVIEW 2026 installation's folder.
fn labview_tree() -> Result<PathBuf, String> {
    let targets = lvpm_core::target::detect().map_err(|e| format!("detecting LabVIEW: {e:#}"))?;
    targets
        .iter()
        .find(|t| t.year() == LABVIEW_YEAR)
        .map(|t| t.path.clone())
        .ok_or_else(|| format!("no LabVIEW {LABVIEW_YEAR} installation found"))
}

/// The file version of the LabVIEW 2026 `LabVIEW.exe`.
fn labview_file_version(labview: &Path) -> Result<String, String> {
    let exe = labview
        .join(LABVIEW_EXE)
        .display()
        .to_string()
        .replace('\'', "''");
    let script = format!("(Get-Item -LiteralPath '{exe}').VersionInfo.FileVersion");
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("reading the version of {LABVIEW_EXE}: {e}"))?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(version)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{LABVIEW_EXE} has no file version"))
}

/// The pilot archives VIPM built, which the variants are compared with.
fn published_pilots() -> Result<Vec<Pkg>, String> {
    archives(&pilots_dir())
        .iter()
        .map(|path| Pkg::read(&read(path)?))
        .collect()
}

/// Pilot A as lvpm builds it, from a copy of its checkout so the checkout is not touched.
fn build_pilot_a(checkout: &Path) -> Result<Pkg, String> {
    let work = tempfile::tempdir().map_err(|e| format!("a temporary folder: {e}"))?;
    let dir = work.path().join("pilot-a");
    copy_tree(&checkout.join("Source"), &dir.join("Source"));
    std::fs::write(dir.join(MANIFEST_FILE), PILOT_A_MANIFEST)
        .map_err(|e| format!("writing pilot A's manifest: {e}"))?;
    let config = lvpm_core::config::Config {
        cache: work.path().join("cache"),
        local_sources: Vec::new(),
        venv_dir: PathBuf::from(".lv-venv"),
    };
    let sink = RecordingSink::default();
    let env = lvpm_core::ops::OpEnv::new(&sink, config, Vec::new(), dir);
    let built =
        lvpm_core::ops::build::build(&env).map_err(|e| format!("lvpm build of pilot A: {e:#}"))?;
    Pkg::read(&read(&built.path)?)
}

/// `git status --porcelain` of the repository.
fn git_status(repo: &Path) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| format!("git status: {e}"))?;
    match out.status.success() {
        true => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        false => Err(format!(
            "git status failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )),
    }
}
