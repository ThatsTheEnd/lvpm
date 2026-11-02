//! Steps of the `lvpm check` scenarios: a project in the work directory made
//! from the fixture VIs, a venv bound to an installation that is a folder in
//! the sandbox, and what `lvpm check` printed. LabVIEW is never detected or
//! started: the binding and the install records are written by hand.

use crate::CliWorld;
use crate::support::golden::Output;
use crate::support::pilots::pilots_dir;
use crate::support::project::{MANIFEST_FILE, ProjectManifest};
use crate::support::venv::predicted_port;
use cucumber::{given, then, when};
use lvpm_core::linker::Rsrc;
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The VIs the scenarios copy into a project, by the names they are given.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/check/project");
/// The folder the venv is bound to, in the sandbox. It holds an empty `vi.lib`
/// until a scenario says otherwise.
const INSTALLATION: &str = "LabVIEW 2026";
/// The one NI file `UsesNi.vi` links to, below the installation.
const NI_FILE: &str = "vi.lib/Utility/Get Current LV Bitness.vi";
/// A VI saved in LabVIEW 2013, taken out of the committed pilot archive that holds it.
const PILOT_VI_2013: &str = "the pilot VI saved in LabVIEW 2013";
const PILOT_ARCHIVE: &str = "labview_open_source_lib_data_manipulation-1.6.0.16.vip";
const PILOT_VI: &str =
    "File Group 0/vi.lib/LabVIEW Open Source Project/Data Manipulation/KVP Parser.vi";
/// The package a scenario's "installed package" is.
const PACKAGE: &str = "check_fixture_dep";

impl CliWorld {
    fn installation(&self) -> PathBuf {
        self.sandbox.root().join(INSTALLATION)
    }

    /// A project: an `lvpm.toml` with `labview` (a `>=YYYY` as written), and
    /// the files `holding` lists. With `venv`, a venv bound to the installation.
    fn check_project(&self, labview: Option<&str>, venv: bool, holding: &str) {
        let manifest = match labview {
            Some(text) => ProjectManifest::new().labview(
                text.strip_prefix(">=")
                    .unwrap_or_else(|| panic!("{text:?} is not a >=YYYY")),
            ),
            None => ProjectManifest::new(),
        };
        let work = self.sandbox.work_dir();
        std::fs::write(work.join(MANIFEST_FILE), manifest.render()).expect("write the manifest");
        if venv {
            self.bind_venv();
        }
        // What `a copy of it` copies: the last file put in.
        let mut last = None;
        for segment in holding.split(" and ") {
            self.hold(segment.trim().trim_end_matches(','), &mut last);
        }
    }

    /// What `lvpm venv create` leaves, as the venv of the project is bound to
    /// the installation, which is made with an empty `vi.lib`.
    fn bind_venv(&self) {
        let installation = self.installation();
        std::fs::create_dir_all(installation.join("vi.lib")).expect("make the installation");
        let venv = self.sandbox.venv_dir();
        std::fs::create_dir_all(venv.join(".lvpm")).expect("make the venv");
        let binding = json!({
            "target_key": "LabVIEW-2026-64bit",
            "target_path": installation.to_string_lossy(),
            "version": 26.3,
            "bitness": 64,
            // The sandbox checks that this is the port lvpm itself would give.
            "port": predicted_port(&venv),
            "created_at": 0,
        });
        std::fs::write(venv.join(".lvpm/venv.json"), binding.to_string()).expect("bind the venv");
    }

    /// One part of a `holding` list: files, a folder, a copy, or a file made
    /// to be broken.
    fn hold(&self, segment: &str, last: &mut Option<PathBuf>) {
        let work = self.sandbox.work_dir();
        let names = quoted(segment);
        if segment.starts_with("an empty folder ") {
            std::fs::create_dir_all(work.join(&names[0])).expect("make the folder");
        } else if segment == PILOT_VI_2013 {
            let bytes = pilot_vi_2013();
            *last = Some(self.write(&work.join("KVP Parser.vi"), &bytes));
        } else if segment.starts_with("no ") {
            // A file the scenario leaves out says so, and nothing is put in.
        } else if segment.starts_with("a copy of it named ") {
            let from = last.as_ref().expect("a file to copy");
            let to = from.with_file_name(&names[0]);
            std::fs::copy(from, &to).expect("copy the file");
            *last = Some(to);
        } else if let Some((file, text)) = segment.split_once(" containing ") {
            let (file, text) = (quoted(file).remove(0), quoted(text).remove(0));
            *last = Some(self.write(&work.join(file), text.as_bytes()));
        } else if segment.ends_with(" with its LIvi section size set past the end of the file") {
            *last = Some(self.write(&work.join(&names[0]), &self.with_a_wild_livi(&names[0])));
        } else {
            let (names, folder) = match segment.split_once(" in folder ") {
                Some((files, folder)) => (quoted(files), quoted(folder).remove(0)),
                None => (names, String::new()),
            };
            for name in names {
                *last = Some(self.put_fixture(&name, &work.join(&folder)));
            }
        }
    }

