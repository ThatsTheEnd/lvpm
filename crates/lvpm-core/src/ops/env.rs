//! [`OpEnv`]: everything an operation needs from its surroundings, handed to it
//! instead of read from the process.
//!
//! An operation reads no environment variable and no working directory. The
//! front end builds one `OpEnv` from its own flags and the process, and a test
//! builds one from nothing at all. Settling *where* a command's packages live
//! and *which indexes* it sees is the first thing every operation does, so it
//! is the first thing here.

use crate::cancel::CancelToken;
use crate::config::Config;
use crate::events::{Event, EventSink};
use crate::github::GitHub;
use crate::index::{self, Index};
use crate::paths;
use crate::project::{self, Project};
use crate::target::{self, LvTarget, Roots};
use crate::venv::{self, Venv};
use anyhow::{Result, bail, ensure};
use std::path::{Path, PathBuf};

/// The flags that say where a command's packages live.
#[derive(Debug, Clone, Default)]
pub struct DestinationFlags {
    /// Install into this LabVIEW version, e.g. 2026.
    pub labview_version: Option<String>,
    /// Pick the 32- or 64-bit LabVIEW when both of a version are installed.
    pub labview_bitness: Option<u8>,
    /// Install into a scratch directory instead of a LabVIEW installation.
    pub prefix: Option<PathBuf>,
    /// Use the LabVIEW installation itself, even from inside a project that
    /// has a venv.
    pub global: bool,
    /// The project whose venv to use, instead of looking upwards from `cwd`.
    pub project: Option<PathBuf>,
}

/// What an operation runs in.
pub struct OpEnv<'a> {
    pub flags: DestinationFlags,
    /// Revalidate remote feeds and re-read local packages instead of trusting
    /// cached stats.
    pub refresh: bool,
    /// The directory the command was run from.
    pub cwd: PathBuf,
    /// Is LabVIEW headless on this machine? `LV_RTE_HEADLESS` is NI's own
    /// global override (LabVIEW 2026+): every LabVIEW start becomes
    /// non-interactive — no activation, no dialogs. Nobody sets it on a
    /// workstation, since the IDE would stop opening, so its presence says
    /// "automation machine, no developer present" more reliably than any flag
    /// of ours could. lvpm reads it for the three decisions that hinge on
    /// exactly that: whether a manifest without a venv means the machine,
    /// whether to relink, whether to rebuild palettes. The front end reads the
    /// variable; operations only read this.
    pub headless: bool,
    pub config: Config,
    /// The LabVIEW installations detected on this machine: the detected list;
    /// choose from [`OpEnv::pickable`].
    pub targets: Vec<LvTarget>,
    pub sink: &'a dyn EventSink,
    /// Does nothing unless the front end keeps a clone and cancels it.
    pub cancel: CancelToken,
    /// Put the machine's own places (ProgramData and the like) under this
    /// directory for a LabVIEW installation. Only a test of an installation
    /// that is not really there sets it; see [`Roots::labview_on`].
    pub machine: Option<PathBuf>,
    /// The API GitHub dependencies are read from, and the token it is told.
    pub github: GitHub,
}

/// Where a command's packages live.
#[derive(Debug)]
pub enum Destination {
    /// The LabVIEW installation itself.
    Global(Roots),
    /// A `--prefix` sandbox.
    Scratch(Roots),
    /// A project's venv.
    Venv(Venv),
}

impl Destination {
    /// Roots for what reads the store or removes from it. Venv-wide for a
    /// venv — each package has roots of its own for the files themselves.
    pub fn store(&self) -> Roots {
        match self {
            Destination::Global(r) | Destination::Scratch(r) => r.clone(),
            Destination::Venv(v) => v.store(),
        }
    }

    /// The LabVIEW the destination is for, if any: a scratch prefix has none.
    pub fn target(&self) -> Option<&LvTarget> {
        match self {
            Destination::Venv(v) => Some(&v.target),
            Destination::Global(r) | Destination::Scratch(r) => r.target.as_ref(),
        }
    }
}

