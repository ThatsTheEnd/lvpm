//! The wording of `lvpm add` and `lvpm remove`: what they did to `lvpm.toml`,
//! and the errors they end in. The lock and the sync they run say what they do
//! in their own words (`lock`, `sync`).

use super::lock::error_for as lock_error;
use super::sync::{error as sync_error, incomplete_body};
use super::{Renderer, Stream};
use lvpm_core::errors::{EditError, LockError, SyncError};
use lvpm_core::events::{Event, ManifestEdit};
use lvpm_core::manifest_edit::DependencyTable;

pub(super) fn render(r: &mut Renderer, event: &Event) {
    match event {
        Event::ManifestEdited {
            edit,
            dev,
            group,
            created_table,
            dry_run,
        } => {
            let table = DependencyTable::from_flags(*dev, group.as_deref());
            r.line(Stream::Out, &edit_line(edit, &table, *dry_run));
            if *created_table {
                let verb = if *dry_run { "would create" } else { "created" };
                r.line(
                    Stream::Out,
                    &format!("{verb} [{}], a new group", table.key()),
                );
            }
        }
        Event::PackageStillRequired { name, required_by } => r.line(
            Stream::Out,
            &format!(
                "{name} is still required by {}: it stays in lvpm.lock and installed",
                required_by.join(", ")
            ),
        ),
        Event::InstallAddsToProject { package } => r.line(
            Stream::Out,
            &format!(
                "adding {package} to lvpm.toml and syncing the project to lvpm.lock (to install \
                 it without changing the project, pass --prefix, --global or --labview-version)"
            ),
        ),
        Event::UninstalledStillListed { name, dev, group } => {
            let table = DependencyTable::from_flags(*dev, group.as_deref());
            r.line(
                Stream::Out,
                &format!(
                    "note: {name} is still in lvpm.toml; `{}` removes it for good (a sync \
                     would install it again)",
                    remove_command(name, &table)
                ),
            )
        }
        _ => unreachable!("routed here by Renderer::render: {event:?}"),
    }
}

/// The flag that names `table` to `add` and `remove`: none for `[dependencies]`.
fn flag_for(table: &DependencyTable) -> String {
    match table {
        DependencyTable::Dependencies => String::new(),
        DependencyTable::DevDependencies => "--dev".to_string(),
        DependencyTable::Group(group) => format!("--group {group}"),
    }
}

/// The `lvpm remove` that takes `name` out of `table`.
fn remove_command(name: &str, table: &DependencyTable) -> String {
    match flag_for(table).as_str() {
        "" => format!("lvpm remove {name}"),
        flag => format!("lvpm remove {name} {flag}"),
    }
}

/// What happened to one manifest entry, or on a dry run what would.
fn edit_line(edit: &ManifestEdit, table: &DependencyTable, dry_run: bool) -> String {
    let header = format!("[{}]", table.key());
    let table = match table {
        DependencyTable::Dependencies => String::new(),
        _ => format!(" {header}"),
    };
    match (edit, dry_run) {
        (ManifestEdit::Added { name, constraint }, false) => {
            format!("lvpm.toml: added {name} = \"{constraint}\" to {header}")
        }
        (ManifestEdit::Added { name, constraint }, true) => {
            format!("lvpm.toml would get: {name} = \"{constraint}\"{table}")
        }
        (ManifestEdit::Updated { name, from, to }, false) => {
            format!("lvpm.toml was updated: {name} \"{from}\" -> \"{to}\"{table}")
        }
        (ManifestEdit::Updated { name, from, to }, true) => {
            format!("lvpm.toml would be updated: {name} \"{from}\" -> \"{to}\"{table}")
        }
        (ManifestEdit::Removed { name, constraint }, false) => {
            format!("lvpm.toml: removed {name} = \"{constraint}\"{table}")
        }
        (ManifestEdit::Removed { name, constraint }, true) => {
            format!("lvpm.toml would lose: {name} = \"{constraint}\"{table}")
        }
    }
}

