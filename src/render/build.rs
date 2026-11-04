//! The wording of `lvpm build`.

use super::{check, lock, sync};
use anyhow::anyhow;
use lvpm_core::errors::CheckError;
use lvpm_core::ops::build::{Built, RunError, Sbom};
use lvpm_core::ops::build_app::{AppBuilt, AppError, AppRefusal};
use lvpm_core::ops::check::Checked;
use serde_json::{Value, json};
use std::path::Path;

/// stderr of a build that ran with `--no-check`.
const SKIPPED: &str = "check skipped (--no-check)\n";

/// The error of a build with no venv to check the library against.
pub fn no_venv(repo: &Path) -> String {
    format!(
        "no venv in {}: run lvpm venv create and lvpm sync, or pass --no-check",
        repo.display()
    )
}

/// What `lvpm build` prints once the package is written.
pub fn built(built: &Built) -> String {
    let with_sbom = match built.sbom {
        Sbom::Included => ", with SBOM",
        _ => "",
    };
    format!(
        "built {} ({} files{with_sbom})\n",
        built.path.display(),
        built.files
    )
}

/// What `lvpm build` prints once the application is built: where, then how
/// many more files LabVIEW made, then the SBOM.
pub fn app_built(built: &AppBuilt) -> String {
    let mut text = format!(
        "built application \"{}\" in {:.1}s: {}
",
        built.build_spec,
        built.took.as_secs_f64(),
        built
            .outputs
            .first()
            .map_or_else(String::new, |o| o.display().to_string())
    );
    if built.outputs.len() > 1 {
        text += &format!(
            "  (+{} more outputs)
",
            built.outputs.len() - 1
        );
    }
    text + &format!(
        "sbom: {}
",
        built.sbom.display()
    )
}

/// The `--json` result of a run that built an application, with the package it
/// built too, if any. A package alone is `{"command": "build"}`, as it always was.
pub fn result(package: Option<&Built>, app: Option<&AppBuilt>) -> Value {
    let mut result = json!({ "command": "build" });
    if let Some(built) = package {
        result["package"] = json!({
            "path": built.path,
            "files": built.files,
            "sbom": built.sbom == Sbom::Included,
        });
    }
    if let Some(built) = app {
        result["application"] = json!({
            "build_spec": built.build_spec,
            "outputs": built.outputs,
            "sbom": built.sbom,
            "took": built.took.as_secs_f64(),
        });
    }
    result
}

/// The error of a run that stopped: what happened, then what to do about it.
pub fn run_error(error: RunError) -> anyhow::Error {
    match error {
        RunError::Package(CheckError::NoVenv(repo)) => anyhow!(no_venv(&repo)),
        RunError::Package(other) => other.into(),
        RunError::App(AppError::Other(other)) => other,
        RunError::App(error) => anyhow!(app_error(&error)),
    }
}

/// An application build that did not happen or did not succeed. A refusal
/// names the command that puts it right.
fn app_error(error: &AppError) -> String {
    match error {
        AppError::Refused(AppRefusal::NoLock) => "no lvpm.lock; run lvpm lock".to_string(),
        AppError::Refused(AppRefusal::UnreadableLock(reason)) => format!(
            "lvpm.lock cannot be read ({}); run lvpm lock to regenerate it",
            reason.lines().next().unwrap_or_default()
        ),
        AppError::Refused(AppRefusal::StaleLock(reason)) => format!(
            "lvpm.lock is out of date ({}); run lvpm lock",
            lock::stale_reason(reason)
        ),
        AppError::Refused(AppRefusal::NoVenv(repo)) => format!(
            "no venv in {}; run lvpm venv create and lvpm sync",
            repo.display()
        ),
        AppError::Refused(AppRefusal::VenvDiffers(step)) => format!(
            "the venv differs from lvpm.lock ({}); run lvpm sync",
            sync::step_line(step)
        ),
        AppError::Launch { year, cause } => format!(
            "{cause:#}
the venv uses LabVIEW {year}; lvpm venv create --labview-version {year} rebinds it"
        ),
        AppError::LabView { .. } => format!(
            "{error}
open the LabVIEW project and run the build spec there to see details"
        ),
        AppError::Other(other) => format!("{other:#}"),
    }
}

