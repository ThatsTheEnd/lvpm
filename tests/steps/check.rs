//! Steps of the `lvpm check` scenarios: a project in the work directory made
//! from the fixture VIs, a venv bound to an installation that is a folder in
//! the sandbox, and what `lvpm check` printed. LabVIEW is never detected or
//! started: the binding and the install records are written by hand.

use crate::CliWorld;
use crate::support::golden::Output;
use crate::support::pilots::{
    PILOT_A_COMMIT, PILOT_A_SUBTREE, PILOT_B_COMMIT, PILOT_B_SUBTREE, archives, copy_tree,
    pilot_checkout, pilots_dir,
};
use crate::support::project::{MANIFEST_FILE, ProjectManifest};
use crate::support::selection::{PILOT_A_ENV_VAR, PILOT_B_ENV_VAR};
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
/// The files the fixture packages hold, below the `vi.lib` the venv puts them in.
const PACKAGES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/check/packages");
/// The one NI file `UsesNi.vi` links to, below the installation.
const NI_FILE: &str = "vi.lib/Utility/Get Current LV Bitness.vi";
/// A VI saved in LabVIEW 2013, taken out of the committed pilot archive that holds it.
const PILOT_VI_2013: &str = "the pilot VI saved in LabVIEW 2013";
const PILOT_ARCHIVE: &str = "labview_open_source_lib_data_manipulation-1.6.0.16.vip";
const PILOT_VI: &str =
    "File Group 0/vi.lib/LabVIEW Open Source Project/Data Manipulation/KVP Parser.vi";
/// The package a scenario's "installed package" is.
const PACKAGE: &str = "check_fixture_dep";
/// The libraries of the pilots' sources, below the project the checkout is copied into.
const PILOT_A_LIBRARY: &str = "Source/HSE-Logger/hse-logger.lvlib";
const PILOT_B_LIBRARY: &str = "source/OpenSerializer.lvlib";

impl CliWorld {
    fn installation(&self) -> PathBuf {
        self.sandbox.root().join(INSTALLATION)
    }

    /// A project: an `lvpm.toml` with `labview` (a `>=YYYY` as written) and
    /// then `tables` (TOML), and the files `holding` lists. With `venv`, a venv
    /// bound to the installation.
    pub(super) fn check_project(
        &self,
        labview: Option<&str>,
        venv: bool,
        tables: &str,
        holding: &str,
    ) {
        let manifest = match labview {
            Some(text) => ProjectManifest::new().labview(
                text.strip_prefix(">=")
                    .unwrap_or_else(|| panic!("{text:?} is not a >=YYYY")),
            ),
            None => ProjectManifest::new(),
        };
        let work = self.sandbox.work_dir();
        let manifest = format!("{}\n{tables}", manifest.render());
        std::fs::write(work.join(MANIFEST_FILE), manifest).expect("write the manifest");
        if venv {
            self.bind_venv();
        }
        // What `a copy of it` copies: the last file put in.
        let mut last = None;
        for segment in holding.split(" and ") {
            self.hold(segment.trim().trim_end_matches(','), &mut last);
        }
    }

    /// A project that declares dependencies, with both fixture packages
    /// installed in its venv. `Lib.lvlib` is there, as the library VIs link to
    /// it, and lists only the members the scenario holds: it holds the VIs it
    /// is about, not every one the fixture lists.
    fn declaring_project(&self, spec: &str, holding: &str) {
        self.check_project(None, true, &manifest_tables(spec), holding);
        self.install_fixture_packages();
        let work = self.sandbox.work_dir();
        if !work.join("Lib.lvlib").exists() {
            self.put_fixture("Lib.lvlib", &work);
        }
        self.keep_held_members();
    }

    /// Both fixture packages, installed in the venv.
    fn install_fixture_packages(&self) {
        for (package, rel) in [
            (PACKAGE, "Check Fixture/Dep.vi"),
            ("check_fixture_other", "Check Fixture Dev/DevDep.vi"),
        ] {
            self.install_in_venv(package, rel, &Path::new(PACKAGES).join(rel));
        }
    }

    /// `Lib.lvlib` without the members the project does not hold.
    fn keep_held_members(&self) {
        let work = self.sandbox.work_dir();
        let text = std::fs::read_to_string(work.join("Lib.lvlib")).expect("the library");
        let held = |line: &&str| match line.split_once("URL=\"../") {
            Some((_, rest)) => work
                .join(rest.split('"').next().unwrap_or_default())
                .is_file(),
            None => true,
        };
        let kept: Vec<&str> = text.lines().filter(held).collect();
        std::fs::write(work.join("Lib.lvlib"), kept.join("\n")).expect("trim the library");
    }

