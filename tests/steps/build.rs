//! Steps of the `lvpm build` scenarios: a small library on disk, the
//! `lvpm.toml` that names it, and the `.lvpkg` that `lvpm build` wrote.
//! The manifest is kept as a table and written before every build, so a
//! scenario's change to it is one edit of that table.

use crate::CliWorld;
use crate::steps::check::quoted;
use crate::steps::pilots::{
    PROJECT_B_DEPENDENCIES, copy_pilots_to, require_installation_without_pilots,
};
use crate::support::events::RecordingSink;
use crate::support::fixtures::{FixturePackage, Payload};
use crate::support::golden::Output;
use crate::support::pilots::copy_tree;
use crate::support::project::MANIFEST_FILE;
use crate::support::sandbox::{DEST_DIR, LABVIEW_YEAR, LOCAL_PACKAGES_DIR};
use cucumber::{given, then, when};
use lvpm_core::config::Config;
use lvpm_core::lvpkg::{self, SBOM};
use lvpm_core::manifest_edit::DependencyTable;
use lvpm_core::ops::OpEnv;
use lvpm_core::ops::build::Outcome;
use serde_json::Value;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, SystemTime};

const LIBRARY: &str = "Lib.lvlib";
const CLASS: &str = "C/C.lvclass";
const DIST: &str = "dist";

/// What a build scenario has set up and seen.
#[derive(Debug, Default)]
pub struct BuildState {
    pub(super) manifest: toml::Table,
    /// Build in-process with no LabVIEW detected, instead of running `lvpm`.
    no_labview: bool,
    /// Every member's path below the work directory, for the timestamp change.
    members: Vec<&'static str>,
    output: Option<Output>,
    /// A package's bytes as an earlier build left them.
    pub(super) earlier: Option<Vec<u8>>,
}

impl CliWorld {
    pub(super) fn work(&self, rel: &str) -> PathBuf {
        self.sandbox.work_dir().join(rel)
    }

    pub(super) fn put(&self, rel: &str, text: &str) {
        let path = self.work(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create a folder");
        std::fs::write(path, text).expect("write a file");
    }

    /// `rel`'s text with `item` added as the last item of its root element.
    pub(super) fn add_item(&self, rel: &str, close: &str, item: &str) {
        let text = std::fs::read_to_string(self.work(rel)).expect("read the library");
        self.put(rel, &text.replace(close, &format!("\t{item}\n{close}")));
    }

    fn table(&mut self, path: &[&str]) -> &mut toml::Table {
        let mut t = &mut self.build.manifest;
        for key in path {
            t = t
                .entry(*key)
                .or_insert_with(|| toml::Value::Table(Default::default()))
                .as_table_mut()
                .expect("a table");
        }
        t
    }

    pub(super) fn set(&mut self, path: &[&str], key: &str, value: impl Into<toml::Value>) {
        self.table(path).insert(key.into(), value.into());
    }

    /// The manifest table, as `lvpm.toml` in the work directory.
    pub(super) fn write_build_manifest(&self) {
        let text = toml::to_string(&self.build.manifest).expect("a manifest");
        std::fs::write(self.work(MANIFEST_FILE), text).expect("write the manifest");
    }

    /// `lvpm build` for the scenarios whose members are text, not VIs, and that
    /// have no venv, so the member check is skipped.
    pub(super) fn lvpm_build_unchecked(&mut self) {
        self.lvpm_build_with(&["--no-check"]);
    }

    /// `lvpm build args...` for the manifest the scenario's table says.
    pub(super) fn lvpm_build_with(&mut self, args: &[&str]) {
        self.write_build_manifest();
        let output = match self.build.no_labview {
            false => self.sandbox.run_lvpm(&[&["build"], args].concat(), None),
            true => {
                let sink = RecordingSink::default();
                let config = Config {
                    cache: self.sandbox.cache_dir(),
                    hooks_run: Vec::new(),
                    local_sources: vec![self.sandbox.root().join(LOCAL_PACKAGES_DIR)],
                    venv_dir: PathBuf::from(".lv-venv"),
                };
                let env = OpEnv::new(&sink, config, Vec::new(), self.sandbox.work_dir());
                let built = lvpm_core::ops::build::build(&env, !args.contains(&"--no-check"));
                Output {
                    stdout: String::new(),
                    stderr: built
                        .as_ref()
                        .err()
                        .map(|e| format!("{e:#}"))
                        .unwrap_or_default(),
                    exit_code: match &built {
                        Ok(Outcome::Refused(_)) | Err(_) => 1,
                        Ok(_) => 0,
                    },
                    tree: None,
                }
            }
        };
        self.last_output = Some(output.clone());
        self.build.output = Some(output);
    }

    /// What the last build printed; or, for a scenario that ran `lvpm` itself
    /// with "I run", what that printed.
    fn built(&self) -> &Output {
        (self.build.output.as_ref().or(self.last_output.as_ref())).expect("lvpm build ran")
    }

    /// The command a scenario ran last, for the steps that read "it succeeds"
    /// and "it fails" of a command that is not `lvpm build`.
    pub(super) fn record_output(&mut self, output: Output) {
        self.build.output = Some(output);
    }

    pub(super) fn succeeded(&self) {
        let out = self.built();
        assert_eq!(out.exit_code, 0, "the command failed: {out:?}");
    }

    pub(super) fn failed(&self) -> String {
        let out = self.built();
        assert_ne!(out.exit_code, 0, "the command succeeded: {out:?}");
        out.stderr.clone()
    }

    fn package_path(&self) -> PathBuf {
        let name = self.build.manifest["project"]["name"]
            .as_str()
            .expect("a name");
        let version = self.build.manifest["project"]["version"]
            .as_str()
            .expect("a version");
        self.work(DIST).join(format!("{name}-{version}.lvpkg"))
    }

    fn archive(&self) -> zip::ZipArchive<std::fs::File> {
        let path = self.package_path();
        let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        zip::ZipArchive::new(file).expect("a zip")
    }

    /// The package as lvpm reads it: `RECORD` is checked on the way.
    fn spec(&self) -> lvpm_core::spec::Spec {
        lvpm_core::lvpkg::read(&mut self.archive()).expect("a package lvpm reads")
    }

    fn package_toml(&self) -> toml::Table {
        let mut text = String::new();
        self.archive()
            .by_name(lvpm_core::lvpkg::PACKAGE_TOML)
            .expect("a package.toml entry")
            .read_to_string(&mut text)
            .expect("a text package.toml");
        text.parse().expect("a package.toml that is TOML")
    }

    fn group_files(&self) -> Vec<String> {
        let spec = self.spec();
        assert_eq!(spec.file_groups.len(), 1, "one file group");
        spec.file_groups[0].files.clone()
    }

    /// No package, and no staged file left behind either.
    pub(super) fn no_package_in_dist(&self) {
        let found: Vec<_> = std::fs::read_dir(self.work(DIST))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert!(found.is_empty(), "dist holds {found:?}");
    }
}

#[given(regex = r#"^a project "([^"]+)" version "([^"]+)" with labview "([^"]+)"$"#)]
async fn project(world: &mut CliWorld, name: String, version: String, labview: String) {
    world.set(&["project"], "name", name);
    world.set(&["project"], "version", version);
    world.set(&["project"], "labview", labview);
}

#[given(
    regex = r#"^\[distribution\] top-level-library "([^"]+)", vendor "([^"]+)", display-name "([^"]+)"$"#
)]
async fn distribution(world: &mut CliWorld, library: String, vendor: String, display: String) {
    world.set(&["distribution"], "top-level-library", library);
    world.set(&["distribution"], "vendor", vendor);
    world.set(&["distribution"], "display-name", display);
}

