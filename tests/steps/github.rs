//! Steps of the GitHub scenarios (S4): the fake of the releases API and the
//! repos it serves, pilot B's project, and the Givens slices 3 and 4 share.
//! The Thens of one slice live with its scenarios; the lock's golden and the
//! staleness outline come first.

use super::lock::sha256_hex;
use super::sync::sorted;
use crate::CliWorld;
use crate::support::fixtures::{FixturePackage, Payload};
use crate::support::golden::Mode;
use crate::support::normalise::TIMESTAMP_TOKEN;
use crate::support::pilots::{check_checksums, pilots_dir};
use crate::support::project::MANIFEST_FILE;
use crate::support::sandbox::LOCAL_PACKAGES_DIR;
use cucumber::{given, then, when};
use lvpm_testkit::github::{FakeRelease, GitHubFake, LoggedRequest, PRIVATE_TOKEN};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const PILOT_B: &str = "labview_open_source_lib_serializer";
const PILOT_B_FILE: &str = "labview_open_source_lib_serializer-1.2.1.16.vip";
const PILOT_B_REPO: &str = "LabVIEW-Open-Source/Serializer";
/// Pilot B's release: the repo tags the package's version without its build.
const PILOT_B_TAG: &str = "1.2.1";
const PILOT_B_CONSTRAINT: &str = ">=1.2";
/// What pilot B needs and the project lists itself.
const PILOT_B_NEEDS: &str = "labview_open_source_lib_data_manipulation";
/// What pilot B needs, which the project's `[sources]` feed holds.
const PILOT_B_DEPENDENCY_FILES: [&str; 2] = [
    "labview_open_source_lib_data_manipulation-1.6.0.16.vip",
    "labview_open_source_project_lib_labview_open_source_palette-1.0.2.7.vip",
];
const PROJECT_LABVIEW: &str = "2024";
/// Where the lock without GitHub dependencies is recorded.
const GOLDEN_LOCK: &str = "tests/golden/s4-no-github.lock";

/// What a GitHub scenario has set up.
#[derive(Debug, Default)]
pub struct GitHubState {
    fake: Option<GitHubFake>,
    token: Option<String>,
    /// What the fake serves under each file name, the last upload of it.
    served: BTreeMap<String, Vec<u8>>,
    /// The bytes the fake serves now in place of a file's own.
    replaced: Option<Vec<u8>>,
}

impl CliWorld {
    /// The fake, started on first use, and the sandbox pointed at it.
    fn github_fake(&mut self) -> &GitHubFake {
        if self.github.fake.is_none() {
            self.github.fake = Some(GitHubFake::start(&[]));
            self.point_github_at_the_fake();
        }
        self.github
            .fake
            .as_ref()
            .expect("the fake was just started")
    }

    fn point_github_at_the_fake(&self) {
        let fake = self.github.fake.as_ref().expect("a fake");
        self.sandbox
            .set_github(&fake.api_url(), self.github.token.as_deref());
    }

    /// `lvpm.toml` of a project that reads the local package folder, with
    /// `lines` as its `[dependencies]`.
    fn write_dependencies(&self, lines: &str) {
        self.sandbox.use_local_project(Some(PROJECT_LABVIEW));
        let path = self.sandbox.work_dir().join(MANIFEST_FILE);
        let mut text = std::fs::read_to_string(&path).expect("read the manifest");
        text.push_str(&format!("\n[dependencies]\n{lines}\n"));
        std::fs::write(path, text).expect("write the manifest");
    }

