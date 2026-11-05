//! Steps of the `lvpm publish` scenarios: a folder feed in the work
//! directory, what `lvpm publish` put there, and a second project that locks
//! from it. The project and its build are the `lvpm build` steps'.

use crate::CliWorld;
use crate::support::golden::Output;
use crate::support::project::MANIFEST_FILE;
use cucumber::{given, then, when};
use lvpm_testkit::oci::Registry;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The project that depends on the published package.
const CONSUMER: &str = "second";

/// What a publish scenario has set up and seen.
#[derive(Debug, Default)]
pub struct PublishState {
    output: Option<Output>,
    /// What lvpm lock printed in the second project.
    lock: Option<Output>,
    /// Publish with no lvpm.toml in the work directory.
    no_manifest: bool,
    /// A file in the feed (or the `--to` target) as it was before publishing;
    /// `None` inside when nothing was there.
    earlier: Option<Option<Vec<u8>>>,
    /// The scenario's loopback OCI registry.
    registry: Option<Registry>,
    /// The package `lvpm build` wrote, and the repository the scenario last
    /// looked at on the registry, with its manifest.
    package: Option<PathBuf>,
    repository: String,
    manifest: Option<serde_json::Value>,
}

impl CliWorld {
    /// `rel`, a `/`-separated path below the work directory.
    fn below_work(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.sandbox.work_dir(), |p, part| p.join(part))
    }

    fn lvpm_publish(&mut self, args: &[&str], from: &Path) {
        match self.publish.no_manifest {
            true => drop(std::fs::remove_file(self.work(MANIFEST_FILE))),
            false => self.write_build_manifest(),
        }
        let mut all = vec!["publish"];
        all.extend_from_slice(args);
        self.publish.output = Some(self.sandbox.run_lvpm_in(from, &all, None));
    }

    /// What `lvpm publish` printed: the folder steps keep it; "I run" keeps the rest.
    fn published(&self) -> &Output {
        (self.publish.output.as_ref().or(self.last_output.as_ref())).expect("lvpm publish ran")
    }

    fn publish_failed(&self) -> String {
        let out = self.published();
        assert_ne!(out.exit_code, 0, "lvpm publish succeeded: {out:?}");
        out.stderr.clone()
    }

    fn names_in(&self, rel: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.below_work(rel))
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn remember(&mut self, rel: &str) {
        self.publish.earlier = Some(std::fs::read(self.below_work(rel)).ok());
    }

    fn build_for(&mut self, version: Option<String>) {
        if let Some(v) = version {
            self.set(&["project"], "version", v);
        }
        self.lvpm_build_unchecked();
        self.succeeded();
    }

    /// The message names the file at `rel`: as the work directory spells it,
    /// or as the file system does. The manifest may be named bare.
    pub(super) fn message_names(&self, message: &str, rel: &str) -> bool {
        let path = self.below_work(rel);
        let mut spellings = vec![path.display().to_string()];
        let real = match path.canonicalize() {
            Ok(real) => Some(real),
            Err(_) => path
                .parent()
                .and_then(|p| p.canonicalize().ok())
                .map(|p| p.join(path.file_name().expect("a file name"))),
        };
        if let Some(real) = real {
            let real = real.display().to_string();
            spellings.push(real.strip_prefix(r"\\?\").unwrap_or(&real).to_string());
        }
        if rel == MANIFEST_FILE {
            spellings.push(rel.to_string());
        }
        spellings.iter().any(|s| message.contains(s.as_str()))
    }
}

// ---- setup ----

#[given(regex = r#"^an empty folder "([^"]+)"$"#)]
async fn empty_folder(world: &mut CliWorld, rel: String) {
    std::fs::create_dir_all(world.below_work(&rel)).expect("create the folder");
}

#[given("lvpm build has run")]
async fn build_has_run(world: &mut CliWorld) {
    world.build_for(None);
}

#[given(regex = r#"^lvpm build has run for version "([^"]+)"$"#)]
async fn build_has_run_for(world: &mut CliWorld, version: String) {
    world.build_for(Some(version));
}

#[given(regex = r#"^"A\.vi" is changed after the build$"#)]
async fn changed_after_build(world: &mut CliWorld) {
    let built = world.names_in("dist").pop().expect("a built package");
    world.remember(&format!("dist/{built}"));
    world.put("A.vi", "changed after the build");
}

#[given(regex = r#"^the version is changed to "([^"]+)" without a build$"#)]
async fn version_changed(world: &mut CliWorld, version: String) {
    world.set(&["project"], "version", version);
}

#[given(regex = r#"^"([^"]+)" already holds "([^"]+)" with (.+)$"#)]
async fn already_holds(world: &mut CliWorld, feed: String, name: String, content: String) {
    let bytes = match content.as_str() {
        "the same bytes as the build" => {
            std::fs::read(world.work("dist").join(&name)).expect("the build")
        }
        "the text \"older package\"" => b"older package".to_vec(),
        other => panic!("no such content: {other}"),
    };
    let rel = format!("{feed}/{name}");
    std::fs::write(world.below_work(&rel), bytes).expect("write the file");
    world.remember(&rel);
}

#[given(regex = r#"^lvpm publish --to "([^"]+)" has run$"#)]
async fn publish_has_run(world: &mut CliWorld, to: String) {
    let work = world.sandbox.work_dir();
    world.lvpm_publish(&["--to", &to], &work);
    let out = world.published();
    assert_eq!(out.exit_code, 0, "lvpm publish failed: {out:?}");
    let name = world.names_in(&to).pop().expect("a published file");
    world.remember(&format!("{to}/{name}"));
}

#[given(regex = r#"^nothing named "([^"]+)" exists$"#)]
async fn nothing_named(world: &mut CliWorld, rel: String) {
    assert!(!world.below_work(&rel).exists(), "{rel} exists");
    world.remember(&rel);
}

#[given(regex = r#"^a file "([^"]+)" holding "([^"]+)"$"#)]
async fn a_file(world: &mut CliWorld, rel: String, text: String) {
    world.put(&rel, &text);
    world.remember(&rel);
}

#[given("the work directory has no lvpm.toml")]
async fn no_manifest(world: &mut CliWorld) {
    world.publish.no_manifest = true;
}

#[given(regex = r#"^dist holds "([^"]+)" and "([^"]+)"$"#)]
async fn dist_holds_two(world: &mut CliWorld, a: String, b: String) {
    for name in [a, b] {
        assert!(
            world.work("dist").join(&name).is_file(),
            "no {name} in dist"
        );
    }
}

#[given(
    regex = r#"^a second project depending on "([^"]+)" "([^"]+)", with "([^"]+)" as its only local source$"#
)]
async fn second_project(world: &mut CliWorld, name: String, constraint: String, feed: String) {
    let dir = world.sandbox.root().join(CONSUMER);
    std::fs::create_dir_all(&dir).expect("create the second project");
    let mut manifest = toml::Table::new();
    let mut project = toml::Table::new();
    project.insert("name".into(), "consumer".into());
    project.insert("version".into(), "0.1.0".into());
    project.insert("labview".into(), ">=2016".into());
    manifest.insert("project".into(), project.into());
    let mut deps = toml::Table::new();
    deps.insert(name, constraint.into());
    manifest.insert("dependencies".into(), deps.into());
    std::fs::write(
        dir.join(MANIFEST_FILE),
        toml::to_string(&manifest).expect("a manifest"),
    )
    .expect("write the manifest");
    let feed = world.below_work(&feed);
    let rel = feed
        .strip_prefix(world.sandbox.root())
        .expect("the feed is in the sandbox");
    world
        .sandbox
        .add_local_source_folder(&rel.to_string_lossy());
}

