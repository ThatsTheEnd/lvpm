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
//!
//! [`run`] is the whole of `lvpm build`: it settles with [`kinds`] which of the
//! package and the application to build, refuses before writing anything when
//! the application is not ready, builds the package, then the application.

use super::build_app::{self, AppBuilt, AppError, AppLauncher};
use super::check::{Library, check_library};
use super::env::OpEnv;
use crate::errors::{CheckError, LockError};
use crate::lockfile::{self, Lockfile, StaleReason};
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

/// What a build did: wrote the package, or refused to, or built the application.
#[derive(Debug)]
pub enum Outcome {
    Built(Built),
    /// A member breaks a rule of the check; nothing was written.
    Refused(Library),
    /// The application was built and its SBOM written.
    AppBuilt(AppBuilt),
}

/// What `lvpm.toml` and the flags have `lvpm build` build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kinds {
    pub package: bool,
    pub app: bool,
}

/// The project `lvpm build` works on, for [`kinds`].
pub fn project(env: &OpEnv) -> Result<Project> {
    Ok(super::project_of(env, "build")?.1)
}

/// What a build builds. With no flag, every table `p` has; with a flag, exactly
/// the flagged tables, each of which must be there. `no_check` skips the
/// package's member check, so it needs a package to build.
pub fn kinds(p: &Project, app: bool, package: bool, no_check: bool) -> Result<Kinds> {
    let has = Kinds {
        package: p.distribution.is_some(),
        app: p.application.is_some(),
    };
    let wanted = match (app, package) {
        (false, false) => has,
        _ => Kinds { package, app },
    };
    if wanted.app && !has.app {
        bail!("--app needs [application] in lvpm.toml (see docs/manifest.md)");
    }
    if wanted.package && !has.package {
        bail!("--package needs [distribution] in lvpm.toml (see docs/manifest.md)");
    }
    if !wanted.package && !wanted.app {
        bail!("nothing to build: lvpm.toml has no [distribution] or [application]");
    }
    if no_check && !wanted.package {
        bail!("--no-check applies only to the package build, and this run builds none");
    }
    Ok(wanted)
}

/// Why a run built less than it was asked to.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Package(#[from] CheckError),
    #[error(transparent)]
    App(#[from] AppError),
}

/// Build what `kinds` says: the application is checked first, so one that is
/// not ready writes no package either; then the package, which a refused check
/// ends the run on; then the application. `on_package` is handed the package
/// once it is written, before the application's LabVIEW is started, so a front
/// end can show it even if the application then fails.
pub fn run(
    env: &OpEnv,
    kinds: Kinds,
    no_check: bool,
    launcher: &dyn AppLauncher,
    mut on_package: impl FnMut(&Outcome),
) -> Result<Vec<Outcome>, RunError> {
    let ready = match kinds.app {
        true => Some(build_app::check_app(env)?),
        false => None,
    };
    let mut outcomes = Vec::new();
    if kinds.package {
        let package = build(env, !no_check)?;
        match &package {
            Outcome::Refused(_) => return Ok(vec![package]),
            built => on_package(built),
        }
        outcomes.push(package);
    }
    if let Some(ready) = ready {
        let built = build_app::build_ready(env, ready, launcher)?;
        outcomes.push(Outcome::AppBuilt(built));
    }
    Ok(outcomes)
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
    match current_lock(root, project) {
        Ok(lock) => (Some(sbom::cyclonedx(Some(&lock), subject)), Sbom::Included),
        Err(LockProblem::Missing) => (None, Sbom::NoLock),
        Err(LockProblem::Stale(reason)) => (None, Sbom::Stale(reason)),
        Err(LockProblem::Unreadable(reason)) => (None, Sbom::Unreadable(reason)),
    }
}

/// Why the lock next to the manifest cannot be listed from.
#[derive(Debug)]
pub(super) enum LockProblem {
    Missing,
    /// There, but not a lock: the reason, in its own words.
    Unreadable(String),
    Stale(StaleReason),
}

/// The lock next to the manifest in `root`, when it is there, can be read and
/// still represents `project`.
pub(super) fn current_lock(root: &Path, project: &Project) -> Result<Lockfile, LockProblem> {
    match super::lock::read_lock(&root.join(lockfile::FILE_NAME)) {
        Err(LockError::NoLock { .. }) => Err(LockProblem::Missing),
        Err(LockError::UnreadableLock { reason, .. }) => Err(LockProblem::Unreadable(reason)),
        Err(other) => Err(LockProblem::Unreadable(other.to_string())),
        Ok(lock) => match lock.staleness(project) {
            Some(reason) => Err(LockProblem::Stale(reason)),
            None => Ok(lock),
        },
    }
}

