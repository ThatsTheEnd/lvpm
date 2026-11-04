//! The SBOM: a CycloneDX 1.5 JSON file listing the locked packages a build
//! ships, each with its version and SHA-256.
//!
//! It lists the `[dependencies]` closure of `lvpm.lock` and nothing a dev or
//! group table needs. Its text is a function of the lock and the subject alone:
//! no `serialNumber`, no timestamp, packages sorted, so the same lock gives the
//! same bytes.

use crate::lockfile::{LockedPackage, Lockfile};
use crate::project::name_key;
use serde::Serialize;
use std::collections::HashMap;

const BOM_FORMAT: &str = "CycloneDX";
const SPEC_VERSION: &str = "1.5";
const HASH_ALG: &str = "SHA-256";
const DISTRIBUTION: &str = "distribution";

/// What the SBOM is about: the project that ships the locked packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    pub name: String,
    pub version: String,
    pub kind: Kind,
}

/// What the subject is, as CycloneDX names the component `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A built application.
    Application,
    /// A `.lvpkg` library.
    Library,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bom {
    bom_format: &'static str,
    spec_version: &'static str,
    version: u32,
    metadata: Metadata,
    components: Vec<Component>,
    dependencies: Vec<Dependency>,
}

#[derive(Serialize)]
struct Metadata {
    component: SubjectComponent,
}

#[derive(Serialize)]
struct SubjectComponent {
    r#type: Kind,
    name: String,
    version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Component {
    r#type: Kind,
    name: String,
    version: String,
    #[serde(rename = "bom-ref")]
    bom_ref: String,
    hashes: Vec<Hash>,
    external_references: Vec<ExternalReference>,
}

#[derive(Serialize)]
struct Hash {
    alg: &'static str,
    content: String,
}

#[derive(Serialize)]
struct ExternalReference {
    r#type: &'static str,
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Dependency {
    r#ref: String,
    depends_on: Vec<String>,
}

fn bom_ref(package: &LockedPackage) -> String {
    format!("{}@{}", package.name, package.version)
}

/// The packages `[dependencies]` needs, sorted by name.
fn closure(lock: Option<&Lockfile>) -> Vec<&LockedPackage> {
    let mut packages: Vec<&LockedPackage> = (lock.iter())
        .flat_map(|lock| &lock.packages)
        .filter(|package| package.groups.is_empty())
        .collect();
    packages.sort_by_cached_key(|package| (name_key(&package.name), package.version.clone()));
    packages
}

fn component(package: &LockedPackage) -> Component {
    Component {
        r#type: Kind::Library,
        name: package.name.clone(),
        version: package.version.clone(),
        bom_ref: bom_ref(package),
        hashes: vec![Hash {
            alg: HASH_ALG,
            content: package.sha256.clone(),
        }],
        external_references: vec![ExternalReference {
            r#type: DISTRIBUTION,
            url: package.source.clone(),
        }],
    }
}

/// What `package` requires, by `bom-ref`. `refs` maps a [`name_key`] to the
/// `bom-ref` of the closure's package of that name.
fn dependency(package: &LockedPackage, refs: &HashMap<String, String>) -> Dependency {
    let depends_on = package
        .requires
        .iter()
        .filter_map(|name| {
            let found = refs.get(&name_key(name)).cloned();
            // A valid lock names only packages it holds, and `[dependencies]`
            // needs everything a package of its closure requires.
            debug_assert!(found.is_some(), "{name} is required but not in the closure");
            found
        })
        .collect();
    Dependency {
        r#ref: bom_ref(package),
        depends_on,
    }
}

/// The SBOM of `lock` for `subject`, as JSON text ending in a newline. `None`,
/// a project with no lock, gives an SBOM with no components.
///
/// shortcut: no schema validation, add the vendored 1.5 schema as a
/// dev-dependency when a consumer rejects the file.
pub fn cyclonedx(lock: Option<&Lockfile>, subject: Subject) -> String {
    let packages = closure(lock);
    let refs: HashMap<String, String> = (packages.iter())
        .map(|package| (name_key(&package.name), bom_ref(package)))
        .collect();
    let bom = Bom {
        bom_format: BOM_FORMAT,
        spec_version: SPEC_VERSION,
        version: 1,
        metadata: Metadata {
            component: SubjectComponent {
                r#type: subject.kind,
                name: subject.name,
                version: subject.version,
            },
        },
        components: packages.iter().map(|package| component(package)).collect(),
        dependencies: (packages.iter())
            .map(|package| dependency(package, &refs))
            .collect(),
    };
    let mut text = serde_json::to_string_pretty(&bom).expect("an SBOM is plain data");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{FORMAT_VERSION, Inputs};

    fn subject(kind: Kind) -> Subject {
        Subject {
            name: "demo".to_string(),
            version: "1.2.3".to_string(),
            kind,
        }
    }

    #[test]
    fn no_lock_gives_the_subject_and_no_components() {
        let expected = r#"{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "version": 1,
  "metadata": {
    "component": {
      "type": "application",
      "name": "demo",
      "version": "1.2.3"
    }
  },
  "components": [],
  "dependencies": []
}
"#;
        assert_eq!(cyclonedx(None, subject(Kind::Application)), expected);
    }

