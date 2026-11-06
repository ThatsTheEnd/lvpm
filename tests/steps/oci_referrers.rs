//! Steps of the OCI referrers scenarios (S7b, slice 4): the package's SBOM
//! attached to its OCI manifest by `lvpm publish`, found through the
//! registry's referrers API or its fallback index, and the Sigstore bundle of
//! a package that `lvpm lock` checks under `[policy]`. The registry and the
//! publish run are the publish steps'; the consumer project is the OCI install
//! steps'; `gh` is the testkit's fake program, which records what it was given.

use super::oci_install::{OLD_ENOUGH, TEAM, now};
use crate::CliWorld;
use cucumber::{given, then, when};
use lvpm_core::lvpkg::{self, Meta};
use lvpm_core::oci::{Access, BUNDLE_TYPE, Oci, Repo, SBOM_TYPE};
use lvpm_testkit::gh::{self, Behavior};
use std::io::{Cursor, Read};

/// The repository a scenario's bundle says built the package, unless it names one.
const BUILT_BY: &str = "https://github.com/o/r";
/// The LabVIEW the packaged projects need.
const LABVIEW: u32 = 21;

impl CliWorld {
    /// The digest `repository:tag` points at on the registry.
    fn tag_of(&self, repository: &str, tag: &str) -> String {
        (self.registry().tag_digest(repository, tag))
            .unwrap_or_else(|| panic!("the registry holds no {repository}:{tag}"))
    }

    /// The package's `bom.cdx.json`, when it has one.
    fn package_sbom(&self) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.publish_package()).expect("the package");
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip");
        let mut entry = zip.by_name(lvpkg::SBOM).ok()?;
        let mut sbom = Vec::new();
        entry.read_to_end(&mut sbom).expect("read the SBOM");
        Some(sbom)
    }
}

/// `repository:tag` as a feature writes it.
fn split_reference(reference: &str) -> (&str, &str) {
    reference.rsplit_once(':').expect("repository:tag")
}

#[given(regex = r#"^the registry (supports|does not offer) the referrers API$"#)]
async fn referrers_api(world: &mut CliWorld, api: String) {
    world.registry().offer_referrers_api(api == "supports");
}

#[given(regex = r#"^a project "([^"]+)" ([0-9.]+) that lvpm build has packaged with its SBOM$"#)]
async fn packaged_with_sbom(world: &mut CliWorld, name: String, version: String) {
    super::publish::package_project(world, name, version, LABVIEW, None).await;
    assert!(world.package_sbom().is_some(), "the build wrote no SBOM");
}

/// `lvpm build` always writes the SBOM, so the package is written again from
/// the same metadata without it, as a package from before the SBOM is.
#[given(regex = r#"^a project "([^"]+)" ([0-9.]+) that lvpm build has packaged without an SBOM$"#)]
async fn packaged_without_sbom(world: &mut CliWorld, name: String, version: String) {
    super::publish::package_project(world, name.clone(), version.clone(), LABVIEW, None).await;
    let meta = Meta {
        name,
        version,
        labview: format!(">={}", 2000 + LABVIEW),
        vendor: "Vendor".into(),
        display_name: "Alpha Lib".into(),
        install_dir: "vi.lib/Vendor/Lib".into(),
        dependencies: Vec::new(),
    };
    let files = [("A.vi".to_string(), b"a vi".to_vec())];
    let bytes = lvpkg::write(&meta, &files, None).expect("a package");
    std::fs::write(world.publish_package(), bytes).expect("replace the built package");
    assert!(world.package_sbom().is_none());
}

#[given(regex = r#"^the registry refuses referrer writes to "([^"]+)"$"#)]
async fn refuses_referrer_writes(world: &mut CliWorld, repository: String) {
    world.registry().refuse_referrer_writes(&repository);
}

/// The package is put in the registry as `lvpm publish` would, and the bundle
/// listed as attached to it by a client of a registry without the API.
#[given(regex = r#"^"([^"]+)" already has a Sigstore bundle referrer in its fallback index$"#)]
async fn has_bundle_in_fallback(world: &mut CliWorld, reference: String) {
    let (repository, tag) = split_reference(&reference);
    let seeded = world
        .registry()
        .seed_package(repository, tag, &[], b"the package");
    world
        .registry()
        .seed_referrer(repository, &seeded.manifest, BUNDLE_TYPE, b"a bundle");
    world.publish.repository = repository.to_string();
}

