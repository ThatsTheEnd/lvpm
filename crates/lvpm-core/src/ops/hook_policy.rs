//! Which packages may run their install hooks.
//!
//! A hook is arbitrary code that runs inside the user's LabVIEW, so it runs
//! only for a package somebody allowed: the root project, with `hooks = "run"`
//! on the package's line of `lvpm.toml`, or the user's own config, for a command
//! that has no project. `--hooks` allows every package for that one command.
//! Nothing else does, and no dependency's manifest is ever consulted.

use crate::config::Config;
use crate::events::SkipReason;
use crate::project::{Project, name_key};
use std::collections::BTreeSet;

/// The packages whose install hooks may run, for one command. The default
/// allows none, so an install that was never given one fails safe.
#[derive(Debug, Clone, Default)]
pub struct HookAllow {
    /// [`name_key`]s: names compare as every other package name does, whole and
    /// in any ASCII case, never by prefix or pattern.
    names: BTreeSet<String>,
}

/// What the install is, as far as hooks are concerned.
#[derive(Debug, Clone, Copy)]
pub struct HookContext {
    /// The destination is a venv, which is no LabVIEW installation.
    pub venv: bool,
    /// LabVIEW runs headless here.
    pub headless: bool,
    /// The command says `--hooks`.
    pub forced: bool,
}

/// What an install does about one package's install hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Run,
    Skip(SkipReason),
    /// A venv runs no hook, and nothing can allow one there.
    Venv,
}

impl HookAllow {
    /// The allowlist of a command: with a project, the root project's own
    /// `hooks = "run"` lines; without one, the user's `hooks.run`. Never both,
    /// so neither widens the other. Pass only the project the command was run
    /// in, never one a dependency carries.
    pub fn for_command(config: &Config, project: Option<&Project>) -> HookAllow {
        let names = match project {
            Some(p) => p.hooks_allowed.iter().map(|n| name_key(n)).collect(),
            None => config.hooks_run.iter().map(|n| name_key(n)).collect(),
        };
        HookAllow { names }
    }

