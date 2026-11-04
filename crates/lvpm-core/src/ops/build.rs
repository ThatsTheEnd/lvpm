//! `build`: package the project's library as a `.lvpkg` in `dist/`.
//!
//! Reads `lvpm.toml`'s `[distribution]` and the library's own member list,
//! and writes the members into the package. No LabVIEW is involved. The
//! package is written to a temporary file and renamed over
//! `dist/<name>-<version>.lvpkg` only once it is complete, so a failed build
//! leaves the previous package as it was.
//!
//! With `check`, the library's members are checked first, as `lvpm check`
//! does, and a member that breaks a rule refuses the build before the members
//! are packaged or the package is written.
//!
//! The package carries an SBOM of the locked `[dependencies]`. A project with
//! no `[dependencies]` needs no lock: its SBOM lists nothing. One that has
//! dependencies but no usable lock (missing, unreadable or out of date) gets a
//! package without an SBOM, and the outcome says why: the build does not fail
//! over the lock.

use super::check::{Library, check_library};
use super::env::OpEnv;
use crate::errors::{CheckError, LockError};
use crate::lockfile::{self, StaleReason};
use crate::project::Project;
use crate::sbom::{self, Kind, Subject};
use crate::{distribution, lvlib, lvpkg};
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};

/// What a build wrote.
#[derive(Debug)]
pub struct Built {
    pub path: PathBuf,
    /// The members packaged, the library itself among them.
    pub files: usize,
    /// What the check made of the library; `None` when it was not run.
    pub library: Option<Library>,
    pub sbom: Sbom,
}

/// What a package build made of the package's SBOM.
#[derive(Debug, PartialEq)]
pub enum Sbom {
    /// The package carries one.
    Included,
    /// `[dependencies]` is not empty and there is no `lvpm.lock` to list it from.
    NoLock,
    /// `lvpm.lock` no longer represents the manifest, so it is not listed from.
    Stale(StaleReason),
    /// `lvpm.lock` is there but cannot be read; the reason is in its own words.
    Unreadable(String),
}

/// What a build did: wrote the package, or refused to.
#[derive(Debug)]
pub enum Outcome {
    Built(Built),
    /// A member breaks a rule of the check; nothing was written.
    Refused(Library),
}

pub fn build(env: &OpEnv, check: bool) -> Result<Outcome, CheckError> {
    let (root, d, project) = super::packaged(env, "build")?;
    let root = root.as_path();
    let checked = match check {
        true => Some(check_library(env)?),
        false => None,
    };
    let checked = match checked {
        Some(library) if !library.checked.passed() => {
            return Ok(Outcome::Refused(library));
        }
        other => other,
    };

    let library = root.join(&d.library);
    let folder = library.parent().context("the library has no folder")?;
    // Checked on disk too: a link inside the folder may lead out of the project.
    let real_folder = folder
        .canonicalize()
        .with_context(|| format!("resolving {}", folder.display()))?;
    let members: Vec<(String, PathBuf)> = lvlib::members(&library)?
        .into_iter()
        .map(|m| {
            let m = lvlib::normalize(&m);
            let rel = m.strip_prefix(lvlib::normalize(folder)).map_err(|_| {
                anyhow::anyhow!(
                    "{} is a member of {} but lies outside its folder {}",
                    m.display(),
                    d.library.display(),
                    folder.display()
                )
            })?;
            let real = m
                .canonicalize()
                .with_context(|| format!("resolving {}", m.display()))?;
            if !real.starts_with(&real_folder) {
                bail!(
                    "{} is a member of {} but is a link to {}, outside its folder",
                    m.display(),
                    d.library.display(),
                    real.display()
                );
            }
            let rel = rel.to_string_lossy().replace('\\', "/");
            // A file named `..\x` on Linux would become a `..` entry in the zip.
            if rel.split('/').any(|s| s == "..") || rel.contains(':') {
                bail!(
                    "{} is a member of {} but its name cannot go into a package",
                    m.display(),
                    d.library.display()
                );
            }
            distribution::spec_safe("a member's path", &rel)?;
            Ok((rel, m))
        })
        .collect::<Result<_>>()?;
    let files: Vec<(String, Vec<u8>)> = members
        .into_iter()
        .map(|(rel, file)| {
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            Ok((rel, bytes))
        })
        .collect::<Result<_>>()?;
    let meta = d.meta();
    let (sbom_text, sbom) = sbom_for(root, &project, &meta);
    let bytes = lvpkg::write(&meta, &files, sbom_text.as_deref().map(str::as_bytes))?;
    let package = super::package_path(root, &d);
    let dist = package.parent().context("the package has no folder")?;
    std::fs::create_dir_all(dist).with_context(|| format!("creating {}", dist.display()))?;
    let path = write_atomically(&package, &bytes)?;
    Ok(Outcome::Built(Built {
        path,
        files: files.len(),
        library: checked,
        sbom,
    }))
}

