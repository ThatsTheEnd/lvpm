//! Steps of the `@ghcr` scenarios (S7b, slice 5): `lvpm` against the real
//! ghcr.io and the test repo `ThatsTheEnd/lvpm-oci-test`, which the user sets
//! up (`docs/oci-test-repo-workflow.yml`). They run only with
//! `LVPM_TEST_GHCR=1`. The project, the lock and the publish run are the OCI
//! install and publish steps'; there is no fake registry here.
//!
//! "The Docker config of the test runner" is the runner's own: `DOCKER_CONFIG`,
//! else `~/.docker`. The sandbox otherwise points every child at an empty one.

use super::oci_install::TEAM;
use crate::CliWorld;
use crate::support::pilots::copy_tree;
use crate::support::selection::GHCR_ENV_VAR;
use cucumber::{given, then};
use lvpm_core::lvpkg;
use lvpm_core::oci::{Access, Manifest, Oci, Repo, SBOM_TYPE};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The registry, and where in it the test repo's packages are.
const GHCR: &str = "ghcr.io";
const PREFIX: &str = "thatstheend/lvpm-oci-test";
/// The `[sources]` name of the test repo.
const SOURCE: &str = TEAM;
/// The repository whose workflow publishes and attests the fixture.
const BUILT_BY: &str = "https://github.com/ThatsTheEnd/lvpm-oci-test";
/// Where the fixture package is, below the package root.
const FIXTURE: &str = "tests/fixtures/ghcr-alpha";

/// The runner's Docker config directory, which must hold a `config.json`.
fn runner_docker_config() -> PathBuf {
    let dir = match std::env::var_os("DOCKER_CONFIG") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
            PathBuf::from(home.expect("a home directory")).join(".docker")
        }
    };
    assert!(
        dir.join("config.json").is_file(),
        "the runner has no Docker config at {}: run docker login ghcr.io",
        dir.display()
    );
    dir
}

/// The runner's GitHub token, for the `gh` that checks signatures: `GH_TOKEN`,
/// else `GITHUB_TOKEN`, else what `gh auth token` prints. Never shown.
fn runner_gh_token() -> Option<String> {
    let set = |name| std::env::var(name).ok().filter(|token| !token.is_empty());
    set("GH_TOKEN").or_else(|| set("GITHUB_TOKEN")).or_else(|| {
        let out = Command::new("gh").args(["auth", "token"]).output().ok()?;
        let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (out.status.success() && !token.is_empty()).then_some(token)
    })
}

/// The OCI manifest of the public `alpha` at `tag`, read anonymously.
fn public_manifest(tag: &str) -> Manifest {
    let repository = format!("{PREFIX}/alpha");
    let repo = Repo {
        registry: GHCR,
        name: &repository,
        access: Access::Pull,
    };
    Oci::default()
        .get_manifest(&repo, tag)
        .expect("alpha on ghcr.io")
}

#[given("the children reach the real ghcr.io")]
async fn reach_ghcr(world: &mut CliWorld) {
    // The harness leaves these scenarios out without the flag; this is the
    // second lock on a door that opens onto the real registry.
    assert_eq!(
        std::env::var(GHCR_ENV_VAR).as_deref(),
        Ok("1"),
        "a @ghcr scenario ran without {GHCR_ENV_VAR}=1"
    );
    // The sandbox sends every request to its own feed unless told not to.
    for name in ["NO_PROXY", "no_proxy"] {
        world.sandbox.set_child_env(name, "*");
    }
    if let Some(token) = runner_gh_token() {
        world.sandbox.set_child_env("GH_TOKEN", &token);
    }
}

#[given(regex = r#"^a project pinned to (\w+) from "(oci://ghcr\.io/[^"]+)"$"#)]
async fn project_pinned(world: &mut CliWorld, name: String, url: String) {
    world.sandbox.make_empty_dest();
    world.set_source(SOURCE, &url);
    world.oci_depend_on(&name, "*", Some(SOURCE));
}

#[given("the Docker config of the test runner")]
async fn runner_config(world: &mut CliWorld) {
    let dir = runner_docker_config();
    world
        .sandbox
        .set_child_env("DOCKER_CONFIG", &dir.to_string_lossy());
}

