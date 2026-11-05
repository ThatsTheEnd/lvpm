//! `build --app`: build the application `[application]` names, in the LabVIEW
//! of the project's venv, and put the SBOM next to the build.
//!
//! Nothing starts LabVIEW until [`check_app`] is satisfied: the manifest's
//! `[application]`, a current lock, a venv, and a venv that holds what the
//! lock lists, in a LabVIEW that is installed here. A refusal costs no
//! LabVIEW start and writes nothing, and a run that also builds the package
//! refuses before it writes that. The
//! LabVIEW itself is behind [`AppLauncher`] and [`AppSession`], which a test
//! replaces.

use super::build::{LockProblem, current_lock, write_atomically};
use super::env::OpEnv;
use super::sync::{Selection, plan_sync};
use crate::application::{self, Application};
use crate::events::{Event, EventSink, SyncStep};
use crate::install;
use crate::launch::{self, VenvLabview};
use crate::lockfile::{Lockfile, StaleReason};
use crate::lvpkg::SBOM;
use crate::relink;
use crate::sbom::{self, Kind, Subject};
use crate::venv::{self, Venv};
use crate::viserver::{Connection, LvValue, VIRef};
use anyhow::{Context, anyhow, bail};
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
    /// The venv's LabVIEW cannot open the build VI.
    #[error(
        "the application build needs LabVIEW {MIN_LABVIEW_YEAR} or later; this venv uses LabVIEW {year}"
    )]
    LabviewTooOld { year: u32 },
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
    /// The venv, bound to a LabVIEW that is installed here.
    pub venv: Venv,
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
/// started and nothing written: the last are that the venv's LabVIEW is
/// installed here, of the bitness `--labview-bitness` asks for, new enough to
/// open the build VI, and not already running.
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
    let venv = venv::load(&root, &env.config.venv_dir, &env.targets)?;
    env.check_bitness(&venv.target)?;
    let year = venv.target.year();
    if year < MIN_LABVIEW_YEAR {
        return Err(AppRefusal::LabviewTooOld { year }.into());
    }
    launch::refuse_running(&venv)?;
    Ok(Ready {
        application,
        lock,
        root,
        venv,
    })
}

/// Check, then build: [`build_ready`].
pub fn build_app(env: &OpEnv, launcher: &dyn AppLauncher) -> Result<AppBuilt, AppError> {
    build_ready(env, check_app(env)?, launcher)
}

/// Build the application of `ready` in the venv's LabVIEW and write
/// `bom.cdx.json` next to its first output. A build that fails writes no SBOM
/// and leaves an earlier one as it was; LabVIEW is closed either way.
pub fn build_ready(
    env: &OpEnv,
    ready: Ready,
    launcher: &dyn AppLauncher,
) -> Result<AppBuilt, AppError> {
    let Ready {
        application,
        lock,
        venv,
        ..
    } = ready;
    let started = Instant::now();
    let year = venv.target.year();
    env.sink.emit(Event::AppBuildStarting {
        build_spec: application.build_spec.clone(),
        labview: year,
    });
    let mut session = launcher
        .start(&venv, env.headless, env.sink)
        .map_err(|cause| match cause.downcast_ref::<launch::Unstartable>() {
            // Only a LabVIEW that is missing or will not start is put right by rebinding the venv.
            Some(_) => AppError::Launch { year, cause },
            None => AppError::Other(cause),
        })?;
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

/// The wrapper VI, embedded so the executable needs no companion file. It
/// calls NI's `BuildTargetBuildSpecification.vi`, whose error cluster lvpm
/// cannot read over VI Server, and flattens its answer into the controls and
/// indicators below. Saved in LabVIEW 2026.
const BUILD_VI_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/lv-src/build-app.vi"
));

/// The oldest LabVIEW that opens the wrapper VI: the one it was saved in.
pub const MIN_LABVIEW_YEAR: u32 = 2026;

/// The wrapper's front panel, by label.
const PROJECT_CONTROL: &str = "project path";
const BUILD_SPEC_CONTROL: &str = "build spec";
const OUTPUTS_INDICATOR: &str = "outputs";
const CODE_INDICATOR: &str = "error code";
const MESSAGE_INDICATOR: &str = "error message";
/// False when the VI starts, true as its last act.
const DONE_INDICATOR: &str = "Done";