    fn edit_the_manifest_text(&self, edit: impl FnOnce(&str) -> String) {
        let path = self.sandbox.work_dir().join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).expect("read the manifest");
        std::fs::write(path, edit(&text)).expect("write the manifest");
    }

    /// Pilot B's project: its line asks for `constraint`, GitHub serves its
    /// .vip as release 1.2.1, and the local packages hold what it needs.
    fn pilot_b_project(&mut self, constraint: &str) {
        if let Err(why) = check_checksums(&pilots_dir()) {
            panic!("{why}");
        }
        // The second line is a short-form dependency, so that a change to
        // `github` alone is what makes a lock stale.
        self.write_dependencies(&format!(
            "{}\n{PILOT_B_NEEDS} = \">=1.6\"",
            pilot_b_line(PILOT_B_REPO, constraint)
        ));
        let packages = self.sandbox.root().join(LOCAL_PACKAGES_DIR);
        for file in PILOT_B_DEPENDENCY_FILES {
            std::fs::copy(pilots_dir().join(file), packages.join(file))
                .unwrap_or_else(|e| panic!("copy {file}: {e}"));
        }
        let vip = std::fs::read(pilots_dir().join(PILOT_B_FILE)).expect("pilot B's .vip");
        self.serve(PILOT_B_REPO, PILOT_B_TAG, &[(PILOT_B_FILE, vip)], false);
    }

    fn lock_is_current(&mut self) {
        self.sandbox.setup_lvpm(&["lock"]);
        self.lock.before = Some(self.lock_bytes().expect("`lvpm lock` wrote a lock"));
    }
}

fn pilot_b_line(repo: &str, constraint: &str) -> String {
    format!("{PILOT_B} = {{ github = \"{repo}\", version = \"{constraint}\" }}")
}

// ---- pilot B's project ----

#[given("the pilot B project")]
async fn the_pilot_b_project(world: &mut CliWorld) {
    world.pilot_b_project(PILOT_B_CONSTRAINT);
}

#[given(regex = r#"^the pilot B project, asking for version "([^"]+)"$"#)]
async fn the_pilot_b_project_asking_for(world: &mut CliWorld, constraint: String) {
    world.pilot_b_project(&constraint);
}

#[given("the pilot B project with a current lvpm.lock")]
async fn the_pilot_b_project_with_a_lock(world: &mut CliWorld) {
    world.pilot_b_project(PILOT_B_CONSTRAINT);
    world.lock_is_current();
}

#[given("the pilot B project with a current lvpm.lock and an empty cache")]
async fn the_pilot_b_project_with_a_lock_and_an_empty_cache(world: &mut CliWorld) {
    world.pilot_b_project(PILOT_B_CONSTRAINT);
    world.lock_is_current();
    world.sandbox.empty_cache();
}

/// The lock run to make the project's lock left the archives in the cache.
#[given("the pilot B project with a current lvpm.lock and a warm cache")]
async fn the_pilot_b_project_with_a_lock_and_a_warm_cache(world: &mut CliWorld) {
    the_pilot_b_project_with_a_lock(world).await;
}

#[given("the pilot B project with GITHUB_API_URL pointing at a closed port")]
async fn the_pilot_b_project_with_a_closed_port(world: &mut CliWorld) {
    world.pilot_b_project(PILOT_B_CONSTRAINT);
    // A port nobody listens on: bind one, then let it go.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a free loopback port");
    world
        .sandbox
        .set_github(&format!("http://{closed}"), world.github.token.as_deref());
}

// ---- repos and projects ----

/// What a file called `file` holds when no scenario says: a package named by
/// the file (`<name>-<version>.<ext>`), or its own name when it is not one.
fn package_bytes(file: &str, inner: Option<(&str, &str)>, display_name: &str) -> Vec<u8> {
    package_bytes_needing(file, inner, display_name, "20.0")
}

/// As [`package_bytes`], for a package that needs LabVIEW `lv_min` (24.0 is 2024).
fn package_bytes_needing(
    file: &str,
    inner: Option<(&str, &str)>,
    display_name: &str,
    lv_min: &str,
) -> Vec<u8> {
    let parsed = file
        .rsplit_once('.')
        .and_then(|(id, _)| id.rsplit_once('-'));
    let Some((name, version)) = inner.or(parsed) else {
        return file.as_bytes().to_vec();
    };
    // Test code: the leak lasts as long as the run.
    let leak = |text: &str| -> &'static str { Box::leak(text.to_string().into_boxed_str()) };
    FixturePackage {
        name: leak(name),
        version: leak(version),
        display_name: leak(display_name),
        requires: &[],
        lv_min: leak(lv_min),
        payload: Payload::Text,
        post_install_hook: None,
    }
    .build_vip()
}

impl CliWorld {
    /// Release `tag` of `repo` holding `files`, which are then what the fake
    /// serves under their names.
    fn serve(&mut self, repo: &str, tag: &str, files: &[(&str, Vec<u8>)], prerelease: bool) {
        let mut release = FakeRelease::new(tag);
        for (file, bytes) in files {
            release = release.asset(file, bytes);
            self.github.served.insert(file.to_string(), bytes.clone());
        }
        if prerelease {
            release = release.prerelease();
        }
        self.github_fake().add_release(repo, release);
    }

