//! Steps of the manifest-editing `@core` scenarios: `lvpm-core` edits and
//! reads a `lvpm.toml` in the sandbox's working directory, in this process.
//! Files are compared as bytes and as whole lines, so a change that is not the
//! one asked for cannot hide in a parsed view of the manifest.

use super::sync::quoted;
use crate::CliWorld;
use crate::support::fixtures::{FixturePackage, GROUP_PACKAGES, Payload};
use crate::support::project::MANIFEST_FILE;
use cucumber::{given, then, when};
use lvpm_core::manifest_edit::{self, DependencyTable};
use lvpm_core::project::{self, Constraint, Project};
use regex::Regex;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

/// The manifest every scenario starts from, then changes as its `Given` says.
const BACKGROUND: &str = "\
# lvpm manifest of the fixture project

[project]
name = \"fixture\"
version = \"0.1.0\"

[sources]
loopback = \"http://127.0.0.1:1/files\"
defaults = false
# nothing else is listed here

[dependencies]
alpha = \"1.0\"
beta = \">=1.0\"
";
const ALPHA_LINE: &str = "alpha = \"1.0\"\n";
const BETA_LINE: &str = "beta = \">=1.0\"\n";
const DEPENDENCIES_HEADER: &str = "[dependencies]";

/// What a manifest scenario has done and seen.
#[derive(Debug, Default)]
pub struct ManifestState {
    /// The file as the scenario's `Given` steps left it.
    before: String,
    /// When the file was last modified, as the `Given` steps set it.
    modified_before: Option<SystemTime>,
    /// The failure of the last edit, if it failed.
    edit_error: Option<String>,
    /// The line of the manifest a `Given` made unparseable (1-based).
    broken_line: Option<usize>,
    /// What reading the manifest gave.
    read: Option<Result<Project, String>>,
}

impl CliWorld {
    fn manifest_path(&self) -> std::path::PathBuf {
        self.sandbox.work_dir().join(MANIFEST_FILE)
    }

    pub(super) fn manifest_text(&self) -> String {
        std::fs::read_to_string(self.manifest_path()).expect("read the manifest")
    }

    /// Replace the manifest, and note it (and its modification time, set long
    /// ago so that a rewrite of identical bytes still shows) as the "before".
    fn write_manifest_as_before(&mut self, text: &str) {
        let path = self.manifest_path();
        std::fs::write(&path, text).expect("write the manifest");
        let long_ago = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open the manifest")
            .set_modified(long_ago)
            .expect("set the modification time");
        self.manifest.before = text.to_string();
        self.manifest.modified_before = Some(long_ago);
    }

    fn edit(&mut self, change: impl FnOnce(&std::path::Path) -> anyhow::Result<()>) {
        let result = change(&self.manifest_path());
        self.manifest.edit_error = result.err().map(|why| format!("{why:#}"));
    }

    /// The manifest after an edit that must have worked.
    fn edited_text(&self) -> String {
        if let Some(why) = &self.manifest.edit_error {
            panic!("the edit failed: {why}");
        }
        self.manifest_text()
    }
}

/// The lines (with their line endings) that `before` and `after` do not share
/// at the start and at the end: what was removed and what was added.
fn changed_lines<'a>(before: &'a str, after: &'a str) -> (Vec<&'a str>, Vec<&'a str>) {
    let before: Vec<&str> = before.split_inclusive('\n').collect();
    let after: Vec<&str> = after.split_inclusive('\n').collect();
    let shared_start = before
        .iter()
        .zip(&after)
        .take_while(|(old, new)| old == new)
        .count();
    let shared_end = before[shared_start..]
        .iter()
        .rev()
        .zip(after[shared_start..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    (
        before[shared_start..before.len() - shared_end].to_vec(),
        after[shared_start..after.len() - shared_end].to_vec(),
    )
}

#[given(
    "a manifest with a header comment, [project], a [sources] table with a trailing comment, and [dependencies] \"alpha\" = \"1.0\", \"beta\" = \">=1.0\""
)]
async fn the_background_manifest(world: &mut CliWorld) {
    world.write_manifest_as_before(BACKGROUND);
}

#[given("[dependencies] holds only \"alpha\"")]
async fn dependencies_hold_only_alpha(world: &mut CliWorld) {
    world.write_manifest_as_before(&BACKGROUND.replace(BETA_LINE, ""));
}

