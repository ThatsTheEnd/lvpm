//! The wording of the install's events: what it reads, what it resolved, what
//! each package did, and the phases that did not run.

use super::headless;
use super::hooks::hook_names;
use super::{Renderer, Stream};
use lvpm_core::events::{AllowLine, Event, HookKind, Phase, PlannedFile, SkipReason};

/// How many of a package's planned writes a dry run lists before "and N more".
const PREVIEW_LIMIT: usize = 6;

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

pub(super) fn render(r: &mut Renderer, event: &Event) {
    match event {
        Event::ManifestInUse {
            path,
            name,
            dependencies,
            labview,
        } => {
            let named = name.as_deref().map(|n| format!(" ({n})")).unwrap_or_default();
            r.line(
                Stream::Err,
                &format!(
                    "manifest: {}{named} — {dependencies} dependenc{}",
                    path.display(),
                    plural(*dependencies, "y", "ies")
                ),
            );
            if let Some(v) = labview {
                r.line(
                    Stream::Err,
                    &format!("      manifest says labview = \">={v}\" (minimum)"),
                );
            }
        }
        Event::VipmManifestConverted {
            path,
            unused,
            labview,
            dev_dependencies,
        } => {
            r.line(
                Stream::Err,
                &format!("manifest: {} (VIPM format, converted)", path.display()),
            );
            if !unused.is_empty() {
                r.line(Stream::Err, &super::printable(format!("  not used: {}", unused.join(", "))));
            }
            if let Some(lv) = labview {
                r.line(
                    Stream::Err,
                    &format!("  labview-version {lv:?} read as minimum labview = \">={lv}\""),
                );
            }
            if *dev_dependencies > 0 {
                r.line(
                    Stream::Err,
                    "  [dev-dependencies] read as lvpm's [dev-dependencies]",
                );
            }
        }
        Event::InstallTarget {
            labview: Some(label),
            path,
        } => r.line(
            Stream::Err,
            &format!("target: {label}  ({})", path.display()),
        ),
        Event::InstallTarget {
            labview: None,
            path,
        } => r.line(
            Stream::Err,
            &format!("target: scratch tree at {}", path.display()),
        ),
        Event::IndexesLoading => r.line(Stream::Err, "loading indexes..."),
        Event::IndexesLoaded { versions } => r.line(
            Stream::Err,
            &format!("  {versions} package versions known"),
        ),
        Event::DependencySatisfied { name, version } => r.line(
            Stream::Err,
            &format!("  = {name} {version} already installed, satisfies dependency"),
        ),
        Event::DependencyUnresolved { name } => r.line(
            Stream::Err,
            &format!("  ! dependency {name} unresolved, skipping"),
        ),
        Event::Resolved { packages } => {
            r.line(
                Stream::Out,
                &format!(
                    "\nresolved {} package{}:",
                    packages.len(),
                    plural(packages.len(), "", "s")
                ),
            );
            for package in packages {
                r.line(
                    Stream::Out,
                    &format!("  {} {}", package.name, package.version),
                );
            }
            r.line(Stream::Out, "");
        }
        Event::NipmNotInstalled { count } => r.line(
            Stream::Out,
            &format!(
                "note: the manifest also lists {count} [nipm.dependencies] — those are NI Package\n      \
                 Manager packages and lvpm does not install them.\n"
            ),
        ),
        Event::PackageAlreadyInstalled { name, version } => r.line(
            Stream::Out,
            &format!("= {name} {version} already installed"),
        ),
        Event::PackageNotDowngraded {
            name,
            installed,
            wanted,
        } => r.line(
            Stream::Out,
            &format!(
                "= {name} {installed} installed, newer than {wanted} — not downgrading (pass --allow-downgrade)"
            ),
        ),
        Event::PackageReplacing {
            name,
            installed,
            wanted,
            upgrade,
            old_files,
            dry_run,
        } => {
            let mark = if *upgrade { "^" } else { "v" };
            let what = match dry_run {
                true => format!("{old_files} old files would be removed first"),
                false => "removing the old version first".to_string(),
            };
            r.line(
                Stream::Out,
                &format!("{mark} {name} {installed} -> {wanted}: {what}"),
            );
        }
        Event::PackageStart {
            name,
            version,
            dry_run,
        } => {
            let mark = if *dry_run { "?" } else { "+" };
            r.package_line = format!("{mark} {name} {version} ... ");
            r.begin(Stream::Out, &r.package_line.clone());
        }
        Event::PackageDone {
            files,
            dry_run: true,
        } => r.end(Stream::Out, &format!("{files} files would be written")),
        Event::PackageDone {
            files,
            dry_run: false,
        } => match r.open {
            Some(Stream::Out) => r.end(Stream::Out, &format!("{files} files")),
            _ => r.line(Stream::Out, &format!("{}{files} files", r.package_line)),
        },
        Event::PlannedWrites { files } => planned_writes(r, files),
        Event::FilesKept { count } => r.line(
            Stream::Out,
            &format!("      {count} existing file(s) left alone (Replace Mode = If Newer)"),
        ),
        Event::GroupSkipped { target_dir, files } => r.line(
            Stream::Out,
            &format!(
                "      {files} file(s) for {target_dir} skipped — a venv holds LabVIEW-tree content only"
            ),
        ),
        Event::MissingFromArchive { entry } => r.line(
            Stream::Out,
            &format!("      ! listed in spec but absent from archive: {entry}"),
        ),
        Event::HooksNotRun { hooks } => r.line(
            Stream::Out,
            &format!("      hooks not run: {}", hook_names(hooks)),
        ),
        Event::HooksSkipped {
            package,
            hooks,
            reason,
            run_now,
            dry_run,
            allow,
        } => {
            let text = hooks_skipped(package, hooks, *reason, *run_now, *dry_run, allow.as_ref());
            r.line(Stream::Err, &super::printable(text))
        }
        Event::InstallDone {
            files,
            dry_run: true,
            ..
        } => r.line(
            Stream::Out,
            &format!("\ndry run: {files} files would be written, nothing changed"),
        ),
        Event::InstallDone {
            files,
            dry_run: false,
            ..
        } => r.line(Stream::Out, &format!("\ndone. {files} files written.")),
        Event::PhaseSkipped {
            phase,
            reason,
            skipped,
        } => phase_skipped(r, *phase, *reason, *skipped),
        Event::LaunchHint => r.line(
            Stream::Out,
            "\nnext: `lvpm launch` starts LabVIEW on this venv — a LabVIEW already open on it\n      \
             has to be restarted to see what was just installed",
        ),
        Event::StagingLeftoverRemoved { dir } => r.line(
            Stream::Out,
            &format!(
                "removed the staging folder an interrupted install left: {}",
                dir.display()
            ),
        ),
        Event::UnrecordedFolderReplaced { dir } => r.line(
            Stream::Out,
            &format!(
                "      replaced {}, which no install record named; files added there by hand are gone",
                dir.display()
            ),
        ),
        Event::ResaveStarting => r.line(
            Stream::Out,
            "starting the venv's LabVIEW for the resave (about a minute per package)",
        ),
        Event::ResaveStarted {
            package,
            position,
            total,
        } => {
            r.resaved = true;
            r.begin(
                Stream::Out,
                &format!("resave ({position}/{total}) {package} ... "),
            )
        }
        Event::ResaveDone { saved: 0, took, .. } => r.end(
            Stream::Out,
            &format!("nothing to save ({}s)", took.as_secs()),
        ),
        Event::ResaveDone { saved, took, .. } => r.end(
            Stream::Out,
            &format!("{saved} saved ({}s)", took.as_secs()),
        ),
        Event::ResaveFailed {
            package,
            error,
            folder,
        } => {
            r.end(Stream::Out, "FAILED");
            r.line(
                Stream::Err,
                &format!(
                    "warning: resave of {package} failed ({}). {package} is installed, but VIs \
                     saved in an older LabVIEW may load broken. Reinstall {package} first; if it \
                     fails again, run lvpm launch and mass compile {}.",
                    one_line(error),
                    folder.display()
                ),
            );
        }
        Event::ResaveSkipped {
            package,
            labview,
            position,
            total,
        } => r.line(
            Stream::Out,
            &format!(
                "resave ({position}/{total}) {package} ... already saved in LabVIEW {labview}, skipped"
            ),
        ),
        Event::ResavePlanned { packages } => {
            r.line(
                Stream::Out,
                &format!(
                    "\nresave: {} packages in the venv's LabVIEW:",
                    packages.len()
                ),
            );
            for package in packages.iter().take(PREVIEW_LIMIT) {
                r.line(Stream::Out, &format!("      {package}"));
            }
            if packages.len() > PREVIEW_LIMIT {
                r.line(
                    Stream::Out,
                    &format!("      ... and {} more", packages.len() - PREVIEW_LIMIT),
                );
            }
        }
        Event::ResaveStartFailed { error, packages } => r.line(
            Stream::Err,
            &format!(
                "warning: the venv's LabVIEW could not start ({}). {} packages were installed \
                 without a resave: {}. Fix the start problem, then reinstall them.",
                one_line(error),
                packages.len(),
                packages.join(", ")
            ),
        ),
        Event::ResaveRecap { failed, packages } => r.line(
            Stream::Out,
            &format!(
                "resave: {} of {packages} packages failed: {} (see the warnings above)",
                failed.len(),
                failed.join(", ")
            ),
        ),
        _ => unreachable!("routed here by Renderer::render: {event:?}"),
    }
}