    /// What the fake serves under `file`.
    fn served(&self, file: &str) -> &[u8] {
        self.github
            .served
            .get(file)
            .unwrap_or_else(|| panic!("the fake serves no file {file}"))
    }
}

#[given(regex = r#"^the repo "([^"]+)" has release "([^"]+)" with "([^"]+)"$"#)]
async fn a_repo_has_a_release(world: &mut CliWorld, repo: String, tag: String, file: String) {
    let bytes = package_bytes(&file, None, "pkg");
    world.serve(&repo, &tag, &[(&file, bytes)], false);
}

#[given(
    regex = r#"^the repo "([^"]+)" has release "([^"]+)" with "([^"]+)" that holds "([^"]+)" (\S+)$"#
)]
async fn a_release_file_holds(
    world: &mut CliWorld,
    repo: String,
    tag: String,
    file: String,
    name: String,
    version: String,
) {
    let bytes = package_bytes(&file, Some((&name, &version)), "pkg");
    world.serve(&repo, &tag, &[(&file, bytes)], false);
}

#[given(
    regex = r#"^the repo "([^"]+)" has release "([^"]+)" with "([^"]+)" that needs LabVIEW (\d+)$"#
)]
async fn a_release_needs_labview(
    world: &mut CliWorld,
    repo: String,
    tag: String,
    file: String,
    year: u32,
) {
    let lv_min = format!("{}.0", year - 2000);
    let bytes = package_bytes_needing(&file, None, "pkg", &lv_min);
    world.serve(&repo, &tag, &[(&file, bytes)], false);
}

#[given(regex = r#"^the repo "([^"]+)" has release "([^"]+)" with "([^"]+)" as a pre-release$"#)]
async fn a_repo_has_a_prerelease(world: &mut CliWorld, repo: String, tag: String, file: String) {
    let bytes = package_bytes(&file, None, "pkg");
    world.serve(&repo, &tag, &[(&file, bytes)], true);
}

#[given(
    regex = r#"^the repo "([^"]+)" has release "([^"]+)" with "([^"]+)" and "([^"]+)" holding different bytes$"#
)]
async fn a_release_with_two_files(
    world: &mut CliWorld,
    repo: String,
    tag: String,
    first: String,
    second: String,
) {
    let files = [
        (first.as_str(), package_bytes(&first, None, "first")),
        (second.as_str(), package_bytes(&second, None, "second")),
    ];
    world.serve(&repo, &tag, &files, false);
}

#[given(regex = r#"^the repo "([^"]+)" is private and has release "([^"]+)" with "([^"]+)"$"#)]
async fn a_private_repo_has_a_release(
    world: &mut CliWorld,
    repo: String,
    tag: String,
    file: String,
) {
    let bytes = package_bytes(&file, None, "pkg");
    world.serve(&repo, &tag, &[(&file, bytes)], false);
    world.github_fake().make_private(&repo);
}

#[given(regex = r#"^the repo "([^"]+)" is private$"#)]
async fn a_repo_is_private(world: &mut CliWorld, repo: String) {
    world.github_fake().make_private(&repo);
}

#[given(
    regex = r#"^a project depending on "([^"]+)" with github "([^"]+)" and version "([^"]+)"$"#
)]
async fn a_project_depending_on_a_github_package(
    world: &mut CliWorld,
    name: String,
    repo: String,
    constraint: String,
) {
    world.github_fake();
    world.write_dependencies(&format!(
        "{name} = {{ github = \"{repo}\", version = \"{constraint}\" }}"
    ));
}

#[given(
    regex = r#"^a project depending on "([^"]+)" with github "([^"]+)" and version "([^"]+)" and a current lvpm\.lock$"#
)]
async fn a_project_with_a_github_package_and_a_lock(
    world: &mut CliWorld,
    name: String,
    repo: String,
    constraint: String,
) {
    a_project_depending_on_a_github_package(world, name, repo, constraint).await;
    world.lock_is_current();
}

