//! `build --app`: build the application `[application]` names, in the LabVIEW
//! of the project's venv, and put the SBOM next to the build.
//!
//! Nothing starts LabVIEW until [`check_app`] is satisfied: the manifest's
//! `[application]`, a current lock, a venv, and a venv that holds what the
//! lock lists. A refusal costs no LabVIEW start and writes nothing. The
//! LabVIEW itself is behind [`AppLauncher`] and [`AppSession`], which a test
//! replaces.

use super::build::{LockProblem, current_lock, write_atomically};
use super::env::OpEnv;
use super::sync::{Selection, plan_sync};
use crate::application::{self, Application};
use crate::events::{EventSink, SyncStep};
use crate::install;
use crate::lockfile::{Lockfile, StaleReason};
use crate::lvpkg::SBOM;
use crate::sbom::{self, Kind, Subject};
use crate::venv::{self, Venv};
use anyhow::{Context, anyhow};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The build target of every application build; the only one this version builds.
pub const TARGET: &str = "My Computer";

/// Starts the LabVIEW an application is built in. The seam the tests replace.
pub trait AppLauncher {
    /// A LabVIEW on the venv, ready to build. Dropping the session closes it.
    fn start(
        &self,
        venv: &Venv,
        headless: bool,
        sink: &dyn EventSink,
    ) -> anyhow::Result<Box<dyn AppSession>>;
}

/// A LabVIEW an application is built in.
pub trait AppSession {
    /// Run the build specification `build_spec` of the LabVIEW project
    /// `project` for `target`, to the end. The files LabVIEW generated, the
    /// application first.
    fn build(
        &mut self,
        project: &Path,
        target: &str,
        build_spec: &str,
    ) -> Result<Vec<PathBuf>, AppBuildError>;
}

/// LabVIEW's own error: what its error cluster held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LvError {
    pub code: i32,
    pub message: String,
}

/// Why a [`AppSession::build`] built nothing.
#[derive(Debug, thiserror::Error)]
pub enum AppBuildError {
    /// LabVIEW ran the build and reported an error.
    #[error("{} {}", .0.code, .0.message)]
    LabView(LvError),
    /// The build did not run to LabVIEW's answer: the connection, the VI.
    #[error(transparent)]
    Other(anyhow::Error),
}

/// Why the venv or the lock does not let the application build. Said before
/// any LabVIEW is started; the front end words what to run.
#[derive(Debug, thiserror::Error)]
pub enum AppRefusal {
    #[error("there is no lvpm.lock")]
    NoLock,
    #[error("lvpm.lock cannot be read: {0}")]
    UnreadableLock(String),
    #[error("lvpm.lock is out of date")]
    StaleLock(StaleReason),
    #[error("there is no venv in {}", .0.display())]
    NoVenv(PathBuf),
    /// The first change a sync would make to the venv.
    #[error("the venv differs from lvpm.lock")]
    VenvDiffers(SyncStep),
}

impl From<LockProblem> for AppRefusal {
    fn from(problem: LockProblem) -> AppRefusal {
        match problem {
            LockProblem::Missing => AppRefusal::NoLock,
            LockProblem::Unreadable(reason) => AppRefusal::UnreadableLock(reason),
            LockProblem::Stale(reason) => AppRefusal::StaleLock(reason),
        }
    }
}

/// Why an application was not built.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The project is not ready; no LabVIEW was started.
    #[error(transparent)]
    Refused(#[from] AppRefusal),
    /// LabVIEW `year`, the venv's, could not be started.
    #[error("{cause:#}")]
    Launch { year: u32, cause: anyhow::Error },
    /// LabVIEW ran the build specification and it failed.
    #[error("LabVIEW could not build \"{build_spec}\": {} {}", .error.code, .error.message)]
    LabView { build_spec: String, error: LvError },
    /// Anything else that went wrong: its own text.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// What [`check_app`] found ready.
#[derive(Debug)]
pub struct Ready {
    pub application: Application,
    /// The lock the SBOM lists, current for the manifest.
    pub lock: Lockfile,
    /// The project folder, with the venv in it.
    pub root: PathBuf,
}

/// What an application build wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct AppBuilt {
    pub build_spec: String,
    /// What LabVIEW generated, the application first.
    pub outputs: Vec<PathBuf>,
    /// The SBOM, in the folder of the first output.
    pub sbom: PathBuf,
    /// From before LabVIEW started to the SBOM written.
    pub took: Duration,
}

