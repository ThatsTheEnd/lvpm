//! Steps of the manifest-editing `@core` scenarios: `lvpm-core` edits and
//! reads a `lvpm.toml` in the sandbox's working directory, in this process.
//! Files are compared as bytes and as whole lines, so a change that is not the
//! one asked for cannot hide in a parsed view of the manifest.

use crate::CliWorld;
use crate::support::fixtures::{FixturePackage, Payload};
use crate::support::project::MANIFEST_FILE;
use cucumber::{given, then, when};
use lvpm_core::manifest_edit::{self, DependencyTable};
use lvpm_core::project::{self, Constraint, Project};
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
        manifest_edit::set_file(path, DependencyTable::Dependencies, &name, &constraint)
    });
}

#[when(expr = "{string} = {string} is written as a dev dependency")]
async fn written_as_dev_dependency(world: &mut CliWorld, name: String, constraint: String) {
    world.edit(|path| {
        manifest_edit::set_file(path, DependencyTable::DevDependencies, &name, &constraint)
    });
}

#[when(expr = "{string} is set to {string}")]
async fn set_to(world: &mut CliWorld, name: String, constraint: String) {
    world.edit(|path| {
        manifest_edit::set_file(path, DependencyTable::Dependencies, &name, &constraint)
    });
}

#[when(expr = "{string} is removed")]
async fn removed(world: &mut CliWorld, name: String) {
    world.edit(|path| manifest_edit::remove_file(path, DependencyTable::Dependencies, &name));
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

#[then(regex = r#"^the command fails naming "(\[[a-z-]+\] [^"]+)"$"#)]
async fn the_command_fails_naming_a_manifest_line(world: &mut CliWorld, line: String) {
    world.assert_failed();
    let report = world.printed();
    assert!(report.contains(&line), "{line:?} is not in:\n{report}");
}

/// `said` is a quoted phrase as a table cell writes it, `\"` for each quote
/// inside it.
#[then(regex = r#"^the command fails naming "(\[[a-z-]+\] [^"]+)" and "(.+)"$"#)]
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
