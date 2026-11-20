//! `lvpm launch`: start LabVIEW on the project's venv.
//!
//! No relinking here — that happened at install — and no waiting: the IDE is
//! the user's from the moment it appears.

use super::env::{Destination, OpEnv};
use super::sync::{Selection, plan_sync};
use crate::events::{Event, SyncStep};
use crate::install::{self, Manifest};
use crate::launch;
use crate::lockfile::{self, Lockfile};
use crate::paths;
use crate::project::{Project, same_name};
use crate::venv::Venv;
use anyhow::{Result, bail, ensure};
use std::path::{Path, PathBuf};

/// A LabVIEW started on a venv.
#[derive(Debug, Clone)]
pub struct Launched {
    pub venv: Venv,
    /// The ini it was started with.
    pub ini: PathBuf,
    /// The project file it was asked to open.
    pub lvproj: Option<PathBuf>,
    /// The `--ini` keys written on top of lvpm's own.
    pub ini_overrides: Vec<(String, String)>,
}

/// `KEY=VALUE` as `--ini` takes it.
fn ini_override(kv: &str) -> Result<(String, String)> {
    let (k, val) = kv
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("--ini {kv}: expected KEY=VALUE"))?;
    ensure!(!k.trim().is_empty(), "--ini {kv}: empty key");
    Ok((k.trim().to_string(), val.trim().to_string()))
}

/// Start the venv's LabVIEW, optionally opening `lvproj` in it, with `ini`
/// (`KEY=VALUE`) written into the venv's ini first. With `check`, refuse first
/// when the venv does not hold what `lvpm.lock` lists ([`problems`]).
pub fn launch(env: &OpEnv, lvproj: Option<&Path>, ini: &[String], check: bool) -> Result<Launched> {
    let ini_overrides = ini
        .iter()
        .map(|kv| ini_override(kv))
        .collect::<Result<Vec<_>>>()?;
    let Destination::Venv(venv) = env.destination()? else {
        bail!("launch needs a venv: run it inside a project that has one, or pass --project <DIR>");
    };
    if check {
        check_against_lock(env, &venv)?;
    }
    // Not a reason to refuse: the copies may be the same version. But the
    // user should know which one LabVIEW is about to load.
    let records = install::list_installed(&venv.store()).unwrap_or_default();
    crate::shadow::emit(env.sink, &venv, &records);
    let venv = crate::venv::ensure_port(&venv, env.sink)?;
    if launch::is_running(&venv) {
        bail!(
            "a LabVIEW is already running or starting on this venv (VI Server port {}) — open the \
             project from that window, or close it and launch again",
            venv.port
        );
    }
    let lvproj = match lvproj {
        Some(p) => {
            let p = paths::resolve(&env.cwd, p);
            ensure!(p.is_file(), "no such project file: {}", p.display());
            Some(p)
        }
        None => None,
    };
    let ini = launch::write_ini(&venv, &ini_overrides)?;
    let child = launch::spawn(&venv, &ini, lvproj.as_deref(), env.sink)?;
    start_watcher(env, &venv, child.id());
    Ok(Launched {
        venv,
        ini,
        lvproj,
        ini_overrides,
    })
}

/// Start the hidden `lvpm title-watch` beside the LabVIEW with process id
/// `pid` (see [`crate::watcher`]): it keeps the venv's name in the window
/// titles and gives the windows the venv's taskbar group and icon. Never
/// fails the launch: what could not be done is a warning. Windows only.
fn start_watcher(env: &OpEnv, v: &Venv, pid: u32) {
    if !cfg!(windows) {
        return;
    }
    let warn = |why: String| env.sink.emit(Event::VenvWindowsWarning { why });
    let pos = env.config.venv_title;
    let name = v
        .repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "venv".into());
    let tag = format!("[{name}]");
    // The taskbar group is always on; its icon is the venv's badged one.
    let (icon, plain) = crate::venv_icon::or_labviews(v);
    if let Some(why) = plain {
        warn(format!("taskbar pin gets LabVIEW's plain icon: {why}"));
    }
    let relaunch = match crate::shortcut::relaunch_command(v) {
        Ok(cmd) => cmd,
        Err(e) => return warn(format!("no taskbar group for the venv: {e:#}")),
    };
    let started = std::env::current_exe().and_then(|exe| {
        let mut cmd = std::process::Command::new(exe);
        cmd.args(["title-watch", "--pid", &pid.to_string(), "--tag", &tag])
            .args(["--position", pos.as_str()])
            .args([
                "--app-id",
                &crate::shortcut::app_id(v),
                "--relaunch",
                &relaunch,
            ])
            .arg("--relaunch-icon")
            .arg(&icon)
            .args(["--relaunch-name", &crate::shortcut::display_name(v)]);
        #[cfg(windows)]
        {
            // lvpm is a console program: without this, its watcher would
            // open a console window of its own for LabVIEW's lifetime.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            std::os::windows::process::CommandExt::creation_flags(&mut cmd, CREATE_NO_WINDOW);
        }
        launch::spawn_detached(&mut cmd)
    });
    if let Err(e) = started {
        warn(format!("could not start the window title watcher: {e}"));
    }
}

