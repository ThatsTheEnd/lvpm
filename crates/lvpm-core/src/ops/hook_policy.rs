//! Which packages may run their install hooks.
//!
//! A hook is arbitrary code that runs inside the user's LabVIEW, so it runs
//! only for a package somebody allowed: the root project, with `hooks = "run"`
//! on the package's line of `lvpm.toml`, or the user's own config, for a command
//! that has no project. `--hooks` allows every package for that one command.
//! Nothing else does, and no dependency's manifest is ever consulted.

use crate::config::Config;
use crate::events::{AllowLine, Event, HookKind, SkipReason};
use crate::project::{Project, name_key};
use std::collections::{BTreeMap, BTreeSet};

/// The packages whose install hooks may run, for one command. The default
/// allows none, so an install that was never given one fails safe.
#[derive(Debug, Clone, Default)]
pub struct HookAllow {
    /// [`name_key`]s: names compare as every other package name does, whole and
    /// in any ASCII case, never by prefix or pattern.
    names: BTreeSet<String>,
    /// Who allowed them, which decides what a skipped package is told to edit.
    source: AllowSource,
}

/// Where the allowlist of a command was written.
#[derive(Debug, Clone)]
enum AllowSource {
    /// The root project's dependency lines. `listed` has every line, by
    /// [`name_key`], allowed or not.
    Project { listed: BTreeMap<String, Listed> },
    /// The user's `hooks.run`, as written.
    Config { names: Vec<String> },
}

impl Default for AllowSource {
    fn default() -> Self {
        AllowSource::Config { names: Vec::new() }
    }
}

/// A dependency line: the name as it is written, and its table.
#[derive(Debug, Clone)]
struct Listed {
    name: String,
    table: String,
}

/// A package whose install hooks an install did not run on a LabVIEW
/// installation: what [`HookAllow::warnings`] turns into events.
#[derive(Debug, Clone, PartialEq)]
pub struct HookSkip {
    pub package: String,
    pub hooks: Vec<HookKind>,
    pub reason: SkipReason,
    pub run_now: bool,
}

/// What the install is, as far as hooks are concerned.
#[derive(Debug, Clone, Copy)]
pub struct HookContext {
    /// The destination is a venv, which is no LabVIEW installation.
    pub venv: bool,
    /// LabVIEW runs headless here.
    pub headless: bool,
    /// The command says `--hooks`: every package is allowed, headless or not.
    pub forced: bool,
}

