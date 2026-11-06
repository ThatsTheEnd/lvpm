//! Steps of the OCI install scenarios (S7b, slice 2): a consumer project in the
//! work directory whose `[sources]` has an OCI source, a loopback registry that
//! holds packages `lvpm publish` would have put there, and what `lvpm lock` and
//! `lvpm sync` make of them. The registry and the publisher side are the
//! publish steps'; a hosted feed is the loopback feed's.
//!
//! The manifest is written again after every step that changes it, so a bare
//! `I run "lvpm lock"` runs against the project the scenario describes.

use super::lock::sha256_hex;
use crate::CliWorld;
use crate::support::feed::{archive_path, listing};
use crate::support::fixtures::{FixturePackage, Payload};
use crate::support::project::MANIFEST_FILE;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use lvpm_core::lvpkg::{self, Meta};
use lvpm_core::oci::{
    ANNOTATION_CREATED, ANNOTATION_LABVIEW, ANNOTATION_REQUIRES, ANNOTATION_TITLE,
    ANNOTATION_VERSION,
};
use lvpm_core::released::SECS_PER_DAY;
use lvpm_core::timefmt::rfc3339;
use lvpm_testkit::oci::Seeded;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The `[sources]` name of the OCI source, and the prefix its packages are in.
pub(super) const TEAM: &str = "team";
/// The hosted feed's folder, and its `[sources]` name.
const HOSTED_DIR: &str = "/hosted/";
const HOSTED: &str = "hosted";
/// The LabVIEW a package needs unless a scenario says it needs a newer one:
/// below the consumer's 2026.
pub(super) const OLD_ENOUGH: &str = "21.0";
const TOO_NEW: &str = "30.0";
/// Where `lvpm sync` puts what it installs.
const DEST: &str = "dest";

/// What an OCI install scenario has set up and seen.
#[derive(Debug, Default)]
pub struct OciState {
    /// `[sources]`: each name and its URL, `<registry>` standing for the registry.
    sources: Vec<(String, String)>,
    /// `[dependencies]`: name, constraint and the `index` pin if it has one.
    dependencies: Vec<(String, String, Option<String>)>,
    min_age_days: u32,
    /// The `[policy]` table, as written, or nothing.
    pub(super) policy: String,
    /// What the scenario put in the registry, by repository and version.
    seeded: BTreeMap<(String, String), Seeded>,
    /// The OCI manifest digest `lvpm publish` printed.
    published: Option<String>,
    /// What the hosted feed lists.
    hosted: Vec<&'static FixturePackage>,
}

impl CliWorld {
    /// `lvpm.toml` of the consumer project, from what the scenario has said.
    pub(super) fn write_consumer(&self) {
        let state = &self.oci;
        let mut text = String::from(
            "[project]\nname = \"consumer\"\nversion = \"0.1.0\"\nlabview = \">=2026\"\n\n\
             [sources]\n",
        );
        for (name, url) in &state.sources {
            text.push_str(&format!("{name} = \"{url}\"\n"));
        }
        text.push_str("defaults = false\n");
        if state.min_age_days > 0 {
            text.push_str(&format!("min-age-days = {}\n", state.min_age_days));
        }
        text.push_str("\n[dependencies]\n");
        for (name, constraint, index) in &state.dependencies {
            text.push_str(&match index {
                Some(index) => {
                    format!("{name} = {{ version = \"{constraint}\", index = \"{index}\" }}\n")
                }
                None => format!("{name} = \"{constraint}\"\n"),
            });
        }
        text.push_str(&state.policy);
        std::fs::write(self.work(MANIFEST_FILE), self.with_placeholders(&text))
            .expect("write the consumer's manifest");
    }

    pub(super) fn set_source(&mut self, name: &str, url: &str) {
        self.oci.sources.retain(|(known, _)| known != name);
        self.oci.sources.push((name.to_string(), url.to_string()));
        self.write_consumer();
    }

    pub(super) fn oci_depend_on(&mut self, name: &str, constraint: &str, index: Option<&str>) {
        self.oci.dependencies.retain(|(known, _, _)| known != name);
        self.oci.dependencies.push((
            name.to_string(),
            constraint.to_string(),
            index.map(str::to_string),
        ));
        self.write_consumer();
    }

