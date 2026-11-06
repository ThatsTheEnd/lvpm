//! Steps of the community-index scenarios (S8): the index repo the GitHub
//! fake serves, the release its entry points at, who attested that release,
//! the projects that read the index, and the entry files `lvpm index add`
//! writes.

use super::github::{
    PILOT_B, PILOT_B_DEPENDENCY_FILES, PILOT_B_FILE, PILOT_B_NEEDS, PILOT_B_TAG, PROJECT_LABVIEW,
};
use super::lock::sha256_hex;
use crate::CliWorld;
use crate::support::fixtures::{FixturePackage, Payload};
use crate::support::pilots::{check_checksums, pilots_dir};
use crate::support::project::MANIFEST_FILE;
use crate::support::sandbox::LOCAL_PACKAGES_DIR;
use cucumber::{given, then, when};
use lvpm_core::community::{EntryFile, FileVersion};
use lvpm_testkit::gh::{self, Behavior};

/// The version of pilot B's file, which the index lists.
const PILOT_B_VERSION: &str = "1.2.1.16";
/// What pilot B needs, as an entry file says it.
const PILOT_B_REQUIRES: &str = "labview_open_source_lib_data_manipulation >=1.6";
/// The shorthand the scenarios use for what pilot B needs.
const DATA_MANIPULATION: &str = PILOT_B_NEEDS;

/// What an index scenario has set up.
#[derive(Debug, Default)]
pub struct CommunityState {
    /// The index repo, `owner/repo`.
    index: String,
    /// The repo the index entry names as pilot B's owner.
    publisher: String,
    /// The bytes the release serves in place of pilot B's file.
    other_bytes: Option<Vec<u8>>,
    /// What the packages folder held before `lvpm index add` ran, by file name.
    packages_before: Vec<(String, Vec<u8>)>,
    /// What an `lvpm index check` scenario has set up.
    check: CheckState,
}

fn pilot_b_vip() -> Vec<u8> {
    std::fs::read(pilots_dir().join(PILOT_B_FILE)).expect("pilot B's .vip")
}

impl CliWorld {
    /// The text of `lvpm.toml` as it is now, changed by `edit`.
    fn change_the_manifest(&self, edit: impl FnOnce(&str) -> String) {
        let path = self.sandbox.work_dir().join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).expect("read the manifest");
        std::fs::write(path, edit(&text)).expect("write the manifest");
    }

    /// A project that reads the local package folder and the index `source`,
    /// with `dependencies` as its `[dependencies]` lines.
    fn community_project(&self, source: &str, dependencies: &str) {
        self.sandbox.use_local_project(Some(PROJECT_LABVIEW));
        self.change_the_manifest(|text| {
            let sources = format!("[sources]\ncommunity = \"{source}\"\n");
            let text = text.replacen("[sources]\n", &sources, 1);
            format!("{text}\n[dependencies]\n{dependencies}\n")
        });
    }

    fn pilot_b_in_the_lock(&self) -> toml::Table {
        self.locked_package(PILOT_B)
    }
}

// ---- the index, its release and its attestation ----

#[given(regex = r#"^a fake GitHub whose index (\S+) lists pilot B, published by "([^"]+)"$"#)]
async fn a_fake_github_with_an_index(world: &mut CliWorld, index: String, publisher: String) {
    if let Err(why) = check_checksums(&pilots_dir()) {
        panic!("{why}");
    }
    let web = world.github_fake().api_url();
    let entry = EntryFile {
        name: PILOT_B.to_string(),
        github: publisher.clone(),
        versions: vec![FileVersion {
            version: PILOT_B_VERSION.to_string(),
            url: format!("{web}/{publisher}/releases/download/{PILOT_B_TAG}/{PILOT_B_FILE}"),
            sha256: sha256_hex(&pilot_b_vip()),
            labview: None,
            requires: vec![PILOT_B_REQUIRES.to_string()],
        }],
    };
    let path = format!("packages/{PILOT_B}.toml");
    world.github_fake().add_file(&index, &path, &entry.write());
    world.community = CommunityState {
        index,
        publisher,
        ..CommunityState::default()
    };
}

#[given(
    regex = r#"^(\S+)'s release serves pilot B's file and its dependency data_manipulation is in the project's local folder$"#
)]
async fn the_release_serves_pilot_b(world: &mut CliWorld, publisher: String) {
    assert_eq!(publisher, world.community.publisher);
    world.serve(
        &publisher,
        PILOT_B_TAG,
        &[(PILOT_B_FILE, pilot_b_vip())],
        false,
    );
    let packages = world.sandbox.root().join(LOCAL_PACKAGES_DIR);
    let file = PILOT_B_DEPENDENCY_FILES[0];
    std::fs::copy(pilots_dir().join(file), packages.join(file))
        .unwrap_or_else(|e| panic!("copy {file}: {e}"));
}

/// The repo whose workflow gh says built the file `built_by`; the API lists
/// the attestation under the repo the index names.
#[given(regex = r"^pilot B's release is attested by (\S+)$")]
async fn pilot_bs_release_is_attested(world: &mut CliWorld, built_by: String) {
    let publisher = world.community.publisher.clone();
    let sha256 = sha256_hex(&pilot_b_vip());
    world.github_fake().add_attestation(&publisher, &sha256);
    world.sandbox.use_fake_gh();
    let verifies = Behavior::Verifies {
        repository: format!("https://github.com/{built_by}"),
    };
    gh::install(&world.sandbox.gh_dir(), &verifies).expect("install the fake gh");
}

/// The fake lists no attestation for it, so gh is never asked.
#[given("pilot B's release is not attested")]
async fn pilot_bs_release_is_not_attested(_: &mut CliWorld) {}