/// The SBOM text of the package of `project` at `root`, and what that says for
/// the outcome. A project with no `[dependencies]` needs no lock, so it is not
/// read: whatever its state, the SBOM lists nothing.
fn sbom_for(root: &Path, project: &Project, meta: &lvpkg::Meta) -> (Option<String>, Sbom) {
    let subject = Subject {
        name: meta.name.clone(),
        version: meta.version.clone(),
        kind: Kind::Library,
    };
    if project.dependencies.is_empty() {
        return (Some(sbom::cyclonedx(None, subject)), Sbom::Included);
    }
    match super::lock::read_lock(&root.join(lockfile::FILE_NAME)) {
        Err(LockError::NoLock { .. }) => (None, Sbom::NoLock),
        Err(LockError::UnreadableLock { reason, .. }) => (None, Sbom::Unreadable(reason)),
        Err(other) => (None, Sbom::Unreadable(other.to_string())),
        Ok(lock) => match lock.staleness(project) {
            Some(reason) => (None, Sbom::Stale(reason)),
            None => (Some(sbom::cyclonedx(Some(&lock), subject)), Sbom::Included),
        },
    }
}

/// Write the package beside `path` and rename it over `path` once complete;
/// on any failure the staged file goes and `path` stays as it was.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let temp = crate::paths::staging_path(path);
    let written = write_synced(&temp, bytes)
        .and_then(|()| std::fs::rename(&temp, path).context("replacing the package"));
    if let Err(why) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(why.context(format!("writing {}", path.display())));
    }
    Ok(path.to_path_buf())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{FORMAT_VERSION, Inputs, LOCAL_SOURCE, LockedPackage, Lockfile};
    use crate::project;

    fn meta() -> lvpkg::Meta {
        lvpkg::Meta {
            name: "demo".into(),
            version: "1.0".into(),
            labview: ">=2024".into(),
            vendor: "acme".into(),
            display_name: "Demo".into(),
            install_dir: "vi.lib/acme/demo".into(),
            dependencies: Vec::new(),
        }
    }

    const MANIFEST: &str = "[project]
name = \"demo\"
version = \"1.0\"
labview = \">=2024\"
";

    fn project(dependencies: &str) -> Project {
        project::parse(&format!("{MANIFEST}{dependencies}")).unwrap()
    }

    /// The lock `lvpm lock` would write for `project`, holding `alpha 1.0`.
    fn lock_of(project: &Project) -> Lockfile {
        Lockfile {
            version: FORMAT_VERSION,
            labview: "2024".into(),
            inputs: Inputs::of(project),
            packages: vec![LockedPackage {
                name: "alpha".into(),
                version: "1.0".into(),
                sha256: "ab".repeat(32),
                md5: None,
                source: LOCAL_SOURCE.into(),
                requires: Vec::new(),
                groups: Vec::new(),
                signer: None,
                subject: None,
            }],
            unresolved: Vec::new(),
        }
    }

    const ALPHA: &str = "