    /// The fixture `name` in `folder`, which is made.
    fn put_fixture(&self, name: &str, folder: &Path) -> PathBuf {
        let bytes = std::fs::read(Path::new(FIXTURES).join(name)).expect("a fixture");
        self.write(&folder.join(name), &bytes)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> PathBuf {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("make the folder");
        std::fs::write(path, bytes).expect("write the file");
        path.to_path_buf()
    }

    /// The fixture `name` with the size field of its `LIvi` section set past
    /// the end of the file.
    fn with_a_wild_livi(&self, name: &str) -> Vec<u8> {
        let mut bytes = std::fs::read(Path::new(FIXTURES).join(name)).expect("a fixture");
        let at = Rsrc::read(&bytes)
            .expect("a VI")
            .blocks
            .iter()
            .find(|b| &b.kind == b"LIvi")
            .expect("a LIvi section")
            .offset;
        bytes[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        bytes
    }

    /// A package the venv holds: `from` as `rel` below its `vi.lib`, and the
    /// install record that lists it.
    fn install_in_venv(&self, package: &str, rel: &str, from: &Path) {
        let tree = self.sandbox.venv_dir().join(package).join("1/vi.lib");
        let to = self.write(&tree.join(rel), &std::fs::read(from).expect("a file"));
        let record = json!({
            "name": package, "version": "1.0", "installed_at": 0,
            "target": self.installation().to_string_lossy(),
            "files": [to.to_string_lossy().replace('\\', "/")], "skipped_hooks": [],
        });
        let record_path = self
            .sandbox
            .venv_dir()
            .join(format!(".lvpm/installed/{package}.json"));
        self.write(&record_path, record.to_string().as_bytes());
    }

    fn checked(&self) -> &Output {
        self.last_output.as_ref().expect("a command was run")
    }

    fn stdout_lines(&self) -> Vec<&str> {
        self.checked().stdout.lines().collect()
    }
}

fn pilot_vi_2013() -> Vec<u8> {
    let archive = std::fs::File::open(pilots_dir().join(PILOT_ARCHIVE)).expect("a pilot archive");
    let mut zip = zip::ZipArchive::new(archive).expect("a zip");
    let mut bytes = Vec::new();
    zip.by_name(PILOT_VI)
        .expect("the pilot VI")
        .read_to_end(&mut bytes)
        .expect("read the pilot VI");
    bytes
}

/// The text between each pair of double quotes in `text`.
fn quoted(text: &str) -> Vec<String> {
    text.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

// ---- setup ----

#[given(regex = r#"^a check project with labview "([^"]+)" holding (.+)$"#)]
async fn project_with_labview(world: &mut CliWorld, labview: String, holding: String) {
    world.check_project(Some(&labview), true, &holding);
}

#[given(regex = r"^a check project with no labview key holding (.+)$")]
async fn project_without_labview(world: &mut CliWorld, holding: String) {
    world.check_project(None, true, &holding);
}

#[given(regex = r"^a check project holding (.+)$")]
async fn project(world: &mut CliWorld, holding: String) {
    world.check_project(None, true, &holding);
}

#[given(regex = r"^a sandbox project with an lvpm\.toml and no venv, holding (.+)$")]
async fn project_without_a_venv(world: &mut CliWorld, holding: String) {
    world.check_project(None, false, &holding);
}

#[given("the bound installation folder holds the vi.lib file it links to")]
async fn installation_holds_the_file(world: &mut CliWorld) {
    world.write(&world.installation().join(NI_FILE), b"");
}

#[given("the bound installation folder does not hold the vi.lib file it links to")]
async fn installation_lacks_the_file(world: &mut CliWorld) {
    assert!(!world.installation().join(NI_FILE).exists());
}

#[given("the bound installation folder does not exist")]
async fn no_installation(world: &mut CliWorld) {
    std::fs::remove_dir_all(world.installation()).expect("remove the installation");
}

#[given(regex = r#"^the venv holds an installed package containing "([^"]+)"$"#)]
async fn venv_holds_a_package(world: &mut CliWorld, name: String) {
    let from = Path::new(FIXTURES).join(&name);
    world.install_in_venv(PACKAGE, &name, &from);
}

#[given(regex = r#"^the project holds "([^"]+)" in folder "([^"]+)"$"#)]
async fn project_holds_in_folder(world: &mut CliWorld, name: String, folder: String) {
    world.put_fixture(&name, &world.sandbox.work_dir().join(folder));
}

// ---- running ----

#[when("lvpm check runs")]
async fn check(world: &mut CliWorld) {
    world.run("lvpm check", None);
}

#[when(regex = r#"^lvpm check runs on "([^"]+)"$"#)]
async fn check_on(world: &mut CliWorld, dir: String) {
    world.run(&format!("lvpm check {dir}"), None);
}

#[when("lvpm check runs with --json")]
async fn check_as_json(world: &mut CliWorld) {
    world.run("lvpm check --json", None);
}

// ---- what it printed ----

#[then(regex = r#"^stdout ends with "([^"]+)"$"#)]
async fn stdout_ends_with(world: &mut CliWorld, end: String) {
    let out = world.checked();
    assert!(out.stdout.trim_end().ends_with(&end), "{out:?}");
}

#[then(regex = r#"^stdout has the line "([^"]+)"$"#)]
async fn stdout_has_the_line(world: &mut CliWorld, line: String) {
    let out = world.checked();
    assert!(world.stdout_lines().contains(&line.as_str()), "{out:?}");
}

#[then(regex = r#"^stdout has a line starting "([^"]+)"$"#)]
async fn stdout_has_a_line_starting(world: &mut CliWorld, start: String) {
    let out = world.checked();
    assert!(
        world.stdout_lines().iter().any(|l| l.starts_with(&start)),
        "{out:?}"
    );
}

#[then(regex = r#"^stdout has no line containing "([^"]+)"$"#)]
async fn stdout_has_no_line_containing(world: &mut CliWorld, text: String) {
    let out = world.checked();
    assert!(!out.stdout.contains(&text), "{out:?}");
}

#[then(regex = r#"^stderr says "([^"]+)"$"#)]
async fn stderr_says(world: &mut CliWorld, text: String) {
    let out = world.checked();
    assert!(out.stderr.contains(&text), "{out:?}");
}

#[then(regex = r#"^stderr has the line "([^"]+)"$"#)]
async fn stderr_has_the_line(world: &mut CliWorld, line: String) {
    let out = world.checked();
    assert!(out.stderr.lines().any(|l| l == line), "{out:?}");
}

#[then("the finding lines are, in order:")]
async fn finding_lines(world: &mut CliWorld, step: &cucumber::gherkin::Step) {
    let want: Vec<&str> = step
        .table
        .as_ref()
        .expect("a table of the lines")
        .rows
        .iter()
        .map(|row| row[0].trim())
        .collect();
    // The findings come first, then the `next:` lines and the summary.
    let got: Vec<&str> = world
        .stdout_lines()
        .into_iter()
        .take_while(|l| !l.starts_with("next: ") && !l.starts_with("VIs checked: "))
        .collect();
    assert_eq!(got, want, "{:?}", world.checked());
}

#[then(regex = r#"^stderr has a line starting "([^"]+)"$"#)]
async fn stderr_has_a_line_starting(world: &mut CliWorld, start: String) {
    let out = world.checked();
    assert!(out.stderr.lines().any(|l| l.starts_with(&start)), "{out:?}");
}

/// The `result` of the `--json` run, which is its one line of output.
fn json_result(world: &CliWorld) -> Value {
    let out = world.checked();
    let line = out.stdout.trim_end();
    let parsed: Value = serde_json::from_str(line).unwrap_or_else(|why| panic!("{why}: {out:?}"));
    parsed["result"].clone()
}

#[then(regex = r#"^the JSON has "ok" (true|false), "vis" (\d+) and no findings$"#)]
async fn json_without_findings(world: &mut CliWorld, ok: bool, vis: u64) {
    let result = json_result(world);
    let want = json!({"ok": ok, "vis": vis, "not_checked": 0, "findings": []});
    assert_eq!(result, want);
}

#[then(
    regex = r#"^the JSON has "ok" (true|false), "vis" (\d+) and one finding with "vi" "([^"]+)", "kind" "([^"]+)" and "link" "([^"]+)"$"#
)]
async fn json_with_a_finding(
    world: &mut CliWorld,
    ok: bool,
    vis: u64,
    vi: String,
    kind: String,
    link: String,
) {
    let result = json_result(world);
    assert_eq!((&result["ok"], &result["vis"]), (&json!(ok), &json!(vis)));
    let findings = result["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{result}");
    let found = &findings[0];
    assert_eq!(
        (&found["vi"], &found["kind"], &found["link"]),
        (&json!(vi), &json!(kind), &json!(link))
    );
}
