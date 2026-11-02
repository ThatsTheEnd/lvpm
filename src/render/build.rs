//! The wording of `lvpm build`.

use super::check;
use lvpm_core::ops::build::Built;
use lvpm_core::ops::check::{Checked, Library};
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
    format!("built {} ({} files)\n", built.path.display(), built.files)
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
/// or that it was skipped.
pub fn built_warning(library: Option<&Library>) -> String {
    match library {
        Some(library) => check::member_warnings(&library.checked) + &check::unused(&library.unused),
        None => SKIPPED.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lvpm_core::ops::check::{Finding, Problem, Unchecked};
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

    #[test]
    fn a_build_names_the_package_and_counts_its_files() {
        let b = Built {
            path: PathBuf::from("dist/demo-1.0.0.0.vip"),
            files: 4,
            library: None,
        };
        assert_eq!(built(&b), format!("built {} (4 files)\n", b.path.display()));
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
        assert_eq!(built_warning(None), SKIPPED);
        let library = Library {
            checked: checked(2, vec![]),
            unused: vec!["spare".into()],
        };
        assert_eq!(
            built_warning(Some(&library)),
            "[dependencies] names spare, which no member links to \
             (ignore this if it is loaded by path at run time)\n"
        );
    }
}
