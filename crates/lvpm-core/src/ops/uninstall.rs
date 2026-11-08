//! Taking installed packages back out, hooks included, on the same session an
//! install runs on: the removals feed [`finish`], so one palette refresh covers
//! them all.
//!
//! [`finish`]: super::install::finish

use super::env::{Destination, OpEnv};
use super::hooks::{HOOK_TIMEOUT_SECS, HookCall, hook_action_info};
use super::install::{Applied, InstallOptions, finish};
use super::session::InstallSession;
use crate::events::{Event, HookKind, KeptBecause, UnrunReason};
use crate::project::same_name;
use crate::{dependency, install, project};
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};

/// What to do with the dependencies only the uninstalled package needed.
pub enum Dependencies<'a> {
    /// Leave them installed (`--keep-deps`).
    Keep,
    /// Remove them too (`--with-deps`).
    Remove,
    /// Ask: the front end is handed their names and says whether they go.
    Ask(&'a dyn Fn(&[String]) -> Result<bool>),
}

/// Remove a package, or every package installed in the destination, and run
/// their uninstall hooks. One bad package does not strand the rest of an `all`
/// sweep; a single named package still fails the command with its own error.
/// A named package takes the dependencies nothing else needs with it, as
/// `deps` says; whatever the project's manifest declares, or needs through
/// what it declares, stays.
pub fn uninstall(env: &OpEnv, package: Option<&str>, all: bool, deps: Dependencies) -> Result<()> {
    let dest = env.destination()?;
    let names: Vec<String> = match (package, all) {
        (Some(p), _) => with_dependencies(env, &dest, p, deps)?,
        (None, true) => install::list_installed(&dest.store())?
            .into_iter()
            .map(|m| m.name)
            .collect(),
        (None, false) => anyhow::bail!("give a package name, or --all"),
    };
    ensure!(
        !names.is_empty(),
        "no packages are installed in this target"
    );

    // From here a cancel is refused: the first file goes.
    env.cancel.begin_unpack()?;

    // One VI Server connection for the whole sweep, and one palette refresh at
    // the end — the hooks are per package, the rebuild is not.
    let mut session = InstallSession::new(dest);
    let mut removed = 0usize;
    let mut failed: Vec<(String, anyhow::Error)> = Vec::new();
    for name in &names {
        match uninstall_one(env, &mut session, name) {
            Ok(files) => removed += files,
            Err(e) => {
                if package.is_none() && names.len() > 1 {
                    env.sink.emit(Event::UninstallFailed {
                        name: name.clone(),
                        error: format!("{e:#}"),
                    });
                }
                failed.push((name.clone(), e));
                // A named package's dependencies follow it: removing them after a
                // failure would break what stays installed.
                if package.is_some() {
                    break;
                }
            }
        }
    }

    // The same reason as on install, in reverse: the packages' `.mnu` files
    // are gone, but LabVIEW still shows the palette entries until told.
    finish(
        env,
        &mut session,
        &Applied::removals(removed),
        &InstallOptions::default(),
    )?;

    if (package.is_some() || names.len() == 1)
        && let Some((_, e)) = failed.pop()
    {
        return Err(e);
    }
    ensure!(
        failed.is_empty(),
        "{} of {} packages failed to uninstall: {}",
        failed.len(),
        names.len(),
        failed
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let (Some(name), false) = (package, all) {
        say_if_still_listed(env, name);
    }
    Ok(())
}

/// A package taken out of the destination but still listed in the project's
/// manifest comes back with the next sync: say how to take it out for good.
/// A manifest that cannot be read has nothing to say.
fn say_if_still_listed(env: &OpEnv, package: &str) {
    let Ok(Some((_, project))) = env.manifest_for(None, None) else {
        return;
    };
    let found = project.tables().find_map(|(table, entries)| {
        let (entry, _) = entries
            .iter()
            .find(|(entry, _)| same_name(entry, package))?;
        Some((entry.clone(), table))
    });
    if let Some((name, table)) = found {
        let (dev, group) = table.flags();
        env.sink
            .emit(Event::UninstalledStillListed { name, dev, group });
    }
}

/// Where the PostUninstall hook is copied to run from: a file directly in the
/// `dir` (`OpEnv::work_dir`), whatever the record's package name holds. The name comes
/// from a file on disk, so anything but letters, digits, `_`, `-` and `.` (a
/// separator, a drive or stream colon) is replaced.
fn staged_hook_path(dir: &Path, package: &str) -> PathBuf {
    let name: String = package
        .chars()
        .map(
            |c| match c.is_alphanumeric() || matches!(c, '_' | '-' | '.') {
                true => c,
                false => '_',
            },
        )
        .collect();
    dir.join(format!("lvpm-{name}-PostUninstall.vi"))
}

/// Why the record's uninstall hooks were not run, read from the record alone.
/// A skipped entry whose VI the record does not hold is from before uninstall
/// hooks were extracted; one with its VI was extracted and never allowed.
fn unrun_reason(m: &install::Manifest, in_venv: bool) -> UnrunReason {
    let held_back_by_install_hook =
        m.hook_skipped(HookKind::PreInstall) || m.hook_skipped(HookKind::PostInstall);
    let nothing_extracted = [
        (HookKind::PreUninstall, &m.pre_uninstall_vi),
        (HookKind::PostUninstall, &m.post_uninstall_vi),
    ]
    .iter()
    .any(|(kind, vi)| m.hook_skipped(*kind) && vi.is_none());
    if in_venv {
        UnrunReason::VenvRunsNoHooks
    } else if held_back_by_install_hook {
        UnrunReason::InstallHookSkipped
    } else if nothing_extracted {
        UnrunReason::InstalledBeforeUninstallHooks
    } else {
        UnrunReason::NotAllowedAtInstall
    }
}

/// The named package, then the dependencies that go with it.
fn with_dependencies(
    env: &OpEnv,
    dest: &Destination,
    package: &str,
    deps: Dependencies,
) -> Result<Vec<String>> {
    // Keeping every dependency is safe without knowing them; removing is not.
    let old = install::records_without_dependencies(&dest.store())?;
    if let Some(p) = old.first().filter(|_| !matches!(deps, Dependencies::Keep)) {
        bail!(
            "reading installation record {} (dependency list required): reinstall it, or pass --keep-deps",
            p.display()
        );
    }
    let installed = install::list_installed(&dest.store())?;
    // Only a venv is bound to a project; a machine-wide install has none.
    let path = match dest {
        Destination::Venv(v) => Some(v.manifest_path()).filter(|p| p.is_file()),
        _ => None,
    };
    let roots: Vec<String> = match &path {
        Some(path) => env
            .read_manifest(path)?
            .tables()
            .flat_map(|(_, lines)| lines.iter().map(|(name, _)| name.clone()))
            .collect(),
        None => Vec::new(),
    };
    let manifest = path
        .as_deref()
        .and_then(Path::file_name)
        .map_or(project::FILE_NAME.into(), |n| {
            n.to_string_lossy().into_owned()
        });
    let plan = dependency::removal_plan(&installed, package, &roots)?;
    if plan.project_declared {
        env.sink.emit(Event::UninstallDeclaredInProject {
            package: plan.root.clone(),
            manifest: manifest.clone(),
        });
    }
    if !plan.dependents.is_empty() {
        env.sink.emit(Event::UninstallRequiredBy {
            package: plan.root.clone(),
            dependents: plan.dependents.clone(),
        });
    }
    for (name, owners) in &plan.kept {
        let why = if !plan.project_kept.iter().any(|k| same_name(k, name)) {
            KeptBecause::RequiredBy {
                owners: owners.clone(),
            }
        } else if roots.iter().any(|r| same_name(r, name)) {
            KeptBecause::Declared {
                manifest: manifest.clone(),
            }
        } else {
            KeptBecause::NeededByDeclared {
                manifest: manifest.clone(),
            }
        };
        env.sink.emit(Event::DependencyKept {
            name: name.clone(),
            why,
        });
    }
    let mut names = vec![plan.root];
    let remove = match deps {
        _ if plan.removable.is_empty() => false,
        Dependencies::Keep => false,
        Dependencies::Remove => true,
        Dependencies::Ask(ask) => ask(&plan.removable)?,
    };
    if remove {
        names.extend(plan.removable);
    }
    Ok(names)
}

/// Remove one package and run its uninstall hooks. Returns the file count, so
/// the caller knows whether anything left disk and a palette rebuild is owed.
pub fn uninstall_one(env: &OpEnv, session: &mut InstallSession, package: &str) -> Result<usize> {
    let roots = session.store().clone();
    let before = install::read_manifest(&roots, package)?;
    // Before the PreUninstall VI runs, not only before the files go.
    install::check_record(&roots, &before)?;
    // Hooks act on the LabVIEW installation; a venv is not one, so none run
    // there. And a package whose record lists a skipped hook did not run its
    // install hook, so its uninstall hooks stay off until `run-hooks` runs it.
    let in_venv = roots.venv().is_some();
    let hooks = !in_venv && !before.any_hook_skipped();

    // PreUninstall runs while the package's files are still on disk. Both
    // uninstall hooks are best-effort: a hook needs a running LabVIEW, and a
    // hook failure must not leave the package half-present.
    if hooks && let (Some(hook), Some(t)) = (&before.pre_uninstall_vi, &roots.target) {
        let info = hook_action_info(
            &before.name,
            before.display_name.as_deref(),
            t,
            &before.files,
        );
        session.run_hook(
            env,
            t,
            &HookCall {
                package: &before.name,
                hook: HookKind::PreUninstall,
                vi: Path::new(hook),
                info: &info,
                timeout_secs: HOOK_TIMEOUT_SECS,
            },
        );
    }

    // PostUninstall runs after the files are gone — including the extracted
    // hook VI itself, so it runs from a copy that outlives the uninstall.
    let post = match &before.post_uninstall_vi {
        Some(hook) if hooks => {
            let dir = env.work_dir();
            let tmp = staged_hook_path(&dir, &before.name);
            match std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(hook, &tmp)) {
                Ok(_) => Some(tmp),
                Err(e) => {
                    env.sink.emit(Event::PostUninstallNotStaged {
                        error: e.to_string(),
                    });
                    None
                }
            }
        }
        _ => None,
    };

    let (removed, m) = install::uninstall(&roots, package).with_context(|| match roots.venv() {
        Some(_) => {
            "removing the package's files — if a LabVIEW is running on this venv, close it first"
        }
        None => "removing the package's files",
    })?;
    env.sink.emit(Event::PackageRemoved {
        name: m.name.clone(),
        version: m.version.clone(),
        files: removed,
    });

    if let (Some(tmp), Some(t)) = (&post, &roots.target) {
        let info = hook_action_info(&m.name, m.display_name.as_deref(), t, &m.files);
        session.run_hook(
            env,
            t,
            &HookCall {
                package: &m.name,
                hook: HookKind::PostUninstall,
                vi: tmp,
                info: &info,
                timeout_secs: HOOK_TIMEOUT_SECS,
            },
        );
        let _ = std::fs::remove_file(tmp);
    }

    // Hooks we still do not run, and hooks we could not run here: an install
    // made before uninstall hooks were extracted has nothing on disk to run.
    // A hook the gate held back is named even when the record does not list it
    // (a failed PostInstall leaves the uninstall hooks pending, not skipped).
    let ran = [hooks && m.pre_uninstall_vi.is_some(), post.is_some()];
    let unrun: Vec<String> = [
        (HookKind::PreUninstall, &m.pre_uninstall_vi),
        (HookKind::PostUninstall, &m.post_uninstall_vi),
    ]
    .into_iter()
    .zip(ran)
    .filter(|(_, ran)| !ran)
    .filter_map(|((kind, vi), _)| match (m.skipped_entry(kind), vi) {
        (Some(listed), _) => Some(listed.clone()),
        (None, Some(vi)) if !hooks => Some(format!("{}={vi}", kind.name())),
        _ => None,
    })
    .collect();
    // A scratch tree has no LabVIEW: no hook runs there, so none is held back.
    if !unrun.is_empty() && roots.target.is_some() {
        env.sink.emit(Event::UninstallHooksNotRun {
            package: m.name.clone(),
            hooks: unrun,
            reason: unrun_reason(&m, in_venv),
            // `run-hooks` runs only a PostInstall; a package without one stays
            // off until it is reinstalled with its hooks allowed.
            run_hooks_helps: m.post_install_vi.is_some(),
        });
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::events::{EventSink, Phase, SkipReason};
    use crate::target::Roots;
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<Event>>);

    impl EventSink for Recorder {
        fn emit(&self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    /// A scratch destination holding one installed package `alpha` of one file.
    fn scratch_with_alpha(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("lvpm-uninstall-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("LabVIEW").join("vi.lib").join("alpha.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "alpha").unwrap();
        let store = Roots::scratch(&dir).store_dir();
        std::fs::create_dir_all(&store).unwrap();
        let manifest = serde_json::json!({
            "name": "alpha", "version": "1.0", "display_name": null, "installed_at": 0,
            "target": "scratch", "files": [file.to_string_lossy().replace('\\', "/")],
            "skipped_hooks": [],
        });
        std::fs::write(store.join("alpha.json"), manifest.to_string()).unwrap();
        (dir, file)
    }

    fn env_for<'s>(sink: &'s Recorder, dir: &std::path::Path) -> OpEnv<'s> {
        let config = Config {
            cache: dir.join("cache"),
            hooks_run: Vec::new(),
            local_sources: Vec::new(),
            venv_dir: PathBuf::from(".lv-venv"),
        };
        let mut env = OpEnv::new(sink, config, Vec::new(), dir.to_path_buf());
        env.flags.prefix = Some(dir.to_path_buf());
        env
    }

    #[test]
    fn the_copy_a_post_uninstall_hook_runs_from_is_a_file_in_the_temp_directory_whatever_the_name()
    {
        let temp = std::env::temp_dir();
        for name in [
            "oglib_error",
            "..\\..\\evil",
            "../../evil",
            "a/b\\c",
            "C:\\evil",
            "stream:name",
            "..",
        ] {
            let staged = staged_hook_path(&temp, name);
            assert_eq!(staged.parent(), Some(temp.as_path()), "{name}: {staged:?}");
            let file = staged.file_name().unwrap().to_string_lossy();
            assert!(
                !file.contains(['/', '\\', ':']) && file.ends_with("-PostUninstall.vi"),
                "{name}: {file}"
            );
        }
        assert_eq!(
            staged_hook_path(&temp, "oglib_error").file_name().unwrap(),
            "lvpm-oglib_error-PostUninstall.vi"
        );
    }

    #[test]
    fn an_uninstall_removes_the_files_and_hands_the_removals_to_finish() {
        let (dir, file) = scratch_with_alpha("removes");
        let sink = Recorder::default();
        uninstall(
            &env_for(&sink, &dir),
            Some("alpha"),
            false,
            Dependencies::Keep,
        )
        .unwrap();
        assert!(!file.exists(), "the package's file is gone");
        assert!(
            !dir.join("cache").exists(),
            "nothing was downloaded or cached"
        );

        let events = sink.0.lock().unwrap().clone();
        assert!(events.contains(&Event::PackageRemoved {
            name: "alpha".into(),
            version: "1.0".into(),
            files: 1
        }));
        // finish ran, over a scratch tree with one removal.
        assert!(events.contains(&Event::PhaseSkipped {
            phase: Phase::Refresh,
            reason: SkipReason::ScratchDestination,
            skipped: 1
        }));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::PhaseStarted { .. }))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_uninstalled_package_still_listed_names_its_table() {
        for (table, dev, group) in [
            ("[dev-dependencies]", true, None),
            ("[dependency-groups.ci]", false, Some("ci".to_string())),
        ] {
            let (dir, _) = scratch_with_alpha("still-listed");
            let manifest = format!("[project]\nlabview = \">=2024\"\n\n{table}\nalpha = \"*\"\n");
            std::fs::write(dir.join("lvpm.toml"), manifest).unwrap();
            let sink = Recorder::default();
            uninstall(
                &env_for(&sink, &dir),
                Some("alpha"),
                false,
                Dependencies::Keep,
            )
            .unwrap();
            let events = sink.0.lock().unwrap().clone();
            assert!(
                events.contains(&Event::UninstalledStillListed {
                    name: "alpha".into(),
                    dev,
                    group,
                }),
                "{table}: {events:?}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_scratch_uninstall_says_nothing_about_the_hooks_its_record_lists_as_skipped() {
        let (dir, _) = scratch_with_alpha("scratch-skipped");
        let record = Roots::scratch(&dir).store_dir().join("alpha.json");
        let mut m: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
        m["skipped_hooks"] = serde_json::json!(["PostInstall=a.vi", "PreUninstall=b.vi"]);
        std::fs::write(&record, m.to_string()).unwrap();

        let sink = Recorder::default();
        uninstall(
            &env_for(&sink, &dir),
            Some("alpha"),
            false,
            Dependencies::Keep,
        )
        .unwrap();
        let events = sink.0.lock().unwrap().clone();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::PackageRemoved { .. })),
            "{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::UninstallHooksNotRun { .. })),
            "{events:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_headless_uninstall_from_the_installation_skips_the_palette_refresh() {
        let dir = crate::ops::testing::scratch_dir("uninstall-headless");
        let fake = crate::ops::testing::FakeInstallation::new(&dir);
        let file = fake.target.path.join("vi.lib").join("alpha.txt");
        std::fs::write(&file, "alpha").unwrap();
        let store = fake.roots().store_dir();
        std::fs::create_dir_all(&store).unwrap();
        let manifest = serde_json::json!({
            "name": "alpha", "version": "1.0", "display_name": null, "installed_at": 0,
            "target": "LabVIEW 2099", "files": [file.to_string_lossy().replace('\\', "/")],
            "skipped_hooks": [],
        });
        std::fs::write(store.join("alpha.json"), manifest.to_string()).unwrap();

        let sink = Recorder::default();
        let mut env = env_for(&sink, &dir);
        env.flags.prefix = None;
        fake.place(&mut env);
        env.headless = true;
        env.flags.global = true;
        env.flags.labview_version = Some("2099".into());
        uninstall(&env, Some("alpha"), false, Dependencies::Keep).unwrap();
        assert!(!file.exists(), "the package's file is gone");

        let events = sink.0.lock().unwrap().clone();
        assert!(events.contains(&Event::PhaseSkipped {
            phase: Phase::Refresh,
            reason: SkipReason::Headless,
            skipped: 1
        }));
        // A refresh would start LabVIEW in a container.
        assert!(!events.iter().any(|e| matches!(
            e,
            Event::PhaseStarted {
                phase: Phase::Refresh
            }
        )));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_uninstall_after_an_accepted_cancel_deletes_nothing_and_one_that_ran_refuses_a_cancel() {
        let (dir, file) = scratch_with_alpha("cancelled");
        let sink = Recorder::default();
        let mut env = env_for(&sink, &dir);
        assert!(env.cancel.cancel());
        let why = uninstall(&env, Some("alpha"), false, Dependencies::Keep).unwrap_err();
        assert!(why.is::<crate::errors::Cancelled>(), "{why:#}");
        assert!(file.exists(), "a cancel that returned true deleted a file");

        env.cancel = crate::cancel::CancelToken::new();
        uninstall(&env, Some("alpha"), false, Dependencies::Keep).unwrap();
        assert!(!env.cancel.cancel(), "the uninstall had begun to delete");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_record_that_names_a_file_outside_the_destination_removes_nothing() {
        let (dir, file) = scratch_with_alpha("outside");
        let outside = dir.with_file_name(format!("lvpm-outside-{}.txt", std::process::id()));
        std::fs::write(&outside, "keep").unwrap();
        // Written as the record would have it: `..` climbs out of the prefix.
        let climbing = format!(
            "{}/../{}",
            dir.to_string_lossy().replace('\\', "/"),
            outside.file_name().unwrap().to_string_lossy()
        );
        let record = Roots::scratch(&dir).store_dir().join("alpha.json");
        let manifest = serde_json::json!({
            "name": "alpha", "version": "1.0", "display_name": null, "installed_at": 0,
            "target": "scratch",
            "files": [file.to_string_lossy().replace('\\', "/"), climbing],
            "skipped_hooks": [],
        });
        std::fs::write(&record, manifest.to_string()).unwrap();

        let sink = Recorder::default();
        let why = uninstall(
            &env_for(&sink, &dir),
            Some("alpha"),
            false,
            Dependencies::Keep,
        )
        .unwrap_err();
        assert!(
            why.to_string().contains("outside this destination"),
            "{why:#}"
        );
        assert!(file.exists() && outside.exists(), "something was deleted");
        assert!(record.exists(), "the record is kept");
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pre_uninstall_vi_outside_the_hooks_directory_is_refused_and_not_run() {
        use crate::ops::testing::{FakeInstallation, Recorded, env_on, scratch_dir};
        let dir = scratch_dir("uninstall-planted-hook");
        let fake = FakeInstallation::new(&dir);
        let planted = dir.join("planted-PreUninstall.vi");
        std::fs::write(&planted, "vi").unwrap();
        let store = fake.roots().store_dir();
        std::fs::create_dir_all(&store).unwrap();
        let record = store.join("alpha.json");
        let manifest = serde_json::json!({
            "name": "alpha", "version": "1.0", "display_name": null, "installed_at": 0,
            "target": "fake", "files": [], "skipped_hooks": [],
            "pre_uninstall_vi": planted.to_string_lossy().replace('\\', "/"),
        });
        std::fs::write(&record, manifest.to_string()).unwrap();

        let sink = Recorded::default();
        let mut env = env_on(&sink, &dir);
        fake.place(&mut env);
        env.flags.labview_version = Some("2099".into());
        let why = uninstall(&env, Some("alpha"), false, Dependencies::Keep).unwrap_err();

        assert!(
            why.to_string().contains("outside this destination"),
            "{why:#}"
        );
        let events = sink.events();
        assert!(
            !events.iter().any(|event| matches!(
                event,
                Event::HookStarting { .. } | Event::HookRan { .. } | Event::HookFailed { .. }
            )),
            "a hook was tried: {events:?}"
        );
        assert!(record.exists(), "the record is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Uninstall `hooked` from the installation that is not there, its record
    /// listing `skipped` as not run and shipping the uninstall hooks named in
    /// `declares`. Returns the events: a hook "started" is a `HookStarting`.
    fn uninstall_hooked(tag: &str, skipped: &[&str], declares: &[&str]) -> Vec<Event> {
        uninstall_hooked_after(tag, skipped, declares, |_| ())
    }

    /// As [`uninstall_hooked`], with `before` run on the destination first.
    fn uninstall_hooked_after(
        tag: &str,
        skipped: &[&str],
        declares: &[&str],
        before: impl FnOnce(&Roots),
    ) -> Vec<Event> {
        use crate::ops::testing::{FakeInstallation, Recorded, env_on, scratch_dir};
        let dir = scratch_dir(tag);
        let fake = FakeInstallation::new(&dir);
        let hooks = fake.roots().hooks_dir().join("hooked");
        std::fs::create_dir_all(&hooks).unwrap();
        let mut manifest = serde_json::json!({
            "name": "hooked", "version": "1.0", "display_name": null, "installed_at": 0,
            "target": "fake", "files": [],
            "skipped_hooks": skipped.iter().map(|h| format!("{h}.vi")).collect::<Vec<_>>(),
        });
        for hook in declares {
            let vi = hooks.join(format!("{hook}.vi"));
            std::fs::write(&vi, "vi").unwrap();
            let key = match *hook {
                "PreUninstall" => "pre_uninstall_vi",
                "PostInstall" => "post_install_vi",
                _ => "post_uninstall_vi",
            };
            manifest[key] = vi.to_string_lossy().replace('\\', "/").into();
        }
        let store = fake.roots().store_dir();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("hooked.json"), manifest.to_string()).unwrap();
        before(&fake.roots());

        let sink = Recorded::default();
        let mut env = env_on(&sink, &dir);
        fake.place(&mut env);
        env.headless = true;
        env.flags.global = true;
        env.flags.labview_version = Some("2099".into());
        uninstall(&env, Some("hooked"), false, Dependencies::Keep).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        sink.events()
    }

    fn started(events: &[Event]) -> Vec<HookKind> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::HookStarting { hook, .. } => Some(*hook),
                _ => None,
            })
            .collect()
    }

    /// The hooks the uninstall said it did not run, and why.
    fn not_run(events: &[Event]) -> Vec<(String, Vec<String>, UnrunReason)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::UninstallHooksNotRun {
                    package,
                    hooks,
                    reason,
                    ..
                } => Some((package.clone(), hooks.clone(), *reason)),
                _ => None,
            })
            .collect()
    }

    const BOTH_UNINSTALL: &[&str] = &["PreUninstall", "PostUninstall"];

    fn held_back(events: &[Event]) {
        assert_eq!(started(events), [], "{events:?}");
        let said = not_run(events);
        assert_eq!(said.len(), 1, "{events:?}");
        let (package, hooks, reason) = &said[0];
        assert_eq!(package, "hooked");
        assert_eq!(*reason, UnrunReason::InstallHookSkipped);
        let names: Vec<&str> = hooks.iter().map(|h| h.split('=').next().unwrap()).collect();
        assert_eq!(names, BOTH_UNINSTALL, "{hooks:?}");
    }

    #[test]
    fn a_package_whose_install_hooks_were_skipped_runs_no_uninstall_hook_and_says_why() {
        // As a skip records it: every declared hook, uninstall ones included.
        let skipped = [
            "PreInstall=a",
            "PostInstall=b",
            "PreUninstall=c",
            "PostUninstall=d",
        ];
        held_back(&uninstall_hooked("skip-all", &skipped, BOTH_UNINSTALL));
    }

    #[test]
    fn a_failed_post_install_keeps_the_pending_uninstall_hooks_off_and_reports_them() {
        // A failed PostInstall lists only itself: the uninstall hooks are
        // pending, not skipped, and are still named in the report.
        let events = uninstall_hooked("failed-post", &["PostInstall=b"], BOTH_UNINSTALL);
        held_back(&events);
    }

    #[test]
    fn a_skipped_install_hook_outranks_uninstall_hooks_that_were_never_extracted() {
        // A headless install from before uninstall hooks were extracted.
        let skipped = [
            "PreInstall=a",
            "PostInstall=b",
            "PreUninstall=c",
            "PostUninstall=d",
        ];
        held_back(&uninstall_hooked("legacy-headless", &skipped, &[]));
    }

    #[test]
    fn a_record_that_lists_uninstall_hooks_it_never_extracted_says_it_predates_them() {
        // From before uninstall hooks were extracted: they are listed as
        // skipped and the record holds no VI for them.
        let events = uninstall_hooked("legacy", &["PreUninstall=c", "PostUninstall=d"], &[]);
        assert_eq!(started(&events), []);
        let said = not_run(&events);
        assert_eq!(said.len(), 1, "{events:?}");
        assert_eq!(said[0].1.len(), 2, "{events:?}");
        assert_eq!(said[0].2, UnrunReason::InstalledBeforeUninstallHooks);
    }

    #[test]
    fn a_record_with_uninstall_hooks_extracted_and_listed_as_skipped_says_they_were_not_allowed() {
        // An uninstall-only package installed without hooks allowed.
        let events = uninstall_hooked("only-uninstall", &["PreUninstall=c"], &["PreUninstall"]);
        assert_eq!(started(&events), []);
        let said = not_run(&events);
        assert_eq!(said.len(), 1, "{events:?}");
        assert_eq!(said[0].2, UnrunReason::NotAllowedAtInstall);
    }

    #[test]
    fn a_venv_with_a_skipped_record_says_a_venv_runs_no_hooks() {
        use crate::ops::testing::{Recorded, env_on, scratch_dir, venv_in};
        let repo = scratch_dir("venv-skipped");
        let venv = venv_in(&repo);
        let store = venv.store().store_dir();
        std::fs::create_dir_all(&store).unwrap();
        let record = serde_json::json!({
            "name": "alpha", "version": "1.0", "display_name": null, "installed_at": 0,
            "target": "venv", "files": [],
            "skipped_hooks": ["PostInstall=b.vi", "PreUninstall=c.vi"],
        });
        std::fs::write(store.join("alpha.json"), record.to_string()).unwrap();

        let sink = Recorded::default();
        uninstall(
            &env_on(&sink, &repo),
            Some("alpha"),
            false,
            Dependencies::Keep,
        )
        .unwrap();

        let events = sink.events();
        assert_eq!(started(&events), []);
        let said = not_run(&events);
        assert_eq!(said.len(), 1, "{events:?}");
        assert_eq!(said[0].2, UnrunReason::VenvRunsNoHooks);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_package_whose_install_hooks_were_cleared_by_run_hooks_runs_its_uninstall_hooks() {
        let skipped = ["PreInstall=a", "PostInstall=b"];
        let events = uninstall_hooked_after("cleared", &skipped, BOTH_UNINSTALL, |roots| {
            let record = crate::install::read_manifest(roots, "hooked").unwrap();
            crate::ops::hooks::settle_rerun(roots, &record, Ok(std::time::Duration::ZERO)).unwrap();
        });
        assert_eq!(
            started(&events),
            [HookKind::PreUninstall, HookKind::PostUninstall]
        );
        assert_eq!(not_run(&events), []);
    }

    #[test]
    fn the_run_hooks_remedy_is_offered_only_to_a_package_with_a_post_install_to_run() {
        let helps = |tag: &str, declares: &[&str]| {
            let events = uninstall_hooked(tag, &["PreInstall=a"], declares);
            events
                .iter()
                .find_map(|e| match e {
                    Event::UninstallHooksNotRun {
                        run_hooks_helps, ..
                    } => Some(*run_hooks_helps),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("nothing said: {events:?}"))
        };
        assert!(helps("helps-post", &["PostInstall", "PreUninstall"]));
        assert!(!helps("helps-pre-only", &["PreUninstall"]));
    }

    #[test]
    fn a_package_that_ran_its_install_hooks_runs_its_uninstall_hooks() {
        let events = uninstall_hooked("allowed", &[], BOTH_UNINSTALL);
        assert_eq!(
            started(&events),
            [HookKind::PreUninstall, HookKind::PostUninstall]
        );
        assert_eq!(not_run(&events), []);
    }

    #[test]
    fn naming_nothing_or_a_package_that_is_not_installed_is_refused() {
        let (dir, _) = scratch_with_alpha("refused");
        let sink = Recorder::default();
        let env = env_for(&sink, &dir);
        let nothing = uninstall(&env, None, false, Dependencies::Keep)
            .unwrap_err()
            .to_string();
        assert_eq!(nothing, "give a package name, or --all");
        let absent = uninstall(&env, Some("beta"), false, Dependencies::Keep)
            .unwrap_err()
            .to_string();
        assert!(
            absent.starts_with("beta is not installed in this target"),
            "{absent}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