/// What `lvpm add` and `lvpm remove` print when they fail: what happened, then
/// the next thing to do. `rerun` is the command to run again, as "run {rerun}
/// again" reads.
pub fn error(error: &EditError, rerun: &str) -> String {
    match error {
        EditError::Lock(LockError::NeedsNewerLabview {
            name, project_year, ..
        }) => format!(
            "{error}\n\
             next: pick a version that runs on LabVIEW {project_year} with \
             `lvpm add {name}@<version>`, or raise labview in lvpm.toml"
        ),
        EditError::Lock(cause) => lock_error(cause, rerun),
        EditError::Sync(SyncError::Incomplete { done, left, cause }) => format!(
            "{}\nlvpm.toml and lvpm.lock were put back as they were\n\
             next: fix what is wrong and run {rerun} again; `lvpm sync` puts the destination \
             back in line with lvpm.lock",
            incomplete_body(done, left, cause)
        ),
        EditError::Sync(cause) => sync_error(cause),
        EditError::Cancelled => {
            format!("{error}; lvpm.toml and lvpm.lock are unchanged\nnext: run {rerun} again")
        }
        EditError::EmptyConstraint { name } => format!(
            "{error}\n\
             quote it, e.g. \"{name}@>=0.5\": an unquoted > sends the output to a file\n\
             the shell may have created a file named \"=0.5\" here; it can be deleted\n\
             next: run `lvpm add \"{name}@>=0.5\"` with the constraint you mean, or \
             `lvpm add {name}` for the newest version"
        ),
        EditError::NotADependency { name } => format!(
            "{error}\n\
             next: name a package that lvpm.toml lists, or add it with `lvpm add {name}`"
        ),
        EditError::NotInGroup { name, group } => format!(
            "{error}\n\
             next: name a package that [dependency-groups.{group}] lists, or add it with \
             `lvpm add {name} --group {group}`"
        ),
        EditError::InOtherTable { name, found_in } => {
            format!("{error}\n{}", other_table_hints(name, found_in))
        }
        EditError::Other(cause) => format!("{cause:#}"),
    }
}

