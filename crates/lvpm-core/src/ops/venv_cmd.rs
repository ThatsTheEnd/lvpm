//! `lvpm venv create | status | remove`.
//!
//! Removing asks nothing here: the front end finds the venv with [`existing`],
//! asks the user, and says what the answer was to [`remove`] (AC-K6).

use super::env::OpEnv;
use crate::events::Event;
use crate::paths;
use crate::venv::{self, Venv};
use crate::{install, launch};
use anyhow::{Result, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Create the venv of the project the command is run in, or of `--project`,
/// and bind it to a LabVIEW: `--labview-version`, else the manifest's.
/// `manifest` (an `lvpm.toml` or a `vipm.toml`) places the venv beside it
/// instead of the project's own.
pub fn create(env: &OpEnv, manifest: Option<&Path>) -> Result<Venv> {
    let start = match &env.flags.project {
        Some(project) => env.cwd.join(project),
        None => env.cwd.clone(),
    };
    let explicit = match manifest {
        Some(p) => {
            let p = paths::resolve(&env.cwd, p);
            let proj = env.read_manifest(&p)?;
            Some((p, proj))
        }
        None => None,
    };
    venv::create(
        &start,
        env.flags.labview_version.as_deref(),
        explicit,
        &env.config.venv_dir,
        &env.pickable()?,
        env.sink,
    )
}

/// What `lvpm venv status` reports about the venv a command run here would use.
#[derive(Debug, Clone, Serialize)]
pub struct VenvStatus {
    pub venv: Venv,
    /// Packages installed in the venv.
    pub installed: usize,
    /// Of those, the ones the relink pass has not run over yet.
    pub not_relinked: usize,
    /// A LabVIEW is answering on the venv's VI Server port.
    pub running: bool,
    /// Not answering yet, but a LabVIEW lvpm started for this venv is alive.
    pub starting: bool,
    /// The venv packages the LabVIEW installation has its own copy of.
    pub shadowed: Vec<crate::shadow::Shadowed>,
}

/// The venv a command run here would use, if there is one.
pub fn status(env: &OpEnv) -> Result<Option<VenvStatus>> {
    let Some(venv) = env.find_venv()? else {
        return Ok(None);
    };
    let installed = install::list_installed(&venv.store())?;
    let running = launch::is_listening(venv.port);
    Ok(Some(VenvStatus {
        installed: installed.len(),
        not_relinked: installed.iter().filter(|m| !m.relinked).count(),
        running,
        starting: !running && launch::is_running(&venv),
        shadowed: crate::shadow::find(&venv, &installed),
        venv,
    }))
}

/// The venv a removal means. There must be one.
pub fn existing(env: &OpEnv) -> Result<Venv> {
    match env.find_venv_for_removal()? {
        Some(venv) => Ok(venv),
        None => bail!("no venv at or above {}", env.cwd.display()),
    }
}

/// Delete the venv and everything installed in it, if `confirmed`; a venv that
/// was not confirmed is left as it is.
pub fn remove(env: &OpEnv, venv: &Venv, confirmed: bool) -> Result<()> {
    venv::remove(venv, confirmed, env.sink)
}

#[cfg(test)]
mod tests {
    use super::super::testing::{self, scratch_dir};
    use super::*;
    use std::net::{Ipv4Addr, TcpListener};
    use std::path::PathBuf;

    /// A repo with a venv whose VI Server port is free to bind. The port
    /// follows from the venv's path, so a path is taken whose port nothing
    /// listens on rather than hoping that none does.
    fn repo_with_a_venv_on_a_free_port(tag: &str) -> (PathBuf, Venv) {
        for attempt in 0..100 {
            let repo = scratch_dir(&format!("{tag}-{attempt}"));
            let venv = testing::venv_in(&repo);
            if TcpListener::bind((Ipv4Addr::LOCALHOST, venv.port)).is_ok() {
                return (repo, venv);
            }
            let _ = std::fs::remove_dir_all(&repo);
        }
        panic!("no scratch path in 100 tries has a free venv port");
    }

    #[test]
    fn status_finds_the_venv_from_below_and_counts_what_is_in_it() {
        let (repo, created) = repo_with_a_venv_on_a_free_port("venv-status");
        let deep = repo.join("src");
        std::fs::create_dir_all(&deep).unwrap();

        let found = status(&testing::env(&deep)).unwrap().unwrap();
        assert_eq!(found.venv.dir, created.dir);
        assert_eq!((found.installed, found.not_relinked), (0, 0));
        assert!(!found.running, "nothing listens on port {}", created.port);

        let _listener = TcpListener::bind((Ipv4Addr::LOCALHOST, created.port)).unwrap();
        let found = status(&testing::env(&deep)).unwrap().unwrap();
        assert!(found.running, "something listens on port {}", created.port);

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn with_no_venv_there_is_none_to_report_and_none_to_remove() {
        let empty = scratch_dir("venv-none");
        let env = testing::env(&empty);
        assert!(status(&env).unwrap().is_none());
        let why = existing(&env).unwrap_err().to_string();
        assert_eq!(why, format!("no venv at or above {}", empty.display()));
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn a_venv_is_deleted_only_when_the_caller_says_it_was_confirmed() {
        let repo = scratch_dir("venv-remove");
        let created = testing::venv_in(&repo);
        let env = testing::env(&repo);

        let why = remove(&env, &created, false).unwrap_err().to_string();
        assert_eq!(why, "not removed");
        assert!(created.dir.is_dir());

        remove(&env, &created, true).unwrap();
        assert!(!created.dir.exists());

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn create_binds_the_project_the_flags_name() {
        let repo = scratch_dir("venv-create");
        let elsewhere = scratch_dir("venv-create-cwd");
        let mut env = testing::env(&elsewhere);
        env.flags.project = Some(repo.clone());
        env.flags.labview_version = Some("2026".into());

        let created = create(&env, None).unwrap();
        assert_eq!(created.dir, repo.join(".lv-venv"));
        assert!(!elsewhere.join(".lv-venv").exists());

        let _ = std::fs::remove_dir_all(&repo);
        let _ = std::fs::remove_dir_all(&elsewhere);
    }
}

/// A shortcut `venv shortcut` wrote.
#[derive(Debug, Clone, Serialize)]
pub struct ShortcutWritten {
    pub path: PathBuf,
    /// The command line after `lvpm.exe`.
    pub arguments: String,
}

/// Write a Windows shortcut that runs `lvpm launch` on the venv: to `out`
/// (a `.lnk` path, or a folder), else the desktop when `desktop`, else the
/// project folder.
pub fn shortcut(
    env: &OpEnv,
    lvproj: Option<&Path>,
    desktop: bool,
    out: Option<&Path>,
) -> Result<ShortcutWritten> {
    let v = existing(env)?;
    let lvproj = match lvproj {
        Some(p) => {
            let p = crate::paths::resolve(&env.cwd, p);
            anyhow::ensure!(p.is_file(), "no such project file: {}", p.display());
            Some(p)
        }
        None => None,
    };
    let name = crate::shortcut::file_name(&v, lvproj.as_deref());
    let path = match (out.map(|p| crate::paths::resolve(&env.cwd, p)), desktop) {
        (Some(p), _) if p.is_dir() => p.join(&name),
        (Some(p), _) => p.with_extension("lnk"),
        (None, true) => crate::shortcut::desktop()?.join(&name),
        (None, false) => v.repo.join(&name),
    };
    let (icon, plain) = crate::venv_icon::or_labviews(&v);
    if let Some(why) = plain {
        env.sink.emit(Event::VenvWindowsWarning {
            why: format!("shortcut gets LabVIEW's plain icon: {why}"),
        });
    }
    crate::shortcut::create(&v, lvproj.as_deref(), &path, &icon)?;
    Ok(ShortcutWritten {
        arguments: crate::shortcut::arguments(&v, lvproj.as_deref()),
        path,
    })
}