#[given(regex = r#"^GH_TOKEN is "([^"]*)"$"#)]
async fn gh_token_is(world: &mut CliWorld, token: String) {
    world.github.token = Some(token);
    world.github_fake();
    world.point_github_at_the_fake();
}

#[given("an empty cache")]
async fn an_empty_cache(world: &mut CliWorld) {
    world.sandbox.empty_cache();
}

#[given("the GitHub fake's request log is cleared")]
async fn the_request_log_is_cleared(world: &mut CliWorld) {
    world.github_fake().clear_requests();
}

// ---- changes to pilot B's line ----

#[when(regex = r#"^I change pilot B's github to "([^"]+)"$"#)]
async fn change_pilot_bs_github(world: &mut CliWorld, repo: String) {
    let old = format!("github = \"{PILOT_B_REPO}\"");
    world.edit_the_manifest_text(|text| {
        assert!(text.contains(&old), "{old} is not in:\n{text}");
        text.replacen(&old, &format!("github = \"{repo}\""), 1)
    });
}

#[when(regex = r#"^I replace pilot B's line with the short form "([^"]+)"$"#)]
async fn replace_pilot_bs_line_with_the_short_form(world: &mut CliWorld, constraint: String) {
    let line = pilot_b_line(PILOT_B_REPO, PILOT_B_CONSTRAINT);
    world.edit_the_manifest_text(|text| {
        assert!(text.contains(&line), "{line} is not in:\n{text}");
        text.replacen(&line, &format!("{PILOT_B} = \"{constraint}\""), 1)
    });
}

/// The short-form line of `name` gains `github` and keeps its constraint, so
/// that `github` alone is what makes the lock stale.
#[when(regex = r#"^I add github "([^"]+)" to the "([^"]+)" line$"#)]
async fn add_github_to_a_line(world: &mut CliWorld, repo: String, name: String) {
    world.edit_the_manifest_text(|text| {
        let start = format!("{name} = \"");
        let (before, rest) = text.split_once(&start).expect("a short-form line");
        let (constraint, after) = rest.split_once('"').expect("a constraint");
        format!("{before}{name} = {{ github = \"{repo}\", version = \"{constraint}\" }}{after}")
    });
}

// ---- the lock without GitHub dependencies ----

#[then("lvpm.lock equals tests/golden/s4-no-github.lock, captured from main before step 3.2")]
async fn the_lock_equals_the_golden(world: &mut CliWorld) {
    let lock = String::from_utf8(world.lock_bytes().expect("lvpm.lock exists"))
        .expect("lvpm.lock is text");
    let lock = world.sandbox.normalised_text(&lock);
    if let Err(why) = check_golden_lock(Mode::from_env(), &lock) {
        panic!("{why}");
    }
}

fn golden_lock_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_LOCK)
}

/// Compare `lock` with the recording, or record it as `LVPM_RECORD` says.
fn check_golden_lock(mode: Mode, lock: &str) -> Result<(), String> {
    let path = golden_lock_path();
    match mode {
        Mode::Compare => {
            let recorded = std::fs::read_to_string(&path).map_err(|e| {
                format!(
                    "no recording at {}: {e}; run with LVPM_RECORD=1",
                    path.display()
                )
            })?;
            match recorded == lock {
                true => Ok(()),
                false => Err(format!(
                    "lvpm.lock differs from {GOLDEN_LOCK}\n--- recorded\n{recorded}\n--- actual\n{lock}"
                )),
            }
        }
        Mode::Record if path.exists() => Err(format!(
            "{GOLDEN_LOCK} exists: LVPM_RECORD=force replaces it"
        )),
        Mode::Record | Mode::Overwrite => {
            std::fs::write(&path, lock).map_err(|e| format!("writing {}: {e}", path.display()))
        }
    }
}

// ---- more of the repo and its fake ----

#[given(regex = r#"^the \[sources\] feed also lists "([^"]+)" (\S+)$"#)]
async fn the_sources_feed_also_lists(world: &mut CliWorld, name: String, version: String) {
    let file = format!("{name}-{version}.vip");
    let packages = world.sandbox.root().join(LOCAL_PACKAGES_DIR);
    std::fs::write(packages.join(&file), package_bytes(&file, None, &name))
        .expect("write the package");
}