/// The warning for a package whose install hooks were not run: what was
/// skipped and why, then what to do now, then what to do for good. The lines
/// the data does not call for are left out, and so is a line to paste that
/// would carry a name that is not safe to paste.
fn hooks_skipped(
    package: &str,
    hooks: &[HookKind],
    reason: SkipReason,
    run_now: bool,
    dry_run: bool,
    allow: Option<&AllowLine>,
) -> String {
    let kinds = hook_names(hooks.iter().map(|hook| hook.name()));
    let s = plural(hooks.len(), "", "s");
    let verb = if dry_run { "would skip" } else { "skipped" };
    let shown = one_line(package);
    let mut lines = Vec::new();
    lines.push(match reason {
        SkipReason::AlreadyInstalled => format!(
            "warning: {shown}: install hook{s} {kinds} {} skipped when it was installed",
            plural(hooks.len(), "was", "were")
        ),
        SkipReason::Headless => format!(
            "warning: {shown}: {verb} install hook{s} {kinds} \
             (LabVIEW runs headless; hooks = \"run\" and hooks.run do not apply)"
        ),
        _ => format!(
            "warning: {shown}: {verb} install hook{s} {kinds} \
             (hooks are off by default; see docs/labview-integration.md#hooks)"
        ),
    });
    if run_now {
        if pasteable(package) {
            lines.push(format!("  run it now:     lvpm run-hooks {package}"));
        }
    } else if reason == SkipReason::NotAllowed && !dry_run {
        lines.push(
            "  run it:         uninstall, allow it, then install again \
             (PreInstall runs only at install)"
                .to_string(),
        );
    }
    lines.extend(allow.and_then(allow_line));
    lines.join("\n")
}

