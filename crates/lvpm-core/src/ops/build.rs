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
//! The package carries an SBOM of the locked `[dependencies]`. A project that
//! has dependencies but no usable lock gets a package without one, and the
//! outcome says why: the build does not fail over the lock.

use super::check::{Library, check_library};
use super::env::OpEnv;
use crate::errors::{CheckError, LockError};
use crate::lockfile::{self, Lockfile, StaleReason};
use crate::project::{self, Project};
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
}

/// What a build did: wrote the package, or refused to.
#[derive(Debug)]
pub enum Outcome {
    Built(Built),
    /// A member breaks a rule of the check; nothing was written.
    Refused(Library),
}

pub fn build(env: &OpEnv, check: bool) -> Result<Outcome, CheckError> {
    let (root, d) = super::packaged(env, "build")?;
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
    let project = env.read_manifest(&root.join(project::FILE_NAME))?;
    let subject = Subject {
        name: meta.name.clone(),
        version: meta.version.clone(),
        kind: Kind::Library,
    };
    let (sbom_text, sbom) = package_sbom(&project, read_lock(root)?.as_ref(), subject);
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

/// `lvpm.lock` beside the manifest in `root`; `None` when there is none.
pub(super) fn read_lock(root: &Path) -> Result<Option<Lockfile>> {
    match super::lock::read_lock(&root.join(lockfile::FILE_NAME)) {
        Ok(lock) => Ok(Some(lock)),
        Err(LockError::NoLock { .. }) => Ok(None),
        Err(other) => Err(other.into()),
    }
}

/// The SBOM text of `lock` for `subject`, and what that says for the outcome.
/// A project with no `[dependencies]` needs no lock: its SBOM lists nothing.
fn package_sbom(
    project: &Project,
    lock: Option<&Lockfile>,
    subject: Subject,
) -> (Option<String>, Sbom) {
    match lock {
        None if !project.dependencies.is_empty() => (None, Sbom::NoLock),
        Some(lock) => match lock.staleness(project) {
            Some(reason) => (None, Sbom::Stale(reason)),
            None => (Some(sbom::cyclonedx(Some(lock), subject)), Sbom::Included),
        },
        None => (Some(sbom::cyclonedx(None, subject)), Sbom::Included),
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
    use crate::lockfile::{FORMAT_VERSION, Inputs, LOCAL_SOURCE, LockedPackage};

    fn subject() -> Subject {
        Subject {
            name: "demo".into(),
            version: "1.0".into(),
            kind: Kind::Library,
        }
    }

    const MANIFEST: &str = "[project]\nname = \"demo\"\nversion = \"1.0\"\nlabview = \">=2024\"\n";

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

    const ALPHA: &str = "\n[dependencies]\nalpha = \"1.0\"\n";

    #[test]
    fn a_current_lock_gives_the_package_an_sbom_of_it() {
        let project = project(ALPHA);
        let (text, sbom) = package_sbom(&project, Some(&lock_of(&project)), subject());
        assert_eq!(sbom, Sbom::Included);
        let text = text.expect("an SBOM");
        assert!(text.contains("\"bom-ref\": \"alpha@1.0\""), "{text}");
    }

    #[test]
    fn no_lock_and_dependencies_gives_no_sbom_and_says_why() {
        let (text, sbom) = package_sbom(&project(ALPHA), None, subject());
        assert_eq!((text, sbom), (None, Sbom::NoLock));
    }

    #[test]
    fn no_lock_and_no_dependencies_gives_an_sbom_with_no_components() {
        let dev_only = "\n[dev-dependencies]\ngamma = \"1.0\"\n";
        for manifest in ["", dev_only] {
            let (text, sbom) = package_sbom(&project(manifest), None, subject());
            assert_eq!(sbom, Sbom::Included, "{manifest:?}");
            let text = text.expect("an SBOM");
            assert!(text.contains("\"components\": []"), "{text}");
            assert!(text.contains("\"type\": \"library\""), "{text}");
        }
    }

    #[test]
    fn a_stale_lock_gives_no_sbom_and_says_why() {
        let locked = project(ALPHA);
        let lock = lock_of(&locked);
        let now = project(&format!("{ALPHA}beta = \"2.0\"\n"));
        let (text, sbom) = package_sbom(&now, Some(&lock), subject());
        let added = StaleReason::DependencyAdded {
            name: "beta".into(),
            table: crate::manifest_edit::DependencyTable::Dependencies,
        };
        assert_eq!((text, sbom), (None, Sbom::Stale(added)));
    }

    #[test]
    fn a_lock_that_is_not_there_is_none_and_one_that_cannot_be_read_is_an_error() {
        let dir = std::env::temp_dir().join(format!("lvpm-build-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_lock(&dir).unwrap().is_none());
        std::fs::write(dir.join(lockfile::FILE_NAME), "not a lock = =").unwrap();
        let why = format!("{:#}", read_lock(&dir).unwrap_err());
        assert!(why.contains("lvpm.lock cannot be read"), "{why}");
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