#[when(regex = r#"^lvpm attaches the SBOM of ([a-z]+) ([0-9.]+)$"#)]
async fn attaches_sbom(world: &mut CliWorld, _name: String, version: String) {
    let repository = world.publish.repository.clone();
    let registry = world.registry().host();
    let repo = Repo {
        registry: &registry,
        name: &repository,
        access: Access::Push,
    };
    let oci = Oci::default();
    let subject = oci.get_manifest(&repo, &version).expect("the package");
    oci.push_referrer(&repo, &subject, SBOM_TYPE, b"{\"bomFormat\":\"CycloneDX\"}")
        .expect("attach the SBOM");
}

#[then(regex = r#"^"([^"]+)" has a referrer of artifactType "([^"]+)"$"#)]
async fn has_referrer(world: &mut CliWorld, reference: String, artifact_type: String) {
    let (repository, tag) = split_reference(&reference);
    let subject = world.tag_of(repository, tag);
    let found: Vec<_> = (world.registry().referrers(repository, &subject).into_iter())
        .filter(|(_, found)| *found == artifact_type)
        .collect();
    assert_eq!(found.len(), 1, "referrers of {reference}: {found:?}");
    let manifest = world
        .registry()
        .manifest(repository, &found[0].0)
        .expect("the referrer's manifest");
    world.publish.manifest = Some(serde_json::from_slice(&manifest).expect("a JSON manifest"));
    world.publish.repository = repository.to_string();
}

#[then("its bytes equal the package's bom.cdx.json")]
async fn bytes_equal_sbom(world: &mut CliWorld) {
    let manifest = world
        .publish
        .manifest
        .clone()
        .expect("a referrer was found");
    let layers = manifest["layers"].as_array().expect("layers");
    assert_eq!(layers.len(), 1, "{layers:?}");
    let digest = layers[0]["digest"].as_str().expect("a digest");
    let held = world.registry().blob(&world.publish.repository, digest);
    let sbom = world.package_sbom().expect("the package has an SBOM");
    assert!(
        held.as_deref() == Some(sbom.as_slice()),
        "the referrer's layer differs from the package's SBOM"
    );
    assert_eq!(layers[0]["size"], sbom.len());
}

#[then(regex = r#"^"([^"]+)" has no referrer$"#)]
async fn has_no_referrer(world: &mut CliWorld, reference: String) {
    let (repository, tag) = split_reference(&reference);
    let subject = world.tag_of(repository, tag);
    let found = world.registry().referrers(repository, &subject);
    assert!(found.is_empty(), "referrers of {reference}: {found:?}");
}

#[then(regex = r#"^the fallback index of "([^"]+)" lists both the bundle and the SBOM$"#)]
async fn fallback_lists_both(world: &mut CliWorld, reference: String) {
    let (repository, tag) = split_reference(&reference);
    let subject = world.tag_of(repository, tag);
    let mut types: Vec<String> = (world.registry().referrers(repository, &subject).into_iter())
        .map(|(_, artifact_type)| artifact_type)
        .collect();
    types.sort();
    assert_eq!(types, [SBOM_TYPE, BUNDLE_TYPE]);
}

// ---- the Sigstore bundle of a package ----

impl CliWorld {
    /// The digest of the OCI manifest of `reference`, once the package is in
    /// the registry: put there as `lvpm publish` would if the scenario has not.
    fn ensure_package(&mut self, reference: &str) -> String {
        let (repository, tag) = split_reference(reference);
        if self.registry().tag_digest(repository, tag).is_none() {
            let name = repository.strip_prefix("team/").expect("a package of team");
            self.seed(name, tag, now(), OLD_ENOUGH, "published", &[]);
        }
        self.tag_of(repository, tag)
    }

    /// The package of `reference` with a bundle whose certificate says
    /// `built_by`, and a gh that does `gh`.
    fn sign(&mut self, reference: &str, built_by: Option<&str>, gh: Behavior) {
        let subject = self.ensure_package(reference);
        let (repository, _) = split_reference(reference);
        let bundle = gh::bundle(built_by);
        self.registry()
            .seed_referrer(repository, &subject, BUNDLE_TYPE, &bundle);
        self.sandbox.use_fake_gh();
        gh::install(&self.sandbox.gh_dir(), &gh).expect("install the fake gh");
    }