    /// Put `name` `version` in the registry as `lvpm publish` would, created at
    /// `created` (Unix seconds), needing `labview` and requiring `requires`
    /// (name, constraint); `marker` tells two packages of one name and version
    /// apart.
    pub(super) fn seed(
        &mut self,
        name: &str,
        version: &str,
        created: i64,
        labview: &str,
        marker: &str,
        requires: &[(&str, &str)],
    ) {
        let needs: Vec<String> = (requires.iter())
            .map(|(name, constraint)| format!("{name}{constraint}"))
            .collect();
        let needs = needs.join(", ");
        let meta = Meta {
            name: name.to_string(),
            version: version.to_string(),
            labview: ">=2021".to_string(),
            vendor: "Vendor".to_string(),
            display_name: name.to_string(),
            install_dir: format!("vi.lib/Vendor/{name}"),
            dependencies: (requires.iter())
                .map(|(name, constraint)| (name.to_string(), constraint.to_string()))
                .collect(),
        };
        let member = format!("{name} {version} {marker}").into_bytes();
        let layer = lvpkg::write(&meta, &[("A.vi".to_string(), member)], None).expect("a package");
        let repository = format!("{TEAM}/{name}");
        let created = rfc3339(created);
        let mut annotations = vec![
            (ANNOTATION_TITLE, name),
            (ANNOTATION_VERSION, version),
            (ANNOTATION_CREATED, created.as_str()),
            (ANNOTATION_LABVIEW, labview),
        ];
        if !needs.is_empty() {
            annotations.push((ANNOTATION_REQUIRES, &needs));
        }
        let seeded = self
            .registry()
            .seed_package(&repository, version, &annotations, &layer);
        self.oci
            .seeded
            .insert((repository, version.to_string()), seeded);
    }

    /// `name` `version` as the scenario put it in the registry.
    fn seeded(&self, name: &str, version: &str) -> &Seeded {
        let key = (format!("{TEAM}/{name}"), version.to_string());
        self.oci
            .seeded
            .get(&key)
            .unwrap_or_else(|| panic!("the scenario put no {name} {version} in the registry"))
    }

    /// The hosted feed lists `name` 1.0, and the project reads the feed.
    fn oci_host(&mut self, name: &str) {
        let package: &'static FixturePackage = Box::leak(Box::new(FixturePackage {
            name: Box::leak(name.to_string().into_boxed_str()),
            version: "1.0",
            display_name: Box::leak(name.to_string().into_boxed_str()),
            requires: &[],
            lv_min: "20.0",
            payload: Payload::Text,
            post_install_hook: None,
        }));
        self.oci.hosted.push(package);
        let body: String = (self.oci.hosted.iter())
            .map(|package| listing(std::slice::from_ref(*package), true))
            .collect();
        let feed = self.sandbox.feed();
        feed.revise(HOSTED_DIR, &body);
        for package in &self.oci.hosted {
            let served = archive_path(HOSTED_DIR, &package.file_name());
            feed.serve(&served, package.build_vip());
        }
        let url = feed.source_url(HOSTED_DIR);
        self.set_source(HOSTED, &url);
    }

    /// What the consumer's lock says of `name` under `key`.
    pub(super) fn lock_text(&self, key: &str, name: &str) -> String {
        let package = self.locked_package(name);
        let value = package.get(key).and_then(toml::Value::as_str);
        value
            .unwrap_or_else(|| panic!("{name} has no {key} in lvpm.lock: {package:?}"))
            .to_string()
    }

    /// The SHA-256 of the archive installed for `name`.
    fn installed_sha256(&self, name: &str) -> String {
        self.installed_archive_sha256(name)
            .unwrap_or_else(|| panic!("{name} is not installed: {:?}", self.installed()))
    }

    /// What the last command failed with.
    pub(super) fn failure(&self) -> String {
        self.assert_failed();
        let output = self.output();
        format!("{}\n{}", output.stderr, output.stdout)
    }
}

pub(super) fn now() -> i64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.expect("a clock after 1970").as_secs() as i64
}

// ---- the project ----

#[given(regex = r#"^a project whose \[sources\] has team = "([^"]+)"$"#)]
async fn project_with_team(world: &mut CliWorld, url: String) {
    world.sandbox.make_empty_dest();
    world.set_source(TEAM, &url);
}

#[given(regex = r#"^\[sources\] has team = "([^"]+)"$"#)]
async fn sources_has_team(world: &mut CliWorld, url: String) {
    world.set_source(TEAM, &url);
}

#[given(
    regex = r#"^the (?:consumer )?project depends on (\w+) = \{ version = "([^"]+)", index = "([^"]+)" \}$"#
)]
async fn depends_pinned(world: &mut CliWorld, name: String, constraint: String, index: String) {
    world.oci_depend_on(&name, &constraint, Some(&index));
}