#[given("the manifest has no [dev-dependencies] table")]
async fn no_dev_dependencies_table(world: &mut CliWorld) {
    assert!(!world.manifest.before.contains("[dev-dependencies]"));
}

#[given("\"beta\" = \">=1.0\"  # pinned for DQMH")]
async fn beta_with_a_comment(world: &mut CliWorld) {
    let commented = BACKGROUND.replace(BETA_LINE, "beta = \">=1.0\"  # pinned for DQMH\n");
    world.write_manifest_as_before(&commented);
}

#[given("the manifest uses CRLF line endings")]
async fn crlf_manifest(world: &mut CliWorld) {
    world.write_manifest_as_before(&BACKGROUND.replace('\n', "\r\n"));
}

#[given("the manifest is not valid TOML")]
async fn invalid_manifest(world: &mut CliWorld) {
    let broken = BACKGROUND.replace(BETA_LINE, "beta >=1.0\n");
    let line = broken
        .lines()
        .position(|line| line == "beta >=1.0")
        .expect("the broken line")
        + 1;
    world.write_manifest_as_before(&broken);
    world.manifest.broken_line = Some(line);
}

#[given(expr = "[dev-dependencies] {string} = {string}")]
async fn dev_dependency_entry(world: &mut CliWorld, name: String, constraint: String) {
    let text = format!("{BACKGROUND}\n[dev-dependencies]\n{name} = \"{constraint}\"\n");
    world.write_manifest_as_before(&text);
}

#[when(expr = "{string} = {string} is written to [dependencies]")]
async fn written_to_dependencies(world: &mut CliWorld, name: String, constraint: String) {
    world.edit(|path| {
        manifest_edit::set_file(path, &DependencyTable::Dependencies, &name, &constraint)
    });
}

#[when(expr = "{string} = {string} is written as a dev dependency")]
async fn written_as_dev_dependency(world: &mut CliWorld, name: String, constraint: String) {
    world.edit(|path| {
        manifest_edit::set_file(path, &DependencyTable::DevDependencies, &name, &constraint)
    });
}

#[when(expr = "{string} is set to {string}")]
async fn set_to(world: &mut CliWorld, name: String, constraint: String) {
    world.edit(|path| {
        manifest_edit::set_file(path, &DependencyTable::Dependencies, &name, &constraint)
    });
}

#[when(expr = "{string} is removed")]
async fn removed(world: &mut CliWorld, name: String) {
    world.edit(|path| manifest_edit::remove_file(path, &DependencyTable::Dependencies, &name));
}

const MIN_AGE_KEY: &str = "min-age-days";

impl CliWorld {
    /// Add `line` right under `[sources]` in the project's manifest, after
    /// dropping any `min-age-days` line, so a scenario can set it again.
    fn set_sources_line(&self, line: Option<&str>) {
        let text = self.manifest_text();
        let mut lines: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with(MIN_AGE_KEY))
            .collect();
        let header = lines
            .iter()
            .position(|l| *l == "[sources]")
            .expect("the manifest has a [sources] table");
        lines.splice(header + 1..header + 1, line);
        std::fs::write(self.manifest_path(), lines.join("\n") + "\n").expect("write lvpm.toml");
    }
}

#[given(regex = r"^the project sets min-age-days to (absent|\d+)$")]
#[when(regex = r"^the project sets min-age-days to (absent|\d+)$")]
async fn the_project_sets_min_age_days(world: &mut CliWorld, value: String) {
    let line = format!("{MIN_AGE_KEY} = {value}");
    world.set_sources_line((value != "absent").then_some(line.as_str()));
}

#[given(regex = r"^the project's \[sources\] has the line '(.+)'$")]
async fn the_sources_have_the_line(world: &mut CliWorld, line: String) {
    world.set_sources_line(Some(&line));
}

/// To end of line, so a quoted value may hold quotes: `(got "14")`. Limited to
/// the min-age-days texts: a looser pattern is ambiguous with the other
/// `fails saying "…" …` steps, whose text also holds quotes.
#[then(regex = r#"^the command fails saying "((?:\[sources\] )?min-age-days.+)"$"#)]
async fn the_command_fails_saying(world: &mut CliWorld, text: String) {
    world.assert_failed();
    let stderr = &world.output().stderr;
    assert!(
        stderr.contains(&text),
        "{text:?} is not in stderr:\n{stderr}"
    );
}