/// What an install does about one package's install hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Its hooks run: PreInstall just before its files are written, PostInstall
    /// after the relink pass.
    Run,
    /// Its hooks do not run, for this reason; the install records and reports
    /// each one it declares.
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
        match project {
            Some(p) => {
                let mut listed = BTreeMap::new();
                for (table, lines) in p.tables() {
                    for (name, _) in lines {
                        listed.entry(name_key(name)).or_insert_with(|| Listed {
                            name: name.clone(),
                            table: table.key(),
                        });
                    }
                }
                HookAllow {
                    names: p.hooks_allowed.iter().map(|n| name_key(n)).collect(),
                    source: AllowSource::Project { listed },
                }
            }
            None => HookAllow {
                names: config.hooks_run.iter().map(|n| name_key(n)).collect(),
                source: AllowSource::Config {
                    names: config.hooks_run.clone(),
                },
            },
        }
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

    /// What to edit so a project allows `name`: the key on its own line, or a
    /// line of its own when none lists it (a transitive dependency). Without a
    /// project, the config line for that one name.
    fn line_for(&self, name: &str) -> AllowLine {
        match &self.source {
            AllowSource::Project { listed } => match listed.get(&name_key(name)) {
                Some(line) => AllowLine::AddKey {
                    name: line.name.clone(),
                    table: line.table.clone(),
                },
                None => AllowLine::AddLine {
                    name: name.to_string(),
                },
            },
            AllowSource::Config { .. } => self.config_line(&[name.to_string()]),
        }
    }

    /// The `hooks.run` that allows every name in `skipped` and keeps the names
    /// allowed now: `config set` replaces the list.
    fn config_line(&self, skipped: &[String]) -> AllowLine {
        let existing = match &self.source {
            AllowSource::Config { names } => names.as_slice(),
            AllowSource::Project { .. } => &[],
        };
        AllowLine::ConfigSet {
            names: existing.iter().chain(skipped).cloned().collect(),
            skipped: skipped.len(),
        }
    }

    /// The warning events for what an install skipped, in the order it
    /// skipped it. Without a project, one config line gathers every package
    /// that was not allowed, and goes on the last of them, so the user pastes
    /// it once.
    pub fn warnings(&self, dry_run: bool, skipped: Vec<HookSkip>) -> Vec<Event> {
        let not_allowed = |s: &HookSkip| s.reason == SkipReason::NotAllowed;
        let names: Vec<String> = (skipped.iter().filter(|s| not_allowed(s)))
            .map(|s| s.package.clone())
            .collect();
        let last = skipped.iter().rposition(not_allowed);
        skipped
            .into_iter()
            .enumerate()
            .map(|(at, s)| {
                let allow = match (s.reason, &self.source) {
                    (SkipReason::Headless, _) => Some(AllowLine::PassHooks),
                    (SkipReason::NotAllowed, AllowSource::Project { .. }) => {
                        Some(self.line_for(&s.package))
                    }
                    (SkipReason::NotAllowed, AllowSource::Config { .. }) => {
                        (Some(at) == last).then(|| self.config_line(&names))
                    }
                    _ => None,
                };
                Event::HooksSkipped {
                    package: s.package,
                    hooks: s.hooks,
                    reason: s.reason,
                    run_now: s.run_now,
                    dry_run,
                    allow,
                }
            })
            .collect()
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
    fn with_no_project_the_config_allows_its_names_and_no_other() {
        let plain = HookContext {
            venv: false,
            headless: false,
            forced: false,
        };
        let allow = HookAllow::for_command(&config(&["pilot_a"]), None);
        assert_eq!(allow.verdict(plain, "pilot_a"), Verdict::Run);
        assert_eq!(
            allow.verdict(plain, "pilot_b"),
            Verdict::Skip(SkipReason::NotAllowed)
        );
    }

    #[test]
    fn only_the_root_projects_own_lines_count() {
        let plain = HookContext {
            venv: false,
            headless: false,
            forced: false,
        };
        let root = project("beta = \"*\"\n");
        let allow = HookAllow::for_command(&config(&[]), Some(&root));
        assert_eq!(
            allow.verdict(plain, "hooked"),
            Verdict::Skip(SkipReason::NotAllowed)
        );
    }

    fn add_key(name: &str, table: &str) -> AllowLine {
        AllowLine::AddKey {
            name: name.into(),
            table: table.into(),
        }
    }

    #[test]
    fn a_listed_line_is_told_to_add_the_key_in_its_own_table_under_the_name_it_is_written() {
        let root = crate::project::parse(
            "[project]\nlabview = \">=2024\"\n\n[dependencies]\nPilot_A = \"*\"\n\n\
             [dev-dependencies]\npilot_b = \"*\"\n\n[dependency-groups.ci]\npilot_c = \"*\"\n",
        )
        .unwrap();
        let allow = HookAllow::for_command(&config(&[]), Some(&root));
        assert_eq!(
            allow.line_for("pilot_a"),
            add_key("Pilot_A", "dependencies")
        );
        assert_eq!(
            allow.line_for("pilot_b"),
            add_key("pilot_b", "dev-dependencies")
        );
        assert_eq!(
            allow.line_for("pilot_c"),
            add_key("pilot_c", "dependency-groups.ci")
        );
    }

    #[test]
    fn a_dependency_no_line_lists_is_offered_a_line_of_its_own_under_its_package_name() {
        let root = project("beta = \"*\"\n");
        let allow = HookAllow::for_command(&config(&[]), Some(&root));
        assert_eq!(
            allow.line_for("hooked"),
            AllowLine::AddLine {
                name: "hooked".into()
            }
        );
    }

    #[test]
    fn the_config_line_keeps_the_allowed_names_and_adds_the_skipped_ones_last() {
        let allow = HookAllow::for_command(&config(&["Other", "pilot_z"]), None);
        assert_eq!(
            allow.config_line(&["a".into(), "b".into()]),
            AllowLine::ConfigSet {
                names: ["Other", "pilot_z", "a", "b"].map(String::from).to_vec(),
                skipped: 2,
            }
        );
        assert_eq!(
            allow.line_for("a"),
            AllowLine::ConfigSet {
                names: ["Other", "pilot_z", "a"].map(String::from).to_vec(),
                skipped: 1,
            }
        );
    }

    fn skipped(package: &str, reason: SkipReason) -> HookSkip {
        HookSkip {
            package: package.into(),
            hooks: vec![HookKind::PostInstall],
            reason,
            run_now: reason != SkipReason::Headless,
        }
    }

    /// The allow line of every warning, in order.
    fn allows(events: &[Event]) -> Vec<Option<AllowLine>> {
        events
            .iter()
            .map(|e| match e {
                Event::HooksSkipped { allow, .. } => allow.clone(),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn without_a_project_one_config_line_goes_on_the_last_package_that_was_not_allowed() {
        let allow = HookAllow::for_command(&config(&["other"]), None);
        let events = allow.warnings(
            false,
            vec![
                skipped("a_hooked", SkipReason::NotAllowed),
                skipped("old", SkipReason::AlreadyInstalled),
                skipped("b_hooked", SkipReason::NotAllowed),
                skipped("later", SkipReason::AlreadyInstalled),
            ],
        );
        assert_eq!(
            allows(&events),
            [
                None,
                None,
                Some(AllowLine::ConfigSet {
                    names: ["other", "a_hooked", "b_hooked"].map(String::from).to_vec(),
                    skipped: 2,
                }),
                None,
            ]
        );
    }

    #[test]
    fn in_a_project_each_package_has_its_own_line_and_headless_is_told_to_pass_hooks() {
        let root = project("a_hooked = \"*\"\n");
        let allow = HookAllow::for_command(&config(&[]), Some(&root));
        let events = allow.warnings(
            false,
            vec![
                skipped("a_hooked", SkipReason::NotAllowed),
                skipped("b_hooked", SkipReason::NotAllowed),
                skipped("c_hooked", SkipReason::Headless),
            ],
        );
        assert_eq!(
            allows(&events),
            [
                Some(add_key("a_hooked", "dependencies")),
                Some(AllowLine::AddLine {
                    name: "b_hooked".into()
                }),
                Some(AllowLine::PassHooks),
            ]
        );
    }

    #[test]
    fn the_warning_carries_what_the_install_skipped_and_whether_it_was_a_dry_run() {
        let events = HookAllow::default().warnings(true, vec![skipped("a", SkipReason::Headless)]);
        assert_eq!(
            events,
            [Event::HooksSkipped {
                package: "a".into(),
                hooks: vec![HookKind::PostInstall],
                reason: SkipReason::Headless,
                run_now: false,
                dry_run: true,
                allow: Some(AllowLine::PassHooks),
            }]
        );
    }
}

/// The commands, run against a LabVIEW installation that is not there: it has
/// no VI Server, so a hook is tried and fails, and "started" means lvpm began
/// it (`HookStarting`). The real run is the `@labview` scenarios'.
#[cfg(test)]
mod installation {
    use crate::events::{AllowLine, Event, EventSink, HookKind, Phase, SkipReason};
    use crate::install;
    use crate::manifest_edit::DependencyTable;
    use crate::ops::add_remove::add;
    use crate::ops::env::OpEnv;
    use crate::ops::install::{InstallOptions, install as install_package};
    use crate::ops::sync::{SyncOptions, sync};
    use crate::ops::testing::{
        FakeInstallation, Recorded, env_on, scratch_dir, venv_in, vip_declaring, vip_in,
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
            let bed = Bed { root, fake };
            // A manifest naming no feed, so a named install reads only the
            // packages beside it, never the network.
            bed.manifest("");
            bed
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

    /// The packages the install warned about, in order.
    fn skipped_packages(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::HooksSkipped { package, .. } => Some(package.clone()),
                _ => None,
            })
            .collect()
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
    fn an_allowed_package_has_both_its_pre_and_post_install_hooks_started() {
        let bed = Bed::new("allowed-both");
        bed.package("hooked", "1.0", "", BOTH);
        let events = install_into(&bed, &["hooked"], "hooked", false);
        let pair = |hook| ("hooked".to_string(), hook);
        assert_eq!(
            started(&events),
            [pair(HookKind::PreInstall), pair(HookKind::PostInstall)]
        );
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
        // What was started is not warned about; what was not, is.
        assert_eq!(skipped_packages(&events), ["other_hooked"]);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::HooksNotRun { .. })),
            "a LabVIEW installation warns with HooksSkipped: {events:?}"
        );
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
        // beta ships the manifest its author wrote, which allows `hooked`.
        let authors =
            format!("[project]\nlabview = \">=2024\"\n\n[dependencies]\nhooked = {RUN}\n");
        vip_declaring(
            &bed.root.join("packages"),
            "beta",
            "1.0",
            "hooked>=1.0",
            &[(crate::project::FILE_NAME, authors.into_bytes())],
            &[],
        );
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
    fn headless_without_hooks_skips_even_an_allowed_package_and_says_so_per_package() {
        let events = headless_install("headless-allowed", &["hooked"], false);
        assert_eq!(started(&events), []);
        assert!(
            !events.iter().any(|e| matches!(
                e,
                Event::PhaseSkipped {
                    phase: Phase::Hooks,
                    ..
                }
            )),
            "the per-package warning replaces the hooks note: {events:?}"
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
    fn a_headless_scratch_install_skips_its_hook_as_headless_and_records_it() {
        let bed = Bed::new("scratch-headless");
        bed.package("hooked", "1.0", "", POST);
        let sink = Recorded::default();
        let mut env = bed.env(&sink, &[]);
        env.flags.prefix = Some(bed.root.join("dest"));
        env.headless = true;
        install_package(&env, Some("hooked"), &options(false)).unwrap();
        let events = sink.events();
        let record = install::read_manifest(&env.roots().unwrap(), "hooked").unwrap();
        assert!(lists_skipped(&record, "PostInstall"), "{record:?}");
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::PhaseSkipped {
                    phase: Phase::Hooks,
                    reason: SkipReason::Headless,
                    skipped: 1,
                }
            )),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::HooksNotRun { hooks } if hooks.len() == 1)),
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

    // ---- the warning ----

    const PRE: &[&str] = &["PreInstall"];

    /// Every warning the install told, in order.
    fn told(events: &[Event]) -> Vec<Event> {
        let warnings = |e: &&Event| matches!(e, Event::HooksSkipped { .. });
        events.iter().filter(warnings).cloned().collect()
    }

    fn warning(
        package: &str,
        hooks: &[HookKind],
        reason: SkipReason,
        run_now: bool,
        allow: Option<AllowLine>,
    ) -> Event {
        Event::HooksSkipped {
            package: package.into(),
            hooks: hooks.to_vec(),
            reason,
            run_now,
            dry_run: false,
            allow,
        }
    }

    fn config_set(names: &[&str], skipped: usize) -> Option<AllowLine> {
        Some(AllowLine::ConfigSet {
            names: names.iter().map(|n| n.to_string()).collect(),
            skipped,
        })
    }

    #[test]
    fn the_skip_warns_after_the_summary_with_the_config_line() {
        let bed = Bed::new("warn-config");
        bed.package("hooked", "1.0", "", POST);
        let events = install_into(&bed, &["other_pkg"], "hooked", false);
        assert_eq!(
            told(&events),
            [warning(
                "hooked",
                &[HookKind::PostInstall],
                SkipReason::NotAllowed,
                true,
                config_set(&["other_pkg", "hooked"], 1),
            )]
        );
        let at = |wanted: fn(&Event) -> bool| events.iter().position(wanted).unwrap();
        assert!(
            at(|e| matches!(e, Event::InstallDone { .. }))
                < at(|e| matches!(e, Event::HooksSkipped { .. })),
            "{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::HooksNotRun { .. })),
            "the warning replaces the hooks-not-run line: {events:?}"
        );
    }

    #[test]
    fn two_skipped_packages_share_one_config_line_that_keeps_both() {
        let bed = Bed::new("warn-two");
        bed.package("a_hooked", "1.0", "", POST);
        bed.package("b_hooked", "1.0", "a_hooked>=1.0", POST);
        let events = install_into(&bed, &["other_pkg"], "b_hooked", false);
        let post = [HookKind::PostInstall];
        assert_eq!(
            told(&events),
            [
                warning("a_hooked", &post, SkipReason::NotAllowed, true, None),
                warning(
                    "b_hooked",
                    &post,
                    SkipReason::NotAllowed,
                    true,
                    config_set(&["other_pkg", "a_hooked", "b_hooked"], 2),
                ),
            ]
        );
    }

    #[test]
    fn a_listed_project_line_is_told_to_add_the_key() {
        let bed = Bed::new("warn-listed");
        bed.package("hooked", "1.0", "", POST);
        bed.manifest("hooked = \"*\"\n");
        let events = sync_into(&bed, &[], false);
        assert_eq!(
            told(&events),
            [warning(
                "hooked",
                &[HookKind::PostInstall],
                SkipReason::NotAllowed,
                true,
                Some(AllowLine::AddKey {
                    name: "hooked".into(),
                    table: "dependencies".into()
                }),
            )]
        );
    }

    #[test]
    fn an_unlisted_transitive_dependency_is_offered_a_version_star_line() {
        let bed = Bed::new("warn-transitive");
        bed.package("hooked", "1.0", "", POST);
        bed.package("beta", "1.0", "hooked>=1.0", &[]);
        bed.manifest("beta = \"*\"\n");
        let events = sync_into(&bed, &[], false);
        assert_eq!(
            told(&events),
            [warning(
                "hooked",
                &[HookKind::PostInstall],
                SkipReason::NotAllowed,
                true,
                Some(AllowLine::AddLine {
                    name: "hooked".into()
                }),
            )]
        );
    }

    #[test]
    fn headless_skips_an_allowed_package_and_says_the_allowlist_does_not_apply() {
        let events = headless_install("warn-headless-allowed", &["hooked"], false);
        assert_eq!(started(&events), []);
        assert_eq!(
            told(&events),
            [warning(
                "hooked",
                &[HookKind::PostInstall],
                SkipReason::Headless,
                false,
                Some(AllowLine::PassHooks),
            )]
        );
    }

    #[test]
    fn headless_and_not_allowed_gets_the_headless_reason() {
        let events = headless_install("warn-headless", &[], false);
        assert!(
            matches!(
                told(&events).as_slice(),
                [Event::HooksSkipped {
                    reason: SkipReason::Headless,
                    ..
                }]
            ),
            "{events:?}"
        );
    }

    #[test]
    fn a_pre_install_only_package_is_warned_with_no_run_it_now() {
        let bed = Bed::new("warn-pre-only");
        bed.package("pre_only", "1.0", "", PRE);
        let events = install_into(&bed, &[], "pre_only", false);
        assert_eq!(started(&events), []);
        assert_eq!(
            told(&events),
            [warning(
                "pre_only",
                &[HookKind::PreInstall],
                SkipReason::NotAllowed,
                false,
                config_set(&["pre_only"], 1),
            )]
        );
    }

    #[test]
    fn both_install_hooks_are_named_in_the_order_they_run() {
        let bed = Bed::new("warn-both");
        bed.package("hooked", "1.0", "", &["PostInstall", "PreInstall"]);
        let events = install_into(&bed, &[], "hooked", false);
        assert!(
            matches!(
                told(&events).as_slice(),
                [Event::HooksSkipped { hooks, .. }]
                    if hooks == &[HookKind::PreInstall, HookKind::PostInstall]
            ),
            "{events:?}"
        );
    }

    #[test]
    fn a_package_with_only_uninstall_hooks_is_skipped_and_recorded_without_a_warning() {
        let bed = Bed::new("warn-uninstall-only");
        bed.package("hooked", "1.0", "", &["PreUninstall"]);
        let events = install_into(&bed, &[], "hooked", false);
        assert_eq!(told(&events), []);
        assert!(lists_skipped(&bed.record("hooked"), "PreUninstall"));
    }

    #[test]
    fn a_dry_run_says_what_it_would_skip_without_a_run_it_now_and_writes_nothing() {
        let bed = Bed::new("warn-dry-run");
        bed.package("hooked", "1.0", "", POST);
        let sink = Recorded::default();
        let env = bed.on_installation(&sink, &[]);
        let opts = InstallOptions {
            dry_run: true,
            ..options(false)
        };
        install_package(&env, Some("hooked"), &opts).unwrap();
        let events = sink.events();
        assert_eq!(
            told(&events),
            [Event::HooksSkipped {
                package: "hooked".into(),
                hooks: vec![HookKind::PostInstall],
                reason: SkipReason::NotAllowed,
                run_now: false,
                dry_run: true,
                allow: config_set(&["hooked"], 1),
            }]
        );
        assert_eq!(started(&events), []);
        assert!(!install::is_installed(&bed.fake.roots(), "hooked"));
    }

    #[test]
    fn allowing_an_installed_package_with_a_skipped_hook_says_to_run_it_and_runs_nothing() {
        let bed = Bed::new("warn-already");
        bed.package("hooked", "1.0", "", POST);
        install_into(&bed, &[], "hooked", false);
        let again = install_into(&bed, &["hooked"], "hooked", false);
        assert_eq!(started(&again), []);
        assert_eq!(
            told(&again),
            [warning(
                "hooked",
                &[HookKind::PostInstall],
                SkipReason::AlreadyInstalled,
                true,
                None,
            )]
        );
    }

    #[test]
    fn a_dry_run_over_an_installed_package_now_allowed_offers_no_run_it_now() {
        let bed = Bed::new("warn-already-dry-run");
        bed.package("hooked", "1.0", "", POST);
        install_into(&bed, &[], "hooked", false);
        let sink = Recorded::default();
        let env = bed.on_installation(&sink, &["hooked"]);
        let opts = InstallOptions {
            dry_run: true,
            ..options(false)
        };
        install_package(&env, Some("hooked"), &opts).unwrap();
        assert_eq!(
            told(&sink.events()),
            [Event::HooksSkipped {
                package: "hooked".into(),
                hooks: vec![HookKind::PostInstall],
                reason: SkipReason::AlreadyInstalled,
                run_now: false,
                dry_run: true,
                allow: None,
            }]
        );
    }

    #[test]
    fn an_installed_package_that_is_still_not_allowed_is_not_warned_about_again() {
        let bed = Bed::new("warn-already-still-off");
        bed.package("hooked", "1.0", "", POST);
        install_into(&bed, &[], "hooked", false);
        let again = install_into(&bed, &[], "hooked", false);
        assert_eq!(told(&again), []);
    }

    #[test]
    fn a_package_whose_skipped_hook_run_hooks_has_run_is_not_warned_about_again() {
        let bed = Bed::new("warn-already-ran");
        bed.package("hooked", "1.0", "", POST);
        install_into(&bed, &[], "hooked", false);
        let roots = bed.fake.roots();
        let record = bed.record("hooked");
        assert!(record.hook_skipped(HookKind::PostInstall));
        crate::ops::hooks::settle_rerun(&roots, "hooked", Ok(std::time::Duration::ZERO)).unwrap();
        let again = install_into(&bed, &["hooked"], "hooked", false);
        assert_eq!(told(&again), []);
    }

    #[test]
    fn a_failure_after_a_skipped_package_still_warns_about_it() {
        let bed = Bed::new("warn-then-fail");
        bed.package("hooked", "1.0", "", POST);
        vip_in(
            &bed.root.join("packages"),
            "broken",
            "1.0",
            "hooked>=1.0",
            &["a.txt"],
            true,
        );
        let sink = Recorded::default();
        let env = bed.on_installation(&sink, &[]);
        install_package(&env, Some("broken"), &options(false)).unwrap_err();
        let events = sink.events();
        assert_eq!(skipped_packages(&events), ["hooked"]);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::InstallDone { .. })),
            "the install did not finish: {events:?}"
        );
    }

    #[test]
    fn nothing_is_warned_about_where_the_hooks_started_or_cannot_run() {
        // Allowed in hooks.run, or forced with --hooks: started, no warning.
        let bed = Bed::new("quiet-allowed");
        bed.package("hooked", "1.0", "", POST);
        assert_eq!(told(&install_into(&bed, &["hooked"], "hooked", false)), []);
        let bed = Bed::new("quiet-forced");
        bed.package("hooked", "1.0", "", POST);
        assert_eq!(told(&install_into(&bed, &[], "hooked", true)), []);

        // A scratch tree, headless or not, has its own notes.
        for headless in [false, true] {
            let bed = Bed::new(&format!("quiet-scratch-{headless}"));
            bed.package("hooked", "1.0", "", POST);
            let sink = Recorded::default();
            let mut env = bed.env(&sink, &[]);
            env.flags.prefix = Some(bed.root.join("dest"));
            env.headless = headless;
            install_package(&env, Some("hooked"), &options(false)).unwrap();
            assert_eq!(told(&sink.events()), [], "headless: {headless}");
        }

        // A venv keeps its hooks-not-run line.
        let bed = Bed::new("quiet-venv");
        bed.package("hooked", "1.0", "", POST);
        bed.manifest(&format!("hooked = {RUN}\n"));
        let _venv = venv_in(&bed.root);
        let sink = Recorded::default();
        let env = bed.env(&sink, &["hooked"]);
        sync(&env, &SyncOptions::default()).unwrap();
        assert_eq!(told(&sink.events()), []);
    }
}
