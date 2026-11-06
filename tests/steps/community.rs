//! Steps of the community-index install scenarios (S8): the index repo the
//! GitHub fake serves, the release its entry points at, who attested that
//! release, and the projects that read the index.

use super::github::{
    PILOT_B, PILOT_B_DEPENDENCY_FILES, PILOT_B_FILE, PILOT_B_NEEDS, PILOT_B_TAG, PROJECT_LABVIEW,
};
use super::lock::sha256_hex;
use crate::CliWorld;
use crate::support::pilots::{check_checksums, pilots_dir};
use crate::support::project::MANIFEST_FILE;
use crate::support::sandbox::LOCAL_PACKAGES_DIR;
use cucumber::{given, then};
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