#[given(regex = r#"^the GitHub fake has no requests left for anonymous callers until "([^"]+)"$"#)]
async fn no_requests_left_for_anonymous_callers(world: &mut CliWorld, reset: String) {
    let fake = world.github_fake();
    fake.limit_rate_for_anonymous(403, unix_seconds(&reset));
}

/// `2099-01-01T00:00:00Z` as Unix seconds (Howard Hinnant's days-from-civil).
fn unix_seconds(utc: &str) -> u64 {
    let parts: Vec<i64> = utc
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse().expect("a number"))
        .collect();
    let [year, month, day, hour, minute, second] = parts[..] else {
        panic!("{utc:?} is not a time like 2099-01-01T00:00:00Z");
    };
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second).expect("a time after 1970")
}

#[given("the GitHub fake now serves other bytes for pilot B's .vip")]
async fn the_fake_serves_other_bytes(world: &mut CliWorld) {
    // Another package of the same name and version: it reads as pilot B.
    let other = package_bytes(PILOT_B_FILE, None, "re-uploaded");
    world
        .github_fake()
        .replace_bytes(PILOT_B_REPO, PILOT_B_FILE, &other);
    world.github.replaced = Some(other);
}

#[given(
    regex = r#"^the GitHub fake now serves other bytes for pilot B's \.vip under (the same|a new) asset id$"#
)]
async fn the_fake_serves_other_bytes_under(world: &mut CliWorld, id: String) {
    let other = package_bytes(PILOT_B_FILE, None, "re-uploaded");
    let fake = world.github_fake();
    match id.as_str() {
        "the same" => fake.replace_bytes(PILOT_B_REPO, PILOT_B_FILE, &other),
        _ => fake.replace_with_new_id(PILOT_B_REPO, PILOT_B_FILE, &other),
    }
    world.github.replaced = Some(other);
}

#[given(regex = r#"^the GitHub fake no longer has release "([^"]+)"$"#)]
async fn the_fake_no_longer_has_a_release(world: &mut CliWorld, tag: String) {
    world.github_fake().delete_release(PILOT_B_REPO, &tag);
}

#[given("the cache no longer holds pilot B's archive")]
async fn the_cache_lost_pilot_bs_archive(world: &mut CliWorld) {
    let sha256 = world.locked_package(PILOT_B)["sha256"]
        .as_str()
        .expect("a SHA-256")
        .to_string();
    std::fs::remove_file(world.sandbox.cache_blob(&sha256)).expect("the cache held the archive");
}

#[given("the GitHub fake now serves pilot B's .vip again under a new asset id")]
async fn the_fake_serves_the_same_bytes_under_a_new_id(world: &mut CliWorld) {
    let bytes = world.served(PILOT_B_FILE).to_vec();
    world
        .github_fake()
        .replace_with_new_id(PILOT_B_REPO, PILOT_B_FILE, &bytes);
}

// ---- what a sync installed ----

#[then(regex = r"^pilot B (\S+) and its dependencies are installed in dest$")]
async fn pilot_b_and_its_dependencies_are_installed(world: &mut CliWorld, version: String) {
    let installed = world.installed();
    assert!(
        installed.contains(&(PILOT_B.to_string(), version)),
        "{installed:?}"
    );
    assert!(installed.len() > 1, "no dependencies in {installed:?}");
    assert_eq!(sorted(installed), sorted(world.locked_versions()));
}

#[then(regex = r"^pilot B (\S+) is installed in dest$")]
async fn pilot_b_is_installed(world: &mut CliWorld, version: String) {
    let installed = world.installed();
    assert!(
        installed.contains(&(PILOT_B.to_string(), version)),
        "{installed:?}\n{}",
        world.printed()
    );
}

#[then(regex = r#"^"([^"]+)" (\S+) is installed in dest$"#)]
async fn a_package_is_installed_in_dest(world: &mut CliWorld, name: String, version: String) {
    let installed = world.installed();
    assert!(
        installed.contains(&(name, version)),
        "{installed:?}\n{}",
        world.printed()
    );
}

#[then(regex = r#"^"([^"]+)" (\S+) is installed in dest from the \.lvpkg's bytes$"#)]
async fn installed_from_the_lvpkg(world: &mut CliWorld, name: String, version: String) {
    let file = format!("{name}-{version}.lvpkg");
    let want = sha256_hex(world.served(&file));
    assert_eq!(
        world.installed_archive_sha256(&name).as_deref(),
        Some(want.as_str()),
        "{:?}\n{}",
        world.installed(),
        world.printed()
    );
}