    /// A project a build can package: `Lib.lvlib` is `library`, and lists only
    /// the members the scenario holds.
    pub(super) fn build_project(&self, labview: &str, library: &str, holding: &str) {
        self.check_project(Some(labview), true, &distribution(library), holding);
        self.keep_held_members();
    }

    /// The fixture VI `name`, listed by `Lib.lvlib`.
    pub(super) fn add_member(&self, name: &str) {
        self.put_fixture(name, &self.sandbox.work_dir());
        self.list_member(name);
    }

    /// `name`, a file in the project, listed by `Lib.lvlib`.
    fn list_member(&self, name: &str) {
        let item = format!("<Item Name=\"{name}\" Type=\"VI\" URL=\"../{name}\"/>");
        self.add_item("Lib.lvlib", "</Library>", &item);
    }

    /// `text` added to `lvpm.toml`: a table the manifest does not have yet.
    pub(super) fn append_to_manifest(&self, text: &str) {
        let path = self.sandbox.work_dir().join(MANIFEST_FILE);
        let manifest = std::fs::read_to_string(&path).expect("read the manifest");
        std::fs::write(path, format!("{manifest}\n{text}")).expect("write the manifest");
    }

    /// The `.lvpkg` files `lvpm build` left in `dist/`.
    fn built_packages(&self) -> Vec<PathBuf> {
        let dist = self.sandbox.work_dir().join("dist");
        let mut found: Vec<PathBuf> = std::fs::read_dir(dist)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "lvpkg"))
            .collect();
        found.sort();
        found
    }

    /// What `lvpm venv create` leaves, as the venv of the project is bound to
    /// the installation, which is made with an empty `vi.lib`.
    pub(super) fn bind_venv(&self) {
        let installation = self.installation();
        std::fs::create_dir_all(installation.join("vi.lib")).expect("make the installation");
        let venv = self.sandbox.venv_dir();
        std::fs::create_dir_all(&venv).expect("make the venv");
        let binding = json!({
            "target_key": "LabVIEW-2026-64bit",
            "target_path": installation.to_string_lossy(),
            "version": 26.3,
            "bitness": 64,
            // The sandbox checks that this is the port lvpm itself would give.
            "port": predicted_port(&venv),
            "created_at": 0,
        });
        std::fs::write(venv.join("venv.json"), binding.to_string()).expect("bind the venv");
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
        let tree = self
            .sandbox
            .venv_dir()
            .join("packages")
            .join(package)
            .join("1/vi.lib");
        let to = self.write(&tree.join(rel), &std::fs::read(from).expect("a file"));
        self.record_install(package, &[to]);
    }

    /// The package of a committed pilot archive, as an install leaves it: the
    /// files of its `File Group 0` below the package's `1/` tree.
    fn install_pilot_archive(&self, package: &str) {
        let prefix = format!("{package}-");
        let archive = archives(&pilots_dir())
            .into_iter()
            .find(|a| {
                a.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
            })
            .unwrap_or_else(|| panic!("no committed pilot archive for {package}"));
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).expect("a pilot archive"))
            .expect("a zip");
        let tree = self
            .sandbox
            .venv_dir()
            .join("packages")
            .join(package)
            .join("1");
        let mut files = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).expect("a zip entry");
            let Some(rel) = entry.name().strip_prefix("File Group 0/").map(String::from) else {
                continue;
            };
            if entry.is_dir() {
                continue;
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).expect("read the entry");
            files.push(self.write(&tree.join(rel), &bytes));
        }
        self.record_install(package, &files);
    }

    fn record_install(&self, package: &str, files: &[PathBuf]) {
        let files: Vec<String> = files
            .iter()
            .map(|f| f.to_string_lossy().replace('\\', "/"))
            .collect();
        let record = json!({
            "name": package, "version": "1.0", "installed_at": 0,
            "target": self.installation().to_string_lossy(),
            "files": files, "skipped_hooks": [],
        });
        let record_path = self
            .sandbox
            .venv_dir()
            .join(format!("installed/{package}.json"));
        self.write(&record_path, record.to_string().as_bytes());
    }

    /// A pilot's source, copied from its pinned checkout into the work
    /// directory, with the manifest tables `tables` (`[distribution]` among
    /// them, or not).
    fn pilot_project(&self, checkout: &Path, subtree: &str, labview: &str, tables: &str) {
        let work = self.sandbox.work_dir();
        copy_tree(&checkout.join(subtree), &work.join(subtree));
        self.check_project(Some(labview), true, tables, "");
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
pub(super) fn quoted(text: &str) -> Vec<String> {
    text.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// The manifest tables a "check project with ..." step says: `[distribution]`
/// (`Lib.lvlib` unless the step says otherwise) and the one dependency table
/// with its packages. Every package is `"*"`.
fn manifest_tables(spec: &str) -> String {
    let (spec, library) = match spec.strip_prefix("no [distribution] table and ") {
        Some(rest) => (rest, None),
        None if spec.starts_with("[distribution] ") => (spec, quoted(spec).pop()),
        None => (spec, Some("Lib.lvlib".to_string())),
    };
    let mut text = library.map_or(String::new(), |lib| {
        format!("[distribution]\ntop-level-library = \"{lib}\"\n")
    });
    let names = quoted(spec);
    let (table, packages) = match spec.split_once("] ") {
        Some(("[dependency-groups", _)) => (format!("dependency-groups.{}", names[0]), &names[1..]),
        Some((table, _)) if !table.starts_with("[distribution") => {
            (table.trim_start_matches('[').to_string(), &names[..])
        }
        _ => return text,
    };
    if !packages.is_empty() {
        text.push_str(&format!("[{table}]\n"));
        for package in packages {
            text.push_str(&format!("{package} = \"*\"\n"));
        }
    }
    text
}

// ---- setup ----

#[given(regex = r#"^a check project with labview "([^"]+)" holding (.+)$"#)]
async fn project_with_labview(world: &mut CliWorld, labview: String, holding: String) {
    world.check_project(Some(&labview), true, "", &holding);
}

#[given(regex = r"^a check project with no labview key holding (.+)$")]
async fn project_without_labview(world: &mut CliWorld, holding: String) {
    world.check_project(None, true, "", &holding);
}

#[given(regex = r"^a check project holding (.+)$")]
async fn project(world: &mut CliWorld, holding: String) {
    world.check_project(None, true, "", &holding);
}

#[given(regex = r"^a sandbox project with an lvpm\.toml and no venv, holding (.+)$")]
async fn project_without_a_venv(world: &mut CliWorld, holding: String) {
    world.check_project(None, false, "", &holding);
}

#[given(
    regex = r#"^a check project with (\[.+?|every dependency table empty|no \[distribution\] table and .+?), holding (.+)$"#
)]
async fn project_declaring(world: &mut CliWorld, spec: String, holding: String) {
    world.declaring_project(&spec, &holding);
}