/// The packages the venv must hold: those `[dependencies]` needs. A dev or
/// group package may stay, since the SBOM does not list it; one the lock does
/// not list may not.
fn wanted_in_the_venv() -> Selection {
    Selection {
        no_dev: true,
        groups: Vec::new(),
        keep_listed: true,
    }
}

/// Every precondition of an application build, in order, with no LabVIEW
/// started and nothing written.
pub fn check_app(env: &OpEnv) -> Result<Ready, AppError> {
    let (root, project) = super::project_of(env, "build")?;
    let application = application::validate(&project, &root)?;
    let lock = current_lock(&root, &project).map_err(AppRefusal::from)?;
    // Tested here, before `venv::locate` can word it as a manifest with no venv.
    if !venv::binding_path(&root.join(&env.config.venv_dir)).is_file() {
        return Err(AppRefusal::NoVenv(root).into());
    }
    let bound = venv::binding(&root, &env.config.venv_dir)?;
    let installed = install::list_installed(&bound.store())?;
    let plan = plan_sync(&lock, &installed, &wanted_in_the_venv());
    if let Some(step) = plan.steps.into_iter().next() {
        return Err(AppRefusal::VenvDiffers(step).into());
    }
    Ok(Ready {
        application,
        lock,
        root,
    })
}

/// Check, then build the application in the venv's LabVIEW and write
/// `bom.cdx.json` next to its first output. A build that fails writes no SBOM
/// and leaves an earlier one as it was; LabVIEW is closed either way.
pub fn build_app(env: &OpEnv, launcher: &dyn AppLauncher) -> Result<AppBuilt, AppError> {
    let Ready {
        application,
        lock,
        root,
    } = check_app(env)?;
    let venv = venv::load(&root, &env.config.venv_dir, &env.targets)?;
    env.check_bitness(&venv.target)?;
    let started = Instant::now();
    let year = venv.target.year();
    let mut session = launcher
        .start(&venv, env.headless, env.sink)
        .map_err(|cause| AppError::Launch { year, cause })?;
    let built = session.build(&application.project, TARGET, &application.build_spec);
    drop(session);
    let build_spec = application.build_spec;
    let outputs = built.map_err(|why| match why {
        AppBuildError::LabView(error) => AppError::LabView {
            build_spec: build_spec.clone(),
            error,
        },
        AppBuildError::Other(cause) => AppError::Other(cause),
    })?;
    let Some(first) = outputs.first() else {
        return Err(anyhow!("LabVIEW reported no output for \"{build_spec}\"").into());
    };
    let folder = first.parent().context("the build output has no folder")?;
    let subject = Subject {
        name: application.name,
        version: application.version,
        kind: Kind::Application,
    };
    let sbom = folder.join(SBOM);
    write_atomically(&sbom, sbom::cyclonedx(Some(&lock), subject).as_bytes()).map_err(|cause| {
        anyhow!(
            "the build output is in {}, but {SBOM} could not be written: {cause:#}",
            folder.display()
        )
    })?;
    Ok(AppBuilt {
        build_spec,
        outputs,
        sbom,
        took: started.elapsed(),
    })
}

/// What the tests of the application build and of the run that sequences it
/// share: a project that is ready to build, and a LabVIEW that is not there.
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use crate::lockfile::{FORMAT_VERSION, Inputs, LOCAL_SOURCE, LockedPackage};
    use crate::ops::testing;
    use crate::project;
    use std::sync::{Arc, Mutex};

    pub(crate) const BUILD_SPEC: &str = "Demo App";

    const MANIFEST: &str = r#"[project]