impl<'a> OpEnv<'a> {
    /// An environment with no destination flags, not headless, not refreshing
    /// and never cancelled; the caller sets what differs.
    pub fn new(
        sink: &'a dyn EventSink,
        config: Config,
        targets: Vec<LvTarget>,
        cwd: PathBuf,
    ) -> OpEnv<'a> {
        OpEnv {
            flags: DestinationFlags::default(),
            refresh: false,
            cwd,
            headless: false,
            config,
            targets,
            sink,
            cancel: CancelToken::default(),
            machine: None,
            github: GitHub::default(),
        }
    }

    fn labview_roots(&self, t: &LvTarget) -> Roots {
        Roots::labview_on(t, self.machine.as_deref())
    }

    /// The installations a pick may choose from: the detected ones, narrowed to
    /// `--labview-bitness` when it is given. Not for finding a venv's own LabVIEW.
    pub fn pickable(&self) -> Result<Vec<LvTarget>> {
        target::of_bitness(&self.targets, self.flags.labview_bitness)
    }

    /// Resolve `--labview-version` / `--prefix` into a set of roots.
    pub fn roots(&self) -> Result<Roots> {
        match (&self.flags.labview_version, &self.flags.prefix) {
            (Some(_), Some(_)) => bail!("--labview-version and --prefix are mutually exclusive"),
            (Some(want), None) => {
                if self.targets.is_empty() {
                    bail!("no LabVIEW installations detected");
                }
                Ok(self.labview_roots(&target::select(&self.pickable()?, want)?))
            }
            (None, Some(p)) => Ok(Roots::scratch(p)),
            (None, None) => bail!(
                "pick a destination: --labview-version <YYYY> for a real install, --prefix <DIR> for a \
                 scratch tree, or run inside a project that has a venv (`lvpm venv create`)\n\
                 hint: `lvpm targets` lists detected LabVIEW installations"
            ),
        }
    }

    /// Settle where a command's packages live, the way cargo settles which crate
    /// you mean: `--prefix` and `--global` say so outright; otherwise a venv at or
    /// above the current directory wins, and only when there is none does
    /// `--labview-version` mean the installation itself.
    ///
    /// A manifest with no venv is refused — except on a headless machine, where
    /// the refusal would protect a developer who is not there: a container's
    /// LabVIEW *is* the sandbox, so `lvpm install` in the project means it.
    pub fn destination(&self) -> Result<Destination> {
        let explicit = |r: Roots| match r.target.is_some() {
            true => Destination::Global(r),
            false => Destination::Scratch(r),
        };
        if self.flags.prefix.is_some() || self.flags.global {
            return self.roots().map(explicit);
        }
        if self.headless
            && self.flags.project.is_none()
            && let venv::Found::ManifestOnly(repo) = venv::probe(&self.cwd, &self.config.venv_dir)
        {
            return self.headless_roots(&repo).map(Destination::Global);
        }
        if let Some(v) = self.find_venv()? {
            self.sink.emit(Event::VenvSelected {
                dir: v.dir.clone(),
                labview: v.target.label(),
            });
            return Ok(Destination::Venv(v));
        }
        self.roots().map(explicit)
    }

    /// The installation a headless `lvpm install` in a venv-less project means:
    /// `--labview-version` if given, else the manifest's `labview` — the same
    /// choice `lvpm venv create` makes, with the same rule that the manifest's
    /// version is a minimum a newer LabVIEW may satisfy and an older one may not.
    fn headless_roots(&self, repo: &Path) -> Result<Roots> {
        let manifest_path = repo.join(project::FILE_NAME);
        let proj = project::read(&manifest_path)?;
        let min_lv = proj.labview.as_deref();
        // Read before anything is chosen, so a `labview` that is no year is
        // refused whichever way the installation is picked.
        let min_year = min_lv.map(project::labview_year).transpose()?;
        let requested = self.flags.labview_version.as_deref();
        ensure!(
            requested.or(min_lv).is_some(),
            "which LabVIEW? pass --labview-version <YYYY>, or set labview in {}\n\
             hint: `lvpm targets` lists what is installed",
            manifest_path.display()
        );
        ensure!(
            !self.targets.is_empty(),
            "no LabVIEW installations detected"
        );
        let t = target::select_for_project(&self.pickable()?, requested, min_year)?
            .expect("checked above: a flag or a manifest year is set");
        self.sink.emit(Event::HeadlessInstallationChosen {
            repo: repo.to_path_buf(),
            labview: t.label(),
        });
        Ok(self.labview_roots(&t))
    }

    /// The venv a command means, if any: `--project` names the repo outright,
    /// otherwise it is looked up from `cwd`.
    pub fn find_venv(&self) -> Result<Option<Venv>> {
        let project = self.flags.project.as_deref().map(|p| self.cwd.join(p));
        let found = venv::find(
            project.as_deref(),
            &self.cwd,
            &self.config.venv_dir,
            &self.targets,
        )?;
        if let Some(v) = &found {
            self.check_bitness(&v.target)?;
        }
        Ok(found)
    }

    /// Refuses a venv bound to another bitness than `--labview-bitness` names.
    pub fn check_bitness(&self, bound_to: &LvTarget) -> Result<()> {
        if let Some(bits) = self.flags.labview_bitness
            && bound_to.bitness != bits
        {
            bail!(
                "this venv is bound to {}, not the {bits}-bit LabVIEW --labview-bitness asks for\n\
                 hint: drop --labview-bitness, or run `lvpm venv remove` without it, then \
                 `lvpm venv create --labview-bitness {bits}`",
                bound_to.label()
            );
        }
        Ok(())
    }

    /// Where the relink VI and a staged PostUninstall hook are written. `%TEMP%`
    /// on Windows; on Linux LabVIEW cannot open files under the private
    /// `/tmp` a sandboxed shell may see, so the cache instead.
    pub fn work_dir(&self) -> PathBuf {
        match cfg!(windows) {
            true => std::env::temp_dir(),
            false => self.config.cache.clone(),
        }
    }

    /// The manifest a command belongs to: `explicit` (`--manifest`) names it;
    /// else the venv's `repo` when one is in effect; else `--project`'s; else
    /// the nearest `lvpm.toml` at or above the working directory. None is fine
    /// — a package can be installed without a project.
    pub fn manifest_for(
        &self,
        explicit: Option<&Path>,
        repo: Option<&Path>,
    ) -> Result<Option<(PathBuf, Project)>> {
        let path = match (explicit, repo, &self.flags.project) {
            (Some(p), _, _) => Some(self.cwd.join(p)),
            // A venv created with `--manifest` may have no `lvpm.toml` beside it.
            (None, Some(r), _) => Some(r.join(project::FILE_NAME)).filter(|p| p.is_file()),
            (None, None, Some(dir)) => {
                Some(self.cwd.join(dir).join(project::FILE_NAME)).filter(|p| p.is_file())
            }
            (None, None, None) => project::find_manifest(&self.cwd),
        };
        match path {
            Some(p) => Ok(Some((
                paths::resolve(&self.cwd, &p),
                self.read_manifest(&p)?,
            ))),
            None => Ok(None),
        }
    }

    /// Read a manifest file. A `vipm.toml` goes through the VIPM conversion; it
    /// is only ever reached through `--manifest`, never discovered.
    pub fn read_manifest(&self, path: &Path) -> Result<Project> {
        match crate::vipm_interop::is_toml(path) {
            true => crate::vipm_interop::read_toml(path, self.sink),
            false => project::read(path),
        }
    }

    /// The configured local package folders, a relative one taken from `cwd`.
    pub(super) fn local_sources(&self) -> Vec<PathBuf> {
        let folders = self.config.local_sources.iter();
        folders.map(|dir| paths::resolve(&self.cwd, dir)).collect()
    }

    /// Every index the command should see: global local folders plus hosted
    /// manifest sources, on top of the public indexes unless disabled.
    pub fn load_index(&self, manifest: Option<&(PathBuf, Project)>) -> Result<Index> {
        let loaded = self.load_index_each(manifest, |_, error| Err(error))?;
        Ok(loaded.index)
    }

    /// [`Self::load_index`], handing a source that cannot be read to `on_fail`
    /// as [`index::load_each`] does.
    pub(super) fn load_index_each(
        &self,
        manifest: Option<&(PathBuf, Project)>,
        on_fail: impl FnMut(&str, anyhow::Error) -> Result<()>,
    ) -> Result<index::Loaded> {
        let mut repos: Vec<String> = Vec::new();
        let mut defaults = true;
        if let Some((_, proj)) = manifest {
            repos.extend(proj.resolved_sources()?);
            defaults = proj.default_sources;
        }
        index::load_each(
            &self.config.cache,
            self.refresh,
            &repos,
            &self.local_sources(),
            defaults,
            self.sink,
            &self.cancel,
            on_fail,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NullSink;

    #[test]
    fn a_relative_local_source_is_taken_from_the_directory_the_command_ran_in() {
        let cwd = std::env::temp_dir().join("lvpm-env-local-sources");
        let elsewhere = std::env::temp_dir().join("lvpm-env-elsewhere");
        let mut config = config();
        config.local_sources = vec![PathBuf::from("./packages"), elsewhere.clone()];
        let env = OpEnv::new(&NullSink, config, Vec::new(), cwd.clone());
        assert_eq!(env.local_sources(), [cwd.join("packages"), elsewhere]);
    }

    fn config() -> Config {
        Config {
            cache: PathBuf::from("cache"),
            local_sources: Vec::new(),
            venv_dir: PathBuf::from(".lv-venv"),
        }
    }

    fn labview_2026() -> LvTarget {
        LvTarget {
            version: 26.3,
            bitness: 64,
            path: PathBuf::from("LabVIEW 2026"),
        }
    }

    fn labview(year: u32, bitness: u8) -> LvTarget {
        LvTarget {
            version: f64::from(year - 2000),
            bitness,
            path: PathBuf::from(format!("LabVIEW {year} {bitness}")),
        }
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lvpm-env-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_command_that_names_no_destination_is_told_how_to_pick_one() {
        let env = OpEnv::new(
            &NullSink,
            config(),
            vec![labview_2026()],
            PathBuf::from("."),
        );
        let why = env.roots().unwrap_err().to_string();
        assert!(why.starts_with("pick a destination:"), "{why}");
    }

    #[test]
    fn a_headless_install_reads_the_manifests_labview_the_way_locking_does() {
        for (labview, said) in [
            ("soon", Some("not a minimum LabVIEW year")),
            ("26", Some("not a minimum LabVIEW year")),
            ("2030", Some("is older than")),
            ("2024", None),
        ] {
            let repo = scratch_dir(&format!("headless-{labview}"));
            std::fs::write(
                repo.join(project::FILE_NAME),
                format!("[project]\nlabview = \">={labview}\"\n"),
            )
            .unwrap();
            let mut env = OpEnv::new(&NullSink, config(), vec![labview_2026()], repo.clone());
            env.flags.labview_version = Some("2026".into());
            match (env.headless_roots(&repo), said) {
                (Ok(_), None) => {}
                (Err(why), Some(said)) => {
                    assert!(format!("{why:#}").contains(said), "{labview}: {why:#}")
                }
                (outcome, _) => panic!("{labview}: {:?}", outcome.map(|_| ())),
            }
            let _ = std::fs::remove_dir_all(&repo);
        }
    }

    #[test]
    fn a_bitness_the_venv_is_not_bound_to_is_refused_and_a_matching_or_absent_one_is_not() {
        let repo = scratch_dir("venv-bitness");
        let bound = LvTarget {
            version: 26.3,
            ..labview(2026, 64)
        };
        let installed = vec![bound.clone(), labview(2026, 32)];
        venv::create(
            &repo,
            Some("2026"),
            None,
            Path::new(".lv-venv"),
            &installed[..1],
            &NullSink,
        )
        .unwrap();
        let mut env = OpEnv::new(&NullSink, config(), installed, repo.clone());

        env.flags.labview_bitness = Some(32);
        assert_eq!(
            env.find_venv().unwrap_err().to_string(),
            "this venv is bound to LabVIEW 2026 (64-bit)  v26.3, not the 32-bit LabVIEW \
             --labview-bitness asks for\n\
             hint: drop --labview-bitness, or run `lvpm venv remove` without it, then \
             `lvpm venv create --labview-bitness 32`"
        );

        for bits in [Some(64), None] {
            env.flags.labview_bitness = bits;
            let venv = env.find_venv().unwrap().expect("the venv is found");
            assert_eq!(venv.target.path, bound.path);
        }
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_labview_version_picks_among_the_injected_installations() {
        let mut env = OpEnv::new(
            &NullSink,
            config(),
            vec![labview_2026()],
            PathBuf::from("."),
        );
        env.flags.labview_version = Some("2026".into());
        assert_eq!(
            env.roots().unwrap().target.unwrap().path,
            labview_2026().path
        );

        env.targets.clear();
        let why = env.roots().unwrap_err().to_string();
        assert_eq!(why, "no LabVIEW installations detected");
    }

    #[test]
    fn a_manifests_minimum_still_lets_a_newer_year_of_the_requested_bitness_stand_in() {
        let repo = scratch_dir("headless-bitness");
        std::fs::write(
            repo.join(project::FILE_NAME),
            "[project]\nlabview = \">=2025\"\n",
        )
        .unwrap();
        let installed = vec![labview(2025, 64), labview(2026, 32)];
        let mut env = OpEnv::new(&NullSink, config(), installed, repo.clone());
        env.flags.labview_bitness = Some(32);
        let chosen = env.headless_roots(&repo).unwrap().target.unwrap();
        assert_eq!(chosen.path, labview(2026, 32).path);

        // Without the flag the manifest's own year is the pick.
        env.flags.labview_bitness = None;
        let chosen = env.headless_roots(&repo).unwrap().target.unwrap();
        assert_eq!(chosen.path, labview(2025, 64).path);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_bitness_picks_between_the_installations_of_a_version_through_roots() {
        let installed = vec![labview(2026, 64), labview(2026, 32)];
        let mut env = OpEnv::new(&NullSink, config(), installed, PathBuf::from("."));
        env.flags.labview_version = Some("2026".into());
        env.flags.labview_bitness = Some(32);
        assert_eq!(
            env.roots().unwrap().target.unwrap().path,
            labview(2026, 32).path
        );

        env.targets = vec![labview(2026, 64)];
        let why = env.roots().unwrap_err().to_string();
        assert_eq!(
            why,
            format!(
                "no 32-bit LabVIEW installation detected (detected: {})\n\
                 hint: `lvpm targets` lists detected LabVIEW installations",
                labview(2026, 64).label()
            )
        );
    }

    #[test]
    fn a_bitness_the_named_path_does_not_have_matches_no_target() {
        let installed = vec![labview(2026, 64), labview(2026, 32)];
        let mut env = OpEnv::new(&NullSink, config(), installed, PathBuf::from("."));
        env.flags.labview_version = Some(labview(2026, 64).path.display().to_string());
        env.flags.labview_bitness = Some(32);
        let why = env.roots().unwrap_err().to_string();
        assert!(why.starts_with("no LabVIEW target matching"), "{why}");
    }

    #[test]
    fn a_bitness_with_nothing_installed_still_says_nothing_is_installed() {
        let mut env = OpEnv::new(&NullSink, config(), Vec::new(), PathBuf::from("."));
        env.flags.labview_version = Some("2026".into());
        env.flags.labview_bitness = Some(32);
        let why = env.roots().unwrap_err().to_string();
        assert_eq!(why, "no LabVIEW installations detected");
    }

    #[test]
    fn the_manifest_is_looked_for_from_the_given_directory_not_the_process_one() {
        let root = scratch_dir("manifest");
        let deep = root.join("src").join("deep");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(
            root.join(project::FILE_NAME),
            "[project]\nname = \"found\"\n",
        )
        .unwrap();

        let mut env = OpEnv::new(&NullSink, config(), Vec::new(), deep.clone());
        let (path, found) = env.manifest_for(None, None).unwrap().unwrap();
        assert_eq!(path, root.join(project::FILE_NAME));
        assert_eq!(found.name.as_deref(), Some("found"));

        // A relative `--manifest` is relative to the given directory too.
        std::fs::write(deep.join("other.toml"), "[project]\nname = \"other\"\n").unwrap();
        let explicit = Path::new("other.toml");
        let (_, other) = env.manifest_for(Some(explicit), None).unwrap().unwrap();
        assert_eq!(other.name.as_deref(), Some("other"));

        // `--project` names the directory, and a directory without one has none.
        env.flags.project = Some(PathBuf::from("..").join(".."));
        let (via_project, _) = env.manifest_for(None, None).unwrap().unwrap();
        assert_eq!(
            via_project.canonicalize().unwrap(),
            root.join(project::FILE_NAME).canonicalize().unwrap()
        );
        env.flags.project = Some(PathBuf::from("src"));
        assert!(env.manifest_for(None, None).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&root);
    }
}