#[given("the release serves other bytes for pilot B's file")]
async fn the_release_serves_other_bytes(world: &mut CliWorld) {
    // Another package of the same name and version: it reads as pilot B.
    let mut other = pilot_b_vip();
    other.extend_from_slice(b"re-uploaded");
    let publisher = world.community.publisher.clone();
    world
        .github_fake()
        .replace_bytes(&publisher, PILOT_B_FILE, &other);
    world.community.other_bytes = Some(other);
}

#[given("the index lists nothing")]
async fn the_index_lists_nothing(world: &mut CliWorld) {
    let index = world.community.index.clone();
    let path = format!("packages/{PILOT_B}.toml");
    world.github_fake().remove_file(&index, &path);
}

// ---- the projects ----

#[given(
    regex = r#"^the pilot B project with community = "([^"]+)" in \[sources\] and pilot B as a plain dependency$"#
)]
async fn the_pilot_b_project(world: &mut CliWorld, source: String) {
    world.community_project(&source, &format!("{PILOT_B} = \">=1.2\""));
}

#[given(
    regex = r#"^the project with community = "([^"]+)" in \[sources\] and data_manipulation as a plain dependency$"#
)]
async fn a_project_with_data_manipulation(world: &mut CliWorld, source: String) {
    world.community_project(&source, &format!("{DATA_MANIPULATION} = \">=1.6\""));
}

#[given(
    regex = r#"^data_manipulation is pinned with index = "community" and the index does not list it$"#
)]
async fn data_manipulation_is_pinned(world: &mut CliWorld) {
    let source = format!("github:{}", world.community.index);
    let line = format!("{DATA_MANIPULATION} = {{ version = \">=1.6\", index = \"community\" }}");
    world.community_project(&source, &line);
}

#[given("that project was synced once and its installed packages are removed")]
async fn synced_once_then_removed(world: &mut CliWorld) {
    world.sandbox.setup_lvpm(&["sync", "--prefix", "dest"]);
    std::fs::remove_dir_all(world.dest_dir()).expect("dest held what the sync installed");
}

// ---- what the lock and the install came to ----

#[then(regex = r#"^lvpm\.lock has pilot B from "([^"]+)" with the index's sha256$"#)]
async fn the_lock_has_pilot_b_from(world: &mut CliWorld, source: String) {
    let package = world.pilot_b_in_the_lock();
    assert_eq!(
        package["source"].as_str(),
        Some(source.as_str()),
        "{package:?}"
    );
    let sha256 = sha256_hex(&pilot_b_vip());
    assert_eq!(
        package["sha256"].as_str(),
        Some(sha256.as_str()),
        "{package:?}"
    );
}

#[then(regex = r#"^lvpm\.lock has pilot B's signer and subject "([^"]+)"$"#)]
async fn the_lock_has_pilot_bs_signer(world: &mut CliWorld, subject: String) {
    let package = world.pilot_b_in_the_lock();
    assert_eq!(
        package["subject"].as_str(),
        Some(subject.as_str()),
        "{package:?}"
    );
    let signer = package["signer"].as_str().expect("a signer");
    assert!(signer.starts_with(&subject), "{signer}");
}

#[then("pilot B and data_manipulation are installed in dest")]
async fn both_are_installed(world: &mut CliWorld) {
    let installed = world.installed();
    let names: Vec<&str> = installed.iter().map(|(name, _)| name.as_str()).collect();
    for name in [PILOT_B, DATA_MANIPULATION] {
        assert!(names.contains(&name), "{names:?}\n{}", world.printed());
    }
}

#[then("the command succeeds and pilot B is installed with the locked sha256")]
async fn installed_with_the_locked_sha256(world: &mut CliWorld) {
    assert_eq!(world.output().exit_code, 0, "{}", world.printed());
    let locked = world.pilot_b_in_the_lock()["sha256"]
        .as_str()
        .map(str::to_string);
    assert!(locked.is_some(), "pilot B has no sha256 in lvpm.lock");
    assert_eq!(world.installed_archive_sha256(PILOT_B), locked);
}

#[then("the command succeeds and lvpm.lock has pilot B with no signer")]
async fn the_lock_has_pilot_b_unsigned(world: &mut CliWorld) {
    assert_eq!(world.output().exit_code, 0, "{}", world.printed());
    let package = world.pilot_b_in_the_lock();
    for key in ["signer", "subject"] {
        assert!(!package.contains_key(key), "{key} in {package:?}");
    }
}

// Local folders rank before every [sources] line, so source order across [sources]
// is covered by tests/github_index.rs, not by this scenario.
#[then("data_manipulation is locked from the local folder")]
async fn data_manipulation_is_locked_from_the_folder(world: &mut CliWorld) {
    assert_eq!(world.output().exit_code, 0, "{}", world.printed());
    let package = world.locked_package(DATA_MANIPULATION);
    assert_eq!(package["source"].as_str(), Some("local"), "{package:?}");
}

// ---- what a refusal said ----

#[then(regex = r#"^the command fails naming pilot B, both hashes and "([^"]+)"$"#)]
async fn fails_naming_both_hashes(world: &mut CliWorld, said: String) {
    world.assert_failed();
    let report = world.printed();
    let other = world
        .community
        .other_bytes
        .as_ref()
        .expect("other bytes were served");
    let (index_sha, actual) = (sha256_hex(&pilot_b_vip()), sha256_hex(other));
    for part in [PILOT_B, index_sha.as_str(), actual.as_str(), said.as_str()] {
        assert!(report.contains(part), "{part:?} is not in:\n{report}");
    }
}