#[when("the manifest is read")]
async fn manifest_is_read(world: &mut CliWorld) {
    let read = project::read(&world.manifest_path()).map_err(|why| format!("{why:#}"));
    world.manifest.read = Some(read);
}

#[then("the file differs from before by exactly one added line in [dependencies]")]
async fn one_added_line_in_dependencies(world: &mut CliWorld) {
    let after = world.edited_text();
    let (removed, added) = changed_lines(&world.manifest.before, &after);
    assert_eq!(removed, Vec::<&str>::new());
    assert_eq!(added, ["gamma = \">=1.0\"\n"]);
    let lines: Vec<&str> = after.lines().collect();
    let header = lines
        .iter()
        .position(|line| *line == DEPENDENCIES_HEADER)
        .expect("the [dependencies] header");
    let added_at = lines
        .iter()
        .position(|line| line.starts_with("gamma"))
        .expect("the added line");
    assert!(
        added_at > header,
        "the new entry is above [dependencies]: {after}"
    );
}

#[then("only the value of \"alpha\" differs from before")]
async fn only_alphas_value_differs(world: &mut CliWorld) {
    let after = world.edited_text();
    let (removed, added) = changed_lines(&world.manifest.before, &after);
    assert_eq!(removed, [ALPHA_LINE]);
    assert_eq!(added, ["alpha = \"2.0\"\n"], "the key and layout are kept");
}

#[then("the file differs from before by exactly the \"alpha\" line")]
async fn exactly_alphas_line_is_gone(world: &mut CliWorld) {
    let after = world.edited_text();
    let (removed, added) = changed_lines(&world.manifest.before, &after);
    assert_eq!(removed, [ALPHA_LINE]);
    assert_eq!(added, Vec::<&str>::new());
}

#[then("[dependencies] is present and empty and nothing else changed")]
async fn dependencies_present_and_empty(world: &mut CliWorld) {
    let after = world.edited_text();
    assert_eq!(after, world.manifest.before.replace(ALPHA_LINE, ""));
    assert!(after.lines().any(|line| line == DEPENDENCIES_HEADER));
    let project = project::read(&world.manifest_path()).expect("the edited manifest reads");
    assert_eq!(project.dependencies, vec![]);
}

#[then("a [dev-dependencies] table with that entry is appended and nothing else changed")]
async fn dev_table_appended(world: &mut CliWorld) {
    let after = world.edited_text();
    let appended = after
        .strip_prefix(world.manifest.before.as_str())
        .unwrap_or_else(|| panic!("the file does not start with what it had: {after}"));
    assert_eq!(appended, "\n[dev-dependencies]\ncaraya = \">=1.0\"\n");
}

#[then("that line reads \"beta\" = \">=2.0\"  # pinned for DQMH")]
async fn the_comment_stayed(world: &mut CliWorld) {
    let after = world.edited_text();
    let (removed, added) = changed_lines(&world.manifest.before, &after);
    assert_eq!(removed, ["beta = \">=1.0\"  # pinned for DQMH\n"]);
    assert_eq!(added, ["beta = \">=2.0\"  # pinned for DQMH\n"]);
}

#[then("the file's bytes and modification time are unchanged")]
async fn file_untouched(world: &mut CliWorld) {
    assert_eq!(world.manifest.edit_error, None);
    let path = world.manifest_path();
    assert_eq!(world.manifest_text(), world.manifest.before);
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .expect("the modification time");
    assert_eq!(Some(modified), world.manifest.modified_before);
}

#[then("every line of the file still ends in CRLF")]
async fn every_line_is_crlf(world: &mut CliWorld) {
    let after = world.edited_text();
    assert!(after.contains("gamma = \">=1.0\"\r\n"), "{after:?}");
    assert_eq!(
        after.matches('\n').count(),
        after.matches("\r\n").count(),
        "a line ends in a bare LF: {after:?}"
    );
}

