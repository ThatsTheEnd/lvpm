//! Steps of the install-hooks scenarios (S12): `hooks = "run"` on a dependency
//! line. The project reads the loopback feed's `first` folder, as the S13
//! scenarios do, so `"first" lists ...` gives it packages.

use super::sources::FIRST;
use crate::CliWorld;
use crate::support::fixtures::FixturePackage;
use crate::support::global::FIXTURE_NAME;
use crate::support::sandbox::LABVIEW_YEAR;
use cucumber::{given, then, when};
use lvpm_core::manifest_edit::DependencyTable;

const PROJECT_LABVIEW: &str = "2024";
/// A dependency on any version of the package.
const ANY: &str = "*";

impl CliWorld {
    /// A new project whose `[table]` has `name` with `keys` after its version.
    fn project_with_keys(&self, table: DependencyTable, name: &str, constraint: &str, keys: &str) {
        self.sandbox.use_project_sources(PROJECT_LABVIEW, &[FIRST]);
        self.depend_with_keys(table, name, constraint, keys);
    }
}

fn table_of(relation: &str) -> DependencyTable {
    match relation {
        "depends on" => DependencyTable::Dependencies,
        "dev-depends on" => DependencyTable::DevDependencies,
        other => panic!("no relation {other:?}"),
    }
}

/// `hooks` is the TOML value as the scenario writes it: `"run"`, `true`, ...
fn hooks_key(hooks: &str) -> String {
    format!("hooks = {hooks}")
}

#[given(
    regex = r#"^a project that (depends on|dev-depends on) "([^"]+)" with hooks set to (\S+)$"#
)]
async fn project_with_hooks(world: &mut CliWorld, relation: String, name: String, hooks: String) {
    world.project_with_keys(table_of(&relation), &name, ANY, &hooks_key(&hooks));
}

#[given(regex = r#"^the project dev-depends on "([^"]+)" with hooks set to (\S+)$"#)]
async fn dev_depends_with_hooks(world: &mut CliWorld, name: String, hooks: String) {
    world.depend_with_keys(
        DependencyTable::DevDependencies,
        &name,
        ANY,
        &hooks_key(&hooks),
    );
}

#[given(
    regex = r#"^a project that depends on "([^"]+)" with github "([^"]+)", version "([^"]+)" and hooks set to (\S+)$"#
)]
async fn github_project_with_hooks(
    world: &mut CliWorld,
    name: String,
    repo: String,
    constraint: String,
    hooks: String,
) {
    let keys = format!("github = \"{repo}\", {}", hooks_key(&hooks));
    world.project_with_keys(DependencyTable::Dependencies, &name, &constraint, &keys);
}

#[given(
    regex = r#"^a project that depends on "([^"]+)" "([^"]*)" with hooks set to (\S+) and lvpm\.lock is current$"#
)]
async fn locked_project_with_hooks(
    world: &mut CliWorld,
    name: String,
    constraint: String,
    hooks: String,
) {
    world.project_with_keys(
        DependencyTable::Dependencies,
        &name,
        &constraint,
        &hooks_key(&hooks),
    );
    // `lvpm add` installs into a destination: the scratch folder `dest`.
    world.sandbox.make_empty_dest();
    world.lock_the_project();
}

#[then(regex = r"^lvpm\.toml has exactly the line: (.+)$")]
async fn manifest_has_the_line(world: &mut CliWorld, line: String) {
    let text = world.manifest_text();
    assert!(
        text.lines().any(|have| have == line),
        "{line:?} is not a line of:\n{text}"
    );
}

#[then(regex = r#"^it is accepted with the github source and hooks allowed for "([^"]+)"$"#)]
async fn accepted_with_github_and_hooks(world: &mut CliWorld, name: String) {
    let project = match world.manifest.read.as_ref().expect("the manifest was read") {
        Ok(project) => project,
        Err(why) => panic!("reading failed: {why}"),
    };
    assert!(project.github.contains_key(&name), "{:?}", project.github);
    assert!(
        project.hooks_allowed_for(&name),
        "{:?}",
        project.hooks_allowed
    );
}

#[given(regex = r#"^the user config has hooks\.run "([^"]+)"$"#)]
async fn user_config_has_hooks_run(world: &mut CliWorld, names: String) {
    let file = world.sandbox.config_file();
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("create the config folder");
    let list: Vec<String> = names.split(',').map(|name| format!("{name:?}")).collect();
    std::fs::write(&file, format!("[hooks]\nrun = [{}]\n", list.join(", ")))
        .expect("write the user config");
}

/// On the spawned command only: the sandbox never touches this process's environment.
#[given(regex = r#"^LVPM_CONFIG_HOOKS_RUN is "([^"]*)" for the command$"#)]
async fn hooks_run_env_for_the_command(world: &mut CliWorld, value: String) {
    world.sandbox.set_child_env("LVPM_CONFIG_HOOKS_RUN", &value);
}