#[given(regex = r#"^library "Lib\.lvlib" holding "A\.vi"$"#)]
async fn library_with_one_member(world: &mut CliWorld) {
    world.put(
        LIBRARY,
        "<?xml version='1.0' encoding='UTF-8'?>\n<Library LVVersion=\"16008000\">\n\
         \t<Item Name=\"A.vi\" Type=\"VI\" URL=\"../A.vi\"/>\n</Library>\n",
    );
    world.put("A.vi", "a vi");
    world.build.members = vec![LIBRARY, "A.vi"];
}

#[given(
    regex = r#"^library "Lib\.lvlib" holding "A\.vi" and class "C/C\.lvclass", which holds "C/M\.vi"$"#
)]
async fn library(world: &mut CliWorld) {
    world.put(
        LIBRARY,
        "<?xml version='1.0' encoding='UTF-8'?>\n<Library LVVersion=\"16008000\">\n\
         \t<Item Name=\"A.vi\" Type=\"VI\" URL=\"../A.vi\"/>\n\
         \t<Item Name=\"C.lvclass\" Type=\"LVClass\" URL=\"../C/C.lvclass\"/>\n</Library>\n",
    );
    world.put(
        CLASS,
        "<?xml version='1.0' encoding='UTF-8'?>\n<LVClass LVVersion=\"16008000\">\n\
         \t<Item Name=\"C.ctl\" Type=\"Class Private Data\" URL=\"C.ctl\"/>\n\
         \t<Item Name=\"M.vi\" Type=\"VI\" URL=\"../M.vi\">\n\t</Item>\n</LVClass>\n",
    );
    world.put("A.vi", "a vi");
    world.put("C/M.vi", "a method");
    world.build.members = vec![LIBRARY, "A.vi", CLASS, "C/M.vi"];
}

#[given(regex = r#"^"Stray\.vi" next to the library, not a member$"#)]
async fn stray(world: &mut CliWorld) {
    world.put("Stray.vi", "not a member");
}

#[given("no LabVIEW installation is visible to lvpm")]
async fn no_labview(world: &mut CliWorld) {
    world.build.no_labview = true;
}

#[given(regex = r#"^the library has an auto-populating folder "Data" holding "d\.txt" on disk$"#)]
async fn auto_populating(world: &mut CliWorld) {
    world.add_item(
        LIBRARY,
        "</Library>",
        r#"<Item Name="Data" Type="Folder" URL="../Data"/>"#,
    );
    world.put("Data/d.txt", "data");
}

#[given(regex = r#"^\[distribution\] install-dir = "([^"]+)"$"#)]
async fn install_dir(world: &mut CliWorld, dir: String) {
    world.set(&["distribution"], "install-dir", dir);
}

#[given(regex = r#"^dist holds an older "([^"]+)" and "([^"]+)"$"#)]
async fn older(world: &mut CliWorld, package: String, notes: String) {
    world.put(&format!("{DIST}/{package}"), "an older package");
    world.put(&format!("{DIST}/{notes}"), "notes");
}

#[given(regex = r#"^dist holds "([^"]+)"$"#)]
async fn dist_holds_old(world: &mut CliWorld, package: String) {
    world.put(&format!("{DIST}/{package}"), "an old package");
    world.build.earlier = Some(std::fs::read(world.work(DIST).join(package)).expect("the file"));
}

#[given(regex = r#"^the project version "([^"]+)"$"#)]
async fn project_version(world: &mut CliWorld, version: String) {
    world.set(&["project"], "version", version);
}

#[given(
    regex = r#"^dependencies a = "1\.0", b = ">=2\.0", c = "\*" and "@hexagon/lib-a" = "3\.0"$"#
)]
async fn dependencies(world: &mut CliWorld) {
    for (name, c) in [
        ("a", "1.0"),
        ("b", ">=2.0"),
        ("c", "*"),
        ("@hexagon/lib-a", "3.0"),
    ] {
        world.set(&["dependencies"], name, c);
    }
}

#[given(
    regex = r#"^the manifest ((?:has (?:no (?:\[distribution\] table|top-level-library|vendor|display-name)|an empty|\[distribution\.vip)|names|sets).*)$"#
)]
async fn manifest_change(world: &mut CliWorld, change: String) {
    let quoted = |s: &str| s.split('"').nth(1).expect("a quoted value").to_string();
    let array = |s: &str| -> toml::Value {
        let list = s.split_once('=').expect("an array").1.trim();
        toml::from_str::<toml::Table>(&format!("v = {list}")).expect("an array")["v"].clone()
    };
    let dist = ["distribution"];
    let vip = ["distribution", "vip"];
    match change.as_str() {
        "has no [distribution] table" => drop(world.build.manifest.remove("distribution")),
        "has no top-level-library" => drop(world.table(&dist).remove("top-level-library")),
        "has no vendor" => drop(world.table(&dist).remove("vendor")),
        "has no display-name" => drop(world.table(&dist).remove("display-name")),
        "has an empty [distribution.vip] table" => drop(world.table(&vip)),
        "has [distribution.vip] with vendor, display-name, name and icon" => {
            for key in ["vendor", "display-name", "name", "icon"] {
                world.set(&vip, key, "x");
            }
        }
        c if c.starts_with("has [distribution.vip") => {
            // `has [distribution.vip.dependency-names] a = "b"`: the table, then one key.
            let (table, pair) = c["has [".len()..]
                .split_once("] ")
                .expect("a table and a pair");
            let table: Vec<&str> = table.split('.').collect();
            let (key, value) = pair.split_once(" = ").expect("a key and a value");
            world.set(&table, key, value.trim_matches('"'));
        }
        c if c.starts_with("names top-level-library") => {
            world.set(&dist, "top-level-library", quoted(c));
        }
        c if c.starts_with("sets kinds") => world.set(&dist, "kinds", array(c)),
        c if c.starts_with("sets formats") => world.set(&dist, "formats", array(c)),
        "sets [distribution] colour = \"red\"" => world.set(&dist, "colour", "red"),
        c if c.starts_with("sets install-dir") => world.set(&dist, "install-dir", quoted(c)),
        c if c.starts_with("sets version") => world.set(&["project"], "version", quoted(c)),
        c if c.starts_with("names the project") => world.set(&["project"], "name", quoted(c)),
        other => panic!("no such manifest change: {other}"),
    }
}

#[given(regex = r#"^the library lists "Gone\.vi", which is not on disk$"#)]
async fn lists_gone(world: &mut CliWorld) {
    world.add_item(
        LIBRARY,
        "</Library>",
        r#"<Item Name="Gone.vi" Type="VI" URL="../Gone.vi"/>"#,
    );
}

#[given(regex = r#"^class "C/C\.lvclass" lists "C/Lost\.vi", not on disk$"#)]
async fn class_lists_lost(world: &mut CliWorld) {
    world.add_item(
        CLASS,
        "</LVClass>",
        r#"<Item Name="Lost.vi" Type="VI" URL="../Lost.vi"/>"#,
    );
}