#[then("nothing is installed in dest")]
async fn nothing_is_installed_in_dest(world: &mut CliWorld) {
    assert_eq!(world.installed(), []);
}

#[then(regex = r#"^the command fails naming "([^"]+)" and nothing is installed in dest$"#)]
async fn fails_naming_and_nothing_installed(world: &mut CliWorld, said: String) {
    world.assert_failed();
    assert!(
        world.printed().contains(&said),
        "{said:?}\n{}",
        world.printed()
    );
    assert_eq!(world.installed(), []);
}

// ---- what the lock took from GitHub ----

impl CliWorld {
    /// `lvpm.lock` as TOML.
    fn github_lock_table(&self) -> toml::Table {
        let text = String::from_utf8(self.lock_bytes().expect("lvpm.lock exists"))
            .expect("lvpm.lock is text");
        text.parse().expect("lvpm.lock is valid TOML")
    }
}

#[then(
    regex = r#"^lvpm\.lock takes "([^"]+)" at version "([^"]+)" from "([^"]+)" with the \.vip's SHA-256$"#
)]
async fn the_lock_takes_the_vip_from(
    world: &mut CliWorld,
    name: String,
    version: String,
    source: String,
) {
    let package = world.locked_package(&name);
    assert_eq!(
        package["version"].as_str(),
        Some(version.as_str()),
        "{package:?}"
    );
    assert_eq!(
        package["source"].as_str(),
        Some(source.as_str()),
        "{package:?}"
    );
    let want = sha256_hex(world.served(PILOT_B_FILE));
    assert_eq!(
        package["sha256"].as_str(),
        Some(want.as_str()),
        "{package:?}"
    );
}

#[then(regex = r#"^lvpm\.lock takes "([^"]+)" at version "([^"]+)" from "([^"]+)"$"#)]
async fn the_lock_takes_a_version_from(
    world: &mut CliWorld,
    name: String,
    version: String,
    source: String,
) {
    let package = world.locked_package(&name);
    assert_eq!(
        package["version"].as_str(),
        Some(version.as_str()),
        "{package:?}"
    );
    assert_eq!(
        package["source"].as_str(),
        Some(source.as_str()),
        "{package:?}"
    );
}

#[then(regex = r#"^lvpm\.lock takes "([^"]+)" at version "([^"]+)"$"#)]
async fn the_lock_takes_a_version(world: &mut CliWorld, name: String, version: String) {
    let package = world.locked_package(&name);
    assert_eq!(
        package["version"].as_str(),
        Some(version.as_str()),
        "{package:?}"
    );
}

#[then(regex = r#"^lvpm\.lock takes "([^"]+)" from the \[sources\] feed$"#)]
async fn the_lock_takes_from_the_sources_feed(world: &mut CliWorld, name: String) {
    let package = world.locked_package(&name);
    // The feed is the folder of local packages.
    assert_eq!(package["source"].as_str(), Some("local"), "{package:?}");
}

#[then(regex = r#"^lvpm\.lock's version for "([^"]+)" is exactly "([^"]+)", not "([^"]+)"$"#)]
async fn the_lock_has_the_files_version(
    world: &mut CliWorld,
    name: String,
    version: String,
    tag: String,
) {
    let package = world.locked_package(&name);
    assert_eq!(
        package["version"].as_str(),
        Some(version.as_str()),
        "{package:?}"
    );
    assert_ne!(version, tag);
}

#[then(regex = r#"^lvpm\.lock has the SHA-256 of "([^"]+)"$"#)]
async fn the_lock_has_the_sha_of(world: &mut CliWorld, file: String) {
    let want = sha256_hex(world.served(&file));
    let lock = world.github_lock_table();
    let shas: Vec<&str> = lock["package"]
        .as_array()
        .expect("packages")
        .iter()
        .filter_map(|package| package["sha256"].as_str())
        .collect();
    assert!(shas.contains(&want.as_str()), "{want} is not in {shas:?}");
}