/// The positive form, `lists "a" and "b"`, is the S8 step in lock.rs.
#[then(regex = r#"^the output lists "([^"]+)" and not "([^"]+)"$"#)]
async fn output_lists_and_not(world: &mut CliWorld, listed: String, absent: String) {
    let stdout = &world.output().stdout;
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    assert!(
        lines.contains(&listed.as_str()),
        "{listed:?} is not in:
{stdout}"
    );
    assert!(
        !stdout.contains(absent.as_str()),
        "{absent:?} is in:
{stdout}"
    );
}

#[then(regex = r#"^stdout does not say "([^"]+)"$"#)]
async fn stdout_does_not_say(world: &mut CliWorld, text: String) {
    let stdout = &world.output().stdout;
    assert!(!stdout.contains(&text), "{text:?} is in:\n{stdout}");
}

/// `lvpm list` on the LabVIEW installation names the hooks it lists as skipped
/// after the package's line: `(hooks skipped: PostInstall)`. Returns the line
/// and those hooks.
fn listed_as_skipped(world: &CliWorld, package: &str) -> (String, Vec<String>) {
    let listing = world
        .sandbox
        .run_lvpm(&["list", "--labview-version", LABVIEW_YEAR], None);
    let line = listing
        .stdout
        .lines()
        .find(|line| line.split_whitespace().next() == Some(package))
        .unwrap_or_else(|| panic!("{package} is not listed in:\n{}", listing.stdout));
    let hooks = line
        .split_once("(hooks skipped: ")
        .map(|(_, rest)| {
            rest.trim_end_matches(')')
                .split(", ")
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    (line.to_string(), hooks)
}

#[then(regex = r#"^"lvpm list" shows "([^"]+)" as skipped for "([^"]+)"$"#)]
async fn list_shows_the_hook_as_skipped(world: &mut CliWorld, hook: String, package: String) {
    let (line, hooks) = listed_as_skipped(world, &package);
    assert!(
        hooks.contains(&hook),
        "{hook} is not listed as skipped in: {line}"
    );
}

#[then(regex = r#"^"lvpm list" no longer shows "([^"]+)" as skipped for "([^"]+)"$"#)]
async fn list_no_longer_shows_the_hook_as_skipped(
    world: &mut CliWorld,
    hook: String,
    package: String,
) {
    let (line, hooks) = listed_as_skipped(world, &package);
    assert!(
        !hooks.contains(&hook),
        "{hook} is still listed as skipped in: {line}"
    );
}

#[then(regex = r#"^stderr does not say "([^"]+)"$"#)]
async fn stderr_does_not_say(world: &mut CliWorld, text: String) {
    let stderr = &world.output().stderr;
    assert!(!stderr.contains(&text), "{text:?} is in:\n{stderr}");
}

/// The fixture is in the local package folder and the installation is proved
/// free of it, as the characterization scenario does. Only the `@unsafe`
/// scenarios use these: the sandbox refuses a `--global` install otherwise.
#[given("the hook fixture package is not installed")]
async fn hook_fixture_not_installed(world: &mut CliWorld) {
    world.sandbox.detect_labview();
    world
        .sandbox
        .put_local_package(FixturePackage::find(FIXTURE_NAME, "1.0"));
    world.sandbox.require_clean_labview_installation();
}

#[given("the hook fixture was installed with its PostInstall skipped")]
async fn hook_fixture_installed_skipped(world: &mut CliWorld) {
    hook_fixture_not_installed(world).await;
    world
        .sandbox
        .setup_lvpm(&["install", FIXTURE_NAME, "--global"]);
    assert_eq!(world.sandbox.hook_marker(), None, "the hook ran on install");
}

#[then("the PostInstall hook did not run")]
async fn hook_did_not_run(world: &mut CliWorld) {
    assert_eq!(world.sandbox.hook_marker(), None, "the hook's marker");
}

// The docs scenario reads the files in its `Then` steps; these two only name it.
#[given("the repository docs")]
async fn the_repository_docs(_: &mut CliWorld) {}

#[when("they are read")]
async fn they_are_read(_: &mut CliWorld) {}

/// `text` is a doc's `"quoted"` list as the scenario writes it, `\"` for a
/// quote inside one: `"a", "b" and "c"`.
fn quoted_items(list: &str) -> Vec<String> {
    let item = regex::Regex::new(r#""((?:[^"\\]|\\.)*)""#).expect("a valid pattern");
    item.captures_iter(list)
        .map(|found| found[1].replace("\\\"", "\""))
        .collect()
}

// The usage and manifest guides have a one-quote step of their own in docs.rs;
// this one takes only the lists that step cannot read: a `\"` inside a quote.
#[then(regex = r#"^(README\.md|docs/(?:labview-integration|cli)\.md) mentions (".+")$"#)]
#[then(regex = r#"^(docs/(?:usage|manifest)\.md) mentions (".*\\".*")$"#)]
async fn doc_mentions_all(_: &mut CliWorld, doc: String, list: String) {
    // Prose wraps: compare with every run of whitespace as one space.
    let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = squash(&super::docs::repo_file(&doc));
    for want in quoted_items(&list) {
        assert!(
            text.contains(&squash(&want)),
            "{doc} does not mention {want}"
        );
    }
}