#[given(regex = r#"^the library lists "\.\./\.\./Outside\.vi", above its folder$"#)]
async fn lists_outside(world: &mut CliWorld) {
    world.add_item(
        LIBRARY,
        "</Library>",
        r#"<Item Name="Outside.vi" Type="VI" URL="../../Outside.vi"/>"#,
    );
    std::fs::write(world.sandbox.root().join("Outside.vi"), "outside").expect("write a file");
}

#[given(regex = r#"^"Lib\.lvlib" is not a readable library$"#)]
async fn unreadable(world: &mut CliWorld) {
    world.put(LIBRARY, "this is not a library");
}

#[given(regex = r#"^dist holds a built "([^"]+)"$"#)]
async fn built_before(world: &mut CliWorld, _package: String) {
    world.build_keeping_the_package();
}

#[when("lvpm build runs")]
async fn build_runs(world: &mut CliWorld) {
    world.lvpm_build_unchecked();
}

#[when(
    "lvpm build runs, every member's modification time is set to a different value, dist is emptied, and lvpm build runs again"
)]
async fn build_twice(world: &mut CliWorld) {
    world.build_keeping_the_package();
    let then = SystemTime::now() - Duration::from_secs(86_400);
    for (i, rel) in world.build.members.clone().iter().enumerate() {
        let file = std::fs::File::options()
            .write(true)
            .open(world.work(rel))
            .expect("a member");
        file.set_modified(then + Duration::from_secs(i as u64 * 60))
            .expect("set a time");
    }
    std::fs::remove_dir_all(world.work(DIST)).expect("empty dist");
    world.lvpm_build_unchecked();
}

#[then(regex = r#"^it succeeds, prints the path of "([^"]+)", and dist holds no \.vip$"#)]
async fn succeeds_printing_path(world: &mut CliWorld, rel: String) {
    world.succeeded();
    let out = world.built();
    assert!(
        out.stdout.starts_with("built "),
        "lvpm build printed {:?}",
        out.stdout
    );
    assert!(
        world.message_names(&out.stdout, &rel),
        "{out:?} does not name {rel}"
    );
    assert!(world.package_path().is_file());
    let vips: Vec<_> = std::fs::read_dir(world.work(DIST))
        .expect("dist")
        .flatten()
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().ends_with(".vip"))
        .collect();
    assert!(vips.is_empty(), "dist holds {vips:?}");
}

async fn succeeds_holding(world: &mut CliWorld, package: String) {
    world.succeeded();
    assert!(world.work(DIST).join(package).is_file());
}

#[then("it succeeds")]
async fn succeeds(world: &mut CliWorld) {
    world.succeeded();
}

#[then(
    regex = r#"^the package's package.toml reads as (\S+) (\S+), labview ("[^"]+", kind "[^"]+", vendor "[^"]+", display-name "[^"]+", install-dir "[^"]+")$"#
)]
async fn package_toml_reads(world: &mut CliWorld, name: String, version: String, rest: String) {
    world.succeeded();
    let toml = world.package_toml();
    let text = |table: &str, key: &str| toml[table][key].as_str().expect("a string").to_string();
    // No [dependencies] and no lock: the package carries an SBOM with no components.
    assert_eq!(toml["format"].as_integer(), Some(2));
    assert_eq!(text("project", "name"), name);
    assert_eq!(text("project", "version"), version);
    // `">=2016", kind "source", vendor "V", ...`: a value, then key-value pairs.
    let (labview, pairs) = rest.split_once(", ").expect("labview and more");
    assert_eq!(text("project", "labview"), labview.trim_matches('"'));
    for pair in pairs.split(", ") {
        let (key, value) = pair.split_once(' ').expect("a key and a value");
        assert_eq!(text("distribution", key), value.trim_matches('"'), "{key}");
    }
    // What makes two builds byte-identical: `RECORD`, `package.toml` and the
    // SBOM first, the payload in order, the zip epoch and one compression
    // method throughout.
    let mut archive = world.archive();
    let names: Vec<String> = archive.file_names().map(String::from).collect();
    let mut payload = names[3..].to_vec();
    payload.sort();
    assert_eq!(names[..3], ["RECORD", "package.toml", SBOM]);
    assert_eq!(names[3..], payload[..], "the payload is in sorted order");
    for i in 0..archive.len() {
        let entry = archive.by_index(i).expect("an entry");
        assert_eq!(
            entry.last_modified(),
            Some(zip::DateTime::default()),
            "{}",
            entry.name()
        );
        assert_eq!(
            entry.compression(),
            zip::CompressionMethod::Deflated,
            "{}",
            entry.name()
        );
        assert!(
            entry.extra_data().is_none_or(|x| x.is_empty()),
            "{} has extra fields",
            entry.name()
        );
    }
}

#[then(
    regex = r#"^the file group holds Lib\.lvlib, A\.vi, C/C\.lvclass and C/M\.vi under "([^"]+)"$"#
)]
async fn holds_all_under(world: &mut CliWorld, dir: String) {
    world.succeeded();
    let want: Vec<String> = ["A.vi", "C/C.lvclass", "C/M.vi", "Lib.lvlib"]
        .iter()
        .map(|f| format!("{dir}/{f}"))
        .collect();
    assert_eq!(world.group_files(), want);
}

#[then(regex = r#"^the file group holds the members under "([^"]+)"$"#)]
async fn members_under(world: &mut CliWorld, dir: String) {
    holds_all_under(world, dir).await;
}

#[then(regex = r#"^the package holds no "([^"]+)"$"#)]
async fn holds_no(world: &mut CliWorld, file: String) {
    world.succeeded();
    let names: Vec<String> = world.archive().file_names().map(String::from).collect();
    assert!(
        !names.iter().any(|n| n.ends_with(&format!("/{file}"))),
        "{names:?}"
    );
}

#[then(regex = r#"^the file group holds "([^"]+)"$"#)]
async fn holds_one(world: &mut CliWorld, a: String) {
    holds(world, &[a]);
}

#[then(regex = r#"^the file group holds "([^"]+)" and "([^"]+)"$"#)]
async fn holds_two(world: &mut CliWorld, a: String, b: String) {
    holds(world, &[a, b]);
}

fn holds(world: &CliWorld, wanted: &[String]) {
    world.succeeded();
    let files = world.group_files();
    for want in wanted {
        assert!(
            files.iter().any(|f| f.ends_with(&format!("/{want}"))),
            "no {want} in {files:?}"
        );
    }
}

#[then("both packages are byte-identical")]
async fn identical(world: &mut CliWorld) {
    world.succeeded();
    let now = world.package_bytes();
    assert!(
        world.build.earlier.as_ref() == Some(&now),
        "the two builds differ"
    );
}

#[then(regex = r#"^"([^"]+)" holds the current members and "([^"]+)" is unchanged$"#)]
async fn replaced(world: &mut CliWorld, _package: String, notes: String) {
    holds_all_under(world, "vi.lib/Vendor/Demo Lib".into()).await;
    let notes = std::fs::read_to_string(world.work(DIST).join(notes)).expect("the notes");
    assert_eq!(notes, "notes");
}

#[then(regex = r#"^dist holds "([^"]+)" and its package.toml version is "([^"]+)"$"#)]
async fn version_written(world: &mut CliWorld, package: String, version: String) {
    succeeds_holding(world, package).await;
    assert_eq!(
        world.package_toml()["project"]["version"].as_str(),
        Some(version.as_str())
    );
}