    /// Whether the install hooks of `name` run, in the order that decides it: a
    /// venv runs none, `--hooks` runs all, headless runs none that is not
    /// forced, and otherwise the allowlist says.
    pub fn verdict(&self, cx: HookContext, name: &str) -> Verdict {
        match cx {
            HookContext { venv: true, .. } => Verdict::Venv,
            HookContext { forced: true, .. } => Verdict::Run,
            HookContext { headless: true, .. } => Verdict::Skip(SkipReason::Headless),
            _ if self.names.contains(&name_key(name)) => Verdict::Run,
            _ => Verdict::Skip(SkipReason::NotAllowed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    fn config(hooks_run: &[&str]) -> Config {
        Config {
            cache: PathBuf::from("cache"),
            hooks_run: hooks_run.iter().map(|n| n.to_string()).collect(),
            local_sources: Vec::new(),
            venv_dir: PathBuf::from(".lv-venv"),
        }
    }

    fn project(dependencies: &str) -> Project {
        crate::project::parse(&format!(
            "[project]\nlabview = \">=2024\"\n\n[dependencies]\n{dependencies}"
        ))
        .unwrap()
    }

    fn allowing(name: &str) -> HookAllow {
        HookAllow::for_command(&config(&[name]), None)
    }

    #[test]
    fn the_verdict_for_every_context_and_both_allowed_rows() {
        use SkipReason::{Headless, NotAllowed};
        use Verdict::{Run, Skip, Venv};
        // (venv, headless, forced) -> (allowed, not allowed)
        let table = [
            ((false, false, false), (Run, Skip(NotAllowed))),
            ((false, false, true), (Run, Run)),
            ((false, true, false), (Skip(Headless), Skip(Headless))),
            ((false, true, true), (Run, Run)),
            ((true, false, false), (Venv, Venv)),
            ((true, false, true), (Venv, Venv)),
            ((true, true, false), (Venv, Venv)),
            ((true, true, true), (Venv, Venv)),
        ];
        let listed = allowing("pilot_a");
        let unlisted = HookAllow::default();
        for ((venv, headless, forced), (when_allowed, when_not)) in table {
            let cx = HookContext {
                venv,
                headless,
                forced,
            };
            assert_eq!(
                listed.verdict(cx, "pilot_a"),
                when_allowed,
                "{cx:?} allowed"
            );
            assert_eq!(
                unlisted.verdict(cx, "pilot_a"),
                when_not,
                "{cx:?} not allowed"
            );
        }
    }

    #[test]
    fn what_nothing_allowed_is_skipped_in_the_plain_context() {
        let plain = HookContext {
            venv: false,
            headless: false,
            forced: false,
        };
        assert_eq!(
            HookAllow::default().verdict(plain, "any"),
            Verdict::Skip(SkipReason::NotAllowed)
        );
    }

    #[test]
    fn a_name_matches_whole_and_in_any_ascii_case_never_by_prefix_or_pattern() {
        let plain = HookContext {
            venv: false,
            headless: false,
            forced: false,
        };
        let allow = allowing("Pilot_A");
        assert_eq!(allow.verdict(plain, "pilot_a"), Verdict::Run);
        assert_eq!(allow.verdict(plain, "PILOT_A"), Verdict::Run);
        for other in ["pilot", "pilot_a2", "pilot_", "xpilot_a", ""] {
            assert_eq!(
                allow.verdict(plain, other),
                Verdict::Skip(SkipReason::NotAllowed),
                "{other:?}"
            );
        }
        for glob in ["*", "pilot_*", "pilot_?"] {
            assert_eq!(
                allowing(glob).verdict(plain, "pilot_a"),
                Verdict::Skip(SkipReason::NotAllowed),
                "{glob:?}"
            );
        }
    }

    #[test]
    fn a_project_allows_by_its_own_lines_and_the_config_adds_nothing() {
        let plain = HookContext {
            venv: false,
            headless: false,
            forced: false,
        };
        let root = project("pilot_a = { version = \"*\", hooks = \"run\" }\npilot_b = \"*\"\n");
        let allow = HookAllow::for_command(&config(&["pilot_b"]), Some(&root));
        assert_eq!(allow.verdict(plain, "pilot_a"), Verdict::Run);
        assert_eq!(
            allow.verdict(plain, "pilot_b"),
            Verdict::Skip(SkipReason::NotAllowed),
            "hooks.run does not allow a project command"
        );
    }

    #[test]
    fn with_no_project_the_config_allows_and_a_project_it_is_not_given_does_not() {
        let plain = HookContext {
            venv: false,
            headless: false,
            forced: false,
        };
        let _elsewhere = project("pilot_b = { version = \"*\", hooks = \"run\" }\n");
        let allow = HookAllow::for_command(&config(&["pilot_a"]), None);
        assert_eq!(allow.verdict(plain, "pilot_a"), Verdict::Run);
        assert_eq!(
            allow.verdict(plain, "pilot_b"),
            Verdict::Skip(SkipReason::NotAllowed)
        );
    }

    #[test]
    fn only_the_root_projects_lines_count_never_a_dependencys_own_manifest() {
        let plain = HookContext {
            venv: false,
            headless: false,
            forced: false,
        };
        let root = project("beta = \"*\"\n");
        let betas_own = project("hooked = { version = \"*\", hooks = \"run\" }\n");
        assert!(
            betas_own.hooks_allowed_for("hooked"),
            "the fixture allows it"
        );
        let allow = HookAllow::for_command(&config(&[]), Some(&root));
        assert_eq!(
            allow.verdict(plain, "hooked"),
            Verdict::Skip(SkipReason::NotAllowed)
        );
    }
}

/// The commands, run against a LabVIEW installation that is not there: it has
/// no VI Server, so a hook is tried and fails, and "started" means lvpm began
/// it (`HookStarting`). The real run is the `@labview` scenarios'.
#[cfg(test)]
mod installation {
    use crate::events::{Event, EventSink, HookKind, Phase, SkipReason};
    use crate::install;
    use crate::manifest_edit::DependencyTable;
    use crate::ops::add_remove::add;
    use crate::ops::env::OpEnv;
    use crate::ops::install::{InstallOptions, install as install_package};
    use crate::ops::sync::{SyncOptions, sync};
    use crate::ops::testing::{
        FakeInstallation, Recorded, env_on, scratch_dir, venv_in, vip_declaring,
    };
    use std::path::PathBuf;

    const BOTH: &[&str] = &["PreInstall", "PostInstall"];
    const POST: &[&str] = &["PostInstall"];
    const RUN: &str = "{ version = \"*\", hooks = \"run\" }";

    /// A project folder with a folder of packages beside the manifest, and the
    /// installation that is not there.
    struct Bed {
        root: PathBuf,
        fake: FakeInstallation,
    }

    impl Bed {
        fn new(tag: &str) -> Bed {
            let root = scratch_dir(&format!("hook-policy-{tag}"));
            std::fs::create_dir_all(root.join("packages")).unwrap();
            let fake = FakeInstallation::new(&root);
            Bed { root, fake }
        }

        fn package(&self, name: &str, version: &str, requires: &str, hooks: &[&str]) {
            let member = [("a.txt", b"payload".to_vec())];
            vip_declaring(
                &self.root.join("packages"),
                name,
                version,
                requires,
                &member,
                hooks,
            );
        }

        /// `lvpm.toml` with these `[dependencies]` lines.
        fn manifest(&self, dependencies: &str) {
            std::fs::write(
                self.root.join(crate::project::FILE_NAME),
                format!(
                    "[project]\nlabview = \">=2024\"\n\n[sources]\ndefaults = false\n\n\
                     [dependencies]\n{dependencies}"
                ),
            )
            .unwrap();
        }

        /// An environment over the packages, where nothing is chosen yet.
        fn env<'a>(&self, sink: &'a dyn EventSink, hooks_run: &[&str]) -> OpEnv<'a> {
            let mut env = env_on(sink, &self.root);
            env.config.cache = self.root.join("cache");
            env.config.local_sources = vec![self.root.join("packages")];
            env.config.hooks_run = hooks_run.iter().map(|n| n.to_string()).collect();
            env
        }

        /// As [`Bed::env`], aimed at the installation, as `--global` is.
        fn on_installation<'a>(&self, sink: &'a dyn EventSink, hooks_run: &[&str]) -> OpEnv<'a> {
            let mut env = self.env(sink, hooks_run);
            self.fake.place(&mut env);
            env.flags.global = true;
            env.flags.labview_version = Some("2099".into());
            env
        }

