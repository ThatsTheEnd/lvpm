//! The wording of the uninstall events.

use super::hooks::hook_names;
use super::install::pasteable;
use super::{Renderer, Stream};
use lvpm_core::events::{Event, KeptBecause, UnrunReason};

pub(super) fn render(r: &mut Renderer, event: &Event) {
    match event {
        Event::PackageRemoved {
            name,
            version,
            files,
        } => r.line(
            Stream::Out,
            &format!("removed {name} {version} ({files} files)"),
        ),
        Event::UninstallFailed { name, error } => {
            r.line(Stream::Out, &format!("uninstall {name} FAILED: {error}"))
        }
        Event::UninstallDeclaredInProject { package, manifest } => r.line(
            Stream::Err,
            &format!(
                "warning: {package} is declared in {manifest}; uninstall does not edit the manifest"
            ),
        ),
        Event::UninstallRequiredBy {
            package,
            dependents,
        } => r.line(
            Stream::Err,
            &format!(
                "warning: {package} is required by installed packages: {}",
                dependents.join(", ")
            ),
        ),
        Event::DependencyKept { name, why } => {
            let why = match why {
                KeptBecause::Declared { manifest } => format!("declared in {manifest}"),
                KeptBecause::NeededByDeclared { manifest } => {
                    format!("required by a package declared in {manifest}")
                }
                KeptBecause::RequiredBy { owners } => {
                    format!("still required by {}", owners.join(", "))
                }
            };
            r.line(Stream::Err, &format!("keeping {name}: {why}"))
        }
        Event::PostUninstallNotStaged { error } => r.line(
            Stream::Out,
            &format!("note: cannot stage PostUninstall.vi ({error}) — not running it"),
        ),
        Event::UninstallHooksNotRun {
            package,
            hooks,
            reason,
            run_hooks_helps,
        } => {
            let why = match reason {
                UnrunReason::InstalledBeforeUninstallHooks => {
                    "installed before uninstall hooks existed".to_string()
                }
                UnrunReason::VenvRunsNoHooks => "hooks do not run in a venv".to_string(),
                // The package name comes from a feed, and this line is pasted:
                // only a plain name goes into the command. A package with no
                // PostInstall has nothing for run-hooks to run. The package is
                // already gone, so the remedy is for the next one.
                UnrunReason::InstallHookSkipped if *run_hooks_helps && pasteable(package) => {
                    format!(
                        "its install hook did not run; next time, run lvpm run-hooks {package} \
                         before uninstalling"
                    )
                }
                UnrunReason::InstallHookSkipped => "its install hook did not run".to_string(),
                UnrunReason::NotAllowedAtInstall => {
                    "hooks were not allowed when it was installed".to_string()
                }
            };
            r.line(
                Stream::Out,
                &format!("note: declared but not run ({why}): {}", hook_names(hooks)),
            );
        }
        _ => unreachable!("routed here by Renderer::render: {event:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::renderer;
    use super::*;

    #[test]
    fn a_removal_and_a_failed_sweep_member_are_lines_on_stdout() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::PackageRemoved {
            name: "beta".into(),
            version: "1.0".into(),
            files: 1,
        });
        renderer.render(&Event::UninstallFailed {
            name: "alpha".into(),
            error: "removing the file".into(),
        });
        assert_eq!(
            out.text(),
            "removed beta 1.0 (1 files)\nuninstall alpha FAILED: removing the file\n"
        );
        assert_eq!(err.text(), "");
    }

    #[test]
    fn an_unrun_hook_is_named_with_the_reason() {
        let (mut renderer, out, _) = renderer();
        renderer.render(&Event::UninstallHooksNotRun {
            package: "alpha".into(),
            hooks: vec!["PreUninstall=a.vi".into(), "PostUninstall=b.vi".into()],
            reason: UnrunReason::VenvRunsNoHooks,
            run_hooks_helps: true,
        });
        renderer.render(&Event::UninstallHooksNotRun {
            package: "alpha".into(),
            hooks: vec!["PreUninstall=a.vi".into()],
            reason: UnrunReason::InstalledBeforeUninstallHooks,
            run_hooks_helps: true,
        });
        renderer.render(&Event::PostUninstallNotStaged {
            error: "access denied".into(),
        });
        assert_eq!(
            out.text(),
            "note: declared but not run (hooks do not run in a venv): PreUninstall, PostUninstall\n\
             note: declared but not run (installed before uninstall hooks existed): PreUninstall\n\
             note: cannot stage PostUninstall.vi (access denied) — not running it\n"
        );
    }

    #[test]
    fn hooks_of_a_package_installed_without_hooks_allowed_say_so_and_offer_no_command() {
        let (mut renderer, out, _) = renderer();
        renderer.render(&Event::UninstallHooksNotRun {
            package: "alpha".into(),
            hooks: vec!["PreUninstall=a.vi".into()],
            reason: UnrunReason::NotAllowedAtInstall,
            run_hooks_helps: true,
        });
        assert_eq!(
            out.text(),
            "note: declared but not run (hooks were not allowed when it was installed): \
             PreUninstall\n"
        );
    }

    #[test]
    fn hooks_held_back_by_a_skipped_install_hook_say_to_run_hooks_before_the_next_uninstall() {
        let (mut renderer, out, _) = renderer();
        let not_run = |package: &str, run_hooks_helps| Event::UninstallHooksNotRun {
            package: package.into(),
            hooks: vec!["PreUninstall=a.vi".into(), "PostUninstall=b.vi".into()],
            reason: UnrunReason::InstallHookSkipped,
            run_hooks_helps,
        };
        renderer.render(&not_run("alpha", true));
        // A name that could be a flag or carry a second command is not pasted.
        renderer.render(&not_run("--all; rm x", true));
        // A PreInstall-only package: run-hooks has nothing to run for it.
        renderer.render(&not_run("alpha", false));
        assert_eq!(
            out.text(),
            "note: declared but not run (its install hook did not run; next time, run lvpm \
             run-hooks alpha before uninstalling): PreUninstall, PostUninstall\n\
             note: declared but not run (its install hook did not run): \
             PreUninstall, PostUninstall\n\
             note: declared but not run (its install hook did not run): \
             PreUninstall, PostUninstall\n"
        );
    }
}