#[given(regex = r#"^the fixture package alpha 1\.0\.0 from tests/fixtures$"#)]
async fn fixture_package(world: &mut CliWorld) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    copy_tree(&source, &world.sandbox.work_dir());
    let built = world.sandbox.run_lvpm(&["build", "--no-check"], None);
    assert_eq!(built.exit_code, 0, "lvpm build failed: {built:?}");
}

#[then(regex = r#"^(\w+) is installed and the lock records its digest and signer$"#)]
async fn installed_with_digest_and_signer(world: &mut CliWorld, name: String) {
    assert!(world.is_installed(&name), "{}", world.printed());
    let digest = world.lock_text("digest", &name);
    let version = world.lock_text("version", &name);
    let held = public_manifest(&version);
    assert_eq!(digest, held.digest, "the lock's digest is not the tag's");
    let signer = world.lock_text("signer", &name);
    assert!(
        signer.starts_with(&format!("{BUILT_BY}/")),
        "{name}'s signer is {signer}, not a workflow of {BUILT_BY}"
    );
    assert_eq!(world.lock_text("subject", &name), BUILT_BY);
}

#[then(regex = r#"^alpha ([0-9.]+) has an SBOM referrer on the registry$"#)]
async fn has_sbom_referrer(_: &mut CliWorld, version: String) {
    let subject = public_manifest(&version);
    let repository = format!("{PREFIX}/alpha");
    let repo = Repo {
        registry: GHCR,
        name: &repository,
        access: Access::Pull,
    };
    let found = Oci::default()
        .referrers(&repo, &subject.digest, SBOM_TYPE)
        .expect("the referrers of alpha");
    assert_eq!(
        found.len(),
        1,
        "SBOM referrers of alpha {version}: {found:?}"
    );
}

#[then("that referrer holds the bom.cdx.json of the published package")]
async fn referrer_holds_the_sbom(_: &mut CliWorld) {
    let manifest = public_manifest("1.0.0");
    let package = manifest.package().expect("alpha 1.0.0 is a package");
    let repository = format!("{PREFIX}/alpha");
    let repo = Repo {
        registry: GHCR,
        name: &repository,
        access: Access::Pull,
    };
    let oci = Oci::default();
    let referrers = oci
        .referrers(&repo, &manifest.digest, SBOM_TYPE)
        .expect("the referrers of alpha");
    let referrer = oci
        .get_manifest(&repo, &referrers[0].digest)
        .expect("the referrer's manifest");
    let referrer: serde_json::Value = serde_json::from_slice(&referrer.bytes).expect("JSON");
    let layer = referrer["layers"][0]["digest"].as_str().expect("a layer");
    let held = oci.get_blob(&repo, layer).expect("the referrer's layer");
    let archive = oci.get_blob(&repo, &package.layer).expect("the package");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).expect("a zip");
    let mut sbom = Vec::new();
    let mut entry = zip.by_name(lvpkg::SBOM).expect("an SBOM in the package");
    std::io::Read::read_to_end(&mut entry, &mut sbom).expect("read the SBOM");
    assert!(
        held == sbom,
        "the referrer's layer is not the package's SBOM"
    );
}

#[then("internal is installed and locked")]
async fn installed_and_locked(world: &mut CliWorld) {
    let output = world.output();
    assert_eq!(output.exit_code, 0, "{}", world.printed());
    assert!(world.is_installed("internal"), "{}", world.printed());
    assert!(!world.lock_text("digest", "internal").is_empty());
}

#[given(regex = r#"^the digest of the tag alpha:([0-9.]+) is noted$"#)]
async fn note_digest(world: &mut CliWorld, tag: String) {
    world.publish.noted_digest = Some(public_manifest(&tag).digest);
}

#[then(regex = r#"^the tag alpha:([0-9.]+) still has the noted digest$"#)]
async fn digest_unchanged(world: &mut CliWorld, tag: String) {
    let noted = world.publish.noted_digest.clone().expect("a noted digest");
    let now = public_manifest(&tag).digest;
    assert_eq!(
        now, noted,
        "THE TAG alpha:{tag} MOVED: a republish was not refused"
    );
}