/// The way out of naming a table that does not hold the package: one `next:`
/// for the one table that does, one line for each of several.
fn other_table_hints(name: &str, found_in: &[DependencyTable]) -> String {
    match found_in {
        [only] => {
            let command = remove_command(name, only);
            match flag_for(only).as_str() {
                "" => format!("next: leave out --dev: `{command}`"),
                flag => format!("next: use {flag}: `{command}`"),
            }
        }
        several => (several.iter())
            .map(|table| {
                format!(
                    "next: `{}` removes it from [{}]",
                    remove_command(name, table),
                    table.key()
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::renderer;
    use super::*;
    use lvpm_core::events::SyncStep;

    fn added() -> ManifestEdit {
        ManifestEdit::Added {
            name: "gamma".into(),
            constraint: ">=1.0".into(),
        }
    }

    #[test]
    fn an_edit_is_a_line_on_stdout_and_a_dry_run_says_it_is_only_a_plan() {
        let (mut renderer, out, err) = renderer();
        for (edit, dev, dry_run) in [
            (added(), false, false),
            (added(), true, false),
            (added(), false, true),
            (
                ManifestEdit::Updated {
                    name: "beta".into(),
                    from: ">=1.0".into(),
                    to: ">=0.5".into(),
                },
                false,
                false,
            ),
            (
                ManifestEdit::Removed {
                    name: "gamma".into(),
                    constraint: ">=1.0".into(),
                },
                true,
                true,
            ),
        ] {
            renderer.render(&Event::ManifestEdited {
                edit,
                dev,
                group: None,
                created_table: false,
                dry_run,
            });
        }
        assert_eq!(
            out.text(),
            "lvpm.toml: added gamma = \">=1.0\" to [dependencies]\n\
             lvpm.toml: added gamma = \">=1.0\" to [dev-dependencies]\n\
             lvpm.toml would get: gamma = \">=1.0\"\n\
             lvpm.toml was updated: beta \">=1.0\" -> \">=0.5\"\n\
             lvpm.toml would lose: gamma = \">=1.0\" [dev-dependencies]\n"
        );
        assert_eq!(err.text(), "");
    }

    #[test]
    fn a_named_install_in_a_project_says_it_is_an_add_and_how_to_install_without_one() {
        let (mut renderer, out, _) = renderer();
        renderer.render(&Event::InstallAddsToProject {
            package: "gamma@1.0".into(),
        });
        assert_eq!(
            out.text(),
            "adding gamma@1.0 to lvpm.toml and syncing the project to lvpm.lock (to install it \
             without changing the project, pass --prefix, --global or --labview-version)\n"
        );
    }

    #[test]
    fn an_uninstalled_package_that_the_manifest_still_lists_says_how_to_remove_it_for_good() {
        let (mut renderer, out, err) = renderer();
        for dev in [false, true] {
            renderer.render(&Event::UninstalledStillListed {
                name: "caraya".into(),
                dev,
                group: None,
            });
        }
        assert_eq!(
            out.text(),
            "note: caraya is still in lvpm.toml; `lvpm remove caraya` removes it for good (a sync \
             would install it again)\n\
             note: caraya is still in lvpm.toml; `lvpm remove caraya --dev` removes it for good \
             (a sync would install it again)\n"
        );
        assert_eq!(err.text(), "");
    }

    fn group(name: &str) -> DependencyTable {
        DependencyTable::Group(name.into())
    }

    #[test]
    fn an_edit_of_a_group_names_its_table_and_a_new_one_says_it_was_made_on_a_line_of_its_own() {
        let (mut renderer, out, err) = renderer();
        for (created_table, dry_run) in [(true, false), (false, false), (true, true)] {
            renderer.render(&Event::ManifestEdited {
                edit: added(),
                dev: false,
                group: Some("ci".into()),
                created_table,
                dry_run,
            });
        }
        renderer.render(&Event::ManifestEdited {
            edit: ManifestEdit::Removed {
                name: "gamma".into(),
                constraint: ">=1.0".into(),
            },
            dev: false,
            group: Some("ci".into()),
            created_table: false,
            dry_run: false,
        });
        assert_eq!(
            out.text(),
            "lvpm.toml: added gamma = \">=1.0\" to [dependency-groups.ci]\n\
             created [dependency-groups.ci], a new group\n\
             lvpm.toml: added gamma = \">=1.0\" to [dependency-groups.ci]\n\
             lvpm.toml would get: gamma = \">=1.0\" [dependency-groups.ci]\n\
             would create [dependency-groups.ci], a new group\n\
             lvpm.toml: removed gamma = \">=1.0\" [dependency-groups.ci]\n"
        );
        assert_eq!(err.text(), "");
    }

    #[test]
    fn an_uninstalled_package_listed_in_a_group_says_to_remove_it_with_that_group() {
        let (mut renderer, out, _) = renderer();
        renderer.render(&Event::UninstalledStillListed {
            name: "ci_tools".into(),
            dev: false,
            group: Some("ci".into()),
        });
        assert_eq!(
            out.text(),
            "note: ci_tools is still in lvpm.toml; `lvpm remove ci_tools --group ci` removes it \
             for good (a sync would install it again)\n"
        );
    }

    #[test]
    fn a_name_in_a_group_is_told_to_use_that_group_and_in_several_tables_gets_a_line_for_each() {
        let in_group = written(&EditError::InOtherTable {
            name: "beta".into(),
            found_in: vec![group("ci")],
        });
        assert_eq!(
            in_group,
            "beta is in [dependency-groups.ci]\nnext: use --group ci: `lvpm remove beta --group ci`"
        );
        let in_several = written(&EditError::InOtherTable {
            name: "beta".into(),
            found_in: vec![
                DependencyTable::Dependencies,
                DependencyTable::DevDependencies,
                group("ci"),
            ],
        });
        assert_eq!(
            in_several,
            "beta is in [dependencies], [dev-dependencies] and [dependency-groups.ci]\n\
             next: `lvpm remove beta` removes it from [dependencies]\n\
             next: `lvpm remove beta --dev` removes it from [dev-dependencies]\n\
             next: `lvpm remove beta --group ci` removes it from [dependency-groups.ci]"
        );
    }

    #[test]
    fn a_name_that_is_not_in_the_group_says_which_group_and_how_to_add_it() {
        let text = written(&EditError::NotInGroup {
            name: "beta".into(),
            group: "ci".into(),
        });
        assert_eq!(
            text,
            "beta is not in [dependency-groups.ci]\n\
             next: name a package that [dependency-groups.ci] lists, or add it with \
             `lvpm add beta --group ci`"
        );
    }

    fn written(error: &EditError) -> String {
        super::error(error, "`lvpm add`")
    }

    #[test]
    fn an_empty_constraint_says_to_quote_it_and_what_the_shell_may_have_left_behind() {
        let text = written(&EditError::EmptyConstraint {
            name: "gamma".into(),
        });
        for said in [
            "a constraint is expected after \"@\" in \"gamma@\"",
            "quote it, e.g. \"gamma@>=0.5\"",
            "the shell may have created a file named \"=0.5\"",
            "can be deleted",
        ] {
            assert!(text.contains(said), "{said:?} is not in:\n{text}");
        }
    }

    #[test]
    fn a_name_in_the_other_table_is_told_which_flag_to_change() {
        let in_dev = written(&EditError::InOtherTable {
            name: "caraya".into(),
            found_in: vec![DependencyTable::DevDependencies],
        });
        assert_eq!(
            in_dev,
            "caraya is in [dev-dependencies]\nnext: use --dev: `lvpm remove caraya --dev`"
        );
        let in_normal = written(&EditError::InOtherTable {
            name: "beta".into(),
            found_in: vec![DependencyTable::Dependencies],
        });
        assert_eq!(
            in_normal,
            "beta is in [dependencies]\nnext: leave out --dev: `lvpm remove beta`"
        );
    }

    #[test]
    fn a_sync_that_stopped_says_both_files_were_put_back_and_how_to_bring_the_destination_in_line()
    {
        let text = written(&EditError::Sync(SyncError::Incomplete {
            done: Vec::new(),
            left: vec![SyncStep::Install {
                name: "gamma".into(),
                version: "1.0".into(),
            }],
            cause: "disk full".into(),
        }));
        assert_eq!(
            text,
            "sync stopped before it was done: disk full\n\
             not done:\n  install gamma 1.0\n\
             lvpm.toml and lvpm.lock were put back as they were\n\
             next: fix what is wrong and run `lvpm add` again; `lvpm sync` puts the destination \
             back in line with lvpm.lock"
        );
    }

    #[test]
    fn every_error_but_a_foreign_one_ends_with_the_next_thing_to_run_and_a_lock_error_reruns_the_command()
     {
        for error in [
            EditError::EmptyConstraint { name: "g".into() },
            EditError::NotADependency { name: "g".into() },
            EditError::NotInGroup {
                name: "g".into(),
                group: "ci".into(),
            },
            EditError::InOtherTable {
                name: "g".into(),
                found_in: vec![DependencyTable::Dependencies, group("ci")],
            },
            EditError::Lock(LockError::NoLabview),
            EditError::Lock(LockError::NeedsNewerLabview {
                name: "omega".into(),
                version: "1.0".into(),
                needs_year: 2026,
                project_year: 2024,
            }),
            EditError::Sync(SyncError::Incomplete {
                done: Vec::new(),
                left: Vec::new(),
                cause: "x".into(),
            }),
        ] {
            let text = written(&error);
            let last = text.lines().last().unwrap();
            assert!(last.starts_with("next: "), "{text}");
        }
        let text = written(&EditError::Lock(LockError::NoLabview));
        assert!(text.ends_with("then run `lvpm add` again"), "{text}");
        assert!(!text.contains("`lvpm lock`"), "{text}");
    }

    #[test]
    fn a_cancelled_edit_says_cancelled_and_that_both_files_are_unchanged() {
        let text = written(&EditError::Cancelled);
        assert_eq!(
            text,
            "cancelled; lvpm.toml and lvpm.lock are unchanged\nnext: run `lvpm add` again"
        );
    }

    #[test]
    fn a_package_that_stays_says_which_packages_require_it() {
        let (mut renderer, out, _) = renderer();
        renderer.render(&Event::PackageStillRequired {
            name: "alpha".into(),
            required_by: vec!["beta".into(), "caraya".into()],
        });
        assert_eq!(
            out.text(),
            "alpha is still required by beta, caraya: it stays in lvpm.lock and installed\n"
        );
    }
}