/// How long to wait between two looks at `Done`.
const POLL: Duration = Duration::from_millis(500);

/// Starts the LabVIEW of the venv, fresh, and builds in it over VI Server.
pub struct VenvAppLauncher {
    /// Where the wrapper VI is written so LabVIEW can open it by path.
    work_dir: PathBuf,
}

impl VenvAppLauncher {
    pub fn new(work_dir: PathBuf) -> VenvAppLauncher {
        VenvAppLauncher { work_dir }
    }
}

impl AppLauncher for VenvAppLauncher {
    fn start(
        &self,
        venv: &Venv,
        headless: bool,
        sink: &dyn EventSink,
    ) -> anyhow::Result<Box<dyn AppSession>> {
        // Before LabVIEW starts, so a folder that cannot be written costs no start.
        let vi = materialize(&self.work_dir)?;
        let labview = launch::connect_venv(venv, headless, sink)?;
        Ok(Box::new(VenvAppSession { labview, vi }))
    }
}

/// Write the wrapper VI into `dir`, as the relink VI is.
fn materialize(dir: &Path) -> anyhow::Result<PathBuf> {
    relink::materialize_vi(dir, "lvpm-build-app", BUILD_VI_BYTES)
}

/// A LabVIEW lvpm talks to over VI Server. Dropping it closes it, whether the
/// build worked or not.
struct VenvAppSession {
    labview: VenvLabview,
    vi: PathBuf,
}

impl AppSession for VenvAppSession {
    fn build(
        &mut self,
        project: &Path,
        target: &str,
        build_spec: &str,
    ) -> Result<Vec<PathBuf>, AppBuildError> {
        let (project, build_spec) = wrapper_inputs(project, target, build_spec)?;
        let connection = self.labview.connection();
        let vi = connection
            .open_vi_reference(&self.vi)
            .with_context(|| format!("opening {}", self.vi.display()))
            .map_err(AppBuildError::Other)?;
        let answer = run_wrapper(connection, vi, &project, &build_spec);
        let _ = connection.release(vi);
        let (code, message, outputs) = answer.map_err(AppBuildError::Other)?;
        wrapper_outputs(code, message, &outputs)
    }
}

/// What the wrapper's two controls are set to. Its target is fixed, so another
/// one is refused rather than ignored.
fn wrapper_inputs(
    project: &Path,
    target: &str,
    build_spec: &str,
) -> Result<(String, String), AppBuildError> {
    if target != TARGET {
        return Err(AppBuildError::Other(anyhow!(
            "the build VI builds the {TARGET} target only, not \"{target}\""
        )));
    }
    // Native separators: LabVIEW splits a Windows path on backslashes only.
    Ok((relink::lv_folder(project), build_spec.to_string()))
}

/// Set the wrapper's inputs, run it, wait for `Done`, and read its answer:
/// the error code, the error message and the outputs string, which
/// `wrapper_outputs` splits one path per line.
fn run_wrapper(
    connection: &mut Connection,
    vi: VIRef,
    project: &str,
    build_spec: &str,
) -> anyhow::Result<(i32, String, String)> {
    connection.ctrl_val_set(vi, PROJECT_CONTROL, LvValue::Str(project.to_string()))?;
    connection.ctrl_val_set(vi, BUILD_SPEC_CONTROL, LvValue::Str(build_spec.to_string()))?;
    connection.ctrl_val_set(vi, DONE_INDICATOR, LvValue::Bool(false))?;
    connection.run_vi_async(vi)?;
    // No overall limit: a build runs until LabVIEW says it is done.
    // shortcut: each poll must answer within CALL_TIMEOUT (180 s); if App Builder blocks VI Server longer, the build fails as Other. Loop on timeout once a real App Builder run shows that happens.
    loop {
        std::thread::sleep(POLL);
        if want_bool(DONE_INDICATOR, connection.ctrl_val_get(vi, DONE_INDICATOR)?)? {
            break;
        }
    }
    let code = want_i32(CODE_INDICATOR, connection.ctrl_val_get(vi, CODE_INDICATOR)?)?;
    let message = want_str(
        MESSAGE_INDICATOR,
        connection.ctrl_val_get(vi, MESSAGE_INDICATOR)?,
    )?;
    let outputs = want_str(
        OUTPUTS_INDICATOR,
        connection.ctrl_val_get(vi, OUTPUTS_INDICATOR)?,
    )?;
    Ok((code, message, outputs))
}