#[then("package.toml's [dependencies] holds those four, as written, in that order")]
async fn dependencies_as_written(world: &mut CliWorld) {
    world.succeeded();
    let toml = world.package_toml();
    let deps: Vec<(&str, &str)> = toml["dependencies"]
        .as_table()
        .expect("a [dependencies] table")
        .iter()
        .map(|(name, c)| (name.as_str(), c.as_str().expect("a string")))
        .collect();
    assert_eq!(
        deps,
        [
            ("a", "1.0"),
            ("b", ">=2.0"),
            ("c", "*"),
            ("@hexagon/lib-a", "3.0")
        ]
    );
}

#[then(regex = r#"^"([^"]+)" is unchanged$"#)]
async fn old_package_unchanged(world: &mut CliWorld, name: String) {
    world.succeeded();
    let now = std::fs::read(world.work(DIST).join(name)).expect("the old package");
    assert!(
        world.build.earlier.as_ref() == Some(&now),
        "the old package changed"
    );
}

#[then(
    regex = r#"^it fails, the message names the key "([^"]+)" exactly, and dist holds no [.]lvpkg$"#
)]
async fn fails_naming_key(world: &mut CliWorld, key: String) {
    let message = world.failed();
    assert!(
        names_key(&message, &key),
        "the message does not name {key}: {message}"
    );
    world.no_package_in_dist();
}

/// `message` names `key` itself, not a longer one it begins: `distribution`
/// must not be found only inside `distribution.vendor`.
pub(super) fn names_key(message: &str, key: &str) -> bool {
    message.match_indices(key).any(|(at, _)| {
        message[at + key.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || "-_.".contains(c)))
    })
}

#[then(regex = r#"^it fails naming "([^"]+)" and dist holds no [.]lvpkg$"#)]
async fn fails_naming(world: &mut CliWorld, name: String) {
    let message = world.failed();
    assert!(
        message.contains(&name),
        "the message does not name {name}: {message}"
    );
    world.no_package_in_dist();
}

#[then(regex = r#"^it fails with the message `(.+)`, and dist holds no [.]lvpkg$"#)]
async fn fails_with_message(world: &mut CliWorld, text: String) {
    let message = world.failed();
    assert!(message.contains(&text), "{text:?} is not in: {message}");
    world.no_package_in_dist();
}

/// The first part of each removed-key message (S7a), asserted word for word.
fn removed_key_message(case: &str) -> &'static str {
    match case {
        "formats" => {
            "distribution.formats was removed (ADR 6, docs/adr/0006-lvpkg-only-vip-read-only.md): \
             lvpm build always writes a .lvpkg. Delete the line."
        }
        "[distribution.vip]" => {
            "[distribution.vip] was removed (ADR 6, docs/adr/0006-lvpkg-only-vip-read-only.md). \
             Move vendor, display-name and install-dir into [distribution]; the package name is \
             [project] name; icon and dependency-names are gone. Found: "
        }
        other => panic!("no such removed-key message: {other}"),
    }
}

#[then(regex = r#"^it fails with the "([^"]+)" message, and dist is empty$"#)]
async fn fails_with_removed_key_message(world: &mut CliWorld, case: String) {
    let message = world.failed();
    let want = removed_key_message(&case);
    assert!(message.contains(want), "{want:?} is not in: {message}");
    world.no_package_in_dist();
}

#[then(regex = r#"^the message's "Found:" lists (.+)$"#)]
async fn found_lists(world: &mut CliWorld, keys: String) {
    let message = world.failed();
    let found = format!("Found: {}", keys.replace(" and ", ", "));
    assert!(
        message.lines().any(|l| l.ends_with(&found)),
        "no line ends with {found:?}: {message}"
    );
}

#[then("it fails and that package is unchanged")]
async fn fails_unchanged(world: &mut CliWorld) {
    world.failed();
    let now = world.package_bytes();
    assert!(
        world.build.earlier.as_ref() == Some(&now),
        "the package changed"
    );
}

// ---- the SBOM in the package ----

const ANY_VERSION: &str = ">=1.0";
/// The consumer's oldest LabVIEW, and the one the library locks for: newer
/// than the fixtures' `20.0`.
const LABVIEW_MINIMUM: &str = ">=2024";
const DEMO_LIB: &str = "demo_lib-1.2.3.lvpkg";
const INSTALL_DIR: &str = "vi.lib/Vendor/Demo Lib";

const fn text_package(
    name: &'static str,
    version: &'static str,
    requires: &'static [(&'static str, &'static str)],
) -> FixturePackage {
    FixturePackage {
        name,
        version,
        display_name: name,
        requires,
        lv_min: "20.0",
        payload: Payload::Text,
        post_install_hook: None,
    }
}

/// What the SBOM scenarios lock from: alpha 1.0 requiring beta >= 2.0, beta
/// 2.0, and gamma 1.0.
const SBOM_PACKAGES: &[FixturePackage] = &[
    text_package("alpha", "1.0", &[("beta", "2.0")]),
    text_package("beta", "2.0", &[]),
    text_package("gamma", "1.0", &[]),
];

/// The entry `name` of the zip `bytes`, when it has one.
fn entry_of(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip");
    let mut content = Vec::new();
    zip.by_name(name).ok()?.read_to_end(&mut content).ok()?;
    Some(content)
}

impl CliWorld {
    fn package_bytes(&self) -> Vec<u8> {
        std::fs::read(self.package_path()).expect("the package")
    }

    /// `lvpm build`, which must succeed, and the package it wrote kept as the
    /// earlier one.
    fn build_keeping_the_package(&mut self) {
        self.lvpm_build_unchecked();
        self.succeeded();
        self.build.earlier = Some(self.package_bytes());
    }

    /// The package's SBOM text, as JSON.
    fn sbom(&self) -> Value {
        let text = entry_of(&self.package_bytes(), SBOM).expect("a bom.cdx.json entry");
        serde_json::from_slice(&text).expect("an SBOM that is JSON")
    }

    /// `lvpm lock` for the manifest the scenario's table says.
    fn lock_now(&self) {
        self.write_build_manifest();
        self.sandbox.setup_lvpm(&["lock"]);
    }

    /// The package, under its name, in the folder lvpm reads packages from.
    fn put_demo_lib_in_package_folder(&self, bytes: &[u8]) {
        let path = self.sandbox.root().join(LOCAL_PACKAGES_DIR).join(DEMO_LIB);
        std::fs::write(path, bytes).expect("write a package into the folder");
    }
}

/// A library project with the fixtures it may depend on published.
async fn library_project(world: &mut CliWorld) {
    project(
        world,
        "demo_lib".into(),
        "1.2.3".into(),
        LABVIEW_MINIMUM.into(),
    )
    .await;
    distribution(
        world,
        "Lib.lvlib".into(),
        "Vendor".into(),
        "Demo Lib".into(),
    )
    .await;
    library_with_one_member(world).await;
    world.set(&["sources"], "defaults", false);
    world.publish(&SBOM_PACKAGES.iter().collect::<Vec<_>>());
}

async fn library_depending_on(world: &mut CliWorld, name: String, version: String) {
    library_project(world).await;
    world.set(&["dependencies"], &name, version);
}

#[given(
    regex = r#"^a library project depending on "([^"]+)" ([\d.]+), which requires "([^"]+)" ([\d.]+)$"#
)]
async fn library_requiring(
    world: &mut CliWorld,
    name: String,
    version: String,
    required: String,
    required_version: String,
) {
    let fixture = &SBOM_PACKAGES[0];
    assert_eq!(
        (fixture.name, fixture.version, fixture.requires),
        (
            name.as_str(),
            version.as_str(),
            &[(required.as_str(), required_version.as_str())][..]
        ),
        "the feature and the fixture table disagree"
    );
    library_depending_on(world, name, version).await;
}