/// The line that says how to allow the hooks for good; None when a name in it
/// is not safe to paste.
fn allow_line(allow: &AllowLine) -> Option<String> {
    match allow {
        AllowLine::AddKey { name, table } if pasteable(name) && pasteable(table) => Some(format!(
            "  always run it:  add hooks = \"run\" to the {name} line in [{table}] of lvpm.toml"
        )),
        AllowLine::AddLine { name } if pasteable(name) => Some(format!(
            "  always run it:  add to [dependencies] in lvpm.toml:  \
             {name} = {{ version = \"*\", hooks = \"run\" }}"
        )),
        AllowLine::ConfigSet { names, skipped } if names.iter().all(|n| pasteable(n)) => {
            Some(format!(
                "  always run {}:  lvpm config set hooks.run {}",
                plural(*skipped, "it", "them"),
                names.join(" ")
            ))
        }
        AllowLine::PassHooks => Some("  always run it:  add --hooks to this command".to_string()),
        _ => None,
    }
}

/// Package names come from feeds unvalidated, and the user pastes these lines
/// into a shell: only a plain name, one that cannot read as a flag, goes in.
fn pasteable(name: &str) -> bool {
    !name.starts_with('-')
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// A cause's text on one line, for a warning that must stay one.
fn one_line(error: &str) -> String {
    error.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn planned_writes(r: &mut Renderer, files: &[PlannedFile]) {
    for file in files.iter().take(PREVIEW_LIMIT) {
        let overwrites = if file.overwrites {
            "   (overwrites)"
        } else {
            ""
        };
        r.line(
            Stream::Out,
            &format!("      {}{overwrites}", file.path.display()),
        );
    }
    if files.len() > PREVIEW_LIMIT {
        r.line(
            Stream::Out,
            &format!("      ... and {} more", files.len() - PREVIEW_LIMIT),
        );
    }
}

/// A phase that left nothing undone says nothing, and so does one whose
/// skipping was never worth a line: a scratch tree's refresh.
fn phase_skipped(r: &mut Renderer, phase: Phase, reason: SkipReason, skipped: usize) {
    if skipped == 0 {
        return;
    }
    let text = match (phase, reason) {
        (Phase::Relink, SkipReason::ScratchDestination) => {
            "\nrelink: skipped — a scratch tree has no LabVIEW to relink with"
        }
        (Phase::Relink, SkipReason::Headless) if r.resaved => {
            "\nrelink: skipped (headless). The resave still ran."
        }
        (phase, SkipReason::Headless) => return headless::phase_skipped(r, phase),
        (Phase::Relink, SkipReason::Declined) if r.resaved => {
            "\nrelink: skipped (--no-relink). The resave still ran."
        }
        (Phase::Relink, SkipReason::Declined) => {
            "\nrelink: skipped (--no-relink). These VIs still declare the paths their\n      \
             build machine wrote, so run `lvpm relink <package>` before using them."
        }
        (Phase::Hooks, SkipReason::ScratchDestination) => {
            "\npost-install hooks: skipped — a scratch tree has no LabVIEW to run them"
        }
        _ => return,
    };
    r.line(Stream::Out, text);
}

#[cfg(test)]
mod tests {
    use super::super::tests::renderer;
    use super::*;
    use lvpm_core::events::PackageRef;
    use std::path::PathBuf;

    fn text_of(events: &[Event]) -> (String, String) {
        let (mut renderer, out, err) = renderer();
        for event in events {
            renderer.render(event);
        }
        (out.text(), err.text())
    }

    #[test]
    fn what_the_install_reads_and_where_it_goes_is_on_stderr() {
        let (out, err) = text_of(&[
            Event::ManifestInUse {
                path: PathBuf::from("proj").join("lvpm.toml"),
                name: Some("demo".into()),
                dependencies: 1,
                labview: Some("2024".into()),
            },
            Event::InstallTarget {
                labview: Some("LabVIEW 2026 (64-bit)  v26.3".into()),
                path: PathBuf::from("LV"),
            },
            Event::InstallTarget {
                labview: None,
                path: PathBuf::from("dest").join("LabVIEW"),
            },
            Event::IndexesLoading,
            Event::IndexesLoaded { versions: 4 },
        ]);
        assert_eq!(out, "");
        assert_eq!(
            err,
            format!(
                "manifest: {} (demo) — 1 dependency\n\
                 \x20     manifest says labview = \">=2024\" (minimum)\n\
                 target: LabVIEW 2026 (64-bit)  v26.3  (LV)\n\
                 target: scratch tree at {}\n\
                 loading indexes...\n  4 package versions known\n",
                PathBuf::from("proj").join("lvpm.toml").display(),
                PathBuf::from("dest").join("LabVIEW").display()
            )
        );
    }

    #[test]
    fn a_resolution_is_a_counted_list_and_a_blank_line() {
        let package = |name: &str, version: &str| PackageRef {
            name: name.into(),
            version: version.into(),
        };
        let (out, _) = text_of(&[Event::Resolved {
            packages: vec![package("alpha", "2.0"), package("beta", "1.0")],
        }]);
        assert_eq!(out, "\nresolved 2 packages:\n  alpha 2.0\n  beta 1.0\n\n");
        let (out, _) = text_of(&[Event::Resolved {
            packages: vec![package("alpha", "2.0")],
        }]);
        assert!(out.starts_with("\nresolved 1 package:\n"), "{out}");
    }

    #[test]
    fn a_package_is_one_line_in_two_parts_that_a_pre_install_hook_extends() {
        let (out, err) = text_of(&[
            Event::PackageStart {
                name: "alpha".into(),
                version: "2.0".into(),
                dry_run: false,
            },
            Event::HookRan {
                package: "alpha".into(),
                hook: lvpm_core::events::HookKind::PreInstall,
                took: std::time::Duration::from_millis(1_250),
            },
            Event::PackageDone {
                files: 3,
                dry_run: false,
            },
        ]);
        assert_eq!(out, "+ alpha 2.0 ... [pre-install ok, 1.2s] 3 files\n");
        assert_eq!(err, "");
    }

    #[test]
    fn a_dry_run_lists_six_planned_writes_and_counts_the_rest() {
        let files: Vec<PlannedFile> = (1..=8)
            .map(|n| PlannedFile {
                path: PathBuf::from(format!("f{n}")),
                overwrites: n == 2,
            })
            .collect();
        let (out, _) = text_of(&[
            Event::PackageStart {
                name: "alpha".into(),
                version: "2.0".into(),
                dry_run: true,
            },
            Event::PackageDone {
                files: 8,
                dry_run: true,
            },
            Event::PlannedWrites { files },
        ]);
        assert_eq!(
            out,
            "? alpha 2.0 ... 8 files would be written\n      f1\n      f2   (overwrites)\n      f3\n      f4\n      f5\n      f6\n      ... and 2 more\n"
        );
    }

    #[test]
    fn a_replaced_package_says_which_way_and_whether_it_happened() {
        let replacing = |upgrade, dry_run| Event::PackageReplacing {
            name: "alpha".into(),
            installed: "1.0".into(),
            wanted: "2.0".into(),
            upgrade,
            old_files: 4,
            dry_run,
        };
        let (out, _) = text_of(&[replacing(true, false), replacing(false, true)]);
        assert_eq!(
            out,
            "^ alpha 1.0 -> 2.0: removing the old version first\n\
             v alpha 1.0 -> 2.0: 4 old files would be removed first\n"
        );
    }

    #[test]
    fn the_totals_differ_on_a_dry_run() {
        let (out, _) = text_of(&[
            Event::InstallDone {
                files: 2,
                dry_run: true,
                resave_failed: Vec::new(),
            },
            Event::InstallDone {
                files: 2,
                dry_run: false,
                resave_failed: Vec::new(),
            },
        ]);
        assert_eq!(
            out,
            "\ndry run: 2 files would be written, nothing changed\n\ndone. 2 files written.\n"
        );
    }

    #[test]
    fn a_skipped_phase_speaks_only_when_something_was_left_undone() {
        let skipped = |phase, reason, skipped| Event::PhaseSkipped {
            phase,
            reason,
            skipped,
        };
        let (out, err) = text_of(&[
            skipped(Phase::Relink, SkipReason::ScratchDestination, 0),
            skipped(Phase::Hooks, SkipReason::ScratchDestination, 0),
            skipped(Phase::Refresh, SkipReason::ScratchDestination, 3),
            skipped(Phase::Relink, SkipReason::ScratchDestination, 2),
            skipped(Phase::Hooks, SkipReason::ScratchDestination, 1),
        ]);
        assert_eq!(
            out,
            "\nrelink: skipped — a scratch tree has no LabVIEW to relink with\n\
             \npost-install hooks: skipped — a scratch tree has no LabVIEW to run them\n"
        );
        assert_eq!(err, "");
    }

    #[test]
    fn a_phase_skipped_because_labview_runs_headless_says_what_to_do_instead() {
        let (out, err) = text_of(&[
            Event::PhaseSkipped {
                phase: Phase::Relink,
                reason: SkipReason::Headless,
                skipped: 1,
            },
            Event::PhaseSkipped {
                phase: Phase::Hooks,
                reason: SkipReason::Headless,
                skipped: 1,
            },
            Event::PhaseSkipped {
                phase: Phase::Refresh,
                reason: SkipReason::Headless,
                skipped: 1,
            },
        ]);
        assert_eq!(
            out,
            "\nrelink: skipped — LabVIEW runs headless here (LV_RTE_HEADLESS); a compile or build\n      \
             resolves the links as it loads. --relink forces the pass, `lvpm relink --all` runs it later.\n\
             \nhooks: skipped — LabVIEW runs headless here (LV_RTE_HEADLESS). Install hooks mostly add\n      \
             palettes, menus or markers for the IDE; a package whose hook does real setup needs --hooks.\n\
             \npalettes and menus: not refreshed — LabVIEW runs headless here (`lvpm refresh` if wanted)\n"
        );
        assert_eq!(err, "");
    }

    #[test]
    fn hooks_not_run_are_named_without_the_path_they_had_on_the_build_machine() {
        let (out, _) = text_of(&[Event::HooksNotRun {
            hooks: vec![
                "PostInstall=<OS Boot Volume Root>\\ci\\PostInstall.vi".into(),
                "PreUninstall=x.vi".into(),
            ],
        }]);
        assert_eq!(out, "      hooks not run: PostInstall, PreUninstall\n");
    }

    #[test]
    fn a_resave_has_one_line_per_package_a_warning_per_failure_and_a_recap() {
        use std::time::Duration;
        let started = |package: &str, position| Event::ResaveStarted {
            package: package.into(),
            position,
            total: 3,
        };
        let (out, err) = text_of(&[
            Event::ResaveStarting,
            started("alpha", 1),
            Event::ResaveFailed {
                package: "alpha".into(),
                error: "1 of 1 files did not save; first a.vi: VI Server\nerror 1003".into(),
                folder: PathBuf::from("venv").join("alpha"),
            },
            started("beta", 2),
            Event::ResaveDone {
                package: "beta".into(),
                saved: 3,
                took: Duration::from_millis(14_400),
            },
            started("gamma", 3),
            Event::ResaveDone {
                package: "gamma".into(),
                saved: 0,
                took: Duration::from_secs(9),
            },
            Event::ResaveRecap {
                failed: vec!["alpha".into()],
                packages: 3,
            },
        ]);
        assert_eq!(
            out,
            "starting the venv's LabVIEW for the resave (about a minute per package)\n\
             resave (1/3) alpha ... FAILED\n\
             resave (2/3) beta ... 3 saved (14s)\n\
             resave (3/3) gamma ... nothing to save (9s)\n\
             resave: 1 of 3 packages failed: alpha (see the warnings above)\n"
        );
        let folder = PathBuf::from("venv").join("alpha");
        assert_eq!(
            err,
            format!(
                "warning: resave of alpha failed (1 of 1 files did not save; first a.vi: VI \
                 Server error 1003). alpha is installed, but VIs saved in an older LabVIEW may \
                 load broken. Reinstall alpha first; if it fails again, run lvpm launch and mass \
                 compile {}.\n",
                folder.display()
            )
        );
    }

    #[test]
    fn a_labview_that_could_not_start_is_one_warning_naming_every_package() {
        let (_, err) = text_of(&[Event::ResaveStartFailed {
            error: "no LabVIEW.exe in LabVIEW 2026".into(),
            packages: vec!["alpha".into(), "beta".into()],
        }]);
        assert_eq!(
            err,
            "warning: the venv's LabVIEW could not start (no LabVIEW.exe in LabVIEW 2026). 2 \
             packages were installed without a resave: alpha, beta. Fix the start problem, then \
             reinstall them.\n"
        );
    }

    #[test]
    fn a_relink_skipped_after_a_resave_says_the_resave_still_ran() {
        let resaved_then = |reason| {
            text_of(&[
                Event::ResaveStarted {
                    package: "alpha".into(),
                    position: 1,
                    total: 1,
                },
                Event::ResaveDone {
                    package: "alpha".into(),
                    saved: 1,
                    took: std::time::Duration::from_secs(9),
                },
                Event::PhaseSkipped {
                    phase: Phase::Relink,
                    reason,
                    skipped: 1,
                },
            ])
            .0
        };
        assert!(
            resaved_then(SkipReason::Declined)
                .ends_with("\nrelink: skipped (--no-relink). The resave still ran.\n")
        );
        assert!(
            resaved_then(SkipReason::Headless)
                .ends_with("\nrelink: skipped (headless). The resave still ran.\n")
        );
    }

    #[test]
    fn a_dry_run_lists_the_resaves_capped() {
        let packages: Vec<String> = (1..=8).map(|i| format!("p{i}")).collect();
        let (out, _) = text_of(&[Event::ResavePlanned { packages }]);
        assert_eq!(
            out,
            "\nresave: 8 packages in the venv's LabVIEW:\n      p1\n      p2\n      p3\n      \
             p4\n      p5\n      p6\n      ... and 2 more\n"
        );
    }

    #[test]
    fn a_package_already_saved_in_the_venvs_labview_is_one_skipped_line() {
        let (out, _) = text_of(&[Event::ResaveSkipped {
            package: "alpha".into(),
            labview: 2026,
            position: 2,
            total: 5,
        }]);
        assert_eq!(
            out,
            "resave (2/5) alpha ... already saved in LabVIEW 2026, skipped\n"
        );
    }

    #[test]
    fn a_package_line_a_resave_came_between_is_written_again_in_full() {
        let (out, _) = text_of(&[
            Event::PackageStart {
                name: "alpha".into(),
                version: "1.0".into(),
                dry_run: false,
            },
            Event::ResaveStarted {
                package: "alpha".into(),
                position: 1,
                total: 1,
            },
            Event::ResaveDone {
                package: "alpha".into(),
                saved: 1,
                took: std::time::Duration::from_secs(9),
            },
            Event::PackageDone {
                files: 2,
                dry_run: false,
            },
        ]);
        assert_eq!(
            out,
            "+ alpha 1.0 ... 
resave (1/1) alpha ... 1 saved (9s)
+ alpha 1.0 ... 2 files
"
        );
    }

    /// A hooks-skipped event as the install tells it.
    fn skipped_hooks(
        package: &str,
        hooks: &[HookKind],
        reason: SkipReason,
        run_now: bool,
        dry_run: bool,
        allow: Option<AllowLine>,
    ) -> Event {
        Event::HooksSkipped {
            package: package.into(),
            hooks: hooks.to_vec(),
            reason,
            run_now,
            dry_run,
            allow,
        }
    }

    const OFF_BY_DEFAULT: &str =
        "(hooks are off by default; see docs/labview-integration.md#hooks)";

    fn config_set(names: &[&str], skipped: usize) -> Option<AllowLine> {
        Some(AllowLine::ConfigSet {
            names: names.iter().map(|n| n.to_string()).collect(),
            skipped,
        })
    }

    /// The rows of the message table in the plan and the spec, word for word.
    #[test]
    fn the_hooks_skipped_warning_reads_as_the_message_table_says() {
        use HookKind::{PostInstall, PreInstall};
        use SkipReason::{AlreadyInstalled, Headless, NotAllowed};
        let key = Some(AllowLine::AddKey {
            name: "pilot_a".into(),
            table: "dependencies".into(),
        });
        let rows: Vec<(&str, Vec<Event>, String)> = vec![
            (
                "not allowed, project, listed",
                vec![skipped_hooks("pilot_a", &[PostInstall], NotAllowed, true, false, key.clone())],
                format!(
                    "warning: pilot_a: skipped install hook PostInstall {OFF_BY_DEFAULT}\n\
                     \x20 run it now:     lvpm run-hooks pilot_a\n\
                     \x20 always run it:  add hooks = \"run\" to the pilot_a line in [dependencies] of lvpm.toml\n"
                ),
            ),
            (
                "not allowed, project, transitive",
                vec![skipped_hooks(
                    "pilot_a",
                    &[PostInstall],
                    NotAllowed,
                    true,
                    false,
                    Some(AllowLine::AddLine { name: "pilot_a".into() }),
                )],
                format!(
                    "warning: pilot_a: skipped install hook PostInstall {OFF_BY_DEFAULT}\n\
                     \x20 run it now:     lvpm run-hooks pilot_a\n\
                     \x20 always run it:  add to [dependencies] in lvpm.toml:  pilot_a = {{ version = \"*\", hooks = \"run\" }}\n"
                ),
            ),
            (
                "not allowed, global, one",
                vec![skipped_hooks(
                    "pilot_a",
                    &[PostInstall],
                    NotAllowed,
                    true,
                    false,
                    config_set(&["pilot_a"], 1),
                )],
                format!(
                    "warning: pilot_a: skipped install hook PostInstall {OFF_BY_DEFAULT}\n\
                     \x20 run it now:     lvpm run-hooks pilot_a\n\
                     \x20 always run it:  lvpm config set hooks.run pilot_a\n"
                ),
            ),
            (
                "not allowed, global, two",
                vec![
                    skipped_hooks("pilot_a", &[PostInstall], NotAllowed, true, false, None),
                    skipped_hooks(
                        "pilot_b",
                        &[PostInstall],
                        NotAllowed,
                        true,
                        false,
                        config_set(&["other_pkg", "pilot_a", "pilot_b"], 2),
                    ),
                ],
                format!(
                    "warning: pilot_a: skipped install hook PostInstall {OFF_BY_DEFAULT}\n\
                     \x20 run it now:     lvpm run-hooks pilot_a\n\
                     warning: pilot_b: skipped install hook PostInstall {OFF_BY_DEFAULT}\n\
                     \x20 run it now:     lvpm run-hooks pilot_b\n\
                     \x20 always run them:  lvpm config set hooks.run other_pkg pilot_a pilot_b\n"
                ),
            ),
            (
                "headless",
                vec![skipped_hooks(
                    "pilot_a",
                    &[PostInstall],
                    Headless,
                    false,
                    false,
                    Some(AllowLine::PassHooks),
                )],
                "warning: pilot_a: skipped install hook PostInstall (LabVIEW runs headless; hooks = \"run\" and hooks.run do not apply)\n\
                 \x20 always run it:  add --hooks to this command\n"
                    .to_string(),
            ),
            (
                "two hooks",
                vec![skipped_hooks(
                    "pilot_a",
                    &[PreInstall, PostInstall],
                    NotAllowed,
                    true,
                    false,
                    key.clone(),
                )],
                format!(
                    "warning: pilot_a: skipped install hooks PreInstall, PostInstall {OFF_BY_DEFAULT}\n\
                     \x20 run it now:     lvpm run-hooks pilot_a\n\
                     \x20 always run it:  add hooks = \"run\" to the pilot_a line in [dependencies] of lvpm.toml\n"
                ),
            ),
            (
                "PreInstall only",
                vec![skipped_hooks("pilot_a", &[PreInstall], NotAllowed, false, false, key.clone())],
                format!(
                    "warning: pilot_a: skipped install hook PreInstall {OFF_BY_DEFAULT}\n\
                     \x20 run it:         uninstall, allow it, then install again (PreInstall runs only at install)\n\
                     \x20 always run it:  add hooks = \"run\" to the pilot_a line in [dependencies] of lvpm.toml\n"
                ),
            ),
            (
                "dry run",
                vec![skipped_hooks("pilot_a", &[PostInstall], NotAllowed, false, true, key)],
                format!(
                    "warning: pilot_a: would skip install hook PostInstall {OFF_BY_DEFAULT}\n\
                     \x20 always run it:  add hooks = \"run\" to the pilot_a line in [dependencies] of lvpm.toml\n"
                ),
            ),
            (
                "headless, dry run",
                vec![skipped_hooks(
                    "pilot_a",
                    &[PostInstall],
                    Headless,
                    false,
                    true,
                    Some(AllowLine::PassHooks),
                )],
                "warning: pilot_a: would skip install hook PostInstall (LabVIEW runs headless; hooks = \"run\" and hooks.run do not apply)\n\
                 \x20 always run it:  add --hooks to this command\n"
                    .to_string(),
            ),
            (
                "already installed, now allowed, dry run",
                vec![skipped_hooks("pilot_a", &[PostInstall], AlreadyInstalled, false, true, None)],
                "warning: pilot_a: install hook PostInstall was skipped when it was installed\n"
                    .to_string(),
            ),
            (
                "already installed, now allowed",
                vec![skipped_hooks("pilot_a", &[PostInstall], AlreadyInstalled, true, false, None)],
                "warning: pilot_a: install hook PostInstall was skipped when it was installed\n\
                 \x20 run it now:     lvpm run-hooks pilot_a\n"
                    .to_string(),
            ),
        ];
        for (case, events, expected) in rows {
            let (out, err) = text_of(&events);
            assert_eq!(err, expected, "{case}");
            assert_eq!(out, "", "{case}: stdout stays clean");
        }
    }

    #[test]
    fn a_control_character_in_a_package_name_cannot_reach_the_terminal_in_the_warning() {
        let (_, err) = text_of(&[skipped_hooks(
            "pilot\u{1b}[2J",
            &[HookKind::PostInstall],
            SkipReason::NotAllowed,
            true,
            false,
            None,
        )]);
        assert!(!err.contains('\u{1b}'), "{err:?}");
        assert!(
            err.starts_with("warning: pilot?[2J: skipped install hook"),
            "{err:?}"
        );
    }

    #[test]
    fn a_newline_in_a_name_cannot_start_a_line_of_its_own_in_the_warning() {
        let post = [HookKind::PostInstall];
        let (_, err) = text_of(&[
            skipped_hooks(
                "a\nwarning: b",
                &post,
                SkipReason::NotAllowed,
                true,
                false,
                None,
            ),
            skipped_hooks(
                "pilot_a",
                &post,
                SkipReason::NotAllowed,
                false,
                false,
                Some(AllowLine::AddKey {
                    name: "pilot_a\nwarning: c".into(),
                    table: "dependencies\nwarning: d".into(),
                }),
            ),
            skipped_hooks(
                "pilot_a",
                &post,
                SkipReason::NotAllowed,
                false,
                false,
                config_set(&["pilot_a\nwarning: e"], 1),
            ),
        ]);
        let forged = |l: &&str| !(l.starts_with("warning: ") || l.starts_with("  "));
        assert_eq!(err.lines().filter(forged).count(), 0, "{err:?}");
        assert_eq!(err.matches("\nwarning: ").count() + 1, 3, "{err:?}");
        assert!(
            err.starts_with("warning: a warning: b: skipped install hook"),
            "{err:?}"
        );
    }

    #[test]
    fn a_name_that_is_not_safe_to_paste_keeps_the_warning_and_loses_the_hint_lines() {
        let post = [HookKind::PostInstall];
        let shell = "x; rm -rf ~";
        let (_, err) = text_of(&[
            skipped_hooks(
                shell,
                &post,
                SkipReason::NotAllowed,
                true,
                false,
                Some(AllowLine::AddLine { name: shell.into() }),
            ),
            skipped_hooks(
                "-all",
                &post,
                SkipReason::NotAllowed,
                true,
                false,
                config_set(&["fine_pkg", "-all"], 1),
            ),
            skipped_hooks(
                "fine_pkg",
                &post,
                SkipReason::NotAllowed,
                true,
                false,
                Some(AllowLine::AddKey {
                    name: "fine_pkg".into(),
                    table: "dependencies; x".into(),
                }),
            ),
        ]);
        assert_eq!(
            err,
            format!(
                "warning: {shell}: skipped install hook PostInstall {OFF_BY_DEFAULT}\n\
                 warning: -all: skipped install hook PostInstall {OFF_BY_DEFAULT}\n\
                 warning: fine_pkg: skipped install hook PostInstall {OFF_BY_DEFAULT}\n\
                 \x20 run it now:     lvpm run-hooks fine_pkg\n"
            )
        );
    }
}