#[then(regex = r#"^the hint says "([^"]+)"$"#)]
async fn the_hint_says(world: &mut CliWorld, hint: String) {
    let report = world.printed();
    assert!(
        report
            .lines()
            .any(|line| line.trim() == format!("hint: {hint}")),
        "no line is \"hint: {hint}\" in:\n{report}"
    );
}

#[then("no lvpm.lock is written and pilot B is not installed")]
async fn no_lock_and_nothing_installed(world: &mut CliWorld) {
    assert!(world.lock_bytes().is_none(), "a lock was written");
    assert_eq!(world.installed(), []);
}

#[then(regex = r#"^the command fails naming data_manipulation and "([^"]+)"$"#)]
async fn fails_naming_data_manipulation(world: &mut CliWorld, said: String) {
    world.assert_failed();
    let report = world.printed();
    for part in [DATA_MANIPULATION, said.as_str()] {
        assert!(report.contains(part), "{part:?} is not in:\n{report}");
    }
}

#[then("data_manipulation is not taken from the local folder")]
async fn data_manipulation_is_not_taken(world: &mut CliWorld) {
    assert!(world.lock_bytes().is_none(), "a lock was written");
}

// ---- lvpm index add ----

/// The folder of entry files `lvpm index add` is given, in the working directory.
const PACKAGES_DIR: &str = "packages";
/// The tag pilot B's release is under in the urls these scenarios give.
const ADD_TAG: &str = "1.2.1";
/// What `lvpm index add` lists of pilot B: what it needs, and its LabVIEW minimum.
const PILOT_B_LISTED_REQUIRES: &str = "labview_open_source_lib_data_manipulation >=1.5.0.15";
const PILOT_B_LISTED_LABVIEW: &str = ">=13.0";

impl CliWorld {
    /// `line` with pilot B's file for `<file>`, when it is an `lvpm index add`.
    pub(crate) fn with_package_file(&self, line: &str) -> String {
        match line.starts_with("lvpm index add") {
            true => line.replace("<file>", PILOT_B_FILE),
            false => line.to_string(),
        }
    }

    fn packages_dir(&self) -> std::path::PathBuf {
        let dir = self.sandbox.work_dir().join(PACKAGES_DIR);
        std::fs::create_dir_all(&dir).expect("create the packages folder");
        dir
    }

    fn entry_file_path(&self) -> std::path::PathBuf {
        self.packages_dir().join(format!("{PILOT_B}.toml"))
    }

    /// The entry file `lvpm index add` wrote.
    fn written_entry_file(&self) -> EntryFile {
        let text = std::fs::read_to_string(self.entry_file_path()).expect("the entry file");
        EntryFile::parse(PILOT_B, &text).expect("an entry file that reads")
    }

    fn packages_snapshot(&self) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(self.packages_dir())
            .expect("read the packages folder")
            .map(|entry| entry.expect("a directory entry"))
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let bytes = std::fs::read(entry.path()).expect("read an entry file");
                (name, bytes)
            })
            .collect();
        files.sort();
        files
    }

    /// An entry file for pilot B owned by `owner`, listing `version`.
    fn list_pilot_b(&self, owner: &str, version: &str) {
        let entry = EntryFile {
            name: PILOT_B.to_string(),
            github: owner.to_string(),
            versions: vec![FileVersion {
                version: version.to_string(),
                url: format!("https://github.com/{owner}/releases/download/{version}/old.lvpkg"),
                sha256: "0".repeat(64),
                labview: None,
                requires: Vec::new(),
            }],
        };
        std::fs::write(self.entry_file_path(), entry.write()).expect("write the entry file");
    }
}

#[given(regex = r"^pilot B's built \.lvpkg as <file>, version 1\.2\.1\.16$")]
async fn pilot_bs_built_package(world: &mut CliWorld) {
    if let Err(why) = check_checksums(&pilots_dir()) {
        panic!("{why}");
    }
    let work = world.sandbox.work_dir();
    std::fs::create_dir_all(&work).expect("create the working directory");
    std::fs::write(work.join(PILOT_B_FILE), pilot_b_vip()).expect("copy pilot B's file");
    // The default API, so the web base is github.com whatever the machine sets.
    world.sandbox.set_github("https://api.github.com", None);
}

#[given("an empty packages folder")]
async fn an_empty_packages_folder(world: &mut CliWorld) {
    world.packages_dir();
}

#[given(
    regex = r"^packages/labview_open_source_lib_serializer\.toml owned by (\S+) with version (\S+)$"
)]
async fn an_entry_file_owned_by(world: &mut CliWorld, owner: String, version: String) {
    world.list_pilot_b(&owner, &version);
}

#[given(regex = r"^the entry file is owned by (\S+)$")]
async fn the_entry_file_is_owned_by(world: &mut CliWorld, owner: String) {
    world.list_pilot_b(&owner, "1.2.0.1");
}

#[given(regex = r"^the entry file already lists (\S+)$")]
async fn the_entry_file_already_lists(world: &mut CliWorld, version: String) {
    world.list_pilot_b("someone/Serializer", &version);
}

#[given("the entry file is not TOML")]
async fn the_entry_file_is_not_toml(world: &mut CliWorld) {
    std::fs::write(world.entry_file_path(), "name = [").expect("write the entry file");
}

