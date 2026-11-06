//! The wording of the community index commands: `lvpm index add`'s result and
//! its refusals, and `lvpm index check`'s verdict lines and the comment the bot
//! leaves on a refused PR.

use super::lock::not_checked;
use super::{Renderer, Stream};
use lvpm_core::community::{Finding, NotAttested};
use lvpm_core::events::Event;
use lvpm_core::ops::community::{AddRefused, Checked};
use lvpm_core::project::name_key;

/// Where a publisher reads how to register a package.
const PUBLISH_DOCS: &str =
    "https://github.com/ThatsTheEnd/lvpm/blob/main/docs/usage.md#publish-to-the-community-index";

pub(super) fn render(r: &mut Renderer, event: &Event) {
    match event {
        Event::IndexEntryAdded {
            name,
            version,
            file,
        } => r.line(Stream::Out, &format!("added {name} {version} to {file}")),
        Event::IndexVersionAccepted { name, version } => {
            r.line(Stream::Out, &plain(format!("accepted {name} {version}")))
        }
        Event::IndexRefused { finding } => r.line(Stream::Err, &refusal_line(finding)),
        Event::IndexNothingToCheck => r.line(Stream::Out, "nothing to check"),
        _ => unreachable!("routed here by Renderer::render: {event:?}"),
    }
}