    /// The SHA-256 of the one bundle of `reference`, as its layer holds it.
    fn bundle_sha256(&self, reference: &str) -> String {
        let (repository, tag) = split_reference(reference);
        let referrers = self
            .registry()
            .referrers(repository, &self.tag_of(repository, tag));
        let bundles: Vec<_> = (referrers.iter())
            .filter(|(_, artifact_type)| artifact_type == BUNDLE_TYPE)
            .collect();
        assert_eq!(bundles.len(), 1, "bundles of {reference}: {referrers:?}");
        let manifest = (self.registry().manifest(repository, &bundles[0].0)).expect("a referrer");
        let manifest: serde_json::Value = serde_json::from_slice(&manifest).expect("JSON");
        let layer = manifest["layers"][0]["digest"].as_str().expect("a layer");
        layer.trim_start_matches("sha256:").to_string()
    }
}

#[given(regex = r#"^"([^"]+)" is in the registry$"#)]
#[given(regex = r#"^"([^"]+)" has no Sigstore bundle referrer$"#)]
async fn package_without_bundle(world: &mut CliWorld, reference: String) {
    world.ensure_package(&reference);
    // A gh that would verify, so that "gh was not run" can fail.
    world.sandbox.use_fake_gh();
    let verifies = Behavior::Verifies {
        repository: BUILT_BY.into(),
    };
    gh::install(&world.sandbox.gh_dir(), &verifies).expect("install the fake gh");
}

#[given(regex = r#"^"([^"]+)" has a Sigstore bundle referrer$"#)]
async fn package_with_bundle(world: &mut CliWorld, reference: String) {
    let verifies = Behavior::Verifies {
        repository: BUILT_BY.into(),
    };
    world.sign(&reference, Some(BUILT_BY), verifies);
}

#[given(
    regex = r#"^"([^"]+)" has a Sigstore bundle referrer that gh verifies as built by "([^"]+)"$"#
)]
async fn package_verified_as(world: &mut CliWorld, reference: String, built_by: String) {
    let verifies = Behavior::Verifies {
        repository: built_by.clone(),
    };
    world.sign(&reference, Some(&built_by), verifies);
}

#[given(regex = r#"^"([^"]+)" has a Sigstore bundle referrer that gh rejects$"#)]
async fn package_rejected(world: &mut CliWorld, reference: String) {
    let rejects = Behavior::Rejects {
        stderr: "verifying with issuer \"sigstore.dev\"".into(),
    };
    world.sign(&reference, Some(BUILT_BY), rejects);
}

#[given(
    regex = r#"^"([^"]+)" has a Sigstore bundle referrer whose certificate names no source repository$"#
)]
async fn package_without_a_repository(world: &mut CliWorld, reference: String) {
    // gh would verify it, so that "gh was not run" is lvpm's doing.
    let verifies = Behavior::Verifies {
        repository: BUILT_BY.into(),
    };
    world.sign(&reference, None, verifies);
}

#[given("gh is not installed")]
async fn gh_not_installed(world: &mut CliWorld) {
    world.sandbox.use_fake_gh();
    std::fs::remove_dir_all(world.sandbox.gh_dir()).expect("remove the fake gh");
    world.sandbox.use_fake_gh();
}

#[given(regex = r#"^the registry answers (\d+) to the referrers lookup of "([^"]+)"$"#)]
async fn referrers_lookup_answers(world: &mut CliWorld, status: u16, reference: String) {
    let (repository, tag) = split_reference(&reference);
    let subject = world.tag_of(repository, tag);
    let path = format!("/v2/{repository}/referrers/{subject}");
    world.registry().answer("GET", &path, status);
}

#[given(regex = r#"^a project pinned to (\w+) from team$"#)]
async fn project_pinned(world: &mut CliWorld, name: String) {
    world.sandbox.make_empty_dest();
    world.sandbox.use_fake_gh();
    world.set_source(TEAM, "oci://<registry>/team");
    world.oci_depend_on(&name, "*", Some(TEAM));
}