#[then("the edit fails naming the line that cannot be parsed and the file is unchanged")]
async fn edit_failed_naming_the_line(world: &mut CliWorld) {
    let why = world
        .manifest
        .edit_error
        .as_deref()
        .expect("the edit should have failed");
    let line = world.manifest.broken_line.expect("a broken line");
    assert!(
        why.contains(&format!("line {line}")),
        "the error does not name line {line}: {why}"
    );
    assert_eq!(world.manifest_text(), world.manifest.before);
}

#[then("\"caraya\" is a dev dependency with any version")]
async fn caraya_is_a_dev_dependency(world: &mut CliWorld) {
    let project = match world.manifest.read.as_ref().expect("the manifest was read") {
        Ok(project) => project,
        Err(why) => panic!("reading failed: {why}"),
    };
    assert_eq!(
        project.dev_dependencies,
        [("caraya".to_string(), Constraint::Any)]
    );
    assert!(
        project
            .dependencies
            .iter()
            .all(|(name, _)| name != "caraya"),
        "caraya is also a regular dependency"
    );
}

#[then("reading fails naming \"caraya\" and the unsupported constraint \"~1.0\"")]
async fn reading_failed(world: &mut CliWorld) {
    match world.manifest.read.as_ref().expect("the manifest was read") {
        Ok(project) => panic!("reading should have failed, got {project:?}"),
        Err(why) => {
            assert!(why.contains("caraya"), "{why}");
            assert!(why.contains("~1.0"), "{why}");
        }
    }
}

// ---- the table form of a dependency line, run through the CLI ----

/// The LabVIEW the table-form scenarios' project is meant for.
const TABLE_PROJECT_LABVIEW: &str = "2024";

impl CliWorld {
    /// A project that reads nothing, with `line` as the only entry of `[table]`.
    fn write_dependency_line(&self, table: &str, line: &str) {
        self.sandbox.use_local_project(Some(TABLE_PROJECT_LABVIEW));
        let mut text = self.manifest_text();
        text.push_str(&format!("\n[{table}]\n{line}\n"));
        std::fs::write(self.manifest_path(), text).expect("write the manifest");
    }
}

#[given(regex = r"^a project whose lvpm\.toml has the dependency line (.+)$")]
async fn a_project_with_a_dependency_line(world: &mut CliWorld, line: String) {
    world.write_dependency_line("dependencies", &line);
}

#[given(regex = r"^a project whose lvpm\.toml has the dev dependency line (.+)$")]
async fn a_project_with_a_dev_dependency_line(world: &mut CliWorld, line: String) {
    world.write_dependency_line("dev-dependencies", &line);
}

#[given(
    regex = r#"^a project whose lvpm\.toml depends on "([^"]+)" "([^"]*)" with a feed that lists pkg 1\.2$"#
)]
async fn a_project_with_a_feed_that_lists_pkg(
    world: &mut CliWorld,
    name: String,
    constraint: String,
) {
    assert_eq!(name, "pkg", "the feed lists pkg 1.2 and nothing else");
    world.sandbox.serve_feed(&[FixturePackage {
        name: "pkg",
        version: "1.2",
        display_name: "pkg",
        requires: &[],
        lv_min: "20.0",
        payload: Payload::Text,
        post_install_hook: None,
    }]);
    world
        .sandbox
        .use_loopback_project_for_labview(TABLE_PROJECT_LABVIEW);
    world.depend(DependencyTable::Dependencies, &name, &constraint);
}

#[then(regex = r#"^the command fails naming "(\[[^\]"]+\] [^"]+)"$"#)]
async fn the_command_fails_naming_a_manifest_line(world: &mut CliWorld, line: String) {
    world.assert_failed();
    let report = world.printed();
    assert!(report.contains(&line), "{line:?} is not in:\n{report}");
}

/// `said` is a quoted phrase as a table cell writes it, `\"` for each quote
/// inside it.
#[then(regex = r#"^the command fails naming "(\[[^\]"]+\](?: [^"]+)?)" and "(.+)"$"#)]
async fn the_command_fails_naming_a_manifest_line_and_saying(
    world: &mut CliWorld,
    line: String,
    said: String,
) {
    world.assert_failed();
    let report = world.printed();
    let said = said.replace("\\\"", "\"");
    for part in [&line, &said] {
        assert!(report.contains(part), "{part:?} is not in:\n{report}");
    }
}