/// `text` with every control character, the line end too, shown as `?`: a PR's
/// names, paths and urls are its author's, and one line must stay one line.
fn plain(text: String) -> String {
    text.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// A refusal of `lvpm index check` as the line the user reads.
pub fn refusal_line(finding: &Finding) -> String {
    let (what, fix) = match finding {
        Finding::OutsideEntries { path } => (
            format!("{path}: only .toml files directly under packages/ may change"),
            "remove this change".to_string(),
        ),
        Finding::WrongFileName { path, name } => (
            format!(
                "{path}: the entry for {name} must be packages/{}.toml",
                name_key(name)
            ),
            "rename the file".to_string(),
        ),
        Finding::OwnerChanged { name, old, new, .. } => (
            format!("{name}: github changed from {old} to {new}, and {name} belongs to {old}"),
            format!("publish from {old}, or register another name"),
        ),
        Finding::VersionChanged {
            name,
            version,
            field,
            old,
            new,
            ..
        } => (
            format!("{name} {version}: already listed with {field} {old}, now {new}"),
            "listed versions cannot change; publish a new version".to_string(),
        ),
        Finding::VersionRemoved { name, version, .. } => (
            format!("{name} {version}: listed versions cannot be removed"),
            "restore it".to_string(),
        ),
        Finding::UrlNotReleaseFile {
            name,
            version,
            url,
            github,
            ..
        } => (
            format!("{name} {version}: {url} is not a release file of {github}"),
            format!("upload the package to a release of {github}"),
        ),
        Finding::Unreadable { path, error } => (
            format!("{path}: does not parse: {error}"),
            "register with the lvpm release template instead of editing by hand".to_string(),
        ),
        Finding::FileDiffers {
            name,
            version,
            actual,
            entry,
            ..
        } => (
            format!("{name} {version}: the file has sha256 {actual}, the entry says {entry}"),
            REPLACED_FILE_FIX.to_string(),
        ),
        Finding::NotDownloaded {
            name,
            version,
            error,
            ..
        } => (
            format!("{name} {version}: the file could not be downloaded: {error}"),
            "check the release is published and public, then push a new tag".to_string(),
        ),
        Finding::NotAPackage {
            name,
            version,
            error,
            ..
        } => (
            format!("{name} {version}: the file is not a package lvpm reads: {error}"),
            "build the package with the lvpm release template, then push a new tag".to_string(),
        ),
        Finding::PackageDiffers {
            name,
            version,
            field,
            actual,
            entry,
            ..
        } => (
            format!("{name} {version}: the package says {field} {actual}, the entry says {entry}"),
            REPLACED_FILE_FIX.to_string(),
        ),
        Finding::NotAttested {
            name,
            version,
            reason,
            ..
        } => (
            format!("{name} {version}: not attested ({})", not_attested(reason)),
            "build with the lvpm release template, which attests the package".to_string(),
        ),
        Finding::AttestedByOther {
            name,
            version,
            subject,
            github,
            ..
        } => (
            format!("{name} {version}: attested by {subject}, not https://github.com/{github}"),
            format!("publish from {github}"),
        ),
    };
    plain(format!("refused {what}. fix: {fix}"))
}

/// What the file was replaced after, the fix for a file that is not the one the
/// entry describes.
const REPLACED_FILE_FIX: &str = "don't replace release files after tagging; push a new tag";

/// Why no attestation was found, in S6's words.
fn not_attested(reason: &NotAttested) -> String {
    match reason {
        NotAttested::Missing => "no attestation".to_string(),
        NotAttested::Unchecked(reason) => not_checked(reason, None).0,
        NotAttested::DoesNotVerify => "does not verify".to_string(),
    }
}

/// The comment the bot leaves on a PR that `lvpm index check` refused: the
/// refusal lines, and what the publisher does next.
pub fn pr_comment(refused: &[Finding]) -> String {
    let lines: String = refused
        .iter()
        .map(|finding| format!("- {}\n", refusal_line(finding)))
        .collect();
    format!(
        "lvpm index check refused this PR:\n\n{lines}\nHow to publish to the community index: \
         {PUBLISH_DOCS}\n\nFix the cause, push a new tag, and close this PR."
    )
}

/// The `--json` result of `lvpm index check`: the comment for a refused PR.
pub fn check_result(checked: &Checked) -> serde_json::Value {
    let comment = (!checked.refused.is_empty()).then(|| pr_comment(&checked.refused));
    serde_json::json!({
        "command": "index-check",
        "accepted": checked.accepted,
        "refused": checked.refused.len(),
        "comment": comment,
    })
}

/// A refusal of `lvpm index add` as the line the user reads; any other error
/// is passed on as it is.
pub fn add_error(why: anyhow::Error) -> anyhow::Error {
    match why.downcast::<AddRefused>() {
        Ok(refused) => anyhow::anyhow!(add_refusal(&refused)),
        Err(other) => other,
    }
}

fn add_refusal(why: &AddRefused) -> String {
    let (what, fix) = match why {
        AddRefused::NotOwnerRepo => (
            "--github must be owner/repo".to_string(),
            "pass the repo that publishes the package, like someone/Serializer".to_string(),
        ),
        AddRefused::NotAPackage { file, error } => (
            format!("{file} is not an .lvpkg: {error}"),
            "pass the package lvpm build made".to_string(),
        ),
        AddRefused::UrlNotInReleases {
            name,
            version,
            web,
            github,
        } => (
            format!(
                "{name} {version}: --url must be {web}/{github}/releases/download/<tag>/<file>"
            ),
            "pass the release asset's download URL".to_string(),
        ),
        AddRefused::NotTheOwner {
            name,
            version,
            owner,
            github,
            entry_file,
        } => (
            format!("{name} {version} belongs to {owner} in {entry_file}, not {github}"),
            format!("publish from {owner}, or register another name"),
        ),
        AddRefused::AlreadyListed {
            name,
            version,
            entry_file,
        } => (
            format!("{name} {version} is already listed in {entry_file}"),
            "listed versions cannot change; publish a new version".to_string(),
        ),
        AddRefused::EntryFileUnreadable { entry_file, error } => (
            format!("{entry_file} does not parse: {error}"),
            "restore it from the index repo".to_string(),
        ),
        AddRefused::EntryFileTooBig {
            name,
            version,
            entry_file,
            limit,
        } => (
            format!("{name} {version}: {entry_file} would exceed {limit} bytes"),
            "ask the index maintainers to split or prune it".to_string(),
        ),
        AddRefused::NotAFolder { dir, error } => (
            format!("{dir} is not a folder: {error}"),
            "pass the index repo's packages folder".to_string(),
        ),
    };
    format!("lvpm index: {what}. fix: {fix}")
}

#[cfg(test)]
mod tests {
    use super::super::tests::renderer;
    use super::*;
    use lvpm_core::verify::NotChecked;

    fn said(why: AddRefused) -> String {
        add_error(why.into()).to_string()
    }

    #[test]
    fn an_added_version_is_one_line() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::IndexEntryAdded {
            name: "foo".into(),
            version: "1.0".into(),
            file: "packages/foo.toml".into(),
        });
        assert_eq!(out.text(), "added foo 1.0 to packages/foo.toml\n");
        assert_eq!(err.text(), "");
    }

    #[test]
    fn each_refusal_names_what_and_says_how_to_fix_it() {
        let entry_file = || "packages/foo.toml".to_string();
        let rows = [
            (
                AddRefused::NotOwnerRepo,
                "lvpm index: --github must be owner/repo. fix: pass the repo that publishes the package, like someone/Serializer",
            ),
            (
                AddRefused::NotAPackage {
                    file: "notes.txt".into(),
                    error: "invalid Zip archive".into(),
                },
                "lvpm index: notes.txt is not an .lvpkg: invalid Zip archive. fix: pass the package lvpm build made",
            ),
            (
                AddRefused::UrlNotInReleases {
                    name: "foo".into(),
                    version: "1.0".into(),
                    web: "https://github.com".into(),
                    github: "a/x".into(),
                },
                "lvpm index: foo 1.0: --url must be https://github.com/a/x/releases/download/<tag>/<file>. fix: pass the release asset's download URL",
            ),
            (
                AddRefused::NotTheOwner {
                    name: "foo".into(),
                    version: "1.0".into(),
                    owner: "a/x".into(),
                    github: "b/y".into(),
                    entry_file: entry_file(),
                },
                "lvpm index: foo 1.0 belongs to a/x in packages/foo.toml, not b/y. fix: publish from a/x, or register another name",
            ),
            (
                AddRefused::AlreadyListed {
                    name: "foo".into(),
                    version: "1.0".into(),
                    entry_file: entry_file(),
                },
                "lvpm index: foo 1.0 is already listed in packages/foo.toml. fix: listed versions cannot change; publish a new version",
            ),
            (
                AddRefused::EntryFileUnreadable {
                    entry_file: entry_file(),
                    error: "not valid TOML at line 2".into(),
                },
                "lvpm index: packages/foo.toml does not parse: not valid TOML at line 2. fix: restore it from the index repo",
            ),
            (
                AddRefused::EntryFileTooBig {
                    name: "foo".into(),
                    version: "1.0".into(),
                    entry_file: entry_file(),
                    limit: 1048576,
                },
                "lvpm index: foo 1.0: packages/foo.toml would exceed 1048576 bytes. fix: ask the index maintainers to split or prune it",
            ),
            (
                AddRefused::NotAFolder {
                    dir: "packages".into(),
                    error: "The system cannot find the path specified. (os error 3)".into(),
                },
                "lvpm index: packages is not a folder: The system cannot find the path specified. (os error 3). fix: pass the index repo's packages folder",
            ),
        ];
        for (why, line) in rows {
            assert_eq!(said(why), line);
        }
    }

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const URL: &str = "https://example.com/a/x/releases/download/1.1.0/foo.lvpkg";

    /// One finding of each rule, with the line the plan pins for it.
    fn pinned_check_lines() -> Vec<(Finding, String)> {
        let (path, name, version) = ("packages/foo.toml", "foo", "1.1.0");
        let at = || (path.to_string(), name.to_string(), version.to_string());
        let fix_replaced = "don't replace release files after tagging; push a new tag";
        let not_attested = |reason| {
            let (path, name, version) = at();
            Finding::NotAttested {
                path,
                name,
                version,
                reason,
            }
        };
        vec![
            (
                Finding::OutsideEntries {
                    path: "notes.md".into(),
                },
                "refused notes.md: only .toml files directly under packages/ may change. fix: remove this change".into(),
            ),
            (
                Finding::WrongFileName {
                    path: "packages/Foo2.toml".into(),
                    name: "Foo2".into(),
                },
                "refused packages/Foo2.toml: the entry for Foo2 must be packages/foo2.toml. fix: rename the file".into(),
            ),
            (
                Finding::OwnerChanged {
                    path: path.into(),
                    name: name.into(),
                    old: "a/x".into(),
                    new: "b/x".into(),
                },
                "refused foo: github changed from a/x to b/x, and foo belongs to a/x. fix: publish from a/x, or register another name".into(),
            ),
            (
                Finding::VersionChanged {
                    path: path.into(),
                    name: name.into(),
                    version: "1.0.0".into(),
                    field: "sha256",
                    old: SHA_A.into(),
                    new: SHA_B.into(),
                },
                format!("refused foo 1.0.0: already listed with sha256 {SHA_A}, now {SHA_B}. fix: listed versions cannot change; publish a new version"),
            ),
            (
                Finding::VersionRemoved {
                    path: path.into(),
                    name: name.into(),
                    version: "1.0.0".into(),
                },
                "refused foo 1.0.0: listed versions cannot be removed. fix: restore it".into(),
            ),
            (
                Finding::UrlNotReleaseFile {
                    path: path.into(),
                    name: name.into(),
                    version: version.into(),
                    url: URL.into(),
                    github: "a/x".into(),
                },
                format!("refused foo 1.1.0: {URL} is not a release file of a/x. fix: upload the package to a release of a/x"),
            ),
            (
                {
                    let (path, name, version) = at();
                    Finding::FileDiffers {
                        path,
                        name,
                        version,
                        actual: SHA_B.into(),
                        entry: SHA_A.into(),
                    }
                },
                format!("refused foo 1.1.0: the file has sha256 {SHA_B}, the entry says {SHA_A}. fix: {fix_replaced}"),
            ),
            (
                {
                    let (path, name, version) = at();
                    Finding::NotDownloaded {
                        path,
                        name,
                        version,
                        error: "HTTP status client error (404 Not Found)".into(),
                    }
                },
                "refused foo 1.1.0: the file could not be downloaded: HTTP status client error (404 Not Found). fix: check the release is published and public, then push a new tag".into(),
            ),
            (
                {
                    let (path, name, version) = at();
                    Finding::PackageDiffers {
                        path,
                        name,
                        version,
                        field: "requires",
                        actual: "bar >=3".into(),
                        entry: "none".into(),
                    }
                },
                format!("refused foo 1.1.0: the package says requires bar >=3, the entry says none. fix: {fix_replaced}"),
            ),
            (
                not_attested(NotAttested::Missing),
                "refused foo 1.1.0: not attested (no attestation). fix: build with the lvpm release template, which attests the package".into(),
            ),
            (
                not_attested(NotAttested::Unchecked(NotChecked::GhMissing)),
                "refused foo 1.1.0: not attested (gh is not installed). fix: build with the lvpm release template, which attests the package".into(),
            ),
            (
                not_attested(NotAttested::DoesNotVerify),
                "refused foo 1.1.0: not attested (does not verify). fix: build with the lvpm release template, which attests the package".into(),
            ),
            (
                {
                    let (path, name, version) = at();
                    Finding::AttestedByOther {
                        path,
                        name,
                        version,
                        subject: "https://github.com/b/x".into(),
                        github: "a/x".into(),
                    }
                },
                "refused foo 1.1.0: attested by https://github.com/b/x, not https://github.com/a/x. fix: publish from a/x".into(),
            ),
            (
                Finding::Unreadable {
                    path: "packages/bar.toml".into(),
                    error: "not valid TOML at line 1".into(),
                },
                "refused packages/bar.toml: does not parse: not valid TOML at line 1. fix: register with the lvpm release template instead of editing by hand".into(),
            ),
        ]
    }

    #[test]
    fn every_finding_renders_as_its_pinned_line_on_stderr() {
        for (finding, line) in pinned_check_lines() {
            let (mut renderer, out, err) = renderer();

            renderer.render(&Event::IndexRefused {
                finding: finding.clone(),
            });

            assert_eq!(err.text(), format!("{line}\n"), "{finding:?}");
            assert_eq!(out.text(), "");
        }
    }

    #[test]
    fn a_file_that_is_no_package_is_refused_with_a_fix() {
        let finding = Finding::NotAPackage {
            path: "packages/foo.toml".into(),
            name: "foo".into(),
            version: "1.1.0".into(),
            error: "invalid Zip archive".into(),
        };
        assert_eq!(
            refusal_line(&finding),
            "refused foo 1.1.0: the file is not a package lvpm reads: invalid Zip archive. fix: build the package with the lvpm release template, then push a new tag"
        );
    }

    #[test]
    fn an_accepted_version_and_nothing_to_check_are_lines_on_stdout() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::IndexVersionAccepted {
            name: "foo".into(),
            version: "1.1.0".into(),
        });
        renderer.render(&Event::IndexNothingToCheck);
        assert_eq!(out.text(), "accepted foo 1.1.0\nnothing to check\n");
        assert_eq!(err.text(), "");
    }

    #[test]
    fn a_name_from_the_pr_cannot_break_a_line_or_drive_the_terminal() {
        let finding = Finding::VersionRemoved {
            path: "packages/foo.toml".into(),
            name: "foo\naccepted bar 1.0\u{1b}[31m".into(),
            version: "1.0".into(),
        };
        assert_eq!(
            refusal_line(&finding),
            "refused foo?accepted bar 1.0?[31m 1.0: listed versions cannot be removed. fix: restore it"
        );
        let (mut renderer, out, _) = renderer();
        renderer.render(&Event::IndexVersionAccepted {
            name: "a\nb".into(),
            version: "1".into(),
        });
        assert_eq!(out.text(), "accepted a?b 1\n");
    }

    #[test]
    fn the_pr_comment_leads_with_the_lines_and_ends_with_what_to_do() {
        let refused: Vec<Finding> = pinned_check_lines().into_iter().map(|(f, _)| f).collect();

        let comment = pr_comment(&refused[..2]);

        assert!(
            comment.starts_with("lvpm index check refused this PR:\n\n"),
            "{comment}"
        );
        for finding in &refused[..2] {
            let line = format!("- {}\n", refusal_line(finding));
            assert!(comment.contains(&line), "{comment}");
        }
        assert!(
            comment.contains("docs/usage.md#publish-to-the-community-index"),
            "{comment}"
        );
        assert!(
            comment.ends_with("Fix the cause, push a new tag, and close this PR."),
            "{comment}"
        );
    }

    #[test]
    fn the_json_result_carries_the_comment_only_for_a_refused_pr() {
        let finding = pinned_check_lines().remove(0).0;
        let refused = Checked {
            accepted: 1,
            refused: vec![finding.clone()],
        };
        let json = check_result(&refused);
        assert_eq!(json["command"], "index-check");
        assert_eq!(json["accepted"], 1);
        assert_eq!(json["refused"], 1);
        assert_eq!(json["comment"], pr_comment(&[finding]));

        let accepted = Checked {
            accepted: 2,
            refused: Vec::new(),
        };
        assert_eq!(check_result(&accepted)["comment"], serde_json::Value::Null);
    }

    #[test]
    fn an_error_that_is_not_a_refusal_is_passed_on() {
        let why = add_error(anyhow::anyhow!("disk full"));
        assert_eq!(why.to_string(), "disk full");
    }
}