/// Refuse to start on a venv that does not hold what the bound manifest's
/// `lvpm.lock` lists. A venv bound to no manifest is not checked, nor is one
/// whose manifest has no dependency and no lock yet.
fn check_against_lock(env: &OpEnv, venv: &Venv) -> Result<()> {
    let manifest = venv.manifest_path();
    if !manifest.is_file() {
        return Ok(());
    }
    let project = env.read_manifest(&manifest)?;
    let lock_path = manifest.with_file_name(lockfile::FILE_NAME);
    let lock = match lock_path.is_file() {
        true => Some(Lockfile::read(&lock_path)?),
        false => None,
    };
    let installed = install::list_installed(&Destination::Venv(venv.clone()).store())?;
    let mut found = problems(&project, lock.as_ref(), &installed, |p| p.exists());
    if let Err(why) = crate::palette::verify(&venv.store()) {
        found.push(format!("palettes: {why:#}"));
    }
    if found.is_empty() {
        return Ok(());
    }
    let mut why = format!("the venv does not match {}:", lock_path.display());
    for problem in &found {
        why.push_str("\n  ");
        why.push_str(problem);
    }
    why.push_str(
        "\nrun `lvpm sync` to bring it in line, or `lvpm launch --no-check` to start anyway",
    );
    bail!(why)
}