#[then("lvpm.lock has no [inputs.github] table")]
async fn the_lock_has_no_github_inputs(world: &mut CliWorld) {
    let text = String::from_utf8(world.lock_bytes().expect("lvpm.lock exists"))
        .expect("lvpm.lock is text");
    let lock: toml::Table = text.parse().expect("lvpm.lock is valid TOML");
    assert!(
        lock.get("inputs")
            .is_none_or(|inputs| inputs.get("github").is_none()),
        "{text}"
    );
    // An empty lock would pass the check above.
    assert!(
        !world.locked_versions().is_empty(),
        "nothing is locked:\n{text}"
    );
}

// ---- dependency groups: the manifest grammar ----

/// One `"name" "constraint"` entry of the grammar.
static ENTRY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^"([^"]+)" "([^"]*)"$"#).expect("the pattern"));

#[given(
    r#"fixture packages "alpha" 1.0, "beta" 1.0, "caraya" 1.0 requiring "delta" >= 1.0, "delta" 1.0, "ci_tools" 1.0 and 2.0 requiring "alpha" >= 1.0"#
)]
async fn group_fixture_packages(world: &mut CliWorld) {
    let ids: Vec<String> = GROUP_PACKAGES.iter().map(FixturePackage::id).collect();
    assert_eq!(
        ids,
        [
            "alpha-1.0",
            "beta-1.0",
            "caraya-1.0",
            "delta-1.0",
            "ci_tools-1.0",
            "ci_tools-2.0"
        ],
        "the feature and the fixture table disagree"
    );
    world.publish(&GROUP_PACKAGES.iter().collect::<Vec<_>>());
}

#[given(
    r#"a project with labview "2024" depending on "alpha" "*", with a scratch destination "dest""#
)]
async fn the_group_project(world: &mut CliWorld) {
    world.sandbox.use_local_project(Some(TABLE_PROJECT_LABVIEW));
    world.depend(DependencyTable::Dependencies, "alpha", "*");
    world.sandbox.make_empty_dest();
    world.note_the_project();
}

/// `lvpm.toml has <header> <entries>; <header> <entries>`: entries are
/// `"name" "constraint"` separated by `,`, or `empty`. Anything else is a raw
/// TOML line, so a table can be written wrongly on purpose. `[dependencies]`
/// and `[dev-dependencies]` are merged into the Background's tables, every
/// other table is appended. The tables may be followed by `and a current
/// lvpm.lock` or `and lvpm.lock holds "name" version`: the project is then
/// locked, and noted as it is. Or by `, <a current|a stale|no> lvpm.lock, and
/// <what dest holds>`, which is [`lock_and_dest`].
#[given(
    regex = r#"^lvpm\.toml has (\[.+?)( and a current lvpm\.lock| and lvpm\.lock holds "[^"]+" \S+|, (?:a current lvpm\.lock|a stale lvpm\.lock|no lvpm\.lock), and (?:dest synced with "[^"]*"|"[^"]+" \S+ installed in dest)|)$"#
)]
async fn the_manifest_has_tables(world: &mut CliWorld, tables: String, then_locked: String) {
    for table in tables.split(';').map(str::trim) {
        let (header, entries) = table.split_once(' ').expect("a header and its entries");
        let lines = entries
            .split(',')
            .map(str::trim)
            .filter(|line| *line != "empty");
        let merged = match header {
            "[dependencies]" => Some(DependencyTable::Dependencies),
            "[dev-dependencies]" => Some(DependencyTable::DevDependencies),
            _ => None,
        };
        let Some(merged) = merged else {
            let mut text = format!("{}\n{header}\n", world.manifest_text());
            for line in lines {
                match ENTRY.captures(line) {
                    Some(entry) => text.push_str(&format!("{} = \"{}\"\n", &entry[1], &entry[2])),
                    None => text.push_str(&format!("{line}\n")),
                }
            }
            std::fs::write(world.manifest_path(), text).expect("write the manifest");
            continue;
        };
        for line in lines {
            let entry = ENTRY.captures(line).expect("a name and a constraint");
            world.depend(merged.clone(), &entry[1], &entry[2]);
        }
    }
    if let Some((lock, dest)) = then_locked
        .trim_start_matches([',', ' '])
        .split_once(", and ")
    {
        lock_and_dest(world, lock, dest);
    } else if then_locked.is_empty() {
        world.note_the_project();
    } else {
        world.lock_the_project();
        for (name, version) in QUOTED_AND_VERSION
            .captures_iter(&then_locked)
            .map(|held| (held[1].to_string(), held[2].to_string()))
        {
            assert!(
                world
                    .locked_versions()
                    .contains(&(name.clone(), version.clone())),
                "{name} {version} is not what lvpm.lock holds: {:?}",
                world.locked_versions()
            );
        }
        world.note_the_project();
    }
}