#[given(regex = r#"^the lock lists "([^"]+)" as a dependency of "([^"]+)"$"#)]
async fn lock_lists(world: &mut CliWorld, dependency: String, of: String) {
    let package = |name: &str, requires: &[&str]| {
        format!(
            "[[package]]\nname = \"{name}\"\nversion = \"1.0\"\nsha256 = \"00\"\nsource = \"local\"\nrequires = {requires:?}\n\n"
        )
    };
    let lock = format!(
        "version = 2\nlabview = \"2026\"\n\n[inputs]\ndefaults = false\n\n[inputs.dependencies]\n{of} = \"*\"\n\n{}{}",
        package(&of, &[&dependency]),
        package(&dependency, &[]),
    );
    let path = world.sandbox.work_dir().join("lvpm.lock");
    std::fs::write(path, lock).expect("write the lock");
}

#[given(regex = r#"^the bound installation folder also holds "([^"]+)"$"#)]
async fn installation_also_holds(world: &mut CliWorld, name: String) {
    world.write(&world.installation().join(name), b"");
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

/// The `[distribution]` table of a project whose library is `library`, with
/// the keys `lvpm build` needs besides it.
fn distribution(library: &str) -> String {
    format!(
        "[distribution]
top-level-library = \"{library}\"
vendor = \"Vendor\"
display-name = \"Check Lib\"
"
    )
}

/// Without a `[distribution]` table no VI is a library member, so the library
/// file is never read: deleting a member leaves a broken link, not an error.
#[given(
    regex = r#"^pilot A's pinned source checkout with labview "([^"]+)"( and no \[distribution\] table)?$"#
)]
async fn pilot_a_project(world: &mut CliWorld, labview: String, no_distribution: String) {
    let checkout = pilot_checkout(PILOT_A_ENV_VAR, PILOT_A_COMMIT, PILOT_A_SUBTREE)
        .unwrap_or_else(|why| panic!("{why}"));
    let tables = match no_distribution.is_empty() {
        true => distribution(PILOT_A_LIBRARY),
        false => String::new(),
    };
    world.pilot_project(&checkout, PILOT_A_SUBTREE, &labview, &tables);
}

