//! The wording of `lvpm check`: a line per finding, a `next:` line per piece of
//! advice, the summary, the warnings for stderr, and the `--json` result. VI
//! and link names come from untrusted files, so the text is made printable.

use super::printable;
use lvpm_core::ops::check::{Checked, Finding, Problem, Unchecked};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;

/// stdout: each finding, the advice for the rules that occurred, in the order
/// the rules are listed in, and the summary.
pub fn listing(checked: &Checked) -> String {
    let mut text = String::new();
    for finding in &checked.findings {
        text.push_str(&format!("{}: {}\n", finding.vi, what(&finding.problem)));
    }
    // `Problem`'s order is the rule order, so sorting puts the advice in it.
    let mut problems: Vec<&Problem> = checked.findings.iter().map(|f| &f.problem).collect();
    problems.sort();
    let mut seen = HashSet::new();
    for advice in problems
        .into_iter()
        .map(next)
        .filter(|a| seen.insert(a.clone()))
    {
        text.push_str(&format!("next: {advice}\n"));
    }
    text.push_str(&format!(
        "VIs checked: {}, problems: {}, links not checked: {}\n",
        checked.vis,
        checked.findings.len(),
        checked.not_checked_total()
    ));
    printable(text)
}

/// stderr: each cause of links not looked for, and what could not be said of
/// the run. `dir` is the folder that was asked for, if one was.
pub fn warnings(checked: &Checked, dir: Option<&Path>) -> String {
    let mut text = String::new();
    for (cause, count) in &checked.not_checked {
        let why = match cause {
            Unchecked::NoInstallation {
                folder,
                installation,
            } => format!(
                "{folder} (the bound installation {} is not on this machine)",
                installation.display()
            ),
            Unchecked::Root(root) => format!("{root} (lvpm does not resolve this root)"),
        };
        text.push_str(&format!("warning: links not checked: {count} into {why}\n"));
    }
    if !checked.saves_checked {
        text.push_str(
            "warning: lvpm.toml has no labview, so save versions were not checked
",
        );
    }
    if checked.vis == 0 {
        let under = dir.map_or_else(|| "the project".to_string(), |d| d.display().to_string());
        text.push_str(&format!("warning: no VIs under {under}\n"));
    }
    printable(text)
}

/// The `--json` result: the same shape whether the check passed or not.
pub fn result(checked: &Checked) -> Value {
    json!({
        "ok": checked.findings.is_empty(),
        "vis": checked.vis,
        "not_checked": checked.not_checked_total(),
        "findings": checked.findings.iter().map(finding).collect::<Vec<_>>(),
    })
}

fn finding(finding: &Finding) -> Value {
    let (kind, link, block, offset) = match &finding.problem {
        Problem::NotResource => ("not_resource", None, None, None),
        Problem::Unreadable { block, offset } => ("unreadable", None, Some(*block), Some(*offset)),
        Problem::TooNew { .. } => ("too_new", None, None, None),
        Problem::NotSeparated => ("not_separated", None, None, None),
        Problem::NotFound { link } => ("not_found", Some(link.as_str()), None, None),
    };
    let (saved, minimum) = match &finding.problem {
        Problem::TooNew { saved, minimum } => (Some(saved), Some(minimum)),
        _ => (None, None),
    };
    json!({
        "vi": finding.vi,
        "kind": kind,
        "link": link,
        "package": null,
        "table": null,
        "saved": saved,
        "minimum": minimum,
        "block": block,
        "offset": offset,
    })
}

/// What a finding says after the VI's name.
fn what(problem: &Problem) -> String {
    match problem {
        Problem::NotResource => "not a LabVIEW resource file".to_string(),
        Problem::Unreadable { block, offset } => format!(
            "linker table not readable ({block} at byte {offset}): format not supported yet"
        ),
        Problem::TooNew { saved, minimum } => {
            format!("saved in LabVIEW {saved}, newer than the minimum {minimum}")
        }
        Problem::NotSeparated => "compiled code is not separated from the source".to_string(),
        Problem::NotFound { link } => format!("link not found: {link}"),
    }
}

