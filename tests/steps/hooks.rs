//! Steps of the install-hooks scenarios (S12): `hooks = "run"` on a dependency
//! line. The project reads the loopback feed's `first` folder, as the S13
//! scenarios do, so `"first" lists ...` gives it packages.

use super::sources::FIRST;
use crate::CliWorld;
use cucumber::{given, then};
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
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.contains(&listed.as_str()),
        "{listed:?} is not in:
{stdout}"
    );
    assert!(
        !lines.contains(&absent.as_str()),
        "{absent:?} is in:
{stdout}"
    );
}