// ---- running ----

#[when(regex = r#"^lvpm publish --to "([^"]+)" runs$"#)]
async fn publish_runs(world: &mut CliWorld, to: String) {
    let work = world.sandbox.work_dir();
    world.lvpm_publish(&["--to", &to], &work);
}

#[when("lvpm publish runs with no --to")]
async fn publish_without_to(world: &mut CliWorld) {
    let work = world.sandbox.work_dir();
    world.lvpm_publish(&[], &work);
}

#[when(regex = r#"^lvpm publish --project <the project> --to "([^"]+)" runs from "([^"]+)"$"#)]
async fn publish_from(world: &mut CliWorld, to: String, from: String) {
    let (work, from) = (world.sandbox.work_dir(), world.below_work(&from));
    let project = work.display().to_string();
    world.lvpm_publish(&["--project", &project, "--to", &to], &from);
}

#[when("lvpm lock runs in the second project")]
async fn lock_in_second(world: &mut CliWorld) {
    let dir = world.sandbox.root().join(CONSUMER);
    world.publish.lock = Some(world.sandbox.run_lvpm_in(&dir, &["lock"], None));
}

// ---- outcomes ----

#[then(regex = r#"^it succeeds and prints the path of "([^"]+)"$"#)]
async fn prints_path(world: &mut CliWorld, rel: String) {
    let out = world.published();
    assert_eq!(out.exit_code, 0, "lvpm publish failed: {out:?}");
    assert!(out.stdout.starts_with("published "), "{out:?}");
    assert!(
        world.message_names(&out.stdout, &rel),
        "{out:?} does not name {rel}"
    );
}