#[given(regex = r#"^the project depends on (\w+) = "([^"]+)" without an index$"#)]
async fn depends_unpinned(world: &mut CliWorld, name: String, constraint: String) {
    world.oci_depend_on(&name, &constraint, None);
}

#[given(regex = r#"^the project's min-age-days is (\d+)$"#)]
async fn min_age_days(world: &mut CliWorld, days: u32) {
    world.oci.min_age_days = days;
    world.write_consumer();
}

#[given(
    regex = r#"^the project (?:also )?depends on a public package "([^"]+)" listed by a hosted feed$"#
)]
#[given(regex = r#"^the project depends only on "([^"]+)" from a hosted feed$"#)]
async fn depends_on_hosted(world: &mut CliWorld, name: String) {
    world.oci_host(&name);
    world.oci_depend_on(&name, "*", None);
}

#[given(regex = r#"^a hosted feed also lists "([^"]+)"$"#)]
async fn hosted_also_lists(world: &mut CliWorld, name: String) {
    world.oci_host(&name);
}

#[given(regex = r#"^no source lists "([^"]+)"$"#)]
async fn no_source_lists(world: &mut CliWorld, name: String) {
    assert!(
        !world.oci.hosted.iter().any(|package| package.name == name),
        "a hosted feed lists {name}"
    );
}

// ---- the publisher ----

#[given(regex = r#"^a publisher project "([^"]+)" ([0-9.]+) that lvpm build has packaged$"#)]
async fn publisher_project(world: &mut CliWorld, name: String, version: String) {
    super::publish::package_project(world, name, version, 21, None).await;
}

#[given(regex = r#"^I ran "([^"]+)" in the publisher project$"#)]
async fn ran_in_publisher(world: &mut CliWorld, command_line: String) {
    world.write_build_manifest();
    world.run(&command_line, None);
    let output = world.output();
    assert_eq!(output.exit_code, 0, "the command failed: {output:?}");
    let digest = (output.stdout.lines())
        .find_map(|line| line.trim().strip_prefix("OCI manifest digest "))
        .unwrap_or_else(|| panic!("no OCI manifest digest in {output:?}"));
    world.oci.published = Some(digest.to_string());
}

#[when(regex = r#"^I run "([^"]+)" in the consumer project$"#)]
async fn run_in_consumer(world: &mut CliWorld, command_line: String) {
    world.write_consumer();
    world.run(&command_line, None);
}

// ---- the registry ----

#[given(
    regex = r#"^the registry holds "([^"]+)" (.+?) under "([^"]+)"( with a layer whose bytes do not match its digest)?$"#
)]
async fn registry_holds(
    world: &mut CliWorld,
    name: String,
    versions: String,
    prefix: String,
    wrong: String,
) {
    assert_eq!(prefix, TEAM, "the steps hold packages under {TEAM}");
    static VERSION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(\d[\w.\-]*)(?: created (?:(\d+) days ago|(today)))?").expect("the pattern")
    });
    let leftover = VERSION.replace_all(&versions, "");
    assert!(
        leftover.split_whitespace().all(|word| word == "and"),
        "{versions:?} says more than a list of versions: {leftover:?} is left"
    );
    for item in VERSION.captures_iter(&versions) {
        let days: i64 = item
            .get(2)
            .map_or(0, |days| days.as_str().parse().expect("days"));
        world.seed(
            &name,
            &item[1],
            now() - days * SECS_PER_DAY,
            OLD_ENOUGH,
            "published",
            &[],
        );
    }
    if !wrong.is_empty() {
        let seeded = world.seeded(&name, versions.trim()).clone();
        world
            .registry()
            .corrupt(&format!("{TEAM}/{name}"), &seeded.layer, b"not the layer");
    }
}

/// `requiring "beta>=1.0"`: the package asks for `beta`, as `lvpm publish` writes it.
#[given(regex = r#"^the registry holds "([^"]+)" (\S+) under "team" requiring "([^"]+)"$"#)]
async fn registry_holds_requiring(
    world: &mut CliWorld,
    name: String,
    version: String,
    requirement: String,
) {
    let at = requirement
        .find(['<', '>', '='])
        .unwrap_or(requirement.len());
    let (required, constraint) = requirement.split_at(at);
    let constraint = if constraint.is_empty() {
        "*"
    } else {
        constraint
    };
    world.seed(
        &name,
        &version,
        now(),
        OLD_ENOUGH,
        "published",
        &[(required, constraint)],
    );
}

