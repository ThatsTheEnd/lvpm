//! The wording of `lvpm build`.

use super::{check, lock};
use lvpm_core::ops::build::{Built, Sbom};
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
    };
    format!("{why}, so the package has no SBOM; run lvpm lock to add one\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lvpm_core::lockfile::StaleReason;
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
        for without in [Sbom::NoLock, Sbom::Stale(StaleReason::LabviewChanged)] {
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
}