#[then(regex = r#"^"([^"]+)" holds exactly "([^"]+)", byte-identical to "([^"]+)"$"#)]
async fn holds_identical(world: &mut CliWorld, feed: String, name: String, original: String) {
    assert_eq!(
        world.names_in(&feed),
        std::slice::from_ref(&name),
        "what {feed} holds"
    );
    let copy = std::fs::read(world.below_work(&format!("{feed}/{name}"))).expect("the copy");
    let original = std::fs::read(world.below_work(&original)).expect("the original");
    assert!(copy == original, "the copy differs from what was built");
}

#[then(regex = r#"^"([^"]+)" holds exactly "([^"]+)", byte-identical to the package as built$"#)]
async fn holds_as_built(world: &mut CliWorld, feed: String, name: String) {
    assert_eq!(
        world.names_in(&feed),
        std::slice::from_ref(&name),
        "what {feed} holds"
    );
    let copy = std::fs::read(world.below_work(&format!("{feed}/{name}"))).ok();
    assert!(
        world.publish.earlier.as_ref() == Some(&copy),
        "the published copy differs from the package as built"
    );
}

#[then(regex = r#"^"([^"]+)" holds exactly "([^"]+)", with its earlier bytes$"#)]
async fn holds_earlier(world: &mut CliWorld, feed: String, name: String) {
    assert_eq!(
        world.names_in(&feed),
        std::slice::from_ref(&name),
        "what {feed} holds"
    );
    let now = std::fs::read(world.below_work(&format!("{feed}/{name}"))).ok();
    assert!(
        world.publish.earlier.as_ref() == Some(&now),
        "{name} changed"
    );
}

#[then(regex = r#"^it fails naming "([^"]+)"$"#)]
async fn fails_naming(world: &mut CliWorld, rel: String) {
    let message = world.publish_failed();
    // The registry is not a file below the work directory: its token is as it is printed.
    let named = match rel == REGISTRY {
        true => message.contains(REGISTRY),
        false => world.message_names(&message, &rel),
    };
    assert!(named, "the message does not name {rel}: {message}");
}

#[then(regex = r#"^it fails naming "([^"]+)" and saying to run lvpm build$"#)]
async fn fails_saying_build(world: &mut CliWorld, rel: String) {
    let message = world.publish_failed();
    assert!(
        world.message_names(&message, &rel),
        "the message does not name {rel}: {message}"
    );
    assert!(message.contains("lvpm build"), "{message}");
}

#[then(regex = r#"^"([^"]+)" is empty$"#)]
async fn is_empty(world: &mut CliWorld, rel: String) {
    assert!(
        world.names_in(&rel).is_empty(),
        "{rel} holds {:?}",
        world.names_in(&rel)
    );
}

#[then(regex = r#"^dist holds no [.]lvpkg$"#)]
async fn dist_no_package(world: &mut CliWorld) {
    world.no_package_in_dist();
}

#[then(regex = r#"^"([^"]+)" is as it was, and dist holds only "([^"]+)"$"#)]
async fn as_it_was(world: &mut CliWorld, rel: String, package: String) {
    // Reading a folder fails like reading nothing, so check for one first.
    assert!(!world.below_work(&rel).is_dir(), "{rel} is now a folder");
    let now = std::fs::read(world.below_work(&rel)).ok();
    assert!(
        world.publish.earlier.as_ref() == Some(&now),
        "{rel} changed"
    );
    assert_eq!(world.names_in("dist"), [package]);
}

#[then(
    regex = r#"^it fails, the message names the key "([^"]+)" exactly, and "([^"]+)" is empty$"#
)]
async fn fails_naming_key(world: &mut CliWorld, key: String, feed: String) {
    let message = world.publish_failed();
    assert!(
        super::build::names_key(&message, &key),
        "the message does not name {key}: {message}"
    );
    is_empty(world, feed).await;
}

#[then("it fails and the usage names --to")]
async fn usage_names_to(world: &mut CliWorld) {
    let message = world.publish_failed();
    assert!(message.contains("--to"), "{message}");
}