#[given(regex = r#"^"([^"]+)" ([\d.]+) only in \[dev-dependencies\]$"#)]
async fn only_a_dev_dependency(world: &mut CliWorld, name: String, version: String) {
    world.set(&["dev-dependencies"], &name, version);
}

#[given("lvpm.lock is current")]
async fn lock_is_current(world: &mut CliWorld) {
    world.lock_now();
}

#[given(regex = r#"^a library project depending on "([^"]+)" ([\d.]+) and lvpm\.lock is current$"#)]
async fn library_with_a_current_lock(world: &mut CliWorld, name: String, version: String) {
    library_depending_on(world, name, version).await;
    world.lock_now();
}

#[given(regex = r#"^a library project depending on "([^"]+)" ([\d.]+) and no lvpm\.lock$"#)]
async fn library_with_no_lock(world: &mut CliWorld, name: String, version: String) {
    library_depending_on(world, name, version).await;
}

#[given(
    regex = r#"^a library project depending on "([^"]+)" ([\d.]+) and a lock written before "([^"]+)" was added to \[dependencies\]$"#
)]
async fn library_with_a_stale_lock(
    world: &mut CliWorld,
    name: String,
    version: String,
    added: String,
) {
    library_depending_on(world, name, version).await;
    world.lock_now();
    world.set(&["dependencies"], &added, "2.0");
}

#[given(regex = r"^a library project with no \[dependencies\] and no lvpm\.lock$")]
async fn library_with_no_dependencies(world: &mut CliWorld) {
    library_project(world).await;
}

#[given(
    regex = r#"^a library project depending on "([^"]+)" ([\d.]+) and an lvpm\.lock that cannot be read$"#
)]
async fn library_with_an_unreadable_lock(world: &mut CliWorld, name: String, version: String) {
    library_depending_on(world, name, version).await;
    std::fs::write(world.lock_path(), "this is not a lock = =").expect("write lvpm.lock");
}

#[when("lvpm build --no-check runs")]
async fn build_without_check(world: &mut CliWorld) {
    world.lvpm_build_unchecked();
}

#[when("lvpm build runs twice, keeping the first package")]
async fn build_twice_keeping_the_first(world: &mut CliWorld) {
    world.build_keeping_the_package();
    world.lvpm_build_unchecked();
}

#[then(regex = r#"^stdout has a line ending "([^"]+)"$"#)]
async fn stdout_has_a_line_ending(world: &mut CliWorld, end: String) {
    let out = world.built();
    assert!(out.stdout.lines().any(|l| l.ends_with(&end)), "{out:?}");
}

#[then(regex = r#"^stderr has a line starting "([^"]+)" and ending "([^"]+)"$"#)]
async fn stderr_has_a_line_starting_and_ending(world: &mut CliWorld, start: String, end: String) {
    let out = world.built();
    let found = out
        .stderr
        .lines()
        .any(|l| l.starts_with(&start) && l.ends_with(&end));
    assert!(found, "{out:?}");
}

#[then(regex = r#"^the package's RECORD lists "([^"]+)" at the root$"#)]
async fn record_lists_at_the_root(world: &mut CliWorld, file: String) {
    world.succeeded();
    let record = entry_of(&world.package_bytes(), lvpkg::RECORD).expect("a RECORD");
    let record = String::from_utf8(record).expect("a text RECORD");
    let at_root = record
        .lines()
        .any(|l| l.split_once("  ").map(|(_, path)| path) == Some(&file));
    assert!(at_root, "{file} is not at the root in:\n{record}");
}

#[then(regex = r"^its package\.toml says format = (\d+)$")]
async fn package_toml_says_format(world: &mut CliWorld, format: i64) {
    assert_eq!(world.package_toml()["format"].as_integer(), Some(format));
}

#[then(regex = r#"^its bom\.cdx\.json has specVersion "([^"]+)" and bomFormat "([^"]+)"$"#)]
async fn sbom_says_its_format(world: &mut CliWorld, spec_version: String, bom_format: String) {
    let sbom = world.sbom();
    assert_eq!(sbom["specVersion"], spec_version.as_str(), "{sbom}");
    assert_eq!(sbom["bomFormat"], bom_format.as_str(), "{sbom}");
}

#[then(regex = r"^its bom\.cdx\.json lists exactly (.+), each with the lock's SHA-256$")]
async fn sbom_lists_exactly(world: &mut CliWorld, listed: String) {
    let want: Vec<(String, String)> = listed
        .split(" and ")
        .map(|item| {
            let (name, version) = item.split_once(' ').expect("a name and a version");
            (name.to_string(), version.to_string())
        })
        .collect();
    let sbom = world.sbom();
    let components = sbom["components"].as_array().expect("components");
    let got: Vec<(String, String)> = components
        .iter()
        .map(|c| {
            (
                c["name"].as_str().expect("a name").to_string(),
                c["version"].as_str().expect("a version").to_string(),
            )
        })
        .collect();
    assert_eq!(got, want, "{sbom}");
    for component in components {
        let name = component["name"].as_str().expect("a name");
        let locked = world.locked_package(name);
        let hash = &component["hashes"][0];
        assert_eq!(hash["alg"], "SHA-256", "{name}");
        assert_eq!(
            hash["content"].as_str(),
            locked["sha256"].as_str(),
            "{name}"
        );
    }
}

#[then(regex = r#"^lvpm\.lock lists "([^"]+)" in the group "([^"]+)"$"#)]
async fn lock_lists_in_group(world: &mut CliWorld, name: String, group: String) {
    let locked = world.locked_package(&name);
    let groups = locked["groups"].as_array().expect("groups");
    assert!(
        groups.iter().any(|g| g.as_str() == Some(&group)),
        "{locked:?}"
    );
}

#[then(regex = r#"^its bom\.cdx\.json has metadata\.component "([^"]+)" ([\d.]+)$"#)]
async fn sbom_describes_the_package(world: &mut CliWorld, name: String, version: String) {
    let sbom = world.sbom();
    let component = &sbom["metadata"]["component"];
    assert_eq!(component["name"], name.as_str(), "{sbom}");
    assert_eq!(component["version"], version.as_str(), "{sbom}");
}

#[then(regex = r#"^(\S+)'s dependencies name "([^"]+)"$"#)]
async fn sbom_dependencies_name(world: &mut CliWorld, package: String, required: String) {
    let sbom = world.sbom();
    let prefix = format!("{package}@");
    let entry = sbom["dependencies"]
        .as_array()
        .expect("dependencies")
        .iter()
        .find(|d| d["ref"].as_str().is_some_and(|r| r.starts_with(&prefix)))
        .unwrap_or_else(|| panic!("no dependencies entry for {package}: {sbom}"));
    let names: Vec<&str> = entry["dependsOn"]
        .as_array()
        .expect("dependsOn")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(names, [required.as_str()], "{sbom}");
}

#[then("the bom.cdx.json inside both packages is byte-identical")]
async fn sboms_are_identical(world: &mut CliWorld) {
    world.succeeded();
    let first = world.build.earlier.as_deref().expect("a first package");
    let first = entry_of(first, SBOM).expect("an SBOM in the first package");
    let second = entry_of(&world.package_bytes(), SBOM).expect("an SBOM in the second package");
    assert_eq!(first, second);
}

