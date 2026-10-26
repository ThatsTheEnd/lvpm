//! Steps of the catalog listing: feeds and folders holding the packages a
//! scenario names, and what `lvpm search --all` printed.

use super::recording::{self, check_or_panic, golden_dir};
use crate::CliWorld;
use crate::support::feed::listing;
use crate::support::fixtures::{FixturePackage, PACKAGES, Payload};
use crate::support::golden::{Golden, Mode};
use crate::support::sandbox::LOCAL_PACKAGES_DIR;
use cucumber::{given, then, when};

/// Where the scenario's feed is served: a folder of its own, so it lists only
/// what the scenario says.
const LISTED_DIR: &str = "/listed/";

/// beta 0.1, requiring alpha >= 1.0, for the local folder.
const BETA_0_1: FixturePackage = FixturePackage {
    name: "beta",
    version: "0.1",
    display_name: "Beta",
    requires: &[("alpha", "1.0")],
    lv_min: "20.0",
    payload: Payload::Text,
    post_install_hook: None,
};

/// An `index.vipr` listing (name, version, display name) entries.
fn plain_listing(entries: &[(String, &str, &str)]) -> String {
    let mut body = String::from("; lvpm acceptance fixture: a catalog\n");
    for (name, version, display) in entries {
        body.push_str(&format!(
            "\n[Package {name}-{version}]\nPackage.URL=packages/{name}-{version}.vip\n\
             Package.Display Name={display}\n"
        ));
    }
    body
}

/// The project's one feed lists `body`.
fn publish_listing(world: &mut CliWorld, body: &str) {
    world.sandbox.feed().publish(LISTED_DIR, body);
    world.sandbox.use_project_source(LISTED_DIR);
}

/// The feed's index URL as the output shows it.
fn feed_index(world: &CliWorld) -> String {
    let url = format!("{}/index.vipr", world.sandbox.feed().source_url(LISTED_DIR));
    world.sandbox.normalised_text(&url)
}

fn stdout(world: &CliWorld) -> &str {
    &world
        .last_output
        .as_ref()
        .expect("a command was run")
        .stdout
}

/// Each row's name and version.
fn listed(world: &CliWorld) -> Vec<String> {
    let row = |line: &str| {
        line.split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
    };
    stdout(world).lines().map(row).collect()
}

#[given(r#"a feed with "alpha" 1.0 and 2.0"#)]
async fn feed_with_two_alphas(world: &mut CliWorld) {
    let alphas = &PACKAGES[..2];
    let ids: Vec<String> = alphas.iter().map(FixturePackage::id).collect();
    assert_eq!(ids, ["alpha-1.0", "alpha-2.0"]);
    publish_listing(world, &listing(alphas, true));
}

#[given(r#"a local folder with "beta" 0.1"#)]
async fn local_folder_with_beta(world: &mut CliWorld) {
    world.sandbox.put_local_package(&BETA_0_1);
}

#[given("a feed with 60 packages of 2 versions each")]
async fn feed_with_sixty_packages(world: &mut CliWorld) {
    let entries: Vec<(String, &str, &str)> = (1..=60)
        .flat_map(|n| ["1.0", "2.0"].map(|v| (format!("bulk_{n:02}"), v, "Bulk")))
        .collect();
    publish_listing(world, &plain_listing(&entries));
}

#[given(r#"a feed with "alpha" 1.0, shown as "Alpha Tools", and "beta" 0.1"#)]
async fn feed_with_alpha_tools_and_beta(world: &mut CliWorld) {
    let entries = [
        ("alpha".to_string(), "1.0", "Alpha Tools"),
        ("beta".to_string(), "0.1", "Beta"),
    ];
    publish_listing(world, &plain_listing(&entries));
}

#[given(r#"a feed with "alpha" 1.0"#)]
async fn feed_with_alpha(world: &mut CliWorld) {
    publish_listing(
        world,
        &plain_listing(&[("alpha".to_string(), "1.0", "Alpha")]),
    );
}

#[given("a feed and a local folder with no package")]
async fn empty_feed_and_folder(world: &mut CliWorld) {
    publish_listing(world, &plain_listing(&[]));
}

#[given(r#"a feed with "alpha" 1.0 that does not answer and has never been cached"#)]
async fn silent_feed_with_alpha(world: &mut CliWorld) {
    feed_with_alpha(world).await;
    world.sandbox.empty_cache();
    world.sandbox.feed().go_down();
}

// The sandbox always configures its local folder; taking it away leaves the
// feed as the only source that could be read.
#[given("the only feed does not answer and has never been cached")]
async fn only_feed_silent(world: &mut CliWorld) {
    silent_feed_with_alpha(world).await;
    world.sandbox.remove_local_packages_dir();
}

#[when(expr = "{string} runs")]
async fn command_runs(world: &mut CliWorld, command: String) {
    world.run(&command, None);
}