/// `<file>` is pilot B's file; any other name is a file that is not a package.
/// With no `--url` given, the url is the file's release under `--github`.
#[when(
    regex = r"^I run lvpm index add on (\S+) with --github (\S+?)( and its release url| --url \S+)?$"
)]
async fn run_index_add(world: &mut CliWorld, file: String, github: String, url: String) {
    let file = match file.as_str() {
        "<file>" => PILOT_B_FILE.to_string(),
        other => {
            std::fs::write(world.sandbox.work_dir().join(other), "not a package")
                .expect("write the file");
            other.to_string()
        }
    };
    let url = match url.trim().strip_prefix("--url ") {
        Some(url) => url.to_string(),
        None => format!("https://github.com/{github}/releases/download/{ADD_TAG}/{file}"),
    };
    world.community.packages_before = world.packages_snapshot();
    world.run(
        &format!("lvpm index add {PACKAGES_DIR} {file} --github {github} --url {url}"),
        None,
    );
}

#[then(regex = r#"^the command succeeds printing "([^"]+)"$"#)]
async fn succeeds_printing(world: &mut CliWorld, line: String) {
    assert_eq!(world.output().exit_code, 0, "{}", world.printed());
    assert!(
        world.output().stdout.lines().any(|printed| printed == line),
        "no line is {line:?} in:\n{}",
        world.printed()
    );
}

/// `<error>` stands for any text; the stderr line (after the `Error: ` the
/// front end puts first) starts with what comes before it and ends with what
/// comes after it.
#[then(regex = r#"^the command fails with exit code 1 printing "([^"]+)"$"#)]
async fn fails_printing(world: &mut CliWorld, line: String) {
    let line = world.with_check_values(&line);
    assert_eq!(world.output().exit_code, 1, "{}", world.printed());
    let report = world.printed();
    let said = |printed: &str| match line.split_once("<error>") {
        Some((start, end)) => {
            printed.len() >= start.len() + end.len()
                && printed.starts_with(start)
                && printed.ends_with(end)
        }
        None => printed == line,
    };
    assert!(
        world
            .output()
            .stderr
            .lines()
            .any(|printed| said(printed.strip_prefix("Error: ").unwrap_or(printed))),
        "no line says {line:?} in:
{report}"
    );
}

#[then("the packages folder is unchanged")]
async fn the_packages_folder_is_unchanged(world: &mut CliWorld) {
    assert_eq!(world.packages_snapshot(), world.community.packages_before);
}

#[then(
    regex = r#"^that file lists github "([^"]+)" and one version with the file's sha256, requires and LabVIEW minimum$"#
)]
async fn that_file_lists_pilot_b(world: &mut CliWorld, github: String) {
    let file = world.written_entry_file();
    assert_eq!(file.github, github);
    let [version] = file.versions.as_slice() else {
        panic!("{file:?}");
    };
    assert_eq!(version.version, PILOT_B_VERSION);
    assert_eq!(version.sha256, sha256_hex(&pilot_b_vip()));
    assert_eq!(version.requires, [PILOT_B_LISTED_REQUIRES]);
    assert_eq!(version.labview.as_deref(), Some(PILOT_B_LISTED_LABVIEW));
    let url = format!("https://github.com/{github}/releases/download/{ADD_TAG}/{PILOT_B_FILE}");
    assert_eq!(version.url, url);
}

#[then("the file lists both versions, the old one unchanged")]
async fn the_file_lists_both_versions(world: &mut CliWorld) {
    assert_eq!(world.output().exit_code, 0, "{}", world.printed());
    let file = world.written_entry_file();
    let versions: Vec<&str> = file.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(versions, ["1.2.0.1", PILOT_B_VERSION]);
    let before = String::from_utf8(world.community.packages_before[0].1.clone()).expect("UTF-8");
    let before = EntryFile::parse(PILOT_B, &before).expect("the entry file before");
    assert_eq!(file.versions[0], before.versions[0]);
}

#[then(regex = r#"^the command succeeds and the file's github stays "([^"]+)"$"#)]
async fn the_file_keeps_its_github(world: &mut CliWorld, github: String) {
    assert_eq!(world.output().exit_code, 0, "{}", world.printed());
    assert_eq!(world.written_entry_file().github, github);
}

// ---- lvpm index check ----

/// The owner foo belongs to, in the index the checks run against.
const FOO_OWNER: &str = "a/x";
/// What foo's package needs and what bar's does, as the packages say it.
const FOO_NEEDS: &[(&str, &str)] = &[("bar", "2.0")];
const BAR_NEEDS: &[(&str, &str)] = &[("baz", "2.0")];
/// What an entry says of those needs, and of the LabVIEW minimum every package here has.
const FOO_LISTED_REQUIRES: &str = "bar >=2.0";
const BAR_LISTED_REQUIRES: &str = "baz >=2.0";
const PACKAGE_LISTED_LABVIEW: &str = ">=20.0";
/// The two checkouts, in the working directory.
const BASE_DIR: &str = "base";
const HEAD_DIR: &str = "head";

/// What an `lvpm index check` scenario has set up.
#[derive(Debug, Default)]
pub struct CheckState {
    /// The SHA-256 of the file the release of foo 1.1.0 serves.
    file_sha256: String,
    /// The SHA-256 the head's entry for foo 1.1.0 says.
    entry_sha256: String,
    /// The two SHA-256 values of the step that changes foo 1.0.0's.
    old_sha256: String,
    new_sha256: String,
}

/// A package `name` at `version` needing `needs`, as a `.vip`.
fn vip(
    name: &'static str,
    version: &'static str,
    needs: &'static [(&'static str, &'static str)],
) -> Vec<u8> {
    FixturePackage {
        name,
        version,
        display_name: name,
        requires: needs,
        lv_min: "20.0",
        payload: Payload::Text,
        post_install_hook: None,
    }
    .build_vip()
}

fn release_file(name: &str, version: &str) -> String {
    format!("{name}-{version}.vip")
}