/// Every way the venv's install records fall short of `lock`, one line each:
/// a package `[dependencies]` needs that is missing, at another version or
/// from another archive; recorded files that are gone; and a `relink = true`
/// package the relink pass has not completed. Dev and the groups are not
/// checked: the venv does not record which of them it was synced with.
// shortcut: a file edited or resaved since install passes, the records hold no hashes; record them to catch it.
fn problems(
    project: &Project,
    lock: Option<&Lockfile>,
    installed: &[Manifest],
    exists: impl Fn(&Path) -> bool,
) -> Vec<String> {
    let Some(lock) = lock else {
        let none = project.tables().all(|(_, lines)| lines.is_empty());
        return match none {
            true => Vec::new(),
            false => vec!["no lvpm.lock beside the manifest".to_string()],
        };
    };
    let main_only = Selection {
        no_dev: true,
        groups: Vec::new(),
        keep_listed: true,
    };
    let mut found: Vec<String> = (plan_sync(lock, installed, &main_only).steps.iter())
        .filter_map(|step| match step {
            SyncStep::Install { name, version } => {
                Some(format!("{name}: not installed (lock: {version})"))
            }
            SyncStep::Replace { name, from, to } if from == to => Some(format!(
                "{name}: {from} installed from another archive than the lock's"
            )),
            SyncStep::Replace { name, from, to } => {
                Some(format!("{name}: {from} installed, lock: {to}"))
            }
            SyncStep::Remove { .. } => None,
        })
        .collect();
    let locked = |name: &str| (lock.packages.iter()).any(|p| same_name(&p.name, name));
    for m in installed.iter().filter(|m| locked(&m.name)) {
        let missing: Vec<&String> = (m.files.iter())
            .filter(|f| !exists(Path::new(f.as_str())))
            .collect();
        if let Some(first) = missing.first() {
            found.push(format!(
                "{}: {} of {} files missing, e.g. {first}",
                m.name,
                missing.len(),
                m.files.len()
            ));
        }
        if project.relink_for(&m.name) && !m.relinked {
            found.push(format!(
                "{}: relink = true but the relink pass has not completed",
                m.name
            ));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::super::testing::{self, scratch_dir};
    use super::*;

    fn record(name: &str, version: &str, files: &[&str], relinked: bool) -> Manifest {
        Manifest {
            name: name.to_string(),
            version: version.to_string(),
            display_name: None,
            installed_at: 0,
            target: String::new(),
            files: files.iter().map(|f| f.to_string()).collect(),
            palette_files: Vec::new(),
            dependencies: Vec::new(),
            skipped_hooks: Vec::new(),
            relinked,
            relink_folders: Vec::new(),
            post_install_vi: None,
            pre_install_vi: None,
            pre_uninstall_vi: None,
            post_uninstall_vi: None,
            skipped_groups: Vec::new(),
            archive_sha256: Some(format!("sha-{name}-{version}")),
        }
    }

    fn lock_of(packages: &[(&str, &str, &[&str])]) -> Lockfile {
        let packages = (packages.iter())
            .map(|(name, version, groups)| crate::lockfile::LockedPackage {
                name: name.to_string(),
                version: version.to_string(),
                sha256: format!("sha-{name}-{version}"),
                md5: None,
                digest: None,
                source: crate::lockfile::LOCAL_SOURCE.to_string(),
                requires: Vec::new(),
                groups: groups.iter().map(|g| g.to_string()).collect(),
                signer: None,
                subject: None,
            })
            .collect();
        Lockfile {
            version: crate::lockfile::FORMAT_VERSION,
            labview: "2024".to_string(),
            inputs: Default::default(),
            packages,
            unresolved: Vec::new(),
        }
    }

    #[test]
    fn the_check_names_each_way_the_venv_falls_short_of_the_lock() {
        let project = crate::project::parse(
            "[dependencies]\nok = \"1\"\nold = \"2\"\nabsent = \"1\"\nrelinkme = { version = \"1\", relink = true }\ngone = \"1\"\n",
        )
        .unwrap();
        let lock = lock_of(&[
            ("ok", "1.0", &[]),
            ("old", "2.0", &[]),
            ("absent", "1.0", &[]),
            ("relinkme", "1.0", &[]),
            ("gone", "1.0", &[]),
            ("devtool", "1.0", &["dev"]),
        ]);
        let installed = [
            record("ok", "1.0", &["c:/v/ok.vi"], false),
            record("old", "1.5", &["c:/v/old.vi"], false),
            record("relinkme", "1.0", &["c:/v/r.vi"], false),
            record("gone", "1.0", &["c:/v/a.vi", "c:/v/b.vi"], false),
            record("stray", "1.0", &["c:/v/stray.vi"], false),
        ];
        let exists = |p: &Path| p != Path::new("c:/v/b.vi");
        let mut found = problems(&project, Some(&lock), &installed, exists);
        found.sort();
        assert_eq!(
            found,
            [
                "absent: not installed (lock: 1.0)",
                "gone: 1 of 2 files missing, e.g. c:/v/b.vi",
                "old: 1.5 installed, lock: 2.0",
                "relinkme: relink = true but the relink pass has not completed",
            ],
            "a dev package not installed and a package the lock lacks pass"
        );

        let installed = [record("relinkme", "1.0", &[], true)];
        let lock = lock_of(&[("relinkme", "1.0", &[])]);
        assert!(problems(&project, Some(&lock), &installed, exists).is_empty());
    }

    #[test]
    fn without_a_lock_only_a_manifest_with_no_dependency_passes() {
        let none = crate::project::parse("[dependencies]\n").unwrap();
        assert!(problems(&none, None, &[], |_| true).is_empty());
        let some = crate::project::parse("[dev-dependencies]\nx = \"1\"\n").unwrap();
        assert_eq!(
            problems(&some, None, &[], |_| true),
            ["no lvpm.lock beside the manifest"]
        );
    }

    #[test]
    fn launching_needs_a_venv() {
        let empty = scratch_dir("launch-none");
        let mut env = testing::env(&empty);
        env.flags.labview_version = Some("2026".into());
        let why = launch(&env, None, &[], true).unwrap_err().to_string();
        assert!(why.starts_with("launch needs a venv:"), "{why}");
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn a_project_file_that_is_not_there_stops_the_launch_before_anything_starts() {
        let repo = scratch_dir("launch-lvproj");
        let venv = testing::venv_in(&repo);
        let env = testing::env(&repo);

        let why = launch(&env, Some(Path::new("missing.lvproj")), &[], true)
            .unwrap_err()
            .to_string();
        assert_eq!(
            why,
            format!(
                "no such project file: {}",
                repo.join("missing.lvproj").display()
            )
        );
        assert!(
            !venv.ini_path().exists(),
            "the ini is written only for a launch"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn an_ini_override_is_key_equals_value_with_a_key() {
        assert_eq!(
            ini_override(" NIER = True").unwrap(),
            ("NIER".into(), "True".into())
        );
        assert_eq!(ini_override("a=b=c").unwrap(), ("a".into(), "b=c".into()));
        for bad in ["NIER", "=True"] {
            assert!(
                ini_override(bad)
                    .unwrap_err()
                    .to_string()
                    .starts_with("--ini")
            );
        }
    }
}