#[given(regex = r#"^"([^"]+)" (\S+) needs a newer LabVIEW than the project's$"#)]
async fn needs_newer_labview(world: &mut CliWorld, name: String, version: String) {
    world.seed(&name, &version, now(), TOO_NEW, "published", &[]);
}

#[given(regex = r#"^tag "([^"]+)" in "team/([^"]+)" now points at other bytes$"#)]
async fn tag_moved(world: &mut CliWorld, version: String, name: String) {
    world.seed(&name, &version, now(), OLD_ENOUGH, "moved", &[]);
}

#[given(regex = r#"^tag "([^"]+)" in "([^"]+)" holds a container image, not an lvpm package$"#)]
async fn tag_is_an_image(world: &mut CliWorld, tag: String, repository: String) {
    world.registry().seed_image(&repository, &tag);
}

#[given(
    regex = r#"^the registry (holds nothing|answers 500 for the tags list|is down) for "([^"]+)"$"#
)]
async fn registry_fails_for(world: &mut CliWorld, how: String, repository: String) {
    match how.as_str() {
        "holds nothing" => {
            let tags = world.registry().tags(&repository);
            assert!(tags.is_empty(), "{repository} holds the tags {tags:?}");
        }
        "is down" => world.registry().go_down(),
        _ => world
            .registry()
            .answer("GET", &format!("/v2/{repository}/tags/list"), 500),
    }
}

#[given(
    regex = r#"^the registry answers (\d+) (?:DENIED after a valid token|without a challenge) for the tags list of "([^"]+)"$"#
)]
async fn registry_refuses_the_list(world: &mut CliWorld, status: u16, repository: String) {
    let path = format!("/v2/{repository}/tags/list");
    world.registry().answer("GET", &path, status);
}

#[given(
    regex = r#"^the registry holds neither, and answers (\d+) DENIED after a valid token for the tags list of "([^"]+)"$"#
)]
async fn registry_holds_neither(world: &mut CliWorld, status: u16, repository: String) {
    registry_refuses_the_list(world, status, repository).await;
}

#[given("the registry deleted that OCI manifest")]
async fn registry_deleted_manifest(world: &mut CliWorld) {
    let digest = world.lock_text("digest", "alpha");
    world.registry().delete_manifest("team/alpha", &digest);
}

#[given("the registry now serves other bytes for that layer digest")]
async fn registry_serves_other_layer(world: &mut CliWorld) {
    let layer = format!("sha256:{}", world.lock_text("sha256", "alpha"));
    world
        .registry()
        .corrupt("team/alpha", &layer, b"other bytes");
}

#[given(regex = r#"^the registry now requires a login for "([^"]+)"$"#)]
async fn registry_wants_a_login(world: &mut CliWorld, prefix: String) {
    world.registry().refuse_under(&prefix, 401);
}

// ---- lock and sync as setup ----

#[given(regex = r#"^the project is locked with (\w+) (\S+)( and sync has installed it once)?$"#)]
async fn project_is_locked(world: &mut CliWorld, name: String, version: String, synced: String) {
    if !world
        .oci
        .dependencies
        .iter()
        .any(|(known, _, _)| *known == name)
    {
        world.oci_depend_on(&name, "*", Some(TEAM));
    }
    world.sandbox.setup_lvpm(&["lock"]);
    world.lock.before = world.lock_bytes();
    assert_eq!(world.lock_text("version", &name), version);
    if !synced.is_empty() {
        world
            .sandbox
            .setup_lvpm(&["sync", "--locked", "--prefix", DEST]);
    }
}

#[given("the package cache is empty")]
async fn cache_is_empty(world: &mut CliWorld) {
    world.sandbox.empty_cache();
}

#[when(regex = r#"^I run "([^"]+)" into a fresh environment$"#)]
async fn run_into_a_fresh_environment(world: &mut CliWorld, command_line: String) {
    let dest = world.work(DEST);
    std::fs::remove_dir_all(&dest).expect("remove the destination");
    std::fs::create_dir_all(&dest).expect("make a fresh destination");
    world.run(&command_line, None);
}

// ---- what lock and sync did ----

#[then(regex = r#"^"([^"]+)" (\S+) is installed with the published bytes$"#)]
async fn installed_with_published_bytes(world: &mut CliWorld, name: String, version: String) {
    let installed = world.installed();
    assert!(
        installed.contains(&(name.clone(), version)),
        "{installed:?}"
    );
    let package = world.publish_package();
    let published = sha256_hex(&std::fs::read(package).expect("the built package"));
    assert_eq!(world.installed_sha256(&name), published);
}