impl CliWorld {
    /// `line` with the values a check scenario's steps set put in for its
    /// placeholders.
    pub(crate) fn with_check_values(&self, line: &str) -> String {
        let check = &self.community.check;
        line.replace("<file-sha256>", &check.file_sha256)
            .replace("<entry-sha256>", &check.entry_sha256)
            .replace("<old>", &check.old_sha256)
            .replace("<new>", &check.new_sha256)
    }

    /// The version of `name` that a release of `github` holds, as the index
    /// lists it: the file's url on the fake, and what the package says of itself.
    fn listed_version(
        &mut self,
        github: &str,
        name: &str,
        version: &str,
        sha256: &str,
        requires: &str,
    ) -> FileVersion {
        let web = self.github_fake().api_url();
        let file = release_file(name, version);
        FileVersion {
            version: version.to_string(),
            url: format!("{web}/{github}/releases/download/{version}/{file}"),
            sha256: sha256.to_string(),
            labview: Some(PACKAGE_LISTED_LABVIEW.to_string()),
            requires: vec![requires.to_string()],
        }
    }

    /// `path` in the checkout `checkout`, whose folder exists.
    fn checkout_file(&self, checkout: &str, path: &str) -> std::path::PathBuf {
        let path = self.sandbox.work_dir().join(checkout).join(path);
        std::fs::create_dir_all(path.parent().expect("a file in a folder")).expect("create it");
        path
    }

    /// foo's entry file in the head, as the head has it now.
    fn head_foo(&self) -> EntryFile {
        let path = self.checkout_file(HEAD_DIR, "packages/foo.toml");
        let text = std::fs::read_to_string(path).expect("the head's foo.toml");
        EntryFile::parse("foo", &text).expect("an entry file that reads")
    }

    fn write_head_foo(&self, file: &EntryFile) {
        let path = self.checkout_file(HEAD_DIR, "packages/foo.toml");
        std::fs::write(path, file.write()).expect("write the head's foo.toml");
    }

    /// The release of foo 1.1.0 serves `bytes` from now on, and the API lists
    /// an attestation for them.
    fn serve_other_foo_file(&mut self, bytes: &[u8]) {
        let sha256 = sha256_hex(bytes);
        let fake = self.github_fake();
        fake.replace_bytes(FOO_OWNER, &release_file("foo", "1.1.0"), bytes);
        fake.add_attestation(FOO_OWNER, &sha256);
        self.community.check.file_sha256 = sha256;
    }

    /// The fake `gh` of this scenario acts as `behavior`.
    fn gh_acts(&self, behavior: &Behavior) {
        self.sandbox.use_fake_gh();
        gh::install(&self.sandbox.gh_dir(), behavior).expect("install the fake gh");
    }
}

#[given(
    regex = r#"^a base index where packages/foo\.toml is owned by "([^"]+)" with version 1\.0\.0$"#
)]
async fn a_base_index(world: &mut CliWorld, owner: String) {
    let sha256 = sha256_hex(&vip("foo", "1.0.0", FOO_NEEDS));
    let version = world.listed_version(&owner, "foo", "1.0.0", &sha256, FOO_LISTED_REQUIRES);
    let file = EntryFile {
        name: "foo".into(),
        github: owner,
        versions: vec![version],
    };
    for checkout in [BASE_DIR, HEAD_DIR] {
        let path = world.checkout_file(checkout, "packages/foo.toml");
        std::fs::write(path, file.write()).expect("write the entry file");
    }
}

#[given(
    regex = r"^a fake GitHub where (\S+)'s releases hold foo 1\.0\.0 and foo 1\.1\.0, attested by (\S+)$"
)]
async fn a_fake_github_with_foo(world: &mut CliWorld, repo: String, built_by: String) {
    for (version, bytes) in [
        ("1.0.0", vip("foo", "1.0.0", FOO_NEEDS)),
        ("1.1.0", vip("foo", "1.1.0", FOO_NEEDS)),
    ] {
        let file = release_file("foo", version);
        world.serve(&repo, version, &[(&file, bytes.clone())], false);
        let sha256 = sha256_hex(&bytes);
        world.github_fake().add_attestation(&repo, &sha256);
        world.community.check.file_sha256 = sha256;
    }
    world.gh_acts(&Behavior::Verifies {
        repository: format!("https://github.com/{built_by}"),
    });
}

// What a row of the outline replaces of the Background.

#[given("the Background")]
async fn the_background(_: &mut CliWorld) {}

#[given("the release file of foo 1.1.0 answers 404")]
async fn the_release_file_answers_404(world: &mut CliWorld) {
    let file = release_file("foo", "1.1.0");
    world.github_fake().remove_asset(FOO_OWNER, &file);
}

#[given("foo 1.1.0's release has no attestation")]
async fn foo_has_no_attestation(world: &mut CliWorld) {
    let sha256 = world.community.check.file_sha256.clone();
    world.github_fake().remove_attestation(FOO_OWNER, &sha256);
}

#[given("gh cannot run")]
async fn gh_cannot_run(world: &mut CliWorld) {
    world.sandbox.use_fake_gh();
    std::fs::remove_dir_all(world.sandbox.gh_dir()).expect("remove the fake gh");
    world.sandbox.use_fake_gh();
}

#[given("foo 1.1.0's attestation does not verify")]
async fn foo_does_not_verify(world: &mut CliWorld) {
    world.gh_acts(&Behavior::Rejects {
        stderr: "verification failed".into(),
    });
}

/// gh vouches for the file whichever repo it is asked about, and names `built_by`.
#[given(regex = r"^foo 1\.1\.0 is attested by (\S+)$")]
async fn foo_is_attested_by(world: &mut CliWorld, built_by: String) {
    world.gh_acts(&Behavior::VerifiesAnyRepo {
        repository: format!("https://github.com/{built_by}"),
    });
}

