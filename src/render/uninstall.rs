//! The wording of the uninstall events.

use super::hooks::hook_names;
use super::install::pasteable;
use super::{Renderer, Stream};
use lvpm_core::events::{Event, UnrunReason};

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
        Event::PostUninstallNotStaged { error } => r.line(
            Stream::Out,
            &format!("note: cannot stage PostUninstall.vi ({error}) — not running it"),
        ),
        Event::UninstallHooksNotRun {
            package,
            hooks,
            reason,
        } => {
            let why = match reason {
                UnrunReason::InstalledBeforeUninstallHooks => {
                    "installed before uninstall hooks existed".to_string()
                }
                UnrunReason::VenvRunsNoHooks => "hooks do not run in a venv".to_string(),
                // The package name comes from a feed, and this line is pasted:
                // only a plain name goes into the command.
                UnrunReason::InstallHookSkipped if pasteable(package) => format!(
                    "its install hook did not run; run lvpm run-hooks {package} first to run them"
                ),
                UnrunReason::InstallHookSkipped => "its install hook did not run".to_string(),
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
        });
        renderer.render(&Event::UninstallHooksNotRun {
            package: "alpha".into(),
            hooks: vec!["PreUninstall=a.vi".into()],
            reason: UnrunReason::InstalledBeforeUninstallHooks,
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
    fn hooks_held_back_by_a_skipped_install_hook_say_to_run_hooks_first() {
        let (mut renderer, out, _) = renderer();
        let not_run = |package: &str| Event::UninstallHooksNotRun {
            package: package.into(),
            hooks: vec!["PreUninstall=a.vi".into(), "PostUninstall=b.vi".into()],
            reason: UnrunReason::InstallHookSkipped,
        };
        renderer.render(&not_run("alpha"));
        // A name that could be a flag or carry a second command is not pasted.
        renderer.render(&not_run("--all; rm x"));
        assert_eq!(
            out.text(),
            "note: declared but not run (its install hook did not run; run lvpm run-hooks alpha \
             first to run them): PreUninstall, PostUninstall\n\
             note: declared but not run (its install hook did not run): \
             PreUninstall, PostUninstall\n"
        );
    }
}