#[when(expr = "the characterization rows {string} and {string} run")]
async fn characterization_rows_run(world: &mut CliWorld, first: String, second: String) {
    // Each row was recorded on a first run, with nothing cached yet.
    for name in [first.clone(), second.clone()] {
        world.sandbox.empty_cache();
        recording::row_runs(world, name).await;
        world
            .row_outputs
            .push(world.last_output.clone().expect("the row ran"));
    }
    world.recordings = vec![first, second];
}

// Always a comparison: these rows were recorded before `--all` existed.
#[then("they still match")]
async fn they_still_match(world: &mut CliWorld) {
    let golden = Golden::new(golden_dir(), Mode::Compare);
    assert_eq!(world.row_outputs.len(), 2, "both rows ran");
    for (name, output) in world.recordings.iter().zip(&world.row_outputs) {
        check_or_panic(&golden, name, output);
    }
}

#[then(r#"it lists "alpha" 2.0, "alpha" 1.0 and "beta" 0.1, in that order"#)]
async fn lists_both_alphas_then_beta(world: &mut CliWorld) {
    assert_eq!(listed(world), ["alpha 2.0", "alpha 1.0", "beta 0.1"]);
}

#[then("each row shows its display name, requirements, minimum LabVIEW and source")]
async fn rows_show_everything(world: &mut CliWorld) {
    let folder = world.sandbox.root().join(LOCAL_PACKAGES_DIR);
    let folder = world.sandbox.normalised_text(&folder.to_string_lossy());
    let lines: Vec<&str> = stdout(world).lines().collect();
    let from_feed = format!("  LabVIEW>=20  from {}", feed_index(world));
    assert!(
        lines[0].ends_with(&format!(" Alpha{from_feed}")),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].ends_with(&format!(" Alpha{from_feed}")),
        "{}",
        lines[1]
    );
    let beta = format!(" Beta  needs alpha>=1.0  LabVIEW>=20  from {folder}");
    assert!(lines[2].ends_with(&beta), "{}", lines[2]);
}

#[then(expr = "the output matches the {string} recording")]
async fn output_matches_recording(world: &mut CliWorld, name: String) {
    let output = world.last_output.as_ref().expect("a command was run");
    check_or_panic(&Golden::new(golden_dir(), Mode::from_env()), &name, output);
}

#[then(r#"it lists 120 rows and no "more" line"#)]
async fn lists_all_rows(world: &mut CliWorld) {
    let rows = listed(world);
    assert_eq!(rows.len(), 120);
    assert!(!stdout(world).contains("more"), "{}", stdout(world));
    assert_eq!(rows[0], "bulk_01 2.0");
    assert_eq!(rows[119], "bulk_60 1.0");
}

#[then(r#"it lists only "alpha" 1.0"#)]
async fn lists_only_alpha(world: &mut CliWorld) {
    assert_eq!(listed(world), ["alpha 1.0"]);
}

#[then(r#"it lists "beta" 0.1"#)]
async fn lists_beta(world: &mut CliWorld) {
    assert_eq!(listed(world), ["beta 0.1"]);
}

#[then("it says that no package was found in that feed and that folder")]
async fn says_nothing_found(world: &mut CliWorld) {
    let folder = world.sandbox.root().join(LOCAL_PACKAGES_DIR);
    let folder = world.sandbox.normalised_text(&folder.to_string_lossy());
    let expected = format!(
        "no packages found in:\n  {folder}\n  {}\n",
        feed_index(world)
    );
    assert_eq!(stdout(world), expected);
}

#[then(expr = "it says no package matches {string}")]
async fn says_no_match(world: &mut CliWorld, query: String) {
    assert_eq!(stdout(world), format!("no packages matching {query:?}\n"));
}

#[then(expr = "it exits {int}")]
async fn exits_with(world: &mut CliWorld, code: i32) {
    let output = world.last_output.as_ref().expect("a command was run");
    assert_eq!(output.exit_code, code, "{output:?}");
}

#[then("stderr names the feed that could not be read")]
async fn stderr_names_unread_feed(world: &mut CliWorld) {
    let output = world.last_output.as_ref().expect("a command was run");
    let warning = format!("warning: could not read {}:", feed_index(world));
    assert!(output.stderr.contains(&warning), "{}", output.stderr);
}

#[then("it fails with exit code 1 and stderr names the feed")]
async fn fails_naming_the_feed(world: &mut CliWorld) {
    let output = world.last_output.as_ref().expect("a command was run");
    assert_eq!(output.exit_code, 1, "{output:?}");
    assert!(stdout(world).is_empty(), "{}", stdout(world));
    assert!(
        output.stderr.contains("no package source could be read"),
        "{}",
        output.stderr
    );
    assert!(
        output.stderr.contains(&feed_index(world)),
        "{}",
        output.stderr
    );
}

#[then("it fails as a usage error with exit code 2")]
async fn usage_error(world: &mut CliWorld) {
    let output = world.last_output.as_ref().expect("a command was run");
    assert_eq!(output.exit_code, 2, "{output:?}");
    assert!(output.stderr.contains("Usage:"), "{}", output.stderr);
}
