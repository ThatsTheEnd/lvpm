//! Steps of the community-index scenarios (S8): the index repo the GitHub
//! fake serves, the release its entry points at, who attested that release,
//! the projects that read the index, and the entry files `lvpm index add`
//! writes.

use super::github::{
    PILOT_B, PILOT_B_DEPENDENCY_FILES, PILOT_B_FILE, PILOT_B_NEEDS, PILOT_B_TAG, PROJECT_LABVIEW,
};
use super::lock::sha256_hex;
use crate::CliWorld;
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
        other_bytes: None,
        packages_before: Vec::new(),
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