// What the head changes.

#[given(regex = r"^the head adds foo 1\.1\.0 (.+)$")]
async fn the_head_adds_foo(world: &mut CliWorld, entry: String) {
    let file_sha256 = world.community.check.file_sha256.clone();
    let mut version =
        world.listed_version(FOO_OWNER, "foo", "1.1.0", &file_sha256, FOO_LISTED_REQUIRES);
    match entry.as_str() {
        "with the release's url and sha256" => {}
        "with a url on example.com" => {
            version.url = "https://example.com/a/x/releases/download/1.1.0/foo-1.1.0.vip".into();
        }
        "with another sha256" => version.sha256 = sha256_hex(b"another file"),
        "whose requires differ from the package" => version.requires = vec!["bar >=3.0".into()],
        "whose labview differs from the package" => version.labview = Some(">=21.0".into()),
        "named foo but the package says bar" => {
            world.serve_other_foo_file(&vip("bar", "1.1.0", FOO_NEEDS));
            version.sha256 = world.community.check.file_sha256.clone();
        }
        "whose version differs from the package" => {
            world.serve_other_foo_file(&vip("foo", "1.1.1", FOO_NEEDS));
            version.sha256 = world.community.check.file_sha256.clone();
        }
        other => panic!("no step knows what the head adds {other:?}"),
    }
    world.community.check.entry_sha256 = version.sha256.clone();
    let mut file = world.head_foo();
    file.versions.push(version);
    world.write_head_foo(&file);
}

#[given(
    regex = r#"^the head adds packages/bar\.toml owned by "([^"]+)", whose release is attested by (\S+)$"#
)]
async fn the_head_adds_bar(world: &mut CliWorld, owner: String, built_by: String) {
    let bytes = vip("bar", "1.0.0", BAR_NEEDS);
    let sha256 = sha256_hex(&bytes);
    let file = release_file("bar", "1.0.0");
    world.serve(&owner, "1.0.0", &[(&file, bytes)], false);
    world.github_fake().add_attestation(&owner, &sha256);
    world.gh_acts(&Behavior::Verifies {
        repository: format!("https://github.com/{built_by}"),
    });
    let version = world.listed_version(&owner, "bar", "1.0.0", &sha256, BAR_LISTED_REQUIRES);
    let entry = EntryFile {
        name: "bar".into(),
        github: owner,
        versions: vec![version],
    };
    let path = world.checkout_file(HEAD_DIR, "packages/bar.toml");
    std::fs::write(path, entry.write()).expect("write the entry file");
}

#[given("the head adds packages/bar.toml that is not TOML")]
async fn the_head_adds_a_file_that_is_not_toml(world: &mut CliWorld) {
    let path = world.checkout_file(HEAD_DIR, "packages/bar.toml");
    std::fs::write(path, "name = [").expect("write the file");
}

#[given("the head adds packages/big.toml of 2 MiB")]
async fn the_head_adds_a_big_file(world: &mut CliWorld) {
    let path = world.checkout_file(HEAD_DIR, "packages/big.toml");
    std::fs::write(path, vec![b' '; 2 * 1024 * 1024]).expect("write the file");
}

/// Changes foo 1.0.0's sha256 in the head, and keeps what it was and what it is.
fn change_foo_sha256(world: &mut CliWorld) {
    let mut file = world.head_foo();
    let listed = &mut file.versions[0];
    let check = &mut world.community.check;
    check.old_sha256 = listed.sha256.clone();
    check.new_sha256 = sha256_hex(b"changed");
    listed.sha256 = check.new_sha256.clone();
    world.write_head_foo(&file);
}

#[given("the head changes foo 1.0.0's sha256 from <old> to <new>")]
async fn the_head_changes_the_hash(world: &mut CliWorld) {
    change_foo_sha256(world);
}

#[given("the head changes foo 1.0.0's sha256 and adds notes.md")]
async fn the_head_changes_the_hash_and_adds_notes(world: &mut CliWorld) {
    change_foo_sha256(world);
    let path = world.checkout_file(HEAD_DIR, "notes.md");
    std::fs::write(path, "notes").expect("write the file");
}

#[given("the head equals the base")]
async fn the_head_equals_the_base(_: &mut CliWorld) {}

#[then("the release file of foo 1.1.0 was downloaded without a token")]
async fn the_release_file_was_downloaded_without_a_token(world: &mut CliWorld) {
    let path = format!(
        "/{FOO_OWNER}/releases/download/1.1.0/{}",
        release_file("foo", "1.1.0")
    );
    let downloads: Vec<_> = (world.github_fake().requests().into_iter())
        .filter(|request| request.path == path)
        .collect();
    assert!(!downloads.is_empty(), "no request for {path}");
    for download in downloads {
        assert_eq!(download.authorization, None, "{download:?}");
    }
}

#[then("the output is the notes.md line, then the foo line, and the exit code is 1")]
async fn the_lines_are_sorted(world: &mut CliWorld) {
    let check = &world.community.check;
    let lines = [
        "refused notes.md: only .toml files directly under packages/ may change. fix: remove this change".to_string(),
        format!(
            "refused foo 1.0.0: already listed with sha256 {}, now {}. fix: listed versions cannot change; publish a new version",
            check.old_sha256, check.new_sha256
        ),
    ];
    let output = world.output();
    assert_eq!(output.exit_code, 1, "{}", world.printed());
    assert_eq!(output.stdout, "", "{}", world.printed());
    assert_eq!(output.stderr.lines().collect::<Vec<_>>(), lines);
}

// ---- the release template's register job (S8 slice 6) ----