/// stderr's last line of a build the check refused: `problems` findings.
fn refused(problems: usize) -> String {
    format!(
        "build refused: {problems} problem(s) in library members, nothing written; \
         fix them or pass --no-check\n"
    )
}

/// stderr of a refused build: what `check` warns of, then the refusal.
pub fn refusal_warning(checked: &Checked) -> String {
    check::member_warnings(checked) + &refused(checked.findings.len())
}

/// The `--json` result of a refused build: `check`'s, and the command.
pub fn refusal_result(checked: &Checked) -> Value {
    let mut result = check::result(checked);
    result["command"] = json!("build");
    result
}

/// stderr of a package that was written: what the check made of the library,
/// or that it was skipped, then why the package has no SBOM when it has none.
pub fn built_warning(built: &Built) -> String {
    let check = match &built.library {
        Some(library) => check::member_warnings(&library.checked) + &check::unused(&library.unused),
        None => SKIPPED.to_string(),
    };
    check + &no_sbom(&built.sbom)
}

/// Why a package has no SBOM and what to run; nothing when it has one.
fn no_sbom(sbom: &Sbom) -> String {
    let why = match sbom {
        Sbom::Included => return String::new(),
        Sbom::NoLock => "no lvpm.lock".to_string(),
        Sbom::Stale(reason) => format!("lvpm.lock is out of date ({})", lock::stale_reason(reason)),
        Sbom::Unreadable(reason) => format!(
            "lvpm.lock cannot be read ({})",
            reason.lines().next().unwrap_or_default()
        ),
    };
    format!("{why}, so the package has no SBOM; run lvpm lock to add one\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lvpm_core::events::SyncStep;
    use lvpm_core::lockfile::StaleReason;
    use lvpm_core::ops::build_app::LvError;
    use lvpm_core::ops::check::{Finding, Library, Problem, Unchecked};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn checked(vis: usize, findings: Vec<Finding>) -> Checked {
        Checked {
            vis,
            not_checked: BTreeMap::new(),
            saves_checked: true,
            findings,
        }
    }

    fn gone() -> Finding {
        Finding {
            vi: "Broken.vi".into(),
            problem: Problem::NotFound {
                link: "Sub/Gone.vi".into(),
            },
        }
    }

    fn a_build(library: Option<Library>, sbom: Sbom) -> Built {
        Built {
            path: PathBuf::from("dist/demo-1.0.0.0.vip"),
            files: 4,
            library,
            sbom,
        }
    }

    #[test]
    fn a_build_names_the_package_counts_its_files_and_says_when_it_has_an_sbom() {
        let with = a_build(None, Sbom::Included);
        let path = with.path.display();
        assert_eq!(built(&with), format!("built {path} (4 files, with SBOM)\n"));
        let without = [
            Sbom::NoLock,
            Sbom::Stale(StaleReason::LabviewChanged),
            Sbom::Unreadable("bad".into()),
        ];
        for without in without {
            assert_eq!(
                built(&a_build(None, without)),
                format!("built {path} (4 files)\n")
            );
        }
    }

    #[test]
    fn a_package_without_an_sbom_says_why_after_what_the_check_said() {
        let no_lock = a_build(None, Sbom::NoLock);
        assert_eq!(
            built_warning(&no_lock),
            format!(
                "{SKIPPED}no lvpm.lock, so the package has no SBOM; run lvpm lock to add one\n"
            )
        );
        let library = Library {
            checked: checked(2, vec![]),
            unused: vec!["spare".into()],
        };
        let stale = a_build(Some(library), Sbom::Stale(StaleReason::LabviewChanged));
        let text = built_warning(&stale);
        let lines: Vec<_> = text.lines().collect();
        assert!(lines[0].starts_with("[dependencies] names spare"), "{text}");
        assert_eq!(
            lines[1],
            "lvpm.lock is out of date (labview changed), so the package has no SBOM; \
             run lvpm lock to add one"
        );
        assert_eq!(lines.len(), 2, "{text}");
    }

    #[test]
    fn a_lock_that_cannot_be_read_says_so_and_why_in_its_first_line() {
        let unreadable = a_build(None, Sbom::Unreadable("expected `=`\nat line 2".into()));
        assert_eq!(
            built_warning(&unreadable),
            format!(
                "{SKIPPED}lvpm.lock cannot be read (expected `=`), so the package has no SBOM; \
                 run lvpm lock to add one\n"
            )
        );
    }

    #[test]
    fn a_refusal_counts_the_problems_and_says_how_to_go_on() {
        assert_eq!(
            refused(2),
            "build refused: 2 problem(s) in library members, nothing written; \
             fix them or pass --no-check\n"
        );
    }

    #[test]
    fn a_build_with_no_venv_says_how_to_go_on() {
        assert_eq!(
            no_venv(Path::new("proj")),
            "no venv in proj: run lvpm venv create and lvpm sync, or pass --no-check"
        );
    }

    #[test]
    fn a_refusal_warns_of_the_unchecked_links_and_ends_on_the_refusal() {
        let mut checked = checked(0, vec![gone()]);
        checked.not_checked.insert(Unchecked::Absolute, 3);
        let text = refusal_warning(&checked);
        let lines: Vec<_> = text.lines().collect();
        assert!(
            lines[0].starts_with("warning: links not checked: 3 absolute"),
            "{text}"
        );
        assert_eq!(lines[1], "warning: no VIs among the library's members");
        assert_eq!(format!("{}\n", lines[2]), refused(1));
        assert_eq!(lines.len(), 3, "{text}");
    }

    #[test]
    fn a_refusal_in_json_is_checks_result_and_names_the_command() {
        let result = refusal_result(&checked(3, vec![gone()]));
        assert_eq!(result["command"], "build");
        assert_eq!(result["ok"], false);
        assert_eq!(result["vis"], 3);
        assert_eq!(result["findings"][0]["vi"], "Broken.vi");
    }

    #[test]
    fn a_built_package_warns_of_the_unused_dependencies_or_that_the_check_was_skipped() {
        assert_eq!(built_warning(&a_build(None, Sbom::Included)), SKIPPED);
        let library = Library {
            checked: checked(2, vec![]),
            unused: vec!["spare".into()],
        };
        assert_eq!(
            built_warning(&a_build(Some(library), Sbom::Included)),
            "[dependencies] names spare, which no member links to \
             (ignore this if it is loaded by path at run time)\n"
        );
    }

    fn an_app(outputs: &[&str]) -> AppBuilt {
        AppBuilt {
            build_spec: "Demo App".into(),
            outputs: outputs.iter().map(PathBuf::from).collect(),
            sbom: PathBuf::from("builds/app/bom.cdx.json"),
            took: std::time::Duration::from_millis(50_340),
        }
    }

    #[test]
    fn an_application_says_where_it_is_how_many_more_files_there_are_and_where_its_sbom_is() {
        assert_eq!(
            app_built(&an_app(&["builds/app/Main.exe"])),
            "built application \"Demo App\" in 50.3s: builds/app/Main.exe\n\
             sbom: builds/app/bom.cdx.json\n"
        );
        assert_eq!(
            app_built(&an_app(&["builds/app/Main.exe", "a", "b"])),
            "built application \"Demo App\" in 50.3s: builds/app/Main.exe\n\
             \x20 (+2 more outputs)\n\
             sbom: builds/app/bom.cdx.json\n"
        );
    }

    #[test]
    fn a_run_that_builds_both_kinds_reports_both_in_json() {
        let package = a_build(None, Sbom::Included);
        let result = result(Some(&package), Some(&an_app(&["builds/app/Main.exe", "x"])));
        assert_eq!(
            result,
            json!({
                "command": "build",
                "package": {"path": "dist/demo-1.0.0.0.vip", "files": 4, "sbom": true},
                "application": {
                    "build_spec": "Demo App",
                    "outputs": ["builds/app/Main.exe", "x"],
                    "sbom": "builds/app/bom.cdx.json",
                    "took": 50.34,
                },
            })
        );
    }

    #[test]
    fn json_omits_the_kind_that_was_not_built_and_a_package_alone_keeps_its_shape() {
        let application = result(None, Some(&an_app(&["m.exe"])));
        assert!(application.get("package").is_none(), "{application}");
        assert_eq!(application["application"]["build_spec"], "Demo App");
        assert_eq!(result(None, None), json!({"command": "build"}));
    }

    #[test]
    fn a_package_without_an_sbom_says_so_in_json() {
        let package = a_build(None, Sbom::NoLock);
        let result = result(Some(&package), Some(&an_app(&["m.exe"])));
        assert_eq!(result["package"]["sbom"], false);
    }

    #[test]
    fn every_refusal_says_what_to_run() {
        let text = |why: AppRefusal| run_error(RunError::App(AppError::Refused(why))).to_string();
        assert_eq!(text(AppRefusal::NoLock), "no lvpm.lock; run lvpm lock");
        assert_eq!(
            text(AppRefusal::StaleLock(StaleReason::LabviewChanged)),
            "lvpm.lock is out of date (labview changed); run lvpm lock"
        );
        assert_eq!(
            text(AppRefusal::UnreadableLock("expected `=`\nat line 2".into())),
            "lvpm.lock cannot be read (expected `=`); run lvpm lock to regenerate it"
        );
        assert_eq!(
            text(AppRefusal::NoVenv(PathBuf::from("proj"))),
            "no venv in proj; run lvpm venv create and lvpm sync"
        );
        assert_eq!(
            text(AppRefusal::VenvDiffers(SyncStep::Replace {
                name: "alpha".into(),
                from: "0.9".into(),
                to: "1.0".into()
            })),
            "the venv differs from lvpm.lock (replace alpha 0.9 -> 1.0); run lvpm sync"
        );
    }

    #[test]
    fn labviews_error_is_named_with_its_code_and_says_where_to_see_more() {
        let error = RunError::App(AppError::LabView {
            build_spec: "Demo App".into(),
            error: LvError {
                code: 1003,
                message: "The VI is not executable".into(),
            },
        });
        assert_eq!(
            run_error(error).to_string(),
            "LabVIEW could not build \"Demo App\": 1003 The VI is not executable\n\
             open the LabVIEW project and run the build spec there to see details"
        );
    }

    #[test]
    fn a_labview_that_did_not_start_names_the_year_the_venv_rebinds_to() {
        let error = RunError::App(AppError::Launch {
            year: 2026,
            cause: anyhow!("LabVIEW 2026 not found"),
        });
        assert_eq!(
            run_error(error).to_string(),
            "LabVIEW 2026 not found\n\
             the venv uses LabVIEW 2026; lvpm venv create --labview-version 2026 rebinds it"
        );
    }

    #[test]
    fn any_other_failure_is_its_own_text_and_never_says_could_not_build() {
        let error = RunError::App(AppError::Other(anyhow!("connection lost")));
        assert_eq!(run_error(error).to_string(), "connection lost");
        let package = RunError::Package(CheckError::NoVenv(PathBuf::from("proj")));
        assert_eq!(run_error(package).to_string(), no_venv(Path::new("proj")));
    }
}