#[then("the consumer's lock records the OCI manifest digest that publish printed")]
async fn lock_has_published_digest(world: &mut CliWorld) {
    let printed = world.oci.published.clone().expect("a publish ran");
    assert_eq!(world.lock_text("digest", "alpha"), printed);
}

#[then(regex = r#"^(\w+) is (not )?installed$"#)]
async fn name_is_installed(world: &mut CliWorld, name: String, not: String) {
    let there = world.is_installed(&name);
    assert_eq!(
        there,
        not.is_empty(),
        "{:?}\n{}",
        world.installed(),
        world.printed()
    );
}

#[then(
    regex = r#"^the lock records (\w+) with source "([^"]+)" and the OCI manifest digest of (\S+)$"#
)]
async fn lock_records_source_and_digest(
    world: &mut CliWorld,
    name: String,
    source: String,
    version: String,
) {
    assert_eq!(
        world.lock_text("source", &name),
        world.with_placeholders(&source)
    );
    assert_eq!(
        world.lock_text("digest", &name),
        world.seeded(&name, &version).manifest
    );
}

#[then(regex = r#"^the lock's sha256 for (\w+) is the sha256 of the installed archive$"#)]
async fn lock_sha_is_installed_sha(world: &mut CliWorld, name: String) {
    assert_eq!(
        world.lock_text("sha256", &name),
        world.installed_sha256(&name)
    );
}

#[then(regex = r#"^the lock's sha256 for (\w+) is the sha256 of the hosted feed's archive$"#)]
async fn lock_sha_is_hosted_sha(world: &mut CliWorld, name: String) {
    let hosted = (world.oci.hosted.iter().find(|package| package.name == name))
        .unwrap_or_else(|| panic!("the hosted feed lists no {name}"));
    assert_eq!(
        world.lock_text("sha256", &name),
        sha256_hex(&hosted.build_vip())
    );
}

#[then(regex = r#"^the locked bytes of (\w+) are installed$"#)]
async fn locked_bytes_installed(world: &mut CliWorld, name: String) {
    lock_sha_is_installed_sha(world, name).await;
}

#[then(regex = r#"^the lock's digest and sha256 for (\w+) both describe the new bytes$"#)]
async fn lock_describes_the_new_bytes(world: &mut CliWorld, name: String) {
    let moved = world.seeded(&name, "1.1.0").clone();
    assert_eq!(world.lock_text("digest", &name), moved.manifest);
    assert_eq!(
        format!("sha256:{}", world.lock_text("sha256", &name)),
        moved.layer
    );
}

#[then("the lock is unchanged")]
async fn lock_is_unchanged(world: &mut CliWorld) {
    assert_eq!(world.lock_bytes(), world.lock.before, "the lock changed");
}

#[then(
    regex = r#"^the output names the digest lvpm.lock holds and the digest the registry now has for (\w+)$"#
)]
async fn output_names_both_digests(world: &mut CliWorld, name: String) {
    let before = world.lock.before.clone().expect("a lock was noted");
    let locked: toml::Table = String::from_utf8(before)
        .expect("a text lock")
        .parse()
        .expect("a lock");
    let locked = (locked["package"].as_array().expect("packages").iter())
        .find(|package| package["name"].as_str() == Some(name.as_str()))
        .and_then(|package| package["digest"].as_str())
        .unwrap_or_else(|| panic!("the lock held no digest for {name}"))
        .to_string();
    let now = world.seeded(&name, "1.1.0").manifest.clone();
    assert_ne!(locked, now, "the tag did not move");
    let failure = world.failure();
    for digest in [&locked, &now] {
        assert!(
            failure.contains(digest.as_str()),
            "{digest} is not in: {failure}"
        );
    }
}

#[then(regex = r#"^(\w+) (\S+) is locked$"#)]
async fn is_locked(world: &mut CliWorld, name: String, version: String) {
    assert_eq!(
        world.lock_text("version", &name),
        version,
        "{}",
        world.printed()
    );
}

#[then(regex = r#"^the lock lists (\w+) among the packages (\w+) requires$"#)]
async fn lock_lists_requirement(world: &mut CliWorld, required: String, name: String) {
    let package = world.locked_package(&name);
    let requires = package.get("requires").and_then(toml::Value::as_array);
    let listed = requires.is_some_and(|all| all.iter().any(|one| one.as_str() == Some(&required)));
    assert!(listed, "{name} requires {requires:?}");
}