#[given(
    regex = r#"^pilot B's pinned source checkout with labview "([^"]+)" and (\[dependencies\] empty|"[^"]+" in \[dependencies\])$"#
)]
async fn pilot_b_project(world: &mut CliWorld, labview: String, dependencies: String) {
    let checkout = pilot_checkout(PILOT_B_ENV_VAR, PILOT_B_COMMIT, PILOT_B_SUBTREE)
        .unwrap_or_else(|why| panic!("{why}"));
    let declared: String = quoted(&dependencies)
        .iter()
        .map(|package| {
            format!(
                "{package} = \"*\"
"
            )
        })
        .collect();
    let tables = format!(
        "{}[dependencies]
{declared}",
        distribution(PILOT_B_LIBRARY)
    );
    world.pilot_project(&checkout, PILOT_B_SUBTREE, &labview, &tables);
}

#[given(regex = r#"^"([^"]+)" installed in the venv from the committed pilot archive$"#)]
async fn installed_from_the_pilot_archive(world: &mut CliWorld, package: String) {
    world.install_pilot_archive(&package);
}

#[given(regex = r#"^"([^"]+)" is deleted from the project$"#)]
async fn deleted_from_the_project(world: &mut CliWorld, file: String) {
    std::fs::remove_file(world.sandbox.work_dir().join(file)).expect("delete the file");
}

/// The library file lists what it holds, so a build that finds a listed file
/// missing stops before it checks anything: the demo takes the file out of
/// the listing too, and its caller's link is then what breaks.
#[given(regex = r#"^"([^"]+)" is no longer listed in "([^"]+)"$"#)]
async fn no_longer_listed(world: &mut CliWorld, item: String, file: String) {
    let path = world.sandbox.work_dir().join(file);
    let text = std::fs::read_to_string(&path).expect("read the library file");
    let from = text
        .find(&format!("<Item Name=\"{item}\""))
        .expect("the item is listed");
    let to = from + text[from..].find("</Item>").expect("the item ends") + "</Item>".len();
    let first = text[..from].rfind('\n').map_or(0, |at| at + 1);
    let last = text[to..].find('\n').map_or(text.len(), |at| to + at + 1);
    std::fs::write(&path, format!("{}{}", &text[..first], &text[last..])).expect("write it back");
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

#[then(regex = r"^stdout reports at least (\d+) VIs checked and (\d+) problems$")]
async fn stdout_reports_the_summary(world: &mut CliWorld, vis: usize, problems: usize) {
    let out = world.checked();
    let summary = world
        .stdout_lines()
        .into_iter()
        .find_map(|l| l.strip_prefix("VIs checked: "))
        .unwrap_or_else(|| panic!("no summary line: {out:?}"));
    // "N, problems: M, links not checked: K"
    let number = |field: usize| -> usize {
        let text = summary.split(", ").nth(field).unwrap_or_default();
        let digits = text.trim_start_matches(|c: char| !c.is_ascii_digit());
        digits.parse().unwrap_or_else(|_| panic!("{summary:?}"))
    };
    assert!(number(0) >= vis, "want {vis} VIs at least: {out:?}");
    assert_eq!(number(1), problems, "{out:?}");
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

#[then(regex = r#"^stderr has a line containing "([^"]+)"$"#)]
async fn stderr_has_a_line_containing(world: &mut CliWorld, text: String) {
    let out = world.checked();
    assert!(out.stderr.lines().any(|l| l.contains(&text)), "{out:?}");
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
    regex = r#"^the JSON has one finding with "kind" "([^"]+)", "package" "([^"]+)" and "table" null$"#
)]
async fn json_with_an_undeclared_finding(world: &mut CliWorld, kind: String, package: String) {
    let result = json_result(world);
    let findings = result["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{result}");
    let found = &findings[0];
    assert_eq!(
        (&found["kind"], &found["package"], &found["table"]),
        (&json!(kind), &json!(package), &Value::Null)
    );
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

// ---- lvpm build runs the check ----

#[given(
    regex = r#"^a check project with labview "([^"]+)" and \[distribution\] naming "([^"]+)", holding (.+)$"#
)]
async fn build_project(world: &mut CliWorld, labview: String, library: String, holding: String) {
    world.build_project(&labview, &library, &holding);
}

