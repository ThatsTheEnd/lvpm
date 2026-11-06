//! Steps of the OCI referrers scenarios (S7b, slice 4): the package's SBOM
//! attached to its OCI manifest by `lvpm publish`, found through the
//! registry's referrers API or its fallback index. The registry and the
//! publish run are the publish steps'.

use crate::CliWorld;
use cucumber::{given, then, when};
use lvpm_core::lvpkg::{self, Meta};
use lvpm_core::oci::{Access, Oci, Repo, SBOM_TYPE};
use std::io::{Cursor, Read};

/// What a Sigstore bundle's referrer is of; the fallback scenario only needs
/// something that is already listed.
const BUNDLE_TYPE: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";
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