#[then(regex = r#"^the lock records (\w+) as unresolved$"#)]
async fn lock_records_unresolved(world: &mut CliWorld, name: String) {
    let table = world.lock_table();
    let unresolved = table.get("unresolved").and_then(toml::Value::as_array);
    let listed =
        unresolved.is_some_and(|all| all.iter().any(|one| one["name"].as_str() == Some(&name)));
    assert!(listed, "{name} is not unresolved in: {unresolved:?}");
}

#[then(regex = r#"^no version of (\w+) is locked$"#)]
async fn none_locked(world: &mut CliWorld, name: String) {
    let locked = world
        .lock_bytes()
        .map(|_| world.locked_versions())
        .unwrap_or_default();
    assert!(
        locked.iter().all(|(known, _)| *known != name),
        "{name} is locked: {locked:?}"
    );
}

#[then(regex = r#"^(\w+) comes from the hosted feed$"#)]
async fn comes_from_hosted(world: &mut CliWorld, name: String) {
    let feed = world.sandbox.feed().source_url(HOSTED_DIR);
    assert_eq!(
        world.lock_text("source", &name),
        format!("{feed}/index.vipr"),
        "{}",
        world.printed()
    );
}

#[then(regex = r#"^the lock records no OCI digest for (\w+)$"#)]
async fn lock_has_no_digest(world: &mut CliWorld, name: String) {
    let package = world.locked_package(&name);
    assert!(!package.contains_key("digest"), "{package:?}");
}

// ---- what the registry was asked ----

#[then(regex = r#"^the registry received no request for "([^"]+)"$"#)]
async fn registry_not_asked_for(world: &mut CliWorld, repository: String) {
    let asked = world.registry().requests();
    let prefix = format!("/v2/{repository}/");
    assert!(
        asked
            .iter()
            .all(|request| !request.path.starts_with(&prefix)),
        "{asked:?}"
    );
}

/// Nothing was listed or read by tag, and the manifest was read by its digest.
#[then(
    regex = r#"^the registry was asked for the OCI manifest by digest and never by tag "([^"]+)"$"#
)]
async fn asked_by_digest(world: &mut CliWorld, tag: String) {
    let asked: Vec<String> = (world.registry().requests().into_iter())
        .filter(|request| request.path.starts_with("/v2/team/alpha/"))
        .map(|request| format!("{} {}", request.method, request.path))
        .collect();
    let by_digest = |line: &String| line.starts_with("GET ") && line.contains("/manifests/sha256:");
    assert!(asked.iter().any(by_digest), "{asked:?}");
    let by_tag = |line: &String| line.ends_with(&format!("/manifests/{tag}"));
    assert!(!asked.iter().any(by_tag), "{asked:?}");
    assert!(
        !asked.iter().any(|line| line.contains("/tags/list")),
        "{asked:?}"
    );
}

// ---- what the command printed ----

#[then(regex = r#"^it fails saying '(.+)'$"#)]
async fn fails_saying_text(world: &mut CliWorld, text: String) {
    let message = world.failure();
    assert!(message.contains(&text), "{text:?} is not in: {message}");
}

#[then(regex = r#"^the output says '(.+)'$"#)]
async fn output_says_text(world: &mut CliWorld, text: String) {
    let printed = world.printed();
    assert!(printed.contains(&text), "{text:?} is not in: {printed}");
}

#[then(regex = r#"^the output warns '(.+)'$"#)]
async fn output_warns(world: &mut CliWorld, text: String) {
    let warned = world.output().stderr.lines().any(|line| {
        line.strip_prefix("warning: ")
            .is_some_and(|rest| rest.contains(&text))
    });
    assert!(warned, "no warning says {text:?}: {}", world.printed());
}

#[then("it fails with these lines, in this order:")]
async fn fails_with_lines(world: &mut CliWorld, step: &Step) {
    let table = step.table.as_ref().expect("a table of lines");
    let wanted: Vec<&str> = table.rows.iter().map(|row| row[0].as_str()).collect();
    let message = world.failure();
    let lines: Vec<&str> = (message.lines())
        .map(|line| line.strip_prefix("Error: ").unwrap_or(line))
        .collect();
    let first = lines
        .iter()
        .position(|line| *line == wanted[0])
        .unwrap_or_else(|| panic!("{:?} is not a line of: {message}", wanted[0]));
    let found: Vec<&str> = lines[first..].iter().take(wanted.len()).copied().collect();
    assert_eq!(found, wanted);
}