#[then(regex = r#"^lvpm\.lock has github "([^"]+)" for "([^"]+)" in \[inputs\.github\]$"#)]
async fn the_lock_has_the_github_input(world: &mut CliWorld, repo: String, name: String) {
    let lock = world.github_lock_table();
    let held = lock["inputs"]["github"][&name].as_str();
    assert_eq!(held, Some(repo.as_str()), "{lock:?}");
}

#[then(regex = r#"^lvpm\.toml's pilot B line still has github "([^"]+)"$"#)]
async fn the_manifest_keeps_the_github_line(world: &mut CliWorld, repo: String) {
    let manifest = world.manifest_text();
    let line = manifest
        .lines()
        .find(|line| line.starts_with(PILOT_B))
        .unwrap_or_else(|| panic!("no pilot B line in:\n{manifest}"));
    assert!(line.contains(&format!("github = \"{repo}\"")), "{line}");
}

#[then(regex = r#"^lvpm\.toml's pilot B line has github "([^"]+)" and version "([^"]+)"$"#)]
async fn the_manifest_line_has_github_and_version(
    world: &mut CliWorld,
    repo: String,
    constraint: String,
) {
    let manifest = world.manifest_text();
    let line = manifest
        .lines()
        .find(|line| line.starts_with(PILOT_B))
        .unwrap_or_else(|| panic!("no pilot B line in:\n{manifest}"));
    assert_eq!(line, pilot_b_line(&repo, &constraint), "{manifest}");
}

#[then("lvpm.toml has no pilot B line")]
async fn the_manifest_has_no_pilot_b_line(world: &mut CliWorld) {
    let manifest = world.manifest_text();
    assert!(
        !manifest.lines().any(|line| line.starts_with(PILOT_B)),
        "{manifest}"
    );
}

// ---- what the command said ----

#[then(regex = r#"^stderr names "([^"]+)"$"#)]
async fn stderr_names(world: &mut CliWorld, said: String) {
    let stderr = &world.output().stderr;
    assert!(stderr.contains(&said), "{said:?} is not in:\n{stderr}");
}

/// `"a", "b" and "c"`: every quoted part must be in what the command printed.
/// (A first part that starts a manifest line, `[`, is the manifest steps', and
/// one part alone is a source's, in the first-source-wins steps.)
#[then(regex = r##"^the command fails naming ("[^"\[][^"]*"(?:(?:, | and )"[^"]*")+)$"##)]
async fn the_command_fails_naming(world: &mut CliWorld, quoted: String) {
    world.assert_failed();
    let report = world.printed();
    let part = Regex::new(r#""([^"]*)""#).expect("a pattern");
    let time = Regex::new(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$").expect("a pattern");
    for said in part.captures_iter(&quoted) {
        // The harness hides the clock in what a command printed.
        let said = match time.is_match(&said[1]) {
            true => TIMESTAMP_TOKEN,
            false => &said[1],
        };
        assert!(report.contains(said), "{said:?} is not in:\n{report}");
    }
}

#[then(regex = r#"^the command fails naming "([^"]+)" and both SHA-256 values$"#)]
async fn the_command_fails_naming_both_hashes(world: &mut CliWorld, name: String) {
    world.assert_failed();
    let report = world.printed();
    let locked = world.locked_package(&name)["sha256"]
        .as_str()
        .expect("a SHA-256")
        .to_string();
    let replaced = world.github.replaced.as_ref();
    let now = sha256_hex(replaced.expect("other bytes were served"));
    for part in [name, locked, now] {
        assert!(report.contains(&part), "{part} is not in:\n{report}");
    }
}

// ---- what the fake saw ----

impl CliWorld {
    fn github_requests(&mut self) -> Vec<LoggedRequest> {
        self.github_fake().requests()
    }
}

#[then("the GitHub fake received no asset request")]
async fn the_fake_received_no_asset_request(world: &mut CliWorld) {
    let assets: Vec<_> = world
        .github_requests()
        .into_iter()
        .filter(|request| {
            request.path.contains("/releases/assets/") || request.path.starts_with("/download/")
        })
        .collect();
    assert!(assets.is_empty(), "{assets:?}");
}

#[then("the GitHub fake received a listing request")]
async fn the_fake_received_a_listing_request(world: &mut CliWorld) {
    let requests = world.github_requests();
    assert!(
        requests
            .iter()
            .any(|request| request.path.ends_with("/releases")),
        "{requests:?}"
    );
}

#[then("the GitHub fake received one request")]
async fn the_fake_received_one_request(world: &mut CliWorld) {
    let requests = world.github_requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
}

#[then("the GitHub fake saw the token on the listing and the asset request")]
async fn the_fake_saw_the_token(world: &mut CliWorld) {
    let requests = world.github_requests();
    let bearer = format!("Bearer {PRIVATE_TOKEN}");
    let listing = requests
        .iter()
        .find(|request| request.path.ends_with("/releases"))
        .unwrap_or_else(|| panic!("no listing request in {requests:?}"));
    assert_eq!(listing.authorization.as_deref(), Some(bearer.as_str()));
    let asset = requests
        .iter()
        .find(|request| request.path.contains("/releases/assets/"))
        .unwrap_or_else(|| panic!("no asset request in {requests:?}"));
    assert_eq!(asset.authorization.as_deref(), Some(bearer.as_str()));
    assert_eq!(asset.accept.as_deref(), Some("application/octet-stream"));
}

#[then(
    regex = r#"^(?:it|the GitHub fake) saw no Authorization header on the request to the redirected host "([^"]+)"$"#
)]
async fn the_fake_saw_no_token_on_the_redirect(world: &mut CliWorld, host: String) {
    let requests = world.github_requests();
    let redirected: Vec<_> = requests
        .iter()
        .filter(|request| {
            let to = request.host.as_deref();
            to.is_some_and(|have| have.starts_with(&format!("{host}:")))
        })
        .collect();
    assert!(
        !redirected.is_empty(),
        "nothing went to {host}: {requests:?}"
    );
    for request in redirected {
        assert_eq!(request.authorization, None, "{request:?}");
    }
}