#[then(regex = r#"^its bom\.cdx\.json has metadata\.component type "([^"]+)" and no components$"#)]
async fn sbom_of_a_library_with_no_components(world: &mut CliWorld, kind: String) {
    let sbom = world.sbom();
    assert_eq!(
        sbom["metadata"]["component"]["type"],
        kind.as_str(),
        "{sbom}"
    );
    assert_eq!(
        sbom["components"].as_array().map(Vec::len),
        Some(0),
        "{sbom}"
    );
}

#[then("the package has no bom.cdx.json")]
async fn package_has_no_sbom(world: &mut CliWorld) {
    world.succeeded();
    assert_eq!(entry_of(&world.package_bytes(), SBOM), None);
}

#[given(regex = r#"^a built "([^"]+)" with bom\.cdx\.json$"#)]
async fn a_built_package_with_an_sbom(world: &mut CliWorld, file: String) {
    assert_eq!(file, DEMO_LIB);
    library_project(world).await;
    world.lvpm_build_unchecked();
    world.succeeded();
    let bytes = world.package_bytes();
    assert!(entry_of(&bytes, SBOM).is_some(), "the build wrote no SBOM");
    world.put_demo_lib_in_package_folder(&bytes);
}

/// A package from before the SBOM: `lvpm build` writes no such package for a
/// library with no dependencies.
#[given(regex = r#"^a built "([^"]+)" written without bom\.cdx\.json$"#)]
async fn a_built_package_without_an_sbom(world: &mut CliWorld, file: String) {
    assert_eq!(file, DEMO_LIB);
    let meta = lvpkg::Meta {
        name: "demo_lib".into(),
        version: "1.2.3".into(),
        labview: ">=2016".into(),
        vendor: "Vendor".into(),
        display_name: "Demo Lib".into(),
        install_dir: INSTALL_DIR.into(),
        dependencies: Vec::new(),
    };
    let files = [("A.vi".to_string(), b"a vi".to_vec())];
    let bytes = lvpkg::write(&meta, &files, None).expect("a package");
    assert_eq!(entry_of(&bytes, SBOM), None);
    world.put_demo_lib_in_package_folder(&bytes);
}

/// The package is installed into the scratch destination `dest`, which stands
/// for the venv: a venv needs a LabVIEW, and the sandbox refuses to touch a
/// real one.
#[when("it is installed into a venv")]
async fn installed_into_a_venv(world: &mut CliWorld) {
    world.sandbox.use_local_project(Some("2024"));
    world.depend(DependencyTable::Dependencies, "demo_lib", ANY_VERSION);
    world.sandbox.make_empty_dest();
    let output = world
        .sandbox
        .run_lvpm(&["install", "--prefix", DEST_DIR], None);
    world.record_output(output);
}

#[then("the install succeeds")]
async fn the_install_succeeds(world: &mut CliWorld) {
    world.succeeded();
    let installed = world.sandbox.list_file_names(DEST_DIR);
    assert!(
        installed
            .lines()
            .any(|l| l.ends_with(&format!("{INSTALL_DIR}/A.vi"))),
        "dest holds:\n{installed}"
    );
}

#[then("no bom.cdx.json is installed")]
async fn no_sbom_is_installed(world: &mut CliWorld) {
    let installed = world.sandbox.list_file_names(DEST_DIR);
    assert!(!installed.contains(SBOM), "dest holds:\n{installed}");
}

// ---- the application build, and what it refuses ----

/// The `.lvproj` and the build specification `[application]` names unless a
/// scenario breaks them. The specification is the one NI's template has.
const APP_PROJECT: &str = "app/Main.lvproj";
const APP_SPEC: &str = "Continuous Measurement and Logging Application";
/// The library members a project that builds both kinds holds.
const LIBRARY_MEMBERS: &str = "\"Lib.lvlib\", \"Good.vi\" and \"Helper.vi\"";
/// What `lvpm lock` resolves `alpha` to: `alpha` and the `beta` it requires.
const LOCKED_BY_ALPHA: [&str; 2] = ["alpha", "beta"];

impl CliWorld {
    /// A project that builds an application: `[application]` as `table`, and
    /// `[dependencies]` of `alpha` when `with_dependency`. It has the
    /// `.lvproj`, a VI beside it, no venv and no lock.
    fn application_project(&mut self, table: toml::Table, with_dependency: bool) {
        let mut tables = Self::application_text(table);
        if with_dependency {
            tables += "\n[dependencies]\nalpha = \"1.0\"\n";
        }
        self.publish(&SBOM_PACKAGES.iter().collect::<Vec<_>>());
        self.check_project(Some(">=2026"), false, &tables, "");
        self.put(APP_PROJECT, "<Project/>");
        self.put("app/Main.vi", "a vi");
    }

    /// `[application]` as `table`, in TOML.
    fn application_text(table: toml::Table) -> String {
        let application = toml::Table::from_iter([("application".to_string(), table.into())]);
        toml::to_string(&application).expect("the table is TOML")
    }

    /// `[application]` with both keys right.
    fn good_application_table() -> toml::Table {
        toml::Table::from_iter([
            ("project".to_string(), APP_PROJECT.into()),
            ("build-spec".to_string(), APP_SPEC.into()),
        ])
    }

    /// A library project (the check fixtures, with a venv) that also has
    /// `[application]`: it builds both kinds.
    fn project_with_both_tables(&self) {
        self.build_project(">=2026", "Lib.lvlib", LIBRARY_MEMBERS);
        self.append_to_manifest(&Self::application_text(Self::good_application_table()));
        self.put(APP_PROJECT, "<Project/>");
    }

    /// The venv's record of `name`, as an install of that archive leaves it.
    fn install_in_venv_as_locked(&self, name: &str, version: &str, sha256: &str) {
        let record = serde_json::json!({
            "name": name, "version": version, "installed_at": 0, "target": "",
            "files": [], "skipped_hooks": [], "archive_sha256": sha256,
        });
        let path = self
            .sandbox
            .venv_dir()
            .join(".lvpm/installed")
            .join(format!("{name}.json"));
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("make the store");
        std::fs::write(path, record.to_string()).expect("write the record");
    }

    /// The venv holds each of `LOCKED_BY_ALPHA` as the lock has it, with the
    /// version `version_of` says.
    fn venv_holding_the_locked_packages(&self, version_of: impl Fn(&str, &str) -> String) {
        self.bind_venv();
        for name in LOCKED_BY_ALPHA {
            let locked = self.locked_package(name);
            let version = locked["version"].as_str().expect("a version");
            let sha256 = locked["sha256"].as_str().expect("a SHA-256");
            self.install_in_venv_as_locked(name, &version_of(name, version), sha256);
        }
    }
}