#[then(
    regex = r#"^it succeeds and lvpm\.lock holds "([^"]+)" version "([^"]+)" from a local source$"#
)]
async fn locked_from_local(world: &mut CliWorld, name: String, version: String) {
    let out = world.publish.lock.as_ref().expect("lvpm lock ran");
    assert_eq!(out.exit_code, 0, "lvpm lock failed: {out:?}");
    let dir = world.sandbox.root().join(CONSUMER);
    let text = std::fs::read_to_string(dir.join("lvpm.lock")).expect("lvpm.lock exists");
    let lock: toml::Table = text.parse().expect("lvpm.lock is valid TOML");
    let package = lock["package"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|p| p["name"].as_str() == Some(name.as_str()))
        .unwrap_or_else(|| panic!("{name} is not in lvpm.lock: {text}"));
    assert_eq!(
        package["version"].as_str(),
        Some(version.as_str()),
        "{text}"
    );
    assert_eq!(package["source"].as_str(), Some("local"), "{text}");
}

// ---- OCI registry (S7b) ----
//
// The scenarios run `lvpm` against a loopback fake registry. `<registry>` in a
// command line is its `host:port`, and in what `lvpm` printed it is the same
// token again (the sandbox normalises it), so the features read the same on
// every run. `<folder>` is a folder feed made on first use.

/// What a scenario's command line and `lvpm`'s output call the registry.
const REGISTRY: &str = "<registry>";
const FOLDER: &str = "<folder>";
/// The folder `<folder>` stands for, beside the work directory.
const FOLDER_FEED: &str = "published";
/// The annotation that holds when the package was published.
const CREATED: &str = "org.opencontainers.image.created";

impl CliWorld {
    /// `line` with `<registry>` and `<folder>` filled in.
    pub(crate) fn with_placeholders(&self, line: &str) -> String {
        let line = match &self.publish.registry {
            Some(registry) => line.replace(REGISTRY, &registry.host()),
            None => line.to_string(),
        };
        match line.contains(FOLDER) {
            true => {
                std::fs::create_dir_all(self.sandbox.root().join(FOLDER_FEED))
                    .expect("create the folder feed");
                line.replace(FOLDER, &format!("../{FOLDER_FEED}"))
            }
            false => line,
        }
    }

    fn registry(&self) -> &Registry {
        self.publish
            .registry
            .as_ref()
            .expect("a loopback OCI registry")
    }

    /// What `lvpm` printed last: the OCI publish's output is `last_output`.
    fn last_printed(&self) -> &Output {
        self.last_output.as_ref().expect("lvpm ran")
    }

    /// The manifest of the tag the scenario last looked at, as JSON.
    fn oci_manifest(&self) -> &serde_json::Value {
        self.publish
            .manifest
            .as_ref()
            .expect("a manifest was looked at")
    }
}

#[given("a loopback OCI registry")]
async fn loopback_registry(world: &mut CliWorld) {
    let registry = Registry::start();
    world.sandbox.set_registry(&registry.host());
    world.publish.registry = Some(registry);
}

#[given(
    regex = r#"^a project "([^"]+)" ([0-9.]+) that lvpm build has packaged, requiring "([^"]+)" and LabVIEW ([0-9]+)\.0$"#
)]
async fn packaged_project(
    world: &mut CliWorld,
    name: String,
    version: String,
    requires: String,
    labview: u32,
) {
    use super::build::{distribution, library_with_one_member, project};
    project(
        world,
        name.clone(),
        version.clone(),
        format!(">={}", 2000 + labview),
    )
    .await;
    distribution(
        world,
        "Lib.lvlib".into(),
        "Vendor".into(),
        "Alpha Lib".into(),
    )
    .await;
    library_with_one_member(world).await;
    let (dependency, constraint) = requires.split_once(' ').expect("a name and a constraint");
    world.set(&["dependencies"], dependency, constraint);
    world.write_build_manifest();
    let built = world.sandbox.run_lvpm(&["build", "--no-check"], None);
    assert_eq!(built.exit_code, 0, "lvpm build failed: {built:?}");
    world.publish.package = Some(world.work("dist").join(format!("{name}-{version}.lvpkg")));
}