/// Write the file beside `path` and rename it over `path` once complete;
/// on any failure the staged file goes and `path` stays as it was.
pub(super) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let temp = crate::paths::staging_path(path);
    let written = write_synced(&temp, bytes)
        .and_then(|()| std::fs::rename(&temp, path).context("replacing the file"));
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
        assert!(why.contains("replacing the file"), "{why}");
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

    fn manifest_with(tables: &[&str]) -> Project {
        project::parse(&format!("{MANIFEST}{}", tables.join("\n"))).unwrap()
    }

    const DISTRIBUTION: &str = "[distribution]\ntop-level-library = \"Lib.lvlib\"\n";
    const APPLICATION: &str = "[application]\nproject = \"a.lvproj\"\nbuild-spec = \"S\"\n";

    fn kinds_of(tables: &[&str], app: bool, package: bool, no_check: bool) -> Result<Kinds> {
        kinds(&manifest_with(tables), app, package, no_check)
    }

    fn only(package: bool, app: bool) -> Kinds {
        Kinds { package, app }
    }

    #[test]
    fn with_no_flag_every_table_the_manifest_has_is_built() {
        let both = [DISTRIBUTION, APPLICATION];
        let none = kinds_of(&both, false, false, false);
        assert_eq!(none.unwrap(), only(true, true));
        let library = kinds_of(&[DISTRIBUTION], false, false, false);
        assert_eq!(library.unwrap(), only(true, false));
        let application = kinds_of(&[APPLICATION], false, false, false);
        assert_eq!(application.unwrap(), only(false, true));
    }

    #[test]
    fn a_flag_builds_exactly_what_it_names() {
        let both = [DISTRIBUTION, APPLICATION];
        assert_eq!(
            kinds_of(&both, true, false, false).unwrap(),
            only(false, true)
        );
        assert_eq!(
            kinds_of(&both, false, true, false).unwrap(),
            only(true, false)
        );
        assert_eq!(
            kinds_of(&both, true, true, false).unwrap(),
            kinds_of(&both, false, false, false).unwrap(),
            "both flags are the same as none, for a manifest with both tables"
        );
    }

    #[test]
    fn a_manifest_with_neither_table_has_nothing_to_build() {
        let why = kinds_of(&[], false, false, false).unwrap_err();
        assert_eq!(
            why.to_string(),
            "nothing to build: lvpm.toml has no [distribution] or [application]"
        );
    }

    #[test]
    fn a_flag_for_a_table_the_manifest_lacks_names_it_and_the_app_is_asked_first() {
        let said = |tables: &[&str], app, package| {
            kinds_of(tables, app, package, false)
                .unwrap_err()
                .to_string()
        };
        let app = "--app needs [application] in lvpm.toml (see docs/manifest.md)";
        let package = "--package needs [distribution] in lvpm.toml (see docs/manifest.md)";
        assert_eq!(said(&[DISTRIBUTION], true, false), app);
        assert_eq!(said(&[APPLICATION], false, true), package);
        assert_eq!(said(&[DISTRIBUTION], true, true), app);
        assert_eq!(said(&[APPLICATION], true, true), package);
        assert_eq!(said(&[], true, false), app);
    }

    #[test]
    fn no_check_needs_a_package_to_build() {
        let both = [DISTRIBUTION, APPLICATION];
        let why = kinds_of(&[APPLICATION], false, false, true).unwrap_err();
        assert_eq!(
            why.to_string(),
            "--no-check applies only to the package build, and this run builds none"
        );
        assert!(kinds_of(&both, true, false, true).is_err());
        assert!(kinds_of(&both, false, true, true).is_ok());
        assert!(kinds_of(&both, false, false, true).is_ok());
    }

    mod run {
        use super::super::*;
        use crate::ops::build_app::fixture::{Answer, Fake, Folder};
        use crate::ops::build_app::{AppError, AppRefusal};
        use crate::ops::testing;

        const BOTH: Kinds = Kinds {
            package: true,
            app: true,
        };

        fn answering(folder: &Folder) -> Fake {
            Fake::answering(Answer::Builds(vec![folder.builds().join("Main.exe")]))
                .watching(folder.dist().join("demo_app-1.2.3.lvpkg"))
        }

        #[test]
        fn both_kinds_give_a_package_and_the_applications_sbom() {
            let folder = Folder::new("run-both");
            let fake = answering(&folder);
            let outcomes = run(&testing::env(&folder.root), BOTH, true, &fake, |_| {}).unwrap();
            let [Outcome::Built(package), Outcome::AppBuilt(app)] = outcomes.as_slice() else {
                panic!("expected the package, then the application: {outcomes:?}");
            };
            assert_eq!(package.sbom, Sbom::Included);
            assert!(package.path.is_file());
            assert_eq!(app.sbom, folder.builds().join("bom.cdx.json"));
            assert!(app.sbom.is_file());
            folder.finish();
        }

        #[test]
        fn the_package_is_written_and_shown_before_labview_starts() {
            let folder = Folder::new("run-order");
            let fake = answering(&folder);
            run(&testing::env(&folder.root), BOTH, true, &fake, |outcome| {
                assert!(matches!(outcome, Outcome::Built(_)));
                fake.note("package");
            })
            .unwrap();
            let log = fake.log();
            assert_eq!(log.order, ["package", "start"]);
            assert_eq!(log.watched_at_start, [true], "the package was there");
            folder.finish();
        }

        #[test]
        fn a_package_alone_starts_no_labview() {
            let folder = Folder::new("run-package-only");
            let fake = answering(&folder);
            let package_only = Kinds {
                package: true,
                app: false,
            };
            let env = testing::env(&folder.root);
            let outcomes = run(&env, package_only, true, &fake, |_| {}).unwrap();
            assert!(matches!(outcomes.as_slice(), [Outcome::Built(_)]));
            assert_eq!(fake.log().starts, 0);
            assert!(!folder.builds().exists());
            folder.finish();
        }

        #[test]
        fn an_application_alone_writes_no_package() {
            let folder = Folder::new("run-app-only");
            let fake = answering(&folder);
            let app_only = Kinds {
                package: false,
                app: true,
            };
            let env = testing::env(&folder.root);
            let outcomes = run(&env, app_only, false, &fake, |_| {
                panic!("no package was built")
            })
            .unwrap();
            assert!(matches!(outcomes.as_slice(), [Outcome::AppBuilt(_)]));
            assert!(!folder.dist().exists());
            folder.finish();
        }

        #[test]
        fn a_refused_package_ends_the_run_before_labview_starts() {
            let folder = Folder::new("run-refused");
            let fake = answering(&folder);
            // `A.vi` is text, so the check refuses it.
            let outcomes = run(&testing::env(&folder.root), BOTH, false, &fake, |_| {
                panic!("a refused package is not shown as built")
            })
            .unwrap();
            assert!(
                matches!(outcomes.as_slice(), [Outcome::Refused(_)]),
                "{outcomes:?}"
            );
            assert_eq!(fake.log().starts, 0);
            assert!(!folder.dist().exists());
            folder.finish();
        }

        #[test]
        fn an_application_that_is_not_ready_writes_no_package() {
            let folder = Folder::new("run-not-ready");
            folder.remove_lock();
            let fake = answering(&folder);
            let why = run(&testing::env(&folder.root), BOTH, true, &fake, |_| {
                panic!("nothing was built")
            })
            .unwrap_err();
            assert!(
                matches!(why, RunError::App(AppError::Refused(AppRefusal::NoLock))),
                "{why:?}"
            );
            assert!(!folder.dist().exists());
            assert_eq!(fake.log().starts, 0);
            folder.finish();
        }

        #[test]
        fn a_venv_bound_to_a_labview_that_is_not_here_writes_no_package() {
            let folder = Folder::new("run-not-here");
            let fake = answering(&folder);
            let mut env = testing::env(&folder.root);
            env.targets.clear();
            let why = run(&env, BOTH, true, &fake, |_| panic!("nothing was built")).unwrap_err();
            assert!(why.to_string().contains("not installed here"), "{why}");
            assert!(!folder.dist().exists());
            assert_eq!(fake.log().starts, 0);
            folder.finish();
        }

        #[test]
        fn an_application_that_fails_leaves_the_package_it_showed() {
            let folder = Folder::new("run-app-fails");
            let fake = Fake::answering(Answer::LabViewFails(1003, "no"));
            let mut shown = 0;
            let env = testing::env(&folder.root);
            let why = run(&env, BOTH, true, &fake, |_| shown += 1).unwrap_err();
            assert!(
                matches!(why, RunError::App(AppError::LabView { .. })),
                "{why:?}"
            );
            assert_eq!(shown, 1);
            assert!(folder.dist().join("demo_app-1.2.3.lvpkg").is_file());
            folder.finish();
        }

        #[test]
        fn a_package_that_cannot_be_built_is_the_runs_error_and_starts_no_labview() {
            let folder = Folder::new("run-package-error");
            let manifest = folder.read("lvpm.toml");
            folder.write("lvpm.toml", &manifest.replace("Lib.lvlib", "Missing.lvlib"));
            let fake = answering(&folder);
            let why = run(&testing::env(&folder.root), BOTH, true, &fake, |_| {}).unwrap_err();
            assert!(matches!(why, RunError::Package(_)), "{why:?}");
            assert_eq!(fake.log().starts, 0);
            folder.finish();
        }
    }
}