#[given(regex = r"^an application project whose \[application\] (.+)$")]
async fn application_project_whose_table(world: &mut CliWorld, problem: String) {
    let mut table = CliWorld::good_application_table();
    let text = |text: &str| toml::Value::String(text.to_string());
    let root = world.sandbox.root().to_path_buf();
    // The projects a table can name outside the folder: they exist, and lie
    // beside it or above it.
    let outside = [root.join("other/X.lvproj"), root.join("X.lvproj")];
    match problem.as_str() {
        "has no project" => drop(table.remove("project")),
        "has no build-spec" => drop(table.remove("build-spec")),
        "has build-spec = \"\"" => drop(table.insert("build-spec".into(), text(""))),
        "has project = \"\"" => drop(table.insert("project".into(), text(""))),
        "has project = 5" => drop(table.insert("project".into(), 5.into())),
        "has build-spec = 5" => drop(table.insert("build-spec".into(), 5.into())),
        key if key.starts_with("has the key ") => {
            let words = quoted(key);
            table.insert(words[0].clone(), text(&words[1]));
        }
        name if name.starts_with("names ") => {
            let named = quoted(name).remove(0);
            let named = match named.strip_prefix("<sandbox parent>/") {
                Some(rest) => root.join(rest).to_string_lossy().into_owned(),
                None => named,
            };
            table.insert("project".into(), text(&named));
        }
        other => panic!("no such [application] problem: {other}"),
    }
    world.application_project(table, false);
    for project in outside {
        std::fs::create_dir_all(project.parent().expect("a folder")).expect("make a folder");
        std::fs::write(project, "<Project/>").expect("write a project outside the folder");
    }
}

#[given("an application project with no version in lvpm.toml")]
async fn application_project_without_a_version(world: &mut CliWorld) {
    world.application_project(CliWorld::good_application_table(), false);
    let manifest = std::fs::read_to_string(world.work(MANIFEST_FILE)).expect("the manifest");
    let without = manifest.replace("version = \"0.1.0\"\n", "");
    assert_ne!(manifest, without, "the manifest had a version");
    world.put(MANIFEST_FILE, &without);
}

#[given("an application project with no [distribution]")]
async fn application_project_without_distribution(world: &mut CliWorld) {
    world.application_project(CliWorld::good_application_table(), false);
}

/// A project that builds an application, in the state the step says: what
/// the lock is, and what the venv holds.
#[given(
    regex = r"^an application project (with no lvpm\.lock|whose lock predates a new \[dependencies\] entry|with a current lock and no venv|whose venv lacks a locked package|whose venv has alpha 0\.9 where the lock says 1\.0|whose venv holds a package the lock does not list)$"
)]
async fn application_project_in_a_state(world: &mut CliWorld, state: String) {
    let predates = "whose lock predates a new [dependencies] entry";
    // The lock that predates the entry is written before the entry is.
    let with_dependency = !["with no lvpm.lock", predates].contains(&state.as_str());
    world.application_project(CliWorld::good_application_table(), with_dependency);
    if state == "with no lvpm.lock" {
        return;
    }
    world.sandbox.setup_lvpm(&["lock"]);
    match state.as_str() {
        s if s == predates => world.append_to_manifest("[dependencies]\nalpha = \"1.0\"\n"),
        "whose venv lacks a locked package" => world.bind_venv(),
        "whose venv has alpha 0.9 where the lock says 1.0" => {
            world.venv_holding_the_locked_packages(|name, locked| match name {
                "alpha" => "0.9".to_string(),
                _ => locked.to_string(),
            });
        }
        "whose venv holds a package the lock does not list" => {
            world.venv_holding_the_locked_packages(|_, locked| locked.to_string());
            world.install_in_venv_as_locked("stray", "1.0", "ab");
        }
        _ => {}
    }
}

#[given(regex = r"^a project with only (\[distribution\]|\[application\])$")]
async fn project_with_one_table(world: &mut CliWorld, table: String) {
    match table.as_str() {
        "[distribution]" => world.build_project(">=2026", "Lib.lvlib", LIBRARY_MEMBERS),
        _ => world.application_project(CliWorld::good_application_table(), false),
    }
}

#[given("a project with no [distribution] and no [application]")]
async fn project_with_no_table(world: &mut CliWorld) {
    world.check_project(Some(">=2026"), false, "", "");
}

#[given("a project with [distribution] and [application], lvpm.lock current, and no venv")]
async fn both_tables_current_lock_no_venv(world: &mut CliWorld) {
    world.project_with_both_tables();
    std::fs::remove_dir_all(world.sandbox.venv_dir()).expect("remove the venv");
    world.sandbox.setup_lvpm(&["lock"]);
}

#[given("a project with [distribution] and [application] and no lvpm.lock")]
async fn both_tables_no_lock(world: &mut CliWorld) {
    world.project_with_both_tables();
}

#[given(
    "a project with [distribution] and [application], lvpm.lock current and a venv bound to a LabVIEW that is not installed here"
)]
async fn both_tables_labview_not_installed(world: &mut CliWorld) {
    world.project_with_both_tables();
    world.sandbox.setup_lvpm(&["lock"]);
}

#[then(regex = r#"^stderr has no line starting "([^"]+)"$"#)]
async fn stderr_has_no_line_starting(world: &mut CliWorld, start: String) {
    let out = world.output();
    assert!(
        !out.stderr.lines().any(|l| l.starts_with(&start)),
        "{out:?}"
    );
}

#[then(regex = r#"^stdout mentions "([^"]+)" and "([^"]+)"$"#)]
async fn stdout_mentions(world: &mut CliWorld, first: String, second: String) {
    let out = world.output();
    for text in [first, second] {
        assert!(out.stdout.contains(&text), "{text:?} is not in: {out:?}");
    }
}

#[then("dist holds no package")]
async fn dist_holds_no_package(world: &mut CliWorld) {
    world.no_package_in_dist();
}

// ---- the pilot application ----

const PILOT_APP: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pilot-app");

fn pilot_app_file(rel: &str) -> String {
    let path = PathBuf::from(PILOT_APP).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|why| panic!("{}: {why}", path.display()))
}

/// The scenario reads the repository's files, so there is nothing to set up.
#[given("the repository")]
async fn the_repository(_: &mut CliWorld) {}

#[then(
    regex = r#"^tests/fixtures/pilot-app/lvpm\.toml has \[application\] build-spec "([^"]+)" and both pilots in \[dependencies\]$"#
)]
async fn pilot_app_manifest(_: &mut CliWorld, build_spec: String) {
    let manifest: toml::Table = pilot_app_file(MANIFEST_FILE)
        .parse()
        .expect("the pilot application's manifest is TOML");
    assert_eq!(
        manifest["application"]["build-spec"].as_str(),
        Some(build_spec.as_str())
    );
    for (name, version) in PROJECT_B_DEPENDENCIES {
        assert_eq!(
            manifest["dependencies"][name].as_str(),
            Some(version),
            "[dependencies] {name}"
        );
    }
}

#[then(regex = r#"^tests/fixtures/pilot-app/README\.md names "([^"]+)", "([^"]+)" and "([^"]+)"$"#)]
async fn pilot_app_readme_names(_: &mut CliWorld, a: String, b: String, c: String) {
    let readme = pilot_app_file("README.md");
    for name in [a, b, c] {
        assert!(readme.contains(&name), "the README does not name {name:?}");
    }
}

#[then(regex = r#"^the README has a section "([^"]+)"$"#)]
async fn pilot_app_readme_section(_: &mut CliWorld, heading: String) {
    let has = pilot_app_file("README.md")
        .lines()
        .any(|line| line.starts_with('#') && line.trim_start_matches('#').trim() == heading);
    assert!(has, "the README has no heading {heading:?}");
}