#[given(regex = r#"^a package "([^"]+)" whose metadata gives version (\S+)$"#)]
async fn package_with_version(world: &mut CliWorld, name: String, version: String) {
    let path = world.publish.package.clone().expect("a built package");
    let meta = lvpm_core::lvpkg::Meta {
        name,
        version,
        labview: ">=2021".into(),
        vendor: "Vendor".into(),
        display_name: "Alpha Lib".into(),
        install_dir: "vi.lib/Vendor/Lib".into(),
        dependencies: Vec::new(),
    };
    let bytes = lvpm_core::lvpkg::write(&meta, &[("A.vi".into(), b"a vi".to_vec())], None)
        .expect("a package");
    std::fs::write(path, bytes).expect("replace the built package");
}

#[given(regex = r#"^the registry already holds tag "([^"]+)" in repository "([^"]+)"$"#)]
async fn registry_holds_tag(world: &mut CliWorld, tag: String, repository: String) {
    world
        .registry()
        .seed_manifest(&repository, &tag, br#"{"schemaVersion":2}"#);
}

#[given(
    regex = r#"^the registry will create tag "([^"]+)" in "([^"]+)" once the first blob is uploaded$"#
)]
async fn registry_creates_tag(world: &mut CliWorld, tag: String, repository: String) {
    world
        .registry()
        .create_tag_after_first_blob(&repository, &tag);
}

#[given("the registry is down")]
async fn registry_is_down(world: &mut CliWorld) {
    world.registry().go_down();
}

/// A publish uploads the config and then the layer: two blobs.
#[given(regex = r#"^the registry went down after the blobs of "([^"]+)" were uploaded$"#)]
async fn registry_down_after_blobs(world: &mut CliWorld, _reference: String) {
    world.registry().go_down_after_blob_uploads(2);
}

#[given(regex = r#"^the registry answers (\d+) to a HEAD of "([^"]+):([^":]+)"$"#)]
async fn registry_answers_to_head(world: &mut CliWorld, status: u16, repo: String, tag: String) {
    world
        .registry()
        .answer("HEAD", &format!("/v2/{repo}/manifests/{tag}"), status);
}

#[given(regex = r#"^the registry answers (\d+) to a (POST|PUT) of "([^"]+)"$"#)]
async fn registry_answers_to_write(
    world: &mut CliWorld,
    status: u16,
    method: String,
    path: String,
) {
    world.registry().answer(&method, &path, status);
}

#[given(regex = r#"^I ran "([^"]+)" and it failed$"#)]
async fn ran_and_failed(world: &mut CliWorld, command_line: String) {
    world.run(&command_line, None);
    let out = world.last_printed();
    assert_ne!(out.exit_code, 0, "the command succeeded: {out:?}");
}

#[when(regex = r#"^the registry is back and I run "([^"]+)"$"#)]
async fn registry_back_and_run(world: &mut CliWorld, command_line: String) {
    world.registry().come_back();
    world.registry().clear_requests();
    world.run(&command_line, None);
}

#[then(regex = r#"^it fails saying "([^"]+)"$"#)]
async fn fails_saying(world: &mut CliWorld, text: String) {
    let message = world.publish_failed();
    assert!(message.contains(&text), "{text:?} is not in: {message}");
}

#[then(regex = r#"^it fails saying "([^"]+)" is not a valid OCI tag$"#)]
async fn fails_saying_not_a_tag(world: &mut CliWorld, version: String) {
    let message = world.publish_failed();
    let text = format!("{version} is not a valid OCI tag");
    assert!(message.contains(&text), "{text:?} is not in: {message}");
}

#[then(regex = r#"^the output does not contain "([^"]+)"$"#)]
async fn output_does_not_contain(world: &mut CliWorld, text: String) {
    let out = world.last_printed();
    assert!(
        !out.stdout.contains(&text) && !out.stderr.contains(&text),
        "{text:?} is in: {out:?}"
    );
}

#[then(regex = r#"^the output gives the OCI manifest digest of "([^":]+):([^":]+)"$"#)]
async fn output_gives_digest(world: &mut CliWorld, repository: String, tag: String) {
    let digest = world
        .registry()
        .tag_digest(&repository, &tag)
        .unwrap_or_else(|| panic!("the registry holds no {repository}:{tag}"));
    let out = world.last_printed();
    let line = format!("OCI manifest digest {digest}");
    assert!(
        out.stdout.lines().any(|l| l.trim() == line),
        "no {line:?} in {out:?}"
    );
}

#[then(regex = r#"^the registry holds tag "([^"]+)" in repository "([^"]+)"$"#)]
async fn registry_has_tag(world: &mut CliWorld, tag: String, repository: String) {
    let registry = world.registry();
    let digest = registry
        .tag_digest(&repository, &tag)
        .unwrap_or_else(|| panic!("the registry holds no {repository}:{tag}"));
    let bytes = registry
        .manifest(&repository, &digest)
        .expect("the manifest of the tag");
    assert_eq!(lvpm_core::oci::digest_of(&bytes), digest);
    world.publish.manifest = Some(serde_json::from_slice(&bytes).expect("a JSON manifest"));
    world.publish.repository = repository;
}