#[given(
    regex = r#"^the project's \[policy\] (has require-signature = true|trusts only "([^"]+)"|is empty)$"#
)]
async fn project_policy(world: &mut CliWorld, policy: String, trusted: String) {
    world.oci.policy = match (policy.as_str(), trusted.as_str()) {
        ("is empty", _) => String::new(),
        (_, "") => "\n[policy]\nrequire-signature = true\n".to_string(),
        (_, pattern) => format!("\n[policy]\ntrusted-publishers = [\"{pattern}\"]\n"),
    };
    world.write_consumer();
}

#[given(
    regex = r#"^a project pinned to (\w+) from team, locked under require-signature with (\w+)'s signer$"#
)]
async fn project_locked_signed(world: &mut CliWorld, name: String, _signed: String) {
    project_pinned(world, name.clone()).await;
    project_policy(world, "has require-signature = true".into(), String::new()).await;
    world.sandbox.setup_lvpm(&["lock"]);
    let locked = world.locked_package(&name);
    assert!(
        locked.contains_key("signer"),
        "{name} has no signer: {locked:?}"
    );
    world.lock.before = world.lock_bytes();
    gh::clear_runs(&world.sandbox.gh_dir());
}

#[then(regex = r#"^the lock records (\w+)'s signer and subject "([^"]+)"$"#)]
async fn lock_has_signer_and_subject(world: &mut CliWorld, name: String, subject: String) {
    let signer = world.lock_text("signer", &name);
    assert!(signer.starts_with(&format!("{subject}/")), "{signer}");
    assert_eq!(world.lock_text("subject", &name), subject);
}

#[then(regex = r#"^the lock records (\w+)'s subject "([^"]+)"$"#)]
async fn lock_has_subject(world: &mut CliWorld, name: String, subject: String) {
    let said = world.printed();
    assert_eq!(world.lock_text("subject", &name), subject, "{said}");
}

#[then(regex = r#"^the lock records (\w+) with no signer$"#)]
async fn lock_has_no_signer(world: &mut CliWorld, name: String) {
    let package = world.locked_package(&name);
    assert!(!package.contains_key("signer"), "{package:?}");
    assert!(!package.contains_key("subject"), "{package:?}");
}

#[then(
    regex = r#"^gh was asked to verify a file whose sha256 is the OCI manifest digest of "([^"]+)"$"#
)]
async fn gh_verified_the_manifest(world: &mut CliWorld, reference: String) {
    let (repository, tag) = split_reference(&reference);
    let digest = world.tag_of(repository, tag);
    let runs = gh::runs(&world.sandbox.gh_dir());
    assert_eq!(runs.len(), 1, "{runs:?}");
    let sha256 = digest.trim_start_matches("sha256:");
    assert_eq!(runs[0].artifact_sha256.as_deref(), Some(sha256), "{runs:?}");
    assert!(runs[0].token_given, "{runs:?}");
}

#[then(regex = r#"^gh was given the bundle of "([^"]+)" byte for byte$"#)]
async fn gh_got_the_bundle(world: &mut CliWorld, reference: String) {
    let runs = gh::runs(&world.sandbox.gh_dir());
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].bundle_sha256, Some(world.bundle_sha256(&reference)));
}

#[then("gh was not run")]
async fn gh_not_run(world: &mut CliWorld) {
    let runs = gh::runs(&world.sandbox.gh_dir());
    assert!(runs.is_empty(), "{runs:?}");
}

/// The last command failed with `text` in what it printed.
fn assert_failed_saying(world: &CliWorld, text: &str) {
    let message = world.failure();
    assert!(message.contains(text), "{text:?} is not in: {message}");
}

#[then(regex = r#"^it fails saying (\w+) has no attestation$"#)]
async fn fails_no_attestation(world: &mut CliWorld, name: String) {
    let said = format!("{name} 1.0.0 from oci://<registry>/team has no attestation");
    assert_failed_saying(world, &said);
}

#[then(regex = r#"^it fails saying (\w+)'s attestation does not verify$"#)]
async fn fails_does_not_verify(world: &mut CliWorld, name: String) {
    let said =
        format!("the attestation of {name} 1.0.0 from oci://<registry>/team does not verify:");
    assert_failed_saying(world, &said);
}

#[then(regex = r#"^it fails naming the subject "([^"]+)"$"#)]
async fn fails_naming_subject(world: &mut CliWorld, subject: String) {
    assert_failed_saying(world, &format!("attested by repository {subject}"));
}