[dependencies]
alpha = \"1.0\"
";

    /// A project folder holding `lock`'s text as `lvpm.lock`, when there is any.
    fn folder_with(name: &str, lock: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lvpm-build-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(text) = lock {
            std::fs::write(dir.join(lockfile::FILE_NAME), text).unwrap();
        }
        dir
    }

    fn lock_text_of(project: &Project) -> String {
        lock_of(project).to_toml().unwrap()
    }

    #[test]
    fn a_current_lock_gives_the_package_an_sbom_of_it() {
        let project = project(ALPHA);
        let dir = folder_with("current", Some(&lock_text_of(&project)));
        let (text, sbom) = sbom_for(&dir, &project, &meta());
        assert_eq!(sbom, Sbom::Included);
        let bom: serde_json::Value = serde_json::from_str(&text.expect("an SBOM")).unwrap();
        let refs: Vec<_> = bom["components"].as_array().unwrap().iter().collect();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0]["bom-ref"], "alpha@1.0");
        assert_eq!(bom["metadata"]["component"]["name"], "demo");
        assert_eq!(bom["metadata"]["component"]["version"], "1.0");
        assert_eq!(bom["metadata"]["component"]["type"], "library");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_lock_and_dependencies_gives_no_sbom_and_says_why() {
        let dir = folder_with("no-lock", None);
        let (text, sbom) = sbom_for(&dir, &project(ALPHA), &meta());
        assert_eq!((text, sbom), (None, Sbom::NoLock));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_dependencies_gives_an_sbom_with_no_components_whatever_the_lock() {
        let dev_only = "
[dev-dependencies]
gamma = \"1.0\"
";
        let held = project(ALPHA);
        // Missing, out of date (it lists a dependency the manifest dropped), unreadable.
        let locks = [
            None,
            Some(lock_text_of(&held)),
            Some("not a lock = =".into()),
        ];
        for manifest in ["", dev_only] {
            for (n, lock) in locks.iter().enumerate() {
                let dir = folder_with(&format!("empty-{n}"), lock.as_deref());
                let (text, sbom) = sbom_for(&dir, &project(manifest), &meta());
                assert_eq!(sbom, Sbom::Included, "{manifest:?} {lock:?}");
                let text = text.expect("an SBOM");
                assert!(text.contains("\"components\": []"), "{text}");
                assert!(text.contains("\"type\": \"library\""), "{text}");
                std::fs::remove_dir_all(&dir).ok();
            }
        }
    }

    #[test]
    fn a_stale_lock_gives_no_sbom_and_says_why() {
        let dir = folder_with("stale", Some(&lock_text_of(&project(ALPHA))));
        let now = project(&format!(
            "{ALPHA}beta = \"2.0\"
"
        ));
        let (text, sbom) = sbom_for(&dir, &now, &meta());
        let added = StaleReason::DependencyAdded {
            name: "beta".into(),
            table: crate::manifest_edit::DependencyTable::Dependencies,
        };
        assert_eq!((text, sbom), (None, Sbom::Stale(added)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_lock_that_cannot_be_read_gives_no_sbom_and_says_why() {
        let dir = folder_with("unreadable", Some("not a lock = ="));
        let (text, sbom) = sbom_for(&dir, &project(ALPHA), &meta());
        assert_eq!(text, None);
        let Sbom::Unreadable(reason) = sbom else {
            panic!("expected Unreadable, got {sbom:?}");
        };
        assert!(!reason.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_write_that_fails_at_the_rename_leaves_what_was_there_and_no_staged_file() {
        let dir = std::env::temp_dir().join(format!("lvpm-build-publish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A folder where the package goes: the rename cannot replace it.
        let path = dir.join("p-1.0.0.lvpkg");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("previous"), "previous").unwrap();
        let why = format!("{:#}", write_atomically(&path, b"new package").unwrap_err());
        assert!(why.contains("replacing the package"), "{why}");
        assert_eq!(
            std::fs::read_to_string(path.join("previous")).unwrap(),
            "previous"
        );
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, [path.file_name().unwrap()], "only the package stays");
        std::fs::remove_dir_all(&dir).ok();
    }
}