    fn locked(name: &str, version: &str, requires: &[&str], groups: &[&str]) -> LockedPackage {
        LockedPackage {
            name: name.to_string(),
            version: version.to_string(),
            sha256: format!("{name}-sha"),
            md5: None,
            source: format!("https://feed.example/{name}"),
            requires: requires.iter().map(|name| name.to_string()).collect(),
            groups: groups.iter().map(|group| group.to_string()).collect(),
            signer: None,
            subject: None,
        }
    }

    fn lock(packages: Vec<LockedPackage>) -> Lockfile {
        Lockfile {
            version: FORMAT_VERSION,
            labview: "2024".to_string(),
            inputs: Inputs::default(),
            packages,
            unresolved: Vec::new(),
        }
    }

    #[test]
    fn the_dependencies_closure_is_listed_and_dev_and_group_packages_are_not() {
        let lock = lock(vec![
            locked("beta", "2.0", &[], &[]),
            locked("alpha", "1.0", &["beta"], &[]),
            locked("gamma", "1.0", &[], &["dev"]),
            locked("delta", "1.0", &[], &["ci"]),
        ]);
        let expected = r#"{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "version": 1,
  "metadata": {
    "component": {
      "type": "library",
      "name": "demo",
      "version": "1.2.3"
    }
  },
  "components": [
    {
      "type": "library",
      "name": "alpha",
      "version": "1.0",
      "bom-ref": "alpha@1.0",
      "hashes": [
        {
          "alg": "SHA-256",
          "content": "alpha-sha"
        }
      ],
      "externalReferences": [
        {
          "type": "distribution",
          "url": "https://feed.example/alpha"
        }
      ]
    },
    {
      "type": "library",
      "name": "beta",
      "version": "2.0",
      "bom-ref": "beta@2.0",
      "hashes": [
        {
          "alg": "SHA-256",
          "content": "beta-sha"
        }
      ],
      "externalReferences": [
        {
          "type": "distribution",
          "url": "https://feed.example/beta"
        }
      ]
    }
  ],
  "dependencies": [
    {
      "ref": "alpha@1.0",
      "dependsOn": [
        "beta@2.0"
      ]
    },
    {
      "ref": "beta@2.0",
      "dependsOn": []
    }
  ]
}
"#;
        assert_eq!(cyclonedx(Some(&lock), subject(Kind::Library)), expected);
    }

    fn parsed(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn the_text_parses_as_cyclonedx_1_5_and_ends_in_a_newline() {
        let text = cyclonedx(None, subject(Kind::Library));
        let bom = parsed(&text);
        assert_eq!(bom["bomFormat"], "CycloneDX");
        assert_eq!(bom["specVersion"], "1.5");
        assert!(text.ends_with(
            "}
"
        ));
    }

    #[test]
    fn the_subject_kind_is_the_metadata_component_type() {
        for (kind, name) in [
            (Kind::Application, "application"),
            (Kind::Library, "library"),
        ] {
            let bom = parsed(&cyclonedx(None, subject(kind)));
            let component = &bom["metadata"]["component"];
            assert_eq!(component["type"], name);
            assert_eq!(component["name"], "demo");
            assert_eq!(component["version"], "1.2.3");
        }
    }

    #[test]
    fn the_same_lock_gives_the_same_bytes_and_no_serial_number_or_timestamp() {
        let lock = lock(vec![locked("alpha", "1.0", &[], &[])]);
        for kind in [Kind::Application, Kind::Library] {
            let first = cyclonedx(Some(&lock), subject(kind));
            assert_eq!(first, cyclonedx(Some(&lock), subject(kind)));
            assert!(!first.contains("serialNumber"));
            assert!(!first.contains("timestamp"));
        }
    }

    #[test]
    fn components_sort_by_lowercase_name_and_requires_match_names_in_any_case() {
        let lock = lock(vec![
            locked("Zeta", "1.0", &["alpha"], &[]),
            locked("alpha", "1.0", &[], &[]),
            locked("Beta", "2.0", &[], &[]),
        ]);
        let bom = parsed(&cyclonedx(Some(&lock), subject(Kind::Library)));
        let refs: Vec<&str> = (bom["components"].as_array().unwrap().iter())
            .map(|component| component["bom-ref"].as_str().unwrap())
            .collect();
        assert_eq!(refs, ["alpha@1.0", "Beta@2.0", "Zeta@1.0"]);
        assert_eq!(bom["dependencies"][2]["dependsOn"][0], "alpha@1.0");
    }
}