const RELEASE_TEMPLATE: &str = ".github/workflow-templates/lvpm-release.yml";

/// The pinned summary lines of the `register` job, as the spec words them. A
/// `<placeholder>` is the shell variable of that name in the template.
const REGISTER_LINES: [&str; 5] = [
    "registered <name> <version>: <url>. The index bot checks it, then merges it or comments the reasons it refused. A PR with no verdict waits for a maintainer.",
    "register failed: the token was refused. fix: save a classic token with public_repo as LVPM_INDEX_TOKEN",
    "register failed: <name> <version> is already listed. fix: push a new tag",
    "register failed: <name> belongs to <owner>. fix: publish from <owner>, or register another name",
    "register failed: <step> failed. fix: see the job log, then re-run the job",
];

/// The lines of job `name` of the release template, after its `  name:` line.
fn template_job(name: &str) -> Vec<String> {
    let text = super::docs::repo_file(RELEASE_TEMPLATE);
    let lines: Vec<&str> = text.lines().collect();
    let start = (lines.iter())
        .position(|line| *line == format!("  {name}:"))
        .unwrap_or_else(|| panic!("{RELEASE_TEMPLATE} has no job {name}"));
    let is_job_header = |line: &&str| line.starts_with("  ") && !line.starts_with("   ");
    (lines[start + 1..].iter())
        .take_while(|line| !is_job_header(line) && !line.starts_with(|c: char| !c.is_whitespace()))
        .map(|line| line.to_string())
        .collect()
}

/// The steps of a job's lines, each as its lines. A step starts at a `      - `.
fn job_steps(job: &[String]) -> Vec<Vec<String>> {
    let first = job
        .iter()
        .position(|line| line == "    steps:")
        .unwrap_or_else(|| panic!("the job has no steps"));
    let mut steps: Vec<Vec<String>> = Vec::new();
    for line in &job[first + 1..] {
        if line.starts_with("      - ") {
            steps.push(Vec::new());
        }
        steps.last_mut().expect("a step").push(line.clone());
    }
    steps
}

/// A step's own `if:`, not one inside its script.
fn step_if(step: &[String]) -> Option<&str> {
    step.iter()
        .find_map(|line| line.strip_prefix("        if: "))
}

fn register_steps() -> Vec<Vec<String>> {
    job_steps(&template_job("register"))
}

fn the_step_has_if(step: &[String], condition: &str) {
    let wanted = condition.strip_prefix("if: ").expect("an if: condition");
    assert_eq!(
        step_if(step),
        Some(wanted),
        "the step:\n{}",
        step.join("\n")
    );
}

#[then(regex = r#"^the release template has a job "([^"]+)" that needs "([^"]+)"$"#)]
async fn the_template_has_a_job_that_needs(_: &mut CliWorld, job: String, needs: String) {
    assert!(
        template_job(&job).contains(&format!("    needs: {needs}")),
        "job {job} of {RELEASE_TEMPLATE} does not need {needs}"
    );
}

#[then(
    regex = r#"^"([^"]+)" sets LVPM_INDEX_TOKEN from the secret, and every step but the last two has "([^"]+)"$"#
)]
async fn the_job_sets_the_token_and_guards_its_steps(
    _: &mut CliWorld,
    job: String,
    condition: String,
) {
    let lines = template_job(&job);
    assert!(
        lines.contains(&"      LVPM_INDEX_TOKEN: ${{ secrets.LVPM_INDEX_TOKEN }}".to_string()),
        "job {job} does not set LVPM_INDEX_TOKEN from the secret"
    );
    let steps = job_steps(&lines);
    assert!(steps.len() > 2, "job {job} has only {} steps", steps.len());
    for step in &steps[..steps.len() - 2] {
        the_step_has_if(step, &condition);
    }
}

#[then(regex = r#"^the step writing "([^"]+)" has "([^"]+)"$"#)]
async fn the_failure_step_runs_on_failure(_: &mut CliWorld, text: String, condition: String) {
    let steps = register_steps();
    let writing: Vec<_> = (steps.iter())
        .enumerate()
        .filter(|(_, step)| step.iter().any(|line| line.contains(&text)))
        .collect();
    assert_eq!(writing.len(), 1, "steps writing {text}: {}", writing.len());
    let (at, step) = writing[0];
    assert_eq!(at, steps.len() - 2, "that step is the one before the last");
    the_step_has_if(step, &condition);
}

#[then(
    regex = r#"^the last step of "register" has "([^"]+)" and writes "([^"]+)" to GITHUB_STEP_SUMMARY$"#
)]
async fn the_last_step_says_it_did_not_register(_: &mut CliWorld, condition: String, line: String) {
    let steps = register_steps();
    let last = steps.last().expect("a last step");
    the_step_has_if(last, &condition);
    assert!(
        last.iter()
            .any(|l| l.contains(&format!("\"{line}\""))
                && l.ends_with(">> \"$GITHUB_STEP_SUMMARY\"")),
        "the last step does not write {line:?} to GITHUB_STEP_SUMMARY:\n{}",
        last.join("\n")
    );
}

#[then(
    regex = r#"^"register" runs "([^"]+)" and "([^"]+)" against LVPM_INDEX_REPO, defaulting to (\S+)$"#
)]
async fn the_job_registers_against_the_index_repo(
    _: &mut CliWorld,
    add: String,
    create: String,
    default: String,
) {
    let job = template_job("register");
    let has = |text: String| job.iter().any(|line| line.contains(&text));
    assert!(has(add.clone()), "register does not run {add}");
    assert!(
        has(format!("{create} --repo \"$INDEX_REPO\"")),
        "register does not run {create} against $INDEX_REPO"
    );
    assert!(
        has(format!(
            "INDEX_REPO: ${{{{ vars.LVPM_INDEX_REPO || '{default}' }}}}"
        )),
        "register's INDEX_REPO is not vars.LVPM_INDEX_REPO, defaulting to {default}"
    );
}