name = "demo_app"
version = "1.2.3"
labview = ">=2026"

[sources]
defaults = false

[dependencies]
alpha = "1.0"

[distribution]
top-level-library = "Lib.lvlib"
vendor = "Vendor"
display-name = "Demo"

[application]
project = "app/Main.lvproj"
build-spec = "Demo App"
"#;

    /// The SHA-256 every package of the fixture's lock has.
    pub(crate) fn sha() -> String {
        "ab".repeat(32)
    }

    /// A project folder that can be built as a package and as an application:
    /// a manifest with both tables, a current lock of `alpha` 1.0, a venv that
    /// holds it, the `.lvproj`, and a library of one member.
    pub(crate) struct Folder {
        pub root: PathBuf,
        pub venv: Venv,
    }

    impl Folder {
        pub(crate) fn new(tag: &str) -> Folder {
            let root = testing::scratch_dir(tag);
            let venv = testing::venv_in(&root);
            let folder = Folder { root, venv };
            folder.write("lvpm.toml", MANIFEST);
            folder.write("app/Main.lvproj", "<Project/>");
            folder.write(
                "Lib.lvlib",
                "<Library>\n\t<Item Name=\"A.vi\" Type=\"VI\" URL=\"../A.vi\"/>\n</Library>\n",
            );
            folder.write("A.vi", "a vi");
            folder.write("lvpm.lock", &folder.lock_text(&[("alpha", &[])]));
            folder.install("alpha", "1.0", &sha());
            folder
        }

        pub(crate) fn write(&self, rel: &str, text: &str) {
            let path = self.root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        pub(crate) fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.root.join(rel)).unwrap()
        }

        pub(crate) fn manifest(&self) -> project::Project {
            project::parse(&self.read("lvpm.toml")).unwrap()
        }

        /// The lock `lvpm lock` would write: each `(name, groups)` at 1.0.
        pub(crate) fn lock_text(&self, packages: &[(&str, &[&str])]) -> String {
            let lock = Lockfile {
                version: FORMAT_VERSION,
                labview: "2026".into(),
                inputs: Inputs::of(&self.manifest()),
                packages: packages
                    .iter()
                    .map(|(name, groups)| LockedPackage {
                        name: name.to_string(),
                        version: "1.0".into(),
                        sha256: sha(),
                        md5: None,
                        source: LOCAL_SOURCE.into(),
                        requires: Vec::new(),
                        groups: groups.iter().map(|g| g.to_string()).collect(),
                        signer: None,
                        subject: None,
                    })
                    .collect(),
                unresolved: Vec::new(),
            };
            lock.to_toml().unwrap()
        }

        /// The record an install of `name` leaves in the venv.
        pub(crate) fn install(&self, name: &str, version: &str, sha256: &str) {
            let record = serde_json::json!({
                "name": name, "version": version, "installed_at": 0, "target": "",
                "files": [], "skipped_hooks": [], "archive_sha256": sha256,
            });
            let path = self.venv.store().store_dir().join(format!("{name}.json"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, record.to_string()).unwrap();
        }

        pub(crate) fn remove_lock(&self) {
            std::fs::remove_file(self.root.join("lvpm.lock")).unwrap();
        }

        /// Where the fake LabVIEW's build goes.
        pub(crate) fn builds(&self) -> PathBuf {
            self.root.join("builds").join("app")
        }

        pub(crate) fn dist(&self) -> PathBuf {
            self.root.join("dist")
        }

        pub(crate) fn finish(self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    /// What the fake LabVIEW does.
    #[derive(Clone)]
    pub(crate) enum Answer {
        /// Generates these files, the first being the application.
        Builds(Vec<PathBuf>),
        /// Reports these files without generating them.
        ReportsOnly(Vec<PathBuf>),
        /// Reports an error in LabVIEW's own words.
        LabViewFails(i32, &'static str),
        /// The build VI cannot be run.
        ConnectionLost(&'static str),
        /// Cannot be started.
        WontStart(&'static str),
    }

    #[derive(Debug, Default, Clone)]
    pub(crate) struct Log {
        pub starts: usize,
        pub closes: usize,
        pub headless: Vec<bool>,
        /// Each build asked for: project, target, build spec.
        pub builds: Vec<(PathBuf, String, String)>,
        /// What happened, in order: `start`, and what a test noted.
        pub order: Vec<&'static str>,
        /// Whether the watched path was there when LabVIEW started.
        pub watched_at_start: Vec<bool>,
    }

    #[derive(Clone)]
    pub(crate) struct Fake {
        log: Arc<Mutex<Log>>,
        answer: Answer,
        watched: Option<PathBuf>,
    }

    impl Fake {
        pub(crate) fn answering(answer: Answer) -> Fake {
            Fake {
                log: Arc::default(),
                answer,
                watched: None,
            }
        }

        pub(crate) fn watching(self, path: PathBuf) -> Fake {
            Fake {
                watched: Some(path),
                ..self
            }
        }

        pub(crate) fn log(&self) -> Log {
            self.log.lock().unwrap().clone()
        }

        /// Something other than the launcher happened.
        pub(crate) fn note(&self, what: &'static str) {
            self.log.lock().unwrap().order.push(what);
        }
    }

    impl AppLauncher for Fake {
        fn start(
            &self,
            _venv: &Venv,
            headless: bool,
            _sink: &dyn EventSink,
        ) -> anyhow::Result<Box<dyn AppSession>> {
            let mut log = self.log.lock().unwrap();
            log.starts += 1;
            log.order.push("start");
            log.headless.push(headless);
            if let Some(path) = &self.watched {
                log.watched_at_start.push(path.exists());
            }
            if let Answer::WontStart(why) = self.answer {
                anyhow::bail!(why);
            }
            Ok(Box::new(Session { fake: self.clone() }))
        }
    }

    struct Session {
        fake: Fake,
    }

    impl AppSession for Session {
        fn build(
            &mut self,
            project: &Path,
            target: &str,
            build_spec: &str,
        ) -> Result<Vec<PathBuf>, AppBuildError> {
            self.fake.log.lock().unwrap().builds.push((
                project.to_path_buf(),
                target.to_string(),
                build_spec.to_string(),
            ));
            match &self.fake.answer {
                Answer::Builds(files) => {
                    for file in files {
                        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                        std::fs::write(file, "built").unwrap();
                    }
                    Ok(files.clone())
                }
                Answer::ReportsOnly(files) => Ok(files.clone()),
                Answer::LabViewFails(code, message) => Err(AppBuildError::LabView(LvError {
                    code: *code,
                    message: message.to_string(),
                })),
                Answer::ConnectionLost(why) => Err(AppBuildError::Other(anyhow!(*why))),
                Answer::WontStart(_) => unreachable!("never started"),
            }
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            self.fake.log.lock().unwrap().closes += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{Answer, BUILD_SPEC, Fake, Folder, sha};
    use super::*;
    use crate::ops::testing;

    fn outputs(folder: &Folder) -> Vec<PathBuf> {
        vec![
            folder.builds().join("Main.exe"),
            folder.builds().join("data"),
        ]
    }

    #[test]
    fn a_ready_project_builds_and_the_sbom_lands_next_to_the_first_output() {
        let folder = Folder::new("app-builds");
        let fake = Fake::answering(Answer::Builds(outputs(&folder)));
        let built = build_app(&testing::env(&folder.root), &fake).unwrap();

        assert_eq!(built.build_spec, BUILD_SPEC);
        assert_eq!(built.outputs, outputs(&folder));
        assert_eq!(built.sbom, folder.builds().join("bom.cdx.json"));
        let text = std::fs::read_to_string(&built.sbom).unwrap();
        let lock = Lockfile::read(&folder.root.join("lvpm.lock")).unwrap();
        let subject = Subject {
            name: "demo_app".into(),
            version: "1.2.3".into(),
            kind: Kind::Application,
        };
        assert_eq!(text, sbom::cyclonedx(Some(&lock), subject));
        assert!(text.contains("\"type\": \"application\""), "{text}");
        assert!(text.contains("\"name\": \"alpha\""), "{text}");
        let log = fake.log();
        assert_eq!((log.starts, log.closes), (1, 1));
        assert_eq!(
            log.builds,
            [(
                folder.root.join("app").join("Main.lvproj"),
                TARGET.to_string(),
                BUILD_SPEC.to_string()
            )]
        );
        folder.finish();
    }

    #[test]
    fn the_headless_flag_reaches_the_launcher() {
        let folder = Folder::new("app-headless");
        let fake = Fake::answering(Answer::Builds(outputs(&folder)));
        let mut env = testing::env(&folder.root);
        env.headless = true;
        build_app(&env, &fake).unwrap();
        assert_eq!(fake.log().headless, [true]);
        folder.finish();
    }

    #[test]
    fn each_refusal_starts_no_labview_and_says_why() {
        type Break = fn(&Folder);
        let cases: [(&str, Break, &str); 8] = [
            ("no lock", |f| f.remove_lock(), "there is no lvpm.lock"),
            (
                "stale lock",
                |f| {
                    f.write(
                        "lvpm.lock",
                        &f.lock_text(&[("alpha", &[])]).replace("2026", "2025"),
                    )
                },
                "lvpm.lock is out of date",
            ),
            (
                "unreadable lock",
                |f| f.write("lvpm.lock", "not a lock = ="),
                "lvpm.lock cannot be read",
            ),
            (
                "no venv",
                |f| std::fs::remove_dir_all(&f.venv.dir).unwrap(),
                "there is no venv in",
            ),
            (
                "venv lacks a locked package",
                |f| std::fs::remove_dir_all(f.venv.store().store_dir()).unwrap(),
                "the venv differs from lvpm.lock",
            ),
            (
                "venv has another version",
                |f| f.install("alpha", "0.9", &sha()),
                "the venv differs from lvpm.lock",
            ),
            (
                "venv has another archive",
                |f| f.install("alpha", "1.0", &"cd".repeat(32)),
                "the venv differs from lvpm.lock",
            ),
            (
                "venv has a package the lock does not list",
                |f| f.install("stray", "1.0", &sha()),
                "the venv differs from lvpm.lock",
            ),
        ];
        for (n, (what, break_it, said)) in cases.into_iter().enumerate() {
            let folder = Folder::new(&format!("app-refused-{n}"));
            break_it(&folder);
            let fake = Fake::answering(Answer::Builds(outputs(&folder)));
            let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
            assert!(matches!(why, AppError::Refused(_)), "{what}: {why:?}");
            assert!(why.to_string().starts_with(said), "{what}: {why}");
            assert_eq!(fake.log().starts, 0, "{what}");
            assert!(!folder.builds().exists(), "{what}");
            folder.finish();
        }
    }

    #[test]
    fn the_first_difference_a_sync_would_make_is_what_the_refusal_carries() {
        let folder = Folder::new("app-first-step");
        folder.install("alpha", "0.9", &sha());
        let Err(AppError::Refused(AppRefusal::VenvDiffers(step))) =
            check_app(&testing::env(&folder.root))
        else {
            panic!("expected a venv that differs");
        };
        assert_eq!(
            step,
            SyncStep::Replace {
                name: "alpha".into(),
                from: "0.9".into(),
                to: "1.0".into()
            }
        );
        folder.finish();
    }

    #[test]
    fn a_dev_package_in_the_venv_is_accepted_and_a_group_one_too() {
        let folder = Folder::new("app-dev");
        folder.write(
            "lvpm.lock",
            &folder.lock_text(&[("alpha", &[]), ("gamma", &["dev"]), ("delta", &["ci"])]),
        );
        folder.install("gamma", "1.0", &sha());
        folder.install("delta", "1.0", &sha());
        let ready = check_app(&testing::env(&folder.root)).unwrap();
        assert_eq!(ready.application.build_spec, BUILD_SPEC);
        folder.finish();
    }

    #[test]
    fn a_bad_application_table_is_refused_before_the_lock_and_the_venv_are_read() {
        let folder = Folder::new("app-table");
        let manifest = folder.read("lvpm.toml");
        folder.write(
            "lvpm.toml",
            &manifest.replace("app/Main.lvproj", "app/Missing.lvproj"),
        );
        folder.remove_lock();
        let why = check_app(&testing::env(&folder.root)).unwrap_err();
        assert_eq!(
            why.to_string(),
            "[application] project \"app/Missing.lvproj\" is not a file"
        );
        folder.finish();
    }

    #[test]
    fn labviews_error_fails_the_build_writes_no_sbom_and_closes_labview() {
        let folder = Folder::new("app-lv-error");
        let fake = Fake::answering(Answer::LabViewFails(1003, "The VI is not executable"));
        let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
        assert_eq!(
            why.to_string(),
            "LabVIEW could not build \"Demo App\": 1003 The VI is not executable"
        );
        assert!(matches!(why, AppError::LabView { .. }));
        assert!(!folder.builds().exists());
        assert_eq!((fake.log().starts, fake.log().closes), (1, 1));
        folder.finish();
    }

    #[test]
    fn a_failed_build_leaves_the_previous_sbom_as_it_was() {
        let folder = Folder::new("app-previous");
        folder.write("builds/app/bom.cdx.json", "the previous SBOM");
        let fake = Fake::answering(Answer::LabViewFails(1003, "no"));
        build_app(&testing::env(&folder.root), &fake).unwrap_err();
        assert_eq!(folder.read("builds/app/bom.cdx.json"), "the previous SBOM");
        folder.finish();
    }

    #[test]
    fn a_lost_connection_is_not_a_labview_error_and_still_closes_labview() {
        let folder = Folder::new("app-lost");
        let fake = Fake::answering(Answer::ConnectionLost("connection lost"));
        let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
        assert!(matches!(why, AppError::Other(_)), "{why:?}");
        let text = why.to_string();
        assert!(text.contains("connection lost"), "{text}");
        assert!(!text.contains("could not build"), "{text}");
        assert!(!folder.builds().exists());
        assert_eq!(fake.log().closes, 1);
        folder.finish();
    }

    #[test]
    fn labview_reporting_no_output_is_an_error() {
        let folder = Folder::new("app-no-output");
        let fake = Fake::answering(Answer::ReportsOnly(Vec::new()));
        let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
        assert_eq!(
            why.to_string(),
            "LabVIEW reported no output for \"Demo App\""
        );
        assert_eq!(fake.log().closes, 1);
        folder.finish();
    }

    #[test]
    fn a_labview_that_cannot_start_is_a_launch_failure_naming_the_venvs_year() {
        let folder = Folder::new("app-wont-start");
        let fake = Fake::answering(Answer::WontStart("LabVIEW 2026 not found"));
        let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
        assert!(
            matches!(&why, AppError::Launch { year: 2026, .. }),
            "{why:?}"
        );
        assert_eq!(why.to_string(), "LabVIEW 2026 not found");
        assert!(!folder.builds().exists());
        folder.finish();
    }

    #[test]
    fn an_sbom_that_cannot_be_written_says_where_the_build_output_is() {
        let folder = Folder::new("app-sbom-fails");
        let fake = Fake::answering(Answer::ReportsOnly(outputs(&folder)));
        let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
        let text = why.to_string();
        let said = format!(
            "the build output is in {}, but bom.cdx.json could not be written: ",
            folder.builds().display()
        );
        assert!(text.starts_with(&said), "{text}");
        folder.finish();
    }

    #[test]
    fn a_venv_bound_to_a_labview_that_is_not_here_starts_no_labview() {
        let folder = Folder::new("app-not-here");
        let fake = Fake::answering(Answer::Builds(outputs(&folder)));
        let mut env = testing::env(&folder.root);
        env.targets.clear();
        let why = build_app(&env, &fake).unwrap_err();
        assert!(why.to_string().contains("not installed here"), "{why}");
        assert_eq!(fake.log().starts, 0);
        folder.finish();
    }
}