// ---- where a token must not be ----

/// Whether `secret` is in the file or directory at `path`, which must hold at
/// least one byte: a search of nothing would pass vacuously.
fn is_in_place(path: &Path, secret: &str) -> bool {
    let mut read = 0;
    let found = is_in(path, secret, &mut read);
    assert!(
        read > 0,
        "cannot check {}: it holds nothing",
        path.display()
    );
    found
}

/// Whether `secret` is in `path`, or in any file under it; `read` counts the
/// bytes searched. A file that cannot be read fails the step.
fn is_in(path: &Path, secret: &str, read: &mut usize) -> bool {
    let unreadable = |e: std::io::Error| -> ! { panic!("cannot read {}: {e}", path.display()) };
    if path.is_dir() {
        let entries = std::fs::read_dir(path).unwrap_or_else(|e| unreadable(e));
        return entries
            .map(|entry| entry.unwrap_or_else(|e| unreadable(e)).path())
            .any(|entry| is_in(&entry, secret, read));
    }
    let bytes = std::fs::read(path).unwrap_or_else(|e| unreadable(e));
    *read += bytes.len();
    bytes
        .windows(secret.len())
        .any(|window| window == secret.as_bytes())
}

#[then(regex = r#"^"([^"]+)" appears in none of (.+)$"#)]
async fn a_secret_appears_in_none_of(world: &mut CliWorld, secret: String, places: String) {
    let work = world.sandbox.work_dir();
    let output = world.output();
    for place in places.split(", ").flat_map(|part| part.split(" and ")) {
        let found = match place {
            "lvpm.toml" => is_in_place(&work.join(MANIFEST_FILE), &secret),
            "lvpm.lock" => is_in_place(&work.join("lvpm.lock"), &secret),
            "the cache directory" => is_in_place(&world.sandbox.cache_dir(), &secret),
            "dest" => is_in_place(&work.join("dest"), &secret),
            "stdout" => output.stdout.contains(&secret),
            "stderr" => output.stderr.contains(&secret),
            other => panic!("do not know where {other:?} is"),
        };
        assert!(!found, "{secret:?} is in {place}");
    }
}

#[then(regex = r#"^"([^"]+)" appears in neither stdout nor stderr$"#)]
async fn a_secret_appears_in_neither(world: &mut CliWorld, secret: String) {
    let output = world.output();
    assert!(!output.stdout.contains(&secret), "{secret:?} is in stdout");
    assert!(!output.stderr.contains(&secret), "{secret:?} is in stderr");
}
