//! Linkfix as an operation: `lvpm linkfix` over a venv or a tree, and the
//! pass an install runs over the LabVIEW files it wrote. See
//! [`crate::linkfix`] for what it changes in a file.

use super::env::{Destination, OpEnv};
use super::install::{Applied, InstallOptions};
use super::session::InstallSession;
use crate::events::{Event, LinkfixReport, Phase, SkipReason};
use crate::linkfix::{self, Changes, Method, Methods, Outcome};
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};

/// What `lvpm linkfix` is asked, without the flags a front end spells them as.
#[derive(Debug, Clone)]
pub struct LinkfixOptions {
    /// A venv store or LabVIEW-shaped tree to fix instead of the venv in effect.
    pub tree: Option<PathBuf>,
    pub dry_run: bool,
    pub verbose: bool,
    pub methods: Vec<Method>,
    /// Write a JSON report of every changed or skipped file here.
    pub report: Option<PathBuf>,
}

impl Default for LinkfixOptions {
    fn default() -> Self {
        LinkfixOptions {
            tree: None,
            dry_run: false,
            verbose: false,
            methods: Method::ALL.to_vec(),
            report: None,
        }
    }
}

/// `lvpm linkfix`: fix every LabVIEW file under the venv in effect, or under
/// `--tree`. A venv store is a set of `<package>/1/` roots; anything else is
/// one LabVIEW-shaped root.
pub fn linkfix(env: &OpEnv, opts: &LinkfixOptions) -> Result<()> {
    let roots = match &opts.tree {
        Some(t) => {
            let store_like = linkfix::venv_roots(t)?;
            if store_like.is_empty() {
                vec![t.clone()]
            } else {
                store_like
            }
        }
        None => match env.destination()? {
            Destination::Venv(v) => linkfix::venv_roots(&v.dir)?,
            Destination::Global(_) | Destination::Scratch(_) => bail!(
                "give --tree <DIR> for an explicit LabVIEW-shaped tree, or run linkfix inside a venv; \
                 install prepares newly written files automatically"
            ),
        },
    };
    ensure!(!roots.is_empty(), "nothing to fix: no package roots found");
    env.sink.emit(Event::LinkfixStarted {
        roots: roots.len(),
        methods: opts.methods.clone(),
        dry_run: opts.dry_run,
    });
    let (_tree, outcomes) = linkfix::fix_tree(&roots, opts.dry_run, Methods::of(&opts.methods))?;
    if let Some(path) = &opts.report {
        write_report(path, &outcomes, opts.dry_run, &opts.methods)?;
        env.sink
            .emit(Event::LinkfixReportWritten { path: path.clone() });
    }
    env.sink.emit(Event::LinkfixDone {
        report: report(&outcomes, opts.dry_run, opts.verbose),
    });
    Ok(())
}

/// The install's linkfix pass, over every LabVIEW-tree file it wrote, after
/// the last package is on disk and before the PostInstall hooks. `--relink`
/// takes LabVIEW's load-and-save pass instead. A failure is reported, not
/// fatal: the files are installed as copied.
pub(super) fn after_install(
    env: &OpEnv,
    session: &InstallSession,
    applied: &Applied,
    opts: &InstallOptions,
) {
    let files = &session.linkfix_files;
    if files.is_empty() {
        return;
    }
    let skipped = |reason| Event::PhaseSkipped {
        phase: Phase::Linkfix,
        reason,
        skipped: files.len(),
    };
    if opts.no_linkfix {
        return env.sink.emit(skipped(SkipReason::Declined));
    }
    if opts.force_relink {
        return env.sink.emit(skipped(SkipReason::RelinkInstead));
    }
    if applied.dry_run {
        return env.sink.emit(Event::LinkfixPlanned { files: files.len() });
    }
    env.sink.emit(Event::PhaseStarted {
        phase: Phase::Linkfix,
    });
    env.sink.emit(Event::LinkfixPreparing);
    let roots = match session.venv() {
        Some(v) => linkfix::venv_roots(&v.dir),
        None => Ok(vec![session.store().application.clone()]),
    };
    match roots
        .and_then(|roots| linkfix::fix_files(&roots, files, false, Methods::of(&Method::ALL)))
    {
        Ok((_, outcomes)) => env.sink.emit(Event::LinkfixDone {
            report: report(&outcomes, false, true),
        }),
        Err(error) => env.sink.emit(Event::LinkfixFailed {
            error: format!("{error:#}"),
        }),
    }
}