#[given(regex = r#"^"([^"]+)" is a member of "Lib\.lvlib" and the folder "([^"]+)" is empty$"#)]
async fn member_and_empty_folder(world: &mut CliWorld, vi: String, folder: String) {
    world.add_member(&vi);
    std::fs::create_dir_all(world.sandbox.work_dir().join(folder)).expect("make the folder");
}

#[given(regex = r#"^"([^"]+)" is a member of "Lib\.lvlib" in the dot-folder "([^"]+)"$"#)]
async fn member_in_a_dot_folder(world: &mut CliWorld, vi: String, folder: String) {
    world.put_fixture(&vi, &world.sandbox.work_dir().join(&folder));
    world.list_member(&format!("{folder}/{vi}"));
}

#[given(regex = r#"^"([^"]+)", a copy of "([^"]+)", is also a member$"#)]
async fn copy_is_also_a_member(world: &mut CliWorld, copy: String, of: String) {
    let work = world.sandbox.work_dir();
    std::fs::copy(work.join(of), work.join(&copy)).expect("copy the VI");
    world.list_member(&copy);
}

#[given(regex = r#"^the project also holds "([^"]+)", not a member$"#)]
async fn holds_a_non_member(world: &mut CliWorld, vi: String) {
    world.put_fixture(&vi, &world.sandbox.work_dir());
}

#[given(regex = r#"^\[dependencies\] names "([^"]+)"(?: and no member links into it)?$"#)]
async fn dependencies_names(world: &mut CliWorld, package: String) {
    world.append_to_manifest(&format!("[dependencies]\n{package} = \"*\"\n"));
}

#[given(
    regex = r#"^member "([^"]+)" links into package "([^"]+)" installed in the venv, and (.+)$"#
)]
async fn member_links_into_a_package(
    world: &mut CliWorld,
    vi: String,
    package: String,
    declared: String,
) {
    assert_eq!(package, PACKAGE, "the fixture VIs link into {PACKAGE}");
    world.add_member(&vi);
    world.install_fixture_packages();
    let table = match declared.as_str() {
        "[dependencies] is empty" => return,
        d if d.ends_with(" only in [dev-dependencies]") => "dev-dependencies",
        d if d.ends_with(" in [dependencies]") => "dependencies",
        other => panic!("no such declaration: {other}"),
    };
    world.append_to_manifest(&format!("[{table}]\n{package} = \"*\"\n"));
}

#[given(regex = r#"^the member "([^"]+)" (is saved in .+|keeps .+|has its LIvi .+|contains .+)$"#)]
async fn member_is_broken(world: &mut CliWorld, vi: String, how: String) {
    let path = world.sandbox.work_dir().join(&vi);
    match how.as_str() {
        "is saved in a LabVIEW newer than the minimum" => {
            let manifest = world.sandbox.work_dir().join(MANIFEST_FILE);
            let text = std::fs::read_to_string(&manifest).expect("read the manifest");
            let lowered = text.replace("labview = \">=2026\"", "labview = \">=2025\"");
            std::fs::write(manifest, lowered).expect("lower the minimum");
        }
        "keeps its compiled code" => {
            let bytes = std::fs::read(Path::new(FIXTURES).join("NotSeparated.vi")).expect("a VI");
            world.write(&path, &bytes);
        }
        "has its LIvi section size set past the end" => {
            world.write(&path, &world.with_a_wild_livi(&vi));
        }
        c if c.starts_with("contains ") => {
            world.write(&path, quoted(c)[0].as_bytes());
        }
        other => panic!("no such change to a member: {other}"),
    }
}

#[given("the project has no venv")]
async fn no_venv(world: &mut CliWorld) {
    std::fs::remove_dir_all(world.sandbox.venv_dir()).expect("remove the venv");
}

#[given("dist holds a built package for this version")]
async fn dist_holds_a_built_package(world: &mut CliWorld) {
    world.run("lvpm build", None);
    assert_eq!(world.checked().exit_code, 0, "{:?}", world.checked());
    let packages = world.built_packages();
    assert_eq!(packages.len(), 1, "{packages:?}");
    world.build.earlier = Some(std::fs::read(&packages[0]).expect("the package"));
}

