//! The wording of the linkfix events, as upstream's `lvpm linkfix` words them.

use super::{Renderer, Stream};
use lvpm_core::events::{Event, LinkfixReport};

pub(super) fn render(r: &mut Renderer, event: &Event) {
    match event {
        Event::LinkfixStarted {
            roots,
            methods,
            dry_run,
        } => {
            let names: Vec<&str> = methods.iter().map(|m| m.name()).collect();
            r.line(
                Stream::Out,
                &format!(
                    "{}fixing under {roots} root(s), methods: {}",
                    if *dry_run { "dry run: " } else { "" },
                    names.join(", ")
                ),
            );
        }
        Event::LinkfixPreparing => r.line(
            Stream::Out,
            "\nlinkfix: preparing installed files (repair links and separate compiled code; no LabVIEW needed)",
        ),
        Event::LinkfixPlanned { .. } => r.line(
            Stream::Out,
            "\nlinkfix: would prepare newly written LabVIEW-tree files (repair links and separate compiled code; no LabVIEW needed)",
        ),
        Event::LinkfixFailed { error } => r.line(
            Stream::Out,
            &format!(
                "  linkfix could not complete: {error}; package files remain installed. \
                 Retry with `lvpm linkfix --tree <DIR> --verbose`."
            ),
        ),
        Event::LinkfixReportWritten { path } => {
            r.line(Stream::Out, &format!("report: {}", path.display()))
        }
        Event::LinkfixDone { report } => done(r, report),
        _ => unreachable!("routed here by Renderer::render: {event:?}"),
    }
}

fn done(r: &mut Renderer, report: &LinkfixReport) {
    // Always shown, not only verbose: the file keeps its embedded compiled
    // code, and LabVIEW may still ask to save it.
    for (path, why) in &report.not_separated {
        r.line(
            Stream::Out,
            &format!("  NOT SEPARATED {}: {why}", path.display()),
        );
    }
    if report.verbose {
        for (path, c) in &report.changed_files {
            r.line(
                Stream::Out,
                &format!(
                    "  {:<60} owner {} rel {} abs {} qual {} vils {} sep {}",
                    path.display(),
                    c.owner,
                    c.relative,
                    c.absolute,
                    c.qualifier,
                    c.vils,
                    c.separated
                ),
            );
        }
        for (path, why) in &report.skipped_files {
            r.line(Stream::Out, &format!("  SKIPPED {}: {why}", path.display()));
        }
    }
    let skipped = report.skipped_files.len();
    let t = &report.totals;
    r.line(
        Stream::Out,
        &format!(
            "\n{} LabVIEW files looked at: {} {}, {} already clean, {skipped} skipped",
            report.looked_at,
            report.changed,
            if report.dry_run {
                "would change"
            } else {
                "changed"
            },
            report.clean
        ),
    );
    r.line(
        Stream::Out,
        &format!(
            "  edits: owner paths {}, relative paths {}, build-machine absolutes {}, qualifiers {}, \
             owner refs {}, compiled code separated {}",
            t.owner, t.relative, t.absolute, t.qualifier, t.vils, t.separated
        ),
    );
    if !report.not_separated.is_empty() {
        r.line(
            Stream::Out,
            &format!(
                "  compiled code left embedded in {} file(s), listed above with the reason; \
                 link repairs were still attempted",
                report.not_separated.len()
            ),
        );
    }
    if skipped > 0 && !report.verbose {
        r.line(Stream::Out, "  (--verbose names the skipped files and why)");
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::renderer;
    use super::*;
    use lvpm_core::linkfix::Changes;
    use std::path::PathBuf;

    #[test]
    fn a_quiet_report_gives_totals_names_unseparated_files_and_points_at_verbose() {
        let (mut renderer, out, _err) = renderer();
        renderer.render(&Event::LinkfixDone {
            report: LinkfixReport {
                dry_run: true,
                verbose: false,
                looked_at: 3,
                changed: 1,
                clean: 1,
                totals: Changes {
                    owner: 2,
                    ..Changes::default()
                },
                not_separated: vec![(PathBuf::from("old.vi"), "LabVIEW 2015".into())],
                changed_files: vec![(PathBuf::from("a.vi"), Changes::default())],
                skipped_files: vec![(PathBuf::from("bad.vi"), "truncated".into())],
            },
        });
        let out = out.text();
        assert!(out.contains("NOT SEPARATED old.vi: LabVIEW 2015"), "{out}");
        assert!(
            out.contains("3 LabVIEW files looked at: 1 would change, 1 already clean, 1 skipped"),
            "{out}"
        );
        assert!(out.contains("owner paths 2,"), "{out}");
        assert!(
            out.contains("(--verbose names the skipped files and why)"),
            "{out}"
        );
        assert!(!out.contains("a.vi"), "quiet names no changed file: {out}");
    }
}