fn report(outcomes: &[Outcome], dry_run: bool, verbose: bool) -> LinkfixReport {
    let mut r = LinkfixReport {
        dry_run,
        verbose,
        looked_at: outcomes.len(),
        changed: 0,
        clean: 0,
        totals: Changes::default(),
        not_separated: Vec::new(),
        changed_files: Vec::new(),
        skipped_files: Vec::new(),
    };
    for o in outcomes {
        match &o.result {
            Ok(c) => {
                if let Some(why) = &c.not_separated {
                    r.not_separated.push((o.path.clone(), why.clone()));
                }
                if c.total() == 0 {
                    r.clean += 1;
                    continue;
                }
                r.changed += 1;
                let t = &mut r.totals;
                t.owner += c.owner;
                t.relative += c.relative;
                t.absolute += c.absolute;
                t.qualifier += c.qualifier;
                t.vils += c.vils;
                t.separated += c.separated;
                r.changed_files.push((o.path.clone(), c.clone()));
            }
            Err(e) => r.skipped_files.push((o.path.clone(), format!("{e:#}"))),
        }
    }
    r
}

/// The JSON `--report`: per changed, skipped or not-separated file, what each
/// method changed or why the file was skipped.
fn write_report(
    path: &Path,
    outcomes: &[Outcome],
    dry_run: bool,
    methods: &[Method],
) -> Result<()> {
    let r = report(outcomes, dry_run, true);
    let mut files = Vec::new();
    for o in outcomes {
        match &o.result {
            Ok(c) if c.total() > 0 || c.not_separated.is_some() => files
                .push(serde_json::json!({ "path": o.path, "methods": c.methods(), "changes": c })),
            Ok(_) => {}
            Err(e) => {
                files.push(serde_json::json!({ "path": o.path, "skipped": format!("{e:#}") }))
            }
        }
    }
    let doc = serde_json::json!({
        "methods": methods,
        "dry_run": dry_run,
        "looked_at": r.looked_at,
        "changed": r.changed,
        "clean": r.clean,
        "skipped": r.skipped_files.len(),
        "not_separated": r.not_separated.len(),
        "files": files,
    });
    std::fs::write(path, serde_json::to_string_pretty(&doc)?)
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(path: &str, result: Result<Changes>) -> Outcome {
        Outcome {
            path: PathBuf::from(path),
            result,
        }
    }

    #[test]
    fn a_report_counts_changed_clean_and_skipped_files_and_names_the_unseparated() {
        let changed = Changes {
            owner: 1,
            relative: 2,
            separated: 1,
            ..Changes::default()
        };
        let kept_code = Changes {
            not_separated: Some("unknown tag layout".into()),
            ..Changes::default()
        };
        let outcomes = vec![
            outcome("a.vi", Ok(changed.clone())),
            outcome("b.vi", Ok(Changes::default())),
            outcome("c.vi", Ok(kept_code)),
            outcome("d.vi", Err(anyhow::anyhow!("truncated heap"))),
        ];
        let r = report(&outcomes, false, true);
        assert_eq!((r.looked_at, r.changed, r.clean), (4, 1, 2));
        assert_eq!(r.totals, changed);
        assert_eq!(r.changed_files, vec![(PathBuf::from("a.vi"), changed)]);
        assert_eq!(
            r.not_separated,
            vec![(PathBuf::from("c.vi"), "unknown tag layout".into())]
        );
        assert_eq!(
            r.skipped_files,
            vec![(PathBuf::from("d.vi"), "truncated heap".into())]
        );
    }
}