#[then(regex = r#"^that OCI manifest has artifactType "([^"]+)"$"#)]
async fn manifest_artifact_type(world: &mut CliWorld, media_type: String) {
    assert_eq!(world.oci_manifest()["artifactType"], media_type.as_str());
}

#[then(regex = r#"^its config has media type "([^"]+)"$"#)]
async fn manifest_config_type(world: &mut CliWorld, media_type: String) {
    assert_eq!(
        world.oci_manifest()["config"]["mediaType"],
        media_type.as_str()
    );
}

#[then(regex = r#"^its one layer has media type "([^"]+)" and the package's bytes$"#)]
async fn manifest_layer(world: &mut CliWorld, media_type: String) {
    let layers = world.oci_manifest()["layers"]
        .as_array()
        .expect("layers")
        .clone();
    assert_eq!(layers.len(), 1, "{layers:?}");
    assert_eq!(layers[0]["mediaType"], media_type.as_str());
    let package =
        std::fs::read(world.publish.package.as_ref().expect("a package")).expect("the package");
    let digest = lvpm_core::oci::digest_of(&package);
    assert_eq!(layers[0]["digest"], digest.as_str());
    assert_eq!(layers[0]["size"], package.len());
    let held = world.registry().blob(&world.publish.repository, &digest);
    assert!(
        held.as_deref() == Some(package.as_slice()),
        "the layer's blob differs from the package"
    );
}

#[then("its annotations are:")]
async fn manifest_annotations(world: &mut CliWorld, step: &cucumber::gherkin::Step) {
    let table = step.table.as_ref().expect("a table of annotations");
    let mut want: Vec<(String, String)> = table
        .rows
        .iter()
        .skip(1)
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect();
    let mut got: Vec<(String, String)> = world.oci_manifest()["annotations"]
        .as_object()
        .expect("annotations")
        .iter()
        .filter(|(key, _)| key.as_str() != CREATED)
        .map(|(key, value)| (key.clone(), value.as_str().expect("text").to_string()))
        .collect();
    want.sort();
    got.sort();
    assert_eq!(got, want);
}

#[then(regex = r#"^annotation "([^"]+)" is an RFC 3339 time$"#)]
async fn annotation_is_a_time(world: &mut CliWorld, key: String) {
    static RFC3339: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$").expect("a valid pattern")
    });
    let value = world.oci_manifest()["annotations"][&key]
        .as_str()
        .unwrap_or_default();
    assert!(RFC3339.is_match(value), "{key} is {value:?}");
}

#[then("the registry received no blob or manifest write")]
async fn registry_no_writes(world: &mut CliWorld) {
    let writes = world.registry().writes();
    assert!(writes.is_empty(), "writes: {writes:?}");
}

#[then("the registry received no manifest write")]
async fn registry_no_manifest_write(world: &mut CliWorld) {
    let writes = world.registry().writes();
    assert!(
        writes.iter().all(|w| !w.path.contains("/manifests/")),
        "writes: {writes:?}"
    );
}

#[then("the registry received no request")]
async fn registry_no_request(world: &mut CliWorld) {
    let requests = world.registry().requests();
    assert!(requests.is_empty(), "requests: {requests:?}");
}

/// Only the rerun's own requests are in the log: it must write the manifest
/// and nothing else.
#[then("the registry received no second upload of a blob it already held")]
async fn registry_no_second_upload(world: &mut CliWorld) {
    let writes = world.registry().writes();
    assert!(
        writes
            .iter()
            .all(|w| w.method == "PUT" && w.path.contains("/manifests/")),
        "writes: {writes:?}"
    );
    assert_eq!(writes.len(), 1, "writes: {writes:?}");
}

#[then("the package file is in the folder")]
async fn package_in_folder(world: &mut CliWorld) {
    let package = world.publish.package.clone().expect("a package");
    let published = world
        .sandbox
        .root()
        .join(FOLDER_FEED)
        .join(package.file_name().expect("a file name"));
    let (copy, built) = (std::fs::read(&published), std::fs::read(&package));
    assert!(
        copy.is_ok() && copy.ok() == built.ok(),
        "{} is not the built package",
        published.display()
    );
}