/// The lock a group scenario starts from, and what its destination holds:
/// `a stale lvpm.lock` is one that `beta` was added after, and `no lvpm.lock`
/// leaves none.
fn lock_and_dest(world: &mut CliWorld, lock: &str, dest: &str) {
    if lock != "no lvpm.lock" {
        world.lock_the_project();
    }
    if lock == "a stale lvpm.lock" {
        world.depend(DependencyTable::Dependencies, "beta", "*");
    }
    if let Some(flags) = dest.strip_prefix("dest synced with ") {
        let flags = quoted(flags).join(" ");
        world.sync_into_dest_with(&flags.split_whitespace().collect::<Vec<_>>());
    } else {
        let installed = QUOTED_AND_VERSION
            .captures(dest)
            .expect("a name and a version installed in dest");
        world.install_into_dest(&installed[1], &installed[2]);
    }
    world.note_the_project();
}

/// `"name" 1.0`: a quoted name and the version after it.
static QUOTED_AND_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""([^"]+)" (\S+)"#).expect("the pattern"));

/// The group table of a `lvpm.toml ... [dependency-groups.<name>]` step.
fn group(name: &str) -> DependencyTable {
    DependencyTable::Group(name.to_string())
}

// ---- dependency groups: changing a manifest as a person would ----

#[given(
    regex = r#"^lvpm\.toml now also lists \[dependency-groups\.([\w-]+)\] "([^"]+)" "([^"]*)"$"#
)]
async fn a_group_gains_an_entry(
    world: &mut CliWorld,
    name: String,
    entry: String,
    constraint: String,
) {
    world.depend(group(&name), &entry, &constraint);
}

#[given(regex = r#"^lvpm\.toml gives \[dependency-groups\.([\w-]+)\] "([^"]+)" "([^"]*)"$"#)]
async fn a_group_entry_changes(
    world: &mut CliWorld,
    name: String,
    entry: String,
    constraint: String,
) {
    world.depend(group(&name), &entry, &constraint);
}

#[given(regex = r#"^lvpm\.toml no longer lists \[dependency-groups\.([\w-]+)\] "([^"]+)"$"#)]
async fn a_group_loses_an_entry(world: &mut CliWorld, name: String, entry: String) {
    manifest_edit::remove_file(&world.manifest_path(), &group(&name), &entry)
        .expect("remove the entry from the manifest");
}

/// The group's table goes, from its header to the next table or the end.
#[given(regex = r"^lvpm\.toml has no \[dependency-groups\.([\w-]+)\] table$")]
async fn a_group_table_goes(world: &mut CliWorld, name: String) {
    let text = world.manifest_text();
    let header = format!("[dependency-groups.{name}]");
    let from = text
        .find(&header)
        .expect("the group's table is in the manifest");
    let rest = &text[from + header.len()..];
    let to = rest
        .find("\n[")
        .map_or(text.len(), |at| from + header.len() + at + 1);
    std::fs::write(
        world.manifest_path(),
        format!("{}{}", &text[..from], &text[to..]),
    )
    .expect("write the manifest");
}

/// A lock as lvpm 0.17 wrote it: format 1, and `dev = true` where only dev
/// dependencies need a package.
#[given(regex = r#"^an lvpm\.lock in format 1 with "([^"]+)" marked dev = true$"#)]
async fn a_format_1_lock(world: &mut CliWorld, dev_dependency: String) {
    world.depend(DependencyTable::DevDependencies, &dev_dependency, "*");
    world.lock_the_project();
    let text = String::from_utf8(world.lock_bytes().expect("`lvpm lock` wrote a lock"))
        .expect("lvpm.lock is text");
    let old = text
        .replacen("version = 2\n", "version = 1\n", 1)
        .replace("groups = [\"dev\"]", "dev = true");
    assert!(old.contains("dev = true"), "nothing is dev only:\n{text}");
    std::fs::write(world.lock_path(), old).expect("write the old lock");
    world.note_the_project();
}