#[when(regex = r"^lvpm build( --no-check)? runs on it(?: with( --json))?$")]
async fn build_runs_on_it(world: &mut CliWorld, no_check: String, json: String) {
    world.run(&format!("lvpm build{no_check}{json}"), None);
}

impl CliWorld {
    fn exited(&self, code: i32) {
        let out = self.checked();
        assert_eq!(out.exit_code, code, "{out:?}");
    }

    fn dist_holds_the_package(&self) {
        let packages = self.built_packages();
        assert_eq!(packages.len(), 1, "{packages:?}: {:?}", self.checked());
    }
}

#[then("it exits 0, dist holds the package, and stderr is empty")]
async fn built_with_no_stderr(world: &mut CliWorld) {
    world.exited(0);
    world.dist_holds_the_package();
    assert_eq!(world.checked().stderr, "", "{:?}", world.checked());
}

#[then("it exits 0 and dist holds the package")]
async fn built_and_dist_holds_the_package(world: &mut CliWorld) {
    world.exited(0);
    world.dist_holds_the_package();
}

#[then("it exits 1 and that package is byte-identical to before")]
async fn refused_and_package_unchanged(world: &mut CliWorld) {
    world.exited(1);
    let packages = world.built_packages();
    assert_eq!(packages.len(), 1, "{packages:?}");
    let now = std::fs::read(&packages[0]).expect("the package");
    assert!(
        world.build.earlier.as_ref() == Some(&now),
        "the package changed"
    );
}

#[then(regex = r"^it exits 1 and dist holds no \.lvpkg$")]
async fn refused_and_no_package(world: &mut CliWorld) {
    world.exited(1);
    world.no_package_in_dist();
}

#[then(regex = r#"^it exits 1, stdout has a line starting "([^"]+)", and dist holds no \.lvpkg$"#)]
async fn refused_with_a_line(world: &mut CliWorld, start: String) {
    world.exited(1);
    stdout_has_a_line_starting(world, start).await;
    world.no_package_in_dist();
}

#[then(regex = r#"^it exits 1, stderr names "([^"]+)" and "([^"]+)", and dist holds no \.lvpkg$"#)]
async fn refused_naming(world: &mut CliWorld, first: String, second: String) {
    world.exited(1);
    stderr_says(world, first).await;
    stderr_says(world, second).await;
    world.no_package_in_dist();
}

#[then(regex = r#"^stderr has no line containing "([^"]+)"$"#)]
async fn stderr_has_no_line_containing(world: &mut CliWorld, text: String) {
    let out = world.checked();
    assert!(!out.stderr.lines().any(|l| l.contains(&text)), "{out:?}");
}

#[then(regex = r#"^stderr's last line is "([^"]+)"$"#)]
async fn stderr_last_line(world: &mut CliWorld, text: String) {
    let out = world.checked();
    assert_eq!(out.stderr.lines().last(), Some(text.as_str()), "{out:?}");
}

#[then("dist holds only that package")]
async fn dist_holds_only_that_package(world: &mut CliWorld) {
    let dist = world.sandbox.work_dir().join("dist");
    let found: Vec<_> = std::fs::read_dir(dist).expect("dist").flatten().collect();
    assert_eq!(found.len(), 1, "dist holds {found:?}");
}

#[then(regex = r#"^stderr is exactly "([^"]+)"$"#)]
async fn stderr_is_exactly(world: &mut CliWorld, text: String) {
    let out = world.checked();
    assert_eq!(out.stderr.trim_end(), text, "{out:?}");
}

#[then(regex = r#"^stderr is exactly the lines "([^"]+)" and "([^"]+)"$"#)]
async fn stderr_is_exactly_two_lines(world: &mut CliWorld, first: String, second: String) {
    let out = world.checked();
    assert_eq!(out.stderr, format!("{first}\n{second}\n"), "{out:?}");
}

#[then(
    regex = r#"^it exits 1, dist holds no \.lvpkg, and the JSON has "ok": false, "command": "build" and one finding for "([^"]+)"$"#
)]
async fn refused_as_json(world: &mut CliWorld, vi: String) {
    world.exited(1);
    world.no_package_in_dist();
    let result = json_result(world);
    assert_eq!(result["ok"], json!(false), "{result}");
    assert_eq!(result["command"], json!("build"), "{result}");
    let findings = result["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{result}");
    assert_eq!(findings[0]["vi"], json!(vi), "{result}");
}