/// What to do about a finding, after `next: `.
fn next(problem: &Problem) -> String {
    match problem {
        Problem::NotResource => "restore the file (is it a git-LFS pointer or a cut-short copy?) \
                                 or delete it"
            .to_string(),
        Problem::Unreadable { .. } => "report it at https://github.com/ThatsTheEnd/lvpm/issues \
                                       with the VI attached"
            .to_string(),
        Problem::TooNew { minimum, .. } => {
            format!("save the VI for LabVIEW {minimum}, or raise labview in lvpm.toml")
        }
        Problem::NotSeparated => {
            "turn on \"Separate compiled code from source file\" in the VI's properties".to_string()
        }
        Problem::NotFound { .. } => {
            "restore the file, or open the VI in LabVIEW and relink it".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn found(vi: &str, problem: Problem) -> Finding {
        Finding {
            vi: vi.into(),
            problem,
        }
    }

    fn gone(vi: &str, link: &str) -> Finding {
        found(vi, Problem::NotFound { link: link.into() })
    }

    fn checked(vis: usize, findings: Vec<Finding>) -> Checked {
        Checked {
            vis,
            not_checked: BTreeMap::new(),
            saves_checked: true,
            findings,
        }
    }

    #[test]
    fn a_clean_check_is_only_the_summary() {
        assert_eq!(
            listing(&checked(2, vec![])),
            "VIs checked: 2, problems: 0, links not checked: 0\n"
        );
    }

    #[test]
    fn each_finding_is_a_line_then_one_next_per_rule_in_rule_order() {
        let unreadable = Problem::Unreadable {
            block: "LIvi",
            offset: 1234,
        };
        // As the op sorts them: by VI, so the rules are not in order here.
        let findings = vec![
            gone("Broken.vi", "Sub/Gone.vi"),
            gone("Broken2.vi", "Sub/Gone.vi"),
            found("Good.vi", unreadable),
            found("x.vi", Problem::NotResource),
        ];
        assert_eq!(
            listing(&checked(4, findings)),
            "Broken.vi: link not found: Sub/Gone.vi\n\
             Broken2.vi: link not found: Sub/Gone.vi\n\
             Good.vi: linker table not readable (LIvi at byte 1234): format not supported yet\n\
             x.vi: not a LabVIEW resource file\n\
             next: restore the file (is it a git-LFS pointer or a cut-short copy?) or delete it\n\
             next: report it at https://github.com/ThatsTheEnd/lvpm/issues with the VI attached\n\
             next: restore the file, or open the VI in LabVIEW and relink it\n\
             VIs checked: 4, problems: 4, links not checked: 0\n"
        );
    }

    #[test]
    fn names_from_files_cannot_drive_the_terminal() {
        let findings = vec![gone("a\u{1b}[31m.vi", "<vilib>/b\u{7}.vi")];
        let text = listing(&checked(1, findings));
        assert!(
            text.starts_with("a?[31m.vi: link not found: <vilib>/b?.vi\n"),
            "{text}"
        );
    }

    #[test]
    fn the_save_rules_are_lines_and_advice_between_the_resource_rules_and_not_found() {
        let findings = vec![
            found(
                "Broken.vi",
                Problem::TooNew {
                    saved: 2026,
                    minimum: 2025,
                },
            ),
            gone("Broken.vi", "Sub/Gone.vi"),
            found("NotSeparated.vi", Problem::NotSeparated),
        ];
        assert_eq!(
            listing(&checked(2, findings)),
            "Broken.vi: saved in LabVIEW 2026, newer than the minimum 2025\n\
             Broken.vi: link not found: Sub/Gone.vi\n\
             NotSeparated.vi: compiled code is not separated from the source\n\
             next: save the VI for LabVIEW 2025, or raise labview in lvpm.toml\n\
             next: turn on \"Separate compiled code from source file\" in the VI's properties\n\
             next: restore the file, or open the VI in LabVIEW and relink it\n\
             VIs checked: 2, problems: 3, links not checked: 0\n"
        );
    }

    #[test]
    fn a_manifest_with_no_labview_is_a_warning_between_the_not_checked_ones_and_no_vis() {
        let mut c = checked(0, vec![]);
        c.saves_checked = false;
        c.not_checked
            .insert(Unchecked::Root("<resource>".into()), 1);
        assert_eq!(
            warnings(&c, None),
            "warning: links not checked: 1 into <resource> (lvpm does not resolve this root)\n\
             warning: lvpm.toml has no labview, so save versions were not checked\n\
             warning: no VIs under the project\n"
        );
    }

    #[test]
    fn the_json_gives_both_years_of_a_too_new_vi_and_nothing_else_for_not_separated() {
        let c = checked(
            1,
            vec![
                found(
                    "a.vi",
                    Problem::TooNew {
                        saved: 2026,
                        minimum: 2025,
                    },
                ),
                found("b.vi", Problem::NotSeparated),
            ],
        );
        let findings = result(&c)["findings"].clone();
        assert_eq!(
            findings[0],
            json!({"vi": "a.vi", "kind": "too_new", "link": null, "package": null,
                   "table": null, "saved": 2026, "minimum": 2025, "block": null, "offset": null})
        );
        assert_eq!(
            findings[1],
            json!({"vi": "b.vi", "kind": "not_separated", "link": null, "package": null,
                   "table": null, "saved": null, "minimum": null, "block": null, "offset": null})
        );
    }

    #[test]
    fn links_not_checked_are_one_warning_per_cause_and_name_the_cause() {
        let mut c = checked(2, vec![]);
        c.not_checked.insert(
            Unchecked::NoInstallation {
                folder: "vi.lib",
                installation: PathBuf::from("LabVIEW 2026"),
            },
            3,
        );
        c.not_checked
            .insert(Unchecked::Root("<resource>".into()), 1);
        assert_eq!(
            warnings(&c, None),
            "warning: links not checked: 3 into vi.lib (the bound installation LabVIEW 2026 is \
             not on this machine)\n\
             warning: links not checked: 1 into <resource> (lvpm does not resolve this root)\n"
        );
        assert_eq!(
            listing(&c),
            "VIs checked: 2, problems: 0, links not checked: 4\n"
        );
    }

    #[test]
    fn no_vis_is_a_warning_naming_the_folder_or_the_project() {
        let none = checked(0, vec![]);
        assert_eq!(
            warnings(&none, Some(Path::new("New"))),
            "warning: no VIs under New\n"
        );
        assert_eq!(warnings(&none, None), "warning: no VIs under the project\n");
        assert_eq!(warnings(&checked(1, vec![]), None), "");
    }

    #[test]
    fn the_json_has_one_shape_with_null_for_what_does_not_apply() {
        let unreadable = Problem::Unreadable {
            block: "LIfp",
            offset: 77,
        };
        let c = checked(
            3,
            vec![
                gone("Broken.vi", "Sub/Gone.vi"),
                found("Good.vi", unreadable),
                found("x.vi", Problem::NotResource),
            ],
        );
        let nulls = |kind: &str, vi: &str| {
            json!({"vi": vi, "kind": kind, "link": null, "package": null, "table": null,
                   "saved": null, "minimum": null, "block": null, "offset": null})
        };
        let mut link = nulls("not_found", "Broken.vi");
        link["link"] = json!("Sub/Gone.vi");
        let mut table = nulls("unreadable", "Good.vi");
        table["block"] = json!("LIfp");
        table["offset"] = json!(77);
        assert_eq!(
            result(&c),
            json!({"ok": false, "vis": 3, "not_checked": 0,
                   "findings": [link, table, nulls("not_resource", "x.vi")]})
        );
        assert_eq!(
            result(&checked(1, vec![])),
            json!({"ok": true, "vis": 1, "not_checked": 0, "findings": []})
        );
    }
}