/// The build spec's destination is relative to the `.lvproj`, which sits in
/// `app/`, so `../builds/` is the fixture's `builds/`.
#[then(regex = r#"^the \.lvproj's build spec writes below the fixture's "([^"]+)" folder$"#)]
async fn pilot_app_builds_below(_: &mut CliWorld, folder: String) {
    let lvproj = pilot_app_file("app/Continuous Measurement.lvproj");
    let spec = format!(
        "<Property Name=\"Bld_buildSpecName\" Type=\"Str\">{}</Property>",
        "Continuous Measurement and Logging Application"
    );
    let after = lvproj.split(&spec).nth(1).expect("the build spec is there");
    let item = after.split("</Item>").next().expect("the spec's item ends");
    let path = item
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("<Property Name=\"Destination[0].path\" Type=\"Path\">")
        })
        .expect("the spec has a Destination[0].path");
    assert!(
        path.starts_with(&format!("../{folder}/")),
        "Destination[0].path is {path}"
    );
}

/// The lock and the manifest, with the pilots as the only source, in the
/// sandbox: `app/` is not copied, so `[application] project` is not a file
/// there, and `lock --check` reads the manifest and the lock only.
#[then("the fixture's lvpm.lock is current")]
async fn pilot_app_lock_is_current(world: &mut CliWorld) {
    for file in [MANIFEST_FILE, "lvpm.lock"] {
        world.put(file, &pilot_app_file(file));
    }
    copy_pilots_to(&world.sandbox.add_local_source_folder("pilots"));
    world.run("lvpm lock --check", None);
    let output = world.output();
    assert_eq!(output.exit_code, 0, "{output:?}");
}

#[then(regex = r#"^"([^"]+)" is in tests/fixtures/pilot-app/app$"#)]
async fn pilot_app_has_vi(_: &mut CliWorld, vi: String) {
    let path = PathBuf::from(PILOT_APP).join("app").join(&vi);
    assert!(path.is_file(), "{} is not a file", path.display());
}

/// A VI keeps the file names of its subVIs uncompressed in its link info, so a
/// caller's bytes hold the callee's name.
#[then(regex = r#"^a template VI's bytes name "([^"]+)"$"#)]
async fn a_template_vi_names(_: &mut CliWorld, vi: String) {
    let mut dirs = vec![PathBuf::from(PILOT_APP).join("app")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("read the pilot application") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|ext| ext == "vi")
                && path.file_name().is_some_and(|name| name != vi.as_str())
            {
                let bytes = std::fs::read(&path).expect("read a template VI");
                if bytes
                    .windows(vi.len())
                    .any(|window| window == vi.as_bytes())
                {
                    return;
                }
            }
        }
    }
    panic!("no template VI names {vi:?}");
}

// ---- the pilot application builds, `@labview` ----

/// How long Main.exe must have been running for the scenario to call it up.
const APP_RUN_TIME: Duration = Duration::from_secs(5);

/// The pilot application's files in the work directory, so that `lvpm.toml` is
/// at its root, and the pilots in a source folder beside it.
#[given("a copy of the pilot application and the pilots, side by side in the sandbox")]
async fn copy_of_the_pilot_application(world: &mut CliWorld) {
    let work = world.sandbox.work_dir();
    for entry in std::fs::read_dir(PILOT_APP).expect("read the pilot application") {
        let path = entry.expect("a directory entry").path();
        let name = path.file_name().expect("a file name");
        // What an earlier build in the fixture itself left is not part of it.
        if name == "builds" {
            continue;
        }
        match path.is_dir() {
            true => copy_tree(&path, &work.join(name)),
            false => drop(std::fs::copy(&path, work.join(name)).expect("copy a file")),
        }
    }
    copy_pilots_to(&world.sandbox.add_local_source_folder("pilots"));
}

#[given("lvpm venv create and lvpm sync have run in it")]
async fn venv_created_and_synced(world: &mut CliWorld) {
    require_installation_without_pilots(world);
    world
        .sandbox
        .setup_lvpm(&["venv", "create", "--labview-version", LABVIEW_YEAR]);
    world.sandbox.setup_lvpm(&["sync"]);
}

#[given("LabVIEW runs headless")]
async fn labview_runs_headless(world: &mut CliWorld) {
    world.sandbox.set_headless(true);
}

impl CliWorld {
    /// Main.exe, as `lvpm build --app` printed its path: the first output of
    /// the build, which the sandbox's output normalising has left spelled in
    /// either separator. Its path below the work directory, starting at `builds/`.
    fn built_app(&self) -> String {
        let printed = self.printed();
        let (_, line) = printed
            .lines()
            .find_map(|line| {
                line.strip_prefix("built application \"")?
                    .split_once("\" in ")
            })
            .unwrap_or_else(|| panic!("no \"built application\" line in:\n{printed}"));
        let path = line
            .split_once(": ")
            .unwrap_or_else(|| panic!("no path in {line:?}"))
            .1
            .replace('\\', "/");
        let at = path
            .rfind("builds/")
            .unwrap_or_else(|| panic!("{path:?} is not below builds/"));
        path[at..].to_string()
    }
}

#[then(
    regex = r#"^"builds/<the build spec's folder>/Main\.exe" exists, with bom\.cdx\.json next to it$"#
)]
async fn the_application_and_its_sbom_exist(world: &mut CliWorld) {
    let exe = world.work(&world.built_app());
    assert_eq!(exe.file_name().and_then(|n| n.to_str()), Some("Main.exe"));
    assert!(exe.is_file(), "{} is not a file", exe.display());
    let sbom = exe.with_file_name(SBOM);
    assert!(sbom.is_file(), "{} is not a file", sbom.display());
}

#[then("bom.cdx.json lists both pilots and their locked dependencies")]
async fn the_sbom_lists_the_locked_packages(world: &mut CliWorld) {
    let sbom = world.work(&world.built_app()).with_file_name(SBOM);
    let text = std::fs::read_to_string(&sbom).expect("read the SBOM");
    let sbom: Value = serde_json::from_str(&text).expect("the SBOM is JSON");
    let listed: Vec<(&str, &str)> = sbom["components"]
        .as_array()
        .expect("components")
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap_or(""),
                c["version"].as_str().unwrap_or(""),
            )
        })
        .collect();
    // The lock is the copy in the work directory, so the versions are its own.
    let locked = world.locked_versions();
    for (name, version) in PROJECT_B_DEPENDENCIES {
        assert!(
            locked.contains(&(name.to_string(), version.to_string())),
            "{name} {version}"
        );
    }
    assert!(
        locked.len() > PROJECT_B_DEPENDENCIES.len(),
        "no transitive package is locked"
    );
    for (name, version) in &locked {
        assert!(
            listed.contains(&(name, version)),
            "{name} {version} is not in: {listed:?}"
        );
    }
}

/// Ends the application it holds, whether the step that launched it passed or panicked.
struct RunningApp(Child);

impl Drop for RunningApp {
    fn drop(&mut self) {
        // Already gone is fine: `kill` then says so, and there is nothing to wait for.
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

#[then("Main.exe, once launched, is still running after 5 seconds")]
async fn the_application_keeps_running(world: &mut CliWorld) {
    let exe = world.work(&world.built_app());
    let mut command = Command::new(&exe);
    command.current_dir(exe.parent().expect("a folder"));
    // The sandbox's own environment, headless included, as the build ran in.
    world.sandbox.apply_environment(&mut command);
    let mut app = RunningApp(
        command
            .spawn()
            .unwrap_or_else(|why| panic!("launch {}: {why}", exe.display())),
    );
    std::thread::sleep(APP_RUN_TIME);
    let status = app.0.try_wait().expect("ask whether Main.exe has ended");
    assert!(
        status.is_none(),
        "Main.exe ended within {APP_RUN_TIME:?}: {status:?}"
    );
}