#[then(
    regex = r#"^"register" writes to GITHUB_STEP_SUMMARY the pinned success line and each pinned "register failed:" line$"#
)]
async fn the_job_writes_the_pinned_lines(_: &mut CliWorld) {
    let steps = register_steps();
    for pinned in REGISTER_LINES {
        let shell = (pinned
            .replace("<name>", "$name")
            .replace("<version>", "$version"))
        .replace("<url>", "$url")
        .replace("<owner>", "$owner")
        .replace("<step>", "$step");
        let step = (steps.iter())
            .find(|step| step.iter().any(|line| line.contains(&shell)))
            .unwrap_or_else(|| panic!("no step of register says {shell:?}"));
        assert!(
            step.iter().any(|line| line.contains("GITHUB_STEP_SUMMARY")),
            "the step that says {shell:?} does not write to GITHUB_STEP_SUMMARY"
        );
    }
}

#[then(regex = r#"^the jobs "([^"]+)" and "([^"]+)" do not mention LVPM_INDEX_TOKEN$"#)]
async fn the_other_jobs_do_not_mention_the_token(_: &mut CliWorld, a: String, b: String) {
    for job in [a, b] {
        let lines = template_job(&job);
        assert!(
            !lines.iter().any(|line| line.contains("LVPM_INDEX_TOKEN")),
            "job {job} mentions LVPM_INDEX_TOKEN"
        );
    }
}

#[then(regex = r#"^the template's header comment documents "([^"]+)"$"#)]
async fn the_header_comment_documents(_: &mut CliWorld, job: String) {
    let text = super::docs::repo_file(RELEASE_TEMPLATE);
    let header = text.split("\non:\n").next().expect("a header");
    assert!(
        header
            .lines()
            .all(|line| line.starts_with('#') || line.starts_with("name:") || line.is_empty()),
        "the text before `on:` is not all comments"
    );
    for word in [job.as_str(), "LVPM_INDEX_TOKEN"] {
        assert!(
            header.contains(word),
            "the header comment does not name {word}"
        );
    }
}

// ---- docs/usage.md on the community index (S8 slice 6) ----

/// The section of docs/usage.md whose heading holds `title`, with every run of
/// whitespace as one space, so a wrapped sentence reads as one line.
fn usage_section(title: &str) -> String {
    let section = super::docs::section_of(&super::docs::repo_file("docs/usage.md"), title);
    section.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The numbered items of `section`, each with its continuation lines.
fn numbered_items(section: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for line in section.lines() {
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && line[digits..].starts_with(". ") {
            items.push(line.to_string());
        } else if line.starts_with("   ")
            && let Some(item) = items.last_mut()
        {
            item.push(' ');
            item.push_str(line.trim());
        }
    }
    items
}

#[then(
    regex = r#"^docs/usage\.md has a section "([^"]+)" with four numbered steps naming lvpm\.toml, lvpm-release\.yml, LVPM_INDEX_TOKEN and a tag$"#
)]
async fn usage_has_the_four_steps(_: &mut CliWorld, title: String) {
    let section = super::docs::section_of(&super::docs::repo_file("docs/usage.md"), &title);
    let items = numbered_items(&section);
    assert_eq!(
        items.len(),
        4,
        "the {title} section's numbered steps: {items:?}"
    );
    for (item, name) in
        items
            .iter()
            .zip(["lvpm.toml", "lvpm-release.yml", "LVPM_INDEX_TOKEN", "tag"])
    {
        assert!(item.contains(name), "step {item:?} does not name {name}");
    }
}

#[then(
    regex = r#"^the section names the token type "([^"]+)" with "([^"]+)", and says who closes a refused PR$"#
)]
async fn the_section_names_the_token_and_who_closes(_: &mut CliWorld, kind: String, scope: String) {
    let section = usage_section("Publish to the community index");
    for wanted in [kind.as_str(), scope.as_str(), "The publisher closes it"] {
        assert!(
            section.contains(wanted),
            "the section does not say {wanted:?}"
        );
    }
}

#[then(regex = r#"^it shows (community = "[^"]+") and says source order matters$"#)]
async fn usage_shows_the_source_and_its_order(_: &mut CliWorld, line: String) {
    assert!(usage_section("Publish to the community index").contains(&line));
    let sources = usage_section("A github: source");
    for wanted in [line.as_str(), "Source order matters"] {
        assert!(
            sources.contains(wanted),
            "the github: source section does not say {wanted:?}"
        );
    }
}

#[then(regex = r#"^the command table lists "([^"]+)" and "([^"]+)"$"#)]
async fn the_command_table_lists_the_index_commands(_: &mut CliWorld, a: String, b: String) {
    let commands = super::docs::section_of(&super::docs::repo_file("docs/usage.md"), "Commands");
    for command in [a, b] {
        let rows = (commands.lines())
            .filter(|line| line.starts_with(&format!("| `{command} ")))
            .count();
        assert_eq!(rows, 1, "rows of the command table for {command}");
    }
}

#[then(regex = r#"^the section "([^"]+)" links to "([^"]+)"$"#)]
async fn the_section_links_to(_: &mut CliWorld, from: String, to: String) {
    let anchor = format!("](#{})", to.to_lowercase().replace(' ', "-"));
    assert!(
        usage_section(&from).contains(&anchor),
        "{from} has no link {anchor}"
    );
}