        fn record(&self, name: &str) -> install::Manifest {
            install::read_manifest(&self.fake.roots(), name).unwrap()
        }
    }

    impl Drop for Bed {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn options(forced: bool) -> InstallOptions {
        InstallOptions {
            no_relink: true,
            run_hooks: forced,
            ..InstallOptions::default()
        }
    }

    /// `lvpm install <package>` into the installation.
    fn install_into(bed: &Bed, hooks_run: &[&str], package: &str, forced: bool) -> Vec<Event> {
        let sink = Recorded::default();
        let env = bed.on_installation(&sink, hooks_run);
        install_package(&env, Some(package), &options(forced)).unwrap();
        sink.events()
    }

    /// `lvpm sync --global`.
    fn sync_into(bed: &Bed, hooks_run: &[&str], forced: bool) -> Vec<Event> {
        let sink = Recorded::default();
        let env = bed.on_installation(&sink, hooks_run);
        let opts = SyncOptions {
            install: options(forced),
            ..SyncOptions::default()
        };
        sync(&env, &opts).unwrap();
        sink.events()
    }

    /// The hooks lvpm began, by package.
    fn started(events: &[Event]) -> Vec<(String, HookKind)> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::HookStarting { package, hook } => Some((package.clone(), *hook)),
                _ => None,
            })
            .collect()
    }

    fn post(package: &str) -> (String, HookKind) {
        (package.to_string(), HookKind::PostInstall)
    }

    fn lists_skipped(record: &install::Manifest, hook: &str) -> bool {
        let prefix = format!("{hook}=");
        (record.skipped_hooks.iter()).any(|h| h.starts_with(&prefix))
    }

    #[test]
    fn a_global_install_skips_a_post_install_hook_nobody_allowed_and_records_it() {
        let bed = Bed::new("nobody");
        bed.package("hooked", "1.0", "", POST);
        let events = install_into(&bed, &[], "hooked", false);
        assert_eq!(started(&events), []);
        assert!(lists_skipped(&bed.record("hooked"), "PostInstall"));
    }

    #[test]
    fn a_skipped_package_records_every_hook_it_declares() {
        let bed = Bed::new("every");
        bed.package("hooked", "1.0", "", BOTH);
        let events = install_into(&bed, &[], "hooked", false);
        assert_eq!(started(&events), []);
        let record = bed.record("hooked");
        for hook in BOTH {
            assert!(lists_skipped(&record, hook), "{hook}: {record:?}");
        }
    }

    #[test]
    fn a_package_in_hooks_run_has_its_hook_started_whatever_the_case() {
        let bed = Bed::new("config");
        bed.package("hooked", "1.0", "", POST);
        let events = install_into(&bed, &["Hooked"], "hooked", false);
        assert_eq!(started(&events), [post("hooked")]);
    }

    #[test]
    fn sync_global_starts_the_hook_of_a_package_with_hooks_run_and_no_other() {
        let bed = Bed::new("sync-run");
        bed.package("hooked", "1.0", "", POST);
        bed.package("other_hooked", "1.0", "", POST);
        bed.manifest(&format!("hooked = {RUN}\nother_hooked = \"*\"\n"));
        let events = sync_into(&bed, &[], false);
        assert_eq!(started(&events), [post("hooked")]);
        assert!(lists_skipped(&bed.record("other_hooked"), "PostInstall"));
        // What was started is not reported as not run.
        let unrun: Vec<&String> = events
            .iter()
            .filter_map(|e| match e {
                Event::HooksNotRun { hooks } => Some(hooks),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(unrun.len(), 1, "only other_hooked's: {unrun:?}");
    }

    #[test]
    fn sync_global_with_hooks_starts_a_hook_nobody_allowed() {
        let bed = Bed::new("sync-forced");
        bed.package("hooked", "1.0", "", POST);
        bed.manifest("hooked = \"*\"\n");
        let events = sync_into(&bed, &[], true);
        assert_eq!(started(&events), [post("hooked")]);
    }

    #[test]
    fn hooks_run_does_not_allow_a_project_sync() {
        let bed = Bed::new("sync-config");
        bed.package("hooked", "1.0", "", POST);
        bed.manifest("hooked = \"*\"\n");
        let events = sync_into(&bed, &["hooked"], false);
        assert_eq!(started(&events), []);
    }

    #[test]
    fn hooks_run_in_a_project_does_not_allow_lvpm_install() {
        let bed = Bed::new("install-in-project");
        bed.package("hooked", "1.0", "", POST);
        bed.manifest(&format!("hooked = {RUN}\n"));
        let events = install_into(&bed, &[], "hooked", false);
        assert_eq!(started(&events), []);
        assert!(lists_skipped(&bed.record("hooked"), "PostInstall"));
    }

    #[test]
    fn add_global_of_a_hooks_run_line_starts_its_hook() {
        let bed = Bed::new("add");
        bed.package("hooked", "1.0", "", POST);
        bed.package("hooked", "2.0", "", POST);
        bed.manifest("hooked = { version = \"1.0\", hooks = \"run\" }\n");
        let sink = Recorded::default();
        let env = bed.on_installation(&sink, &[]);
        add(
            &env,
            "hooked@2.0",
            DependencyTable::Dependencies,
            None,
            &options(false),
        )
        .unwrap();
        assert_eq!(started(&sink.events()), [post("hooked")]);
    }

    #[test]
    fn a_transitive_dependency_listed_with_version_star_and_hooks_run_starts_its_hook() {
        let bed = Bed::new("transitive");
        bed.package("hooked", "1.0", "", POST);
        bed.package("beta", "1.0", "hooked>=1.0", &[]);
        bed.manifest(&format!("beta = \"*\"\nhooked = {RUN}\n"));
        let events = sync_into(&bed, &[], false);
        assert_eq!(started(&events), [post("hooked")]);
    }

    #[test]
    fn a_transitive_dependency_nobody_listed_is_skipped() {
        let bed = Bed::new("transitive-unlisted");
        bed.package("hooked", "1.0", "", POST);
        bed.package("beta", "1.0", "hooked>=1.0", &[]);
        bed.manifest("beta = \"*\"\n");
        let events = sync_into(&bed, &[], false);
        assert_eq!(started(&events), []);
    }

    #[test]
    fn hooks_run_in_a_dependencys_own_manifest_allows_nothing() {
        let bed = Bed::new("dependency-manifest");
        bed.package("hooked", "1.0", "", POST);
        bed.package("beta", "1.0", "hooked>=1.0", &[]);
        // beta's source tree, with the manifest its author wrote.
        let tree = bed.root.join("beta-source");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(
            tree.join(crate::project::FILE_NAME),
            format!("[project]\nlabview = \">=2024\"\n\n[dependencies]\nhooked = {RUN}\n"),
        )
        .unwrap();
        bed.manifest("beta = \"*\"\n");
        let events = sync_into(&bed, &[], false);
        assert_eq!(started(&events), []);
        assert!(lists_skipped(&bed.record("hooked"), "PostInstall"));
    }

    #[test]
    fn hooks_starts_every_hook_of_that_one_command() {
        let bed = Bed::new("forced");
        bed.package("other_hooked", "1.0", "", BOTH);
        bed.package("hooked", "1.0", "other_hooked>=1.0", BOTH);
        let events = install_into(&bed, &[], "hooked", true);
        let mut got = started(&events);
        got.sort_by_key(|(package, hook)| (package.clone(), *hook as u8));
        let pair = |package: &str, hook| (package.to_string(), hook);
        assert_eq!(
            got,
            [
                pair("hooked", HookKind::PreInstall),
                pair("hooked", HookKind::PostInstall),
                pair("other_hooked", HookKind::PreInstall),
                pair("other_hooked", HookKind::PostInstall),
            ]
        );
    }

    /// `lvpm install hooked`, headless, in a bed of its own.
    fn headless_install(tag: &str, hooks_run: &[&str], forced: bool) -> Vec<Event> {
        let bed = Bed::new(tag);
        bed.package("hooked", "1.0", "", POST);
        let sink = Recorded::default();
        let mut env = bed.on_installation(&sink, hooks_run);
        env.headless = true;
        install_package(&env, Some("hooked"), &options(forced)).unwrap();
        sink.events()
    }

    #[test]
    fn headless_with_hooks_starts_the_hook() {
        let events = headless_install("headless-forced", &[], true);
        assert_eq!(started(&events), [post("hooked")]);
    }

    #[test]
    fn headless_without_hooks_skips_even_an_allowed_package_and_keeps_its_note() {
        let events = headless_install("headless-allowed", &["hooked"], false);
        assert_eq!(started(&events), []);
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::PhaseSkipped {
                    phase: Phase::Hooks,
                    reason: SkipReason::Headless,
                    skipped: 1,
                }
            )),
            "the headless note is unchanged: {events:?}"
        );
    }

    #[test]
    fn a_venv_install_runs_no_hook_and_reports_it_as_not_run() {
        let bed = Bed::new("venv");
        bed.package("hooked", "1.0", "", POST);
        bed.manifest(&format!("hooked = {RUN}\n"));
        let _venv = venv_in(&bed.root);
        let sink = Recorded::default();
        let env = bed.env(&sink, &["hooked"]);
        let opts = SyncOptions {
            install: options(true),
            ..SyncOptions::default()
        };
        sync(&env, &opts).unwrap();
        let events = sink.events();
        assert_eq!(started(&events), []);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::HooksNotRun { hooks } if hooks.len() == 1)),
            "{events:?}"
        );
    }

    #[test]
    fn a_scratch_install_keeps_its_scratch_note_and_records_no_skipped_hook() {
        let bed = Bed::new("scratch");
        bed.package("hooked", "1.0", "", POST);
        let sink = Recorded::default();
        let mut env = bed.env(&sink, &[]);
        env.flags.prefix = Some(bed.root.join("dest"));
        install_package(&env, Some("hooked"), &options(false)).unwrap();
        let events = sink.events();
        let record = install::read_manifest(&env.roots().unwrap(), "hooked").unwrap();
        assert!(record.skipped_hooks.is_empty(), "{record:?}");
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::PhaseSkipped {
                    phase: Phase::Hooks,
                    reason: SkipReason::ScratchDestination,
                    skipped: 1,
                }
            )),
            "{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::HooksNotRun { .. })),
            "{events:?}"
        );
    }

    #[test]
    fn a_dry_run_starts_nothing_and_writes_no_record() {
        let bed = Bed::new("dry-run");
        bed.package("hooked", "1.0", "", POST);
        let sink = Recorded::default();
        let env = bed.on_installation(&sink, &[]);
        let opts = InstallOptions {
            dry_run: true,
            ..options(false)
        };
        install_package(&env, Some("hooked"), &opts).unwrap();
        assert_eq!(started(&sink.events()), []);
        assert!(!install::is_installed(&bed.fake.roots(), "hooked"));
    }
}