/// What the wrapper's indicator `name` was read back as, if it is a boolean.
fn want_bool(name: &str, value: LvValue) -> anyhow::Result<bool> {
    match value {
        LvValue::Bool(done) => Ok(done),
        other => bail!("{name} read back as {other}, not a boolean"),
    }
}

/// As [`want_bool`], an I32.
fn want_i32(name: &str, value: LvValue) -> anyhow::Result<i32> {
    match value {
        LvValue::I32(code) => Ok(code),
        other => bail!("{name} read back as {other}, not an I32"),
    }
}

/// As [`want_bool`], a string.
fn want_str(name: &str, value: LvValue) -> anyhow::Result<String> {
    match value {
        LvValue::Str(text) => Ok(text),
        other => bail!("{name} read back as {other}, not a string"),
    }
}

/// The wrapper's answer: code 0 is success and `outputs` holds the generated
/// files, one per line; any other code is LabVIEW's error.
fn wrapper_outputs(
    code: i32,
    message: String,
    outputs: &str,
) -> Result<Vec<PathBuf>, AppBuildError> {
    if code != 0 {
        return Err(AppBuildError::LabView(LvError { code, message }));
    }
    Ok(outputs
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect())
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
                        digest: None,
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
    }

    /// Why a fake LabVIEW does not start.
    #[derive(Clone, Copy)]
    pub(crate) enum StartFailure {
        /// The venv's LabVIEW is missing or never answers.
        Unstartable(&'static str),
        /// Anything else that stops a start: a LabVIEW already running, a folder.
        Other(&'static str),
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

    // double-waiver: B1 - the real AppLauncher spawns LabVIEW and opens a VI Server socket
    #[derive(Clone)]
    pub(crate) struct Fake {
        log: Arc<Mutex<Log>>,
        answer: Answer,
        watched: Option<PathBuf>,
        start_fails: Option<StartFailure>,
    }

    impl Fake {
        pub(crate) fn answering(answer: Answer) -> Fake {
            Fake {
                log: Arc::default(),
                answer,
                watched: None,
                start_fails: None,
            }
        }

        /// Makes every start fail, as `why` says.
        pub(crate) fn failing_to_start(self, why: StartFailure) -> Fake {
            Fake {
                start_fails: Some(why),
                ..self
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
            match self.start_fails {
                Some(StartFailure::Unstartable(why)) => {
                    return Err(launch::unstartable(anyhow!(why)));
                }
                Some(StartFailure::Other(why)) => anyhow::bail!(why),
                None => {}
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
    use super::fixture::{Answer, BUILD_SPEC, Fake, Folder, StartFailure, sha};
    use super::*;
    use crate::ops::testing;
    use crate::target::LvTarget;

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

    /// Notes, in the fake's order, that the build was announced.
    struct Announcements<'a>(&'a Fake, testing::Recorded);

    impl EventSink for Announcements<'_> {
        fn emit(&self, event: Event) {
            if matches!(event, Event::AppBuildStarting { .. }) {
                self.0.note("announced");
            }
            self.1.emit(event);
        }
    }

    #[test]
    fn the_build_is_announced_with_the_spec_and_year_just_before_labview_starts() {
        let folder = Folder::new("app-announced");
        let fake = Fake::answering(Answer::Builds(outputs(&folder)));
        let sink = Announcements(&fake, testing::Recorded::default());
        build_app(&testing::env_on(&sink, &folder.root), &fake).unwrap();
        assert_eq!(fake.log().order, ["announced", "start"]);
        assert_eq!(
            sink.1.events(),
            [Event::AppBuildStarting {
                build_spec: BUILD_SPEC.into(),
                labview: 2026
            }]
        );
        folder.finish();
    }

    #[test]
    fn a_refusal_announces_nothing() {
        let folder = Folder::new("app-not-announced");
        folder.remove_lock();
        let fake = Fake::answering(Answer::Builds(outputs(&folder)));
        let sink = testing::Recorded::default();
        build_app(&testing::env_on(&sink, &folder.root), &fake).unwrap_err();
        assert_eq!(sink.events(), []);
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
        type Variant = fn(&AppRefusal) -> bool;
        let differs: Variant = |r| matches!(r, AppRefusal::VenvDiffers(_));
        let cases: [(&str, Break, Variant, &str); 8] = [
            (
                "no lock",
                |f| f.remove_lock(),
                |r| matches!(r, AppRefusal::NoLock),
                "there is no lvpm.lock",
            ),
            (
                "stale lock",
                |f| {
                    f.write(
                        "lvpm.lock",
                        &f.lock_text(&[("alpha", &[])]).replace("2026", "2025"),
                    )
                },
                |r| matches!(r, AppRefusal::StaleLock(_)),
                "lvpm.lock is out of date",
            ),
            (
                "unreadable lock",
                |f| f.write("lvpm.lock", "not a lock = ="),
                |r| matches!(r, AppRefusal::UnreadableLock(_)),
                "lvpm.lock cannot be read",
            ),
            (
                "no venv",
                |f| std::fs::remove_dir_all(&f.venv.dir).unwrap(),
                |r| matches!(r, AppRefusal::NoVenv(_)),
                "there is no venv in",
            ),
            (
                "venv lacks a locked package",
                |f| std::fs::remove_dir_all(f.venv.store().store_dir()).unwrap(),
                differs,
                "the venv differs from lvpm.lock",
            ),
            (
                "venv has another version",
                |f| f.install("alpha", "0.9", &sha()),
                differs,
                "the venv differs from lvpm.lock",
            ),
            (
                "venv has another archive",
                |f| f.install("alpha", "1.0", &"cd".repeat(32)),
                differs,
                "the venv differs from lvpm.lock",
            ),
            (
                "venv has a package the lock does not list",
                |f| f.install("stray", "1.0", &sha()),
                differs,
                "the venv differs from lvpm.lock",
            ),
        ];
        for (n, (what, break_it, is_variant, said)) in cases.into_iter().enumerate() {
            let folder = Folder::new(&format!("app-refused-{n}"));
            break_it(&folder);
            let fake = Fake::answering(Answer::Builds(outputs(&folder)));
            let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
            assert!(
                matches!(&why, AppError::Refused(r) if is_variant(r)),
                "{what}: {why:?}"
            );
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
    fn a_labview_already_on_the_venvs_port_is_refused_before_labview_starts() {
        let folder = Folder::new("app-running");
        let _held = std::net::TcpListener::bind(("127.0.0.1", folder.venv.port)).unwrap();
        let fake = Fake::answering(Answer::Builds(outputs(&folder)));
        let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
        assert!(matches!(&why, AppError::Other(_)), "{why:?}");
        assert!(
            why.to_string()
                .starts_with("a LabVIEW is already running or starting on this venv"),
            "{why}"
        );
        assert_eq!(fake.log().starts, 0);
        folder.finish();
    }

    #[test]
    fn a_venv_without_its_dev_and_group_packages_is_accepted() {
        let folder = Folder::new("app-no-dev");
        folder.write(
            "lvpm.lock",
            &folder.lock_text(&[("alpha", &[]), ("gamma", &["dev"]), ("delta", &["ci"])]),
        );
        check_app(&testing::env(&folder.root)).unwrap();
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
        assert_eq!(why.to_string(), "connection lost");
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
        assert!(!folder.builds().exists());
        assert_eq!(fake.log().closes, 1);
        folder.finish();
    }

    #[test]
    fn a_labview_that_cannot_start_is_a_launch_failure_naming_the_venvs_year() {
        let folder = Folder::new("app-wont-start");
        let fake = Fake::answering(Answer::Builds(outputs(&folder)))
            .failing_to_start(StartFailure::Unstartable("LabVIEW 2026 not found"));
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
    fn a_start_refused_for_another_reason_is_not_a_launch_failure() {
        let folder = Folder::new("app-start-refused");
        let said = "a LabVIEW is already running or starting on this venv (VI Server port 3364) - close it first.";
        let fake = Fake::answering(Answer::Builds(outputs(&folder)))
            .failing_to_start(StartFailure::Other(said));
        let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
        assert!(matches!(&why, AppError::Other(_)), "{why:?}");
        assert_eq!(why.to_string(), said);
        folder.finish();
    }

    #[test]
    fn an_sbom_that_cannot_be_written_says_where_the_build_output_is() {
        let folder = Folder::new("app-sbom-fails");
        // The fake only reports the outputs, so their folder is never made and the SBOM cannot land in it.
        let fake = Fake::answering(Answer::ReportsOnly(outputs(&folder)));
        let why = build_app(&testing::env(&folder.root), &fake).unwrap_err();
        let text = why.to_string();
        let said = format!(
            "the build output is in {}, but bom.cdx.json could not be written: ",
            folder.builds().display()
        );
        assert!(text.starts_with(&said), "{text}");
        assert!(text.len() > said.len(), "the cause follows: {text}");
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

    #[test]
    fn a_venv_of_the_other_bitness_than_asked_for_is_refused_before_labview_starts() {
        let folder = Folder::new("app-bitness");
        let fake = Fake::answering(Answer::Builds(outputs(&folder)));
        let mut env = testing::env(&folder.root);
        env.flags.labview_bitness = Some(32);
        let why = build_app(&env, &fake).unwrap_err();
        assert!(matches!(why, AppError::Other(_)), "{why:?}");
        assert!(
            why.to_string()
                .contains("not the 32-bit LabVIEW --labview-bitness asks for"),
            "{why}"
        );
        assert_eq!(fake.log().starts, 0);
        assert!(!folder.builds().exists());
        folder.finish();
    }

    #[test]
    fn a_venv_of_a_labview_older_than_the_build_vi_is_refused_before_labview_starts() {
        let mut folder = Folder::new("app-too-old");
        let binding = venv::binding_path(&folder.venv.dir);
        let text = std::fs::read_to_string(&binding).unwrap();
        assert!(text.contains("LabVIEW-2026-64bit"), "{text}");
        std::fs::write(
            &binding,
            text.replace("LabVIEW-2026-64bit", "LabVIEW-2025-64bit")
                .replace("26.3", "25.3"),
        )
        .unwrap();
        let old = LvTarget {
            version: 25.3,
            ..testing::labview_2026()
        };
        folder.venv = venv::load(
            &folder.root,
            Path::new(".lv-venv"),
            std::slice::from_ref(&old),
        )
        .unwrap();
        folder.install("alpha", "1.0", &sha());
        let fake = Fake::answering(Answer::Builds(outputs(&folder)));
        let mut env = testing::env(&folder.root);
        env.targets = vec![old];
        let why = build_app(&env, &fake).unwrap_err();
        assert!(
            matches!(
                why,
                AppError::Refused(AppRefusal::LabviewTooOld { year: 2025 })
            ),
            "{why:?}"
        );
        assert_eq!(
            why.to_string(),
            "the application build needs LabVIEW 2026 or later; this venv uses LabVIEW 2025"
        );
        assert_eq!(fake.log().starts, 0);
        folder.finish();
    }

    #[test]
    fn the_embedded_wrapper_is_a_labview_2026_vi_that_has_every_label_lvpm_uses() {
        let vi = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lv-src/build-app.vi");
        assert_eq!(crate::ops::resave::saved_major(&vi), Some(26));
        for label in [
            PROJECT_CONTROL,
            BUILD_SPEC_CONTROL,
            OUTPUTS_INDICATOR,
            CODE_INDICATOR,
            MESSAGE_INDICATOR,
            DONE_INDICATOR,
        ] {
            let found = BUILD_VI_BYTES
                .windows(label.len())
                .any(|window| window == label.as_bytes());
            assert!(found, "the wrapper VI has no label {label:?}");
        }
    }

    #[test]
    fn the_wrapper_is_written_where_asked_and_is_the_embedded_one() {
        let dir = testing::scratch_dir("app-materialize").join("work");
        let vi = materialize(&dir).unwrap();
        assert_eq!(vi.parent(), Some(dir.as_path()));
        assert!(
            vi.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("lvpm-build-app-")
        );
        assert_eq!(std::fs::read(&vi).unwrap(), BUILD_VI_BYTES);
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    #[test]
    fn a_folder_that_cannot_be_written_stops_the_launcher_before_labview_starts() {
        let dir = testing::scratch_dir("app-materialize-fails");
        // A file where the folder should be: nothing can be created in it.
        let blocked = dir.join("file");
        std::fs::write(&blocked, "x").unwrap();
        let work = blocked.join("work");
        let launcher = VenvAppLauncher::new(work.clone());
        let venv = testing::venv_in(&dir);
        let failure = launcher
            .start(&venv, true, &crate::events::NullSink)
            .err()
            .expect("the wrapper cannot be written");
        let said = format!("{failure:#}");
        assert!(
            said.starts_with(&format!("creating {}: ", work.display())),
            "{said}"
        );
        // Not the venv's LabVIEW failing, so the front end adds no rebinding hint.
        assert!(failure.downcast_ref::<launch::Unstartable>().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_wrapper_gets_the_project_with_native_separators_and_the_spec_as_it_is() {
        let (project, spec) =
            wrapper_inputs(Path::new("C:/work/app/Main.lvproj"), TARGET, "Demo App").unwrap();
        #[cfg(windows)]
        assert_eq!(project, r"C:\work\app\Main.lvproj");
        #[cfg(not(windows))]
        assert_eq!(project, "C:/work/app/Main.lvproj");
        assert_eq!(spec, "Demo App");
    }

    #[test]
    fn a_target_the_wrapper_cannot_build_is_refused_not_ignored() {
        let why = wrapper_inputs(Path::new("Main.lvproj"), "RT CompactRIO", "S").unwrap_err();
        let AppBuildError::Other(cause) = why else {
            panic!("not LabVIEW's error");
        };
        assert_eq!(
            cause.to_string(),
            "the build VI builds the My Computer target only, not \"RT CompactRIO\""
        );
    }

    #[test]
    fn an_indicator_of_another_type_is_named_with_what_it_read_back_as() {
        assert!(want_bool("Done", LvValue::Bool(true)).unwrap());
        assert_eq!(want_i32("error code", LvValue::I32(7)).unwrap(), 7);
        assert_eq!(want_str("outputs", LvValue::Str("a".into())).unwrap(), "a");
        assert_eq!(
            want_bool("Done", LvValue::I32(1)).unwrap_err().to_string(),
            "Done read back as 1, not a boolean"
        );
        assert_eq!(
            want_i32("error code", LvValue::Str("x".into()))
                .unwrap_err()
                .to_string(),
            "error code read back as \"x\", not an I32"
        );
        assert_eq!(
            want_str("outputs", LvValue::Bool(false))
                .unwrap_err()
                .to_string(),
            "outputs read back as false, not a string"
        );
    }

    #[test]
    fn code_zero_hands_on_the_outputs_one_per_line_and_skips_blank_lines() {
        let said = "b/Main.exe\n\nb/data\r\nb/Main.ini\n";
        assert_eq!(
            wrapper_outputs(0, String::new(), said).unwrap(),
            [
                PathBuf::from("b/Main.exe"),
                PathBuf::from("b/data"),
                PathBuf::from("b/Main.ini"),
            ]
        );
        assert!(wrapper_outputs(0, String::new(), "").unwrap().is_empty());
    }

    #[test]
    fn any_other_code_is_labviews_error_with_its_message_whatever_the_outputs_say() {
        let why =
            wrapper_outputs(1003, "The VI is not executable".into(), "b/Main.exe").unwrap_err();
        let AppBuildError::LabView(error) = why else {
            panic!("expected LabVIEW's error");
        };
        assert_eq!(
            error,
            LvError {
                code: 1003,
                message: "The VI is not executable".into()
            }
        );
        let negative = wrapper_outputs(-1, String::new(), "").unwrap_err();
        assert!(matches!(
            negative,
            AppBuildError::LabView(LvError { code: -1, .. })
        ));
    }
}
