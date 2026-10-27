//! The findings table: one line per variant, and where lvpm's build differs
//! from the published archive. Written under `target/`, never committed.

use crate::variants::RefDiff;

/// What the table's header names.
pub struct Header {
    pub vipm: String,
    pub labview: String,
}

/// One variant's outcome.
pub struct Row {
    pub variant: String,
    pub accepted: bool,
    pub exit: Option<i32>,
    /// The first line of VIPM's output, or why the probe rejected it.
    pub note: String,
}

/// The first line of `output` that says anything.
pub fn first_line(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// A cell on one line: a pipe would end it, a newline would end its row.
pub fn escape(cell: &str) -> String {
    cell.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace("\r\n", "\\n")
        .replace(['\n', '\r'], "\\n")
}

fn list(title: &str, items: &[String]) -> String {
    let mut out = format!("\n### {title}\n\n");
    match items.is_empty() {
        true => out.push_str("none\n"),
        false => items
            .iter()
            .for_each(|item| out.push_str(&format!("- {}\n", escape(item)))),
    }
    out
}

pub fn render(header: &Header, rows: &[Row], diff: &RefDiff, not_varied: &[String]) -> String {
    let mut out = format!(
        "# VIPM probe\n\nVIPM: {}\nLabVIEW: {}\n\n\
         | variant | verdict | exit | first line of VIPM's output |\n|---|---|---|---|\n",
        escape(&header.vipm),
        escape(&header.labview)
    );
    for row in rows {
        let verdict = if row.accepted { "accepted" } else { "rejected" };
        let exit = row.exit.map_or("-".to_string(), |code| code.to_string());
        out.push_str(&format!(
            "| {} | {verdict} | {exit} | {} |\n",
            escape(&row.variant),
            escape(&row.note)
        ));
    }
    out.push_str("\n## lvpm's build of pilot A against the published archive\n");
    out.push_str(&list("Entries only in lvpm's build", &diff.only_ours));
    out.push_str(&list(
        "Entries only in the published archive",
        &diff.only_reference,
    ));
    out.push_str(&list(
        "Spec keys only in lvpm's build",
        &diff.keys_only_ours,
    ));
    out.push_str(&list(
        "Spec keys only in the published archive",
        &diff.keys_only_reference,
    ));
    let changed: Vec<String> = diff
        .keys_changed
        .iter()
        .map(|(key, ours, theirs)| format!("{key}: lvpm {ours}, published {theirs}"))
        .collect();
    out.push_str(&list("Spec keys whose value differs", &changed));
    out.push_str(&list(
        "Keys in every published spec that lvpm does not write and no variant sets",
        not_varied,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Header {
        Header {
            vipm: "vipm 1.2.3".into(),
            labview: "26.3.0".into(),
        }
    }

    fn row(variant: &str, accepted: bool, exit: Option<i32>, note: &str) -> Row {
        Row {
            variant: variant.into(),
            accepted,
            exit,
            note: note.into(),
        }
    }

    fn lines_of_rows(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|l| l.starts_with("| "))
            .skip(1) // the header line
            .collect()
    }

    #[test]
    fn table_has_one_line_per_variant() {
        let rows = [
            row("base", true, Some(0), "ok"),
            row("ID=empty", false, Some(3), "no"),
            row("icon.bmp=present", false, None, "timed out after 600s"),
        ];
        let text = render(&header(), &rows, &RefDiff::default(), &[]);
        let lines = lines_of_rows(&text);
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(lines[0], "| base | accepted | 0 | ok |");
        assert_eq!(lines[1], "| ID=empty | rejected | 3 | no |");
        assert_eq!(
            lines[2],
            "| icon.bmp=present | rejected | - | timed out after 600s |"
        );
        assert!(text.contains("VIPM: vipm 1.2.3") && text.contains("LabVIEW: 26.3.0"));
    }

    #[test]
    fn table_escapes_pipes_and_newlines() {
        let rows = [row("a|b", false, Some(1), "x | y\nz\r\nw")];
        let text = render(&header(), &rows, &RefDiff::default(), &[]);
        assert_eq!(
            lines_of_rows(&text),
            ["| a\\|b | rejected | 1 | x \\| y\\nz\\nw |"]
        );
    }

    #[test]
    fn the_first_line_is_the_first_one_with_text() {
        assert_eq!(first_line("\n  \n  boom here \nsecond"), "boom here");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn the_reference_diff_is_in_the_table() {
        let diff = RefDiff {
            only_ours: vec!["a.txt".into()],
            only_reference: vec!["icon.bmp".into()],
            keys_only_ours: vec!["[Package]/X".into()],
            keys_only_reference: vec!["[Package]/ID".into()],
            keys_changed: vec![("[Package]/Version".into(), "1".into(), "2".into())],
        };
        let text = render(&header(), &[], &diff, &["[LabVIEW]/y".into()]);
        for want in [
            "- a.txt",
            "- icon.bmp",
            "- [Package]/X",
            "- [Package]/ID",
            "- [Package]/Version: lvpm 1, published 2",
            "- [LabVIEW]/y",
        ] {
            assert!(text.contains(want), "{want} missing:\n{text}");
        }
    }
}