// ---- dependency groups: what the lock says ----

/// `lvpm.lock is format 2`, or `... and has no "dev =" line`.
#[then(regex = r#"^lvpm\.lock is format (\d+)(?: and has no "([^"]+)" line)?$"#)]
async fn the_lock_is_in_a_format(world: &mut CliWorld, format: String, absent_line: String) {
    let text = String::from_utf8(world.lock_bytes().expect("lvpm.lock exists")).expect("text");
    assert!(
        text.lines()
            .any(|line| line == format!("version = {format}")),
        "not format {format}:\n{text}"
    );
    assert!(
        absent_line.is_empty() || !text.lines().any(|line| line.starts_with(&absent_line)),
        "a line starts with {absent_line:?}:\n{text}"
    );
}

/// One `"name" and "name" [version] groups ["a", "b"]` or `... no groups`.
static LOCKED_GROUPS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?P<names>"[^"]+"(?: and "[^"]+")*)(?: (?P<version>\d\S*))? (?:groups (?P<groups>\[[^\]]*\])|(?P<none>no groups))"#,
    )
    .expect("the pattern")
});

#[then(regex = r"^lvpm\.lock gives (.+)$")]
async fn the_lock_gives_groups(world: &mut CliWorld, wanted: String) {
    let mut seen = 0;
    for pair in LOCKED_GROUPS.captures_iter(&wanted) {
        let expected = pair
            .name("groups")
            .map_or(Vec::new(), |groups| quoted(groups.as_str()));
        for name in quoted(&pair["names"]) {
            let package = world.locked_package(&name);
            let groups: Vec<String> = package
                .get("groups")
                .and_then(toml::Value::as_array)
                .map_or(Vec::new(), |groups| {
                    groups
                        .iter()
                        .map(|group| group.as_str().expect("a group name").to_string())
                        .collect()
                });
            assert_eq!(groups, expected, "the groups of {name} in {package:?}");
            if let Some(version) = pair.name("version") {
                assert_eq!(
                    package["version"].as_str(),
                    Some(version.as_str()),
                    "{name}"
                );
            }
            seen += 1;
        }
    }
    assert!(seen > 0, "no package named in: {wanted}");
}

#[then(regex = r#"^\[dependency-groups\.([\w-]+)\] says "([^"]+)" = "([^"]*)"$"#)]
async fn the_group_says(world: &mut CliWorld, name: String, entry: String, constraint: String) {
    let manifest: toml::Table = world.manifest_text().parse().expect("the manifest is TOML");
    assert_eq!(
        manifest["dependency-groups"][&name][&entry].as_str(),
        Some(constraint.as_str()),
        "{manifest:?}"
    );
}

// ---- dependency groups: what the command said ----

#[then(regex = r"^the command fails saying lvpm\.lock is out of date: (.+)$")]
async fn fails_out_of_date_because(world: &mut CliWorld, reason: String) {
    world.assert_failed();
    let said = format!("lvpm.lock is out of date: {reason}");
    assert!(
        world.printed().contains(&said),
        "{said:?} is not in:\n{}",
        world.printed()
    );
}

#[then(regex = r#"^the command fails saying "(lvpm\.lock is out of date: .+)"$"#)]
async fn fails_saying_out_of_date_in_full(world: &mut CliWorld, said: String) {
    world.assert_failed();
    assert!(
        world.printed().contains(&said),
        "{said:?} is not in:\n{}",
        world.printed()
    );
}

#[then("the command fails saying the lock format is outdated")]
async fn fails_saying_the_format_is_outdated(world: &mut CliWorld) {
    world.assert_failed();
    let said = "the lock format is outdated (this lvpm writes format 2)";
    assert!(
        world.printed().contains(said),
        "{said:?} is not in:\n{}",
        world.printed()
    );
}

#[then("dest and lvpm.lock are byte-identical to before")]
async fn dest_and_lock_are_byte_identical(world: &mut CliWorld) {
    world.assert_dest_unchanged();
    assert_eq!(world.lock_bytes(), world.lock.before, "the lock changed");
}
