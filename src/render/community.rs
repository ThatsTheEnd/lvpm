//! The wording of the community index commands: `lvpm index add`'s result and
//! its refusals, and `lvpm index check`'s verdict lines and the comment the bot
//! leaves on a refused PR.

use super::lock::not_checked;
use super::{Renderer, Stream};
use lvpm_core::community::{Finding, MAX_ENTRY_FILES_PER_PR, MAX_NEW_VERSIONS_PER_PR, NotAttested};
use lvpm_core::events::Event;
use lvpm_core::ops::community::{AddRefused, Checked};
use lvpm_core::project::name_key;

/// The most characters of one value from a PR that a line shows.
const MAX_SHOWN_CHARS: usize = 200;

/// The most refusals the PR comment lists; the rest are counted.
const MAX_LISTED_REFUSALS: usize = 50;

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
        Event::IndexVersionAccepted { name, version } => r.line(
            Stream::Out,
            &format!("accepted {} {}", untrusted(name), untrusted(version)),
        ),
        Event::IndexRefused { finding } => r.line(Stream::Err, &refusal_line(finding)),
        Event::IndexNothingToCheck => r.line(Stream::Out, "nothing to check"),
        _ => unreachable!("routed here by Renderer::render: {event:?}"),
    }
}

/// `text` with every control character, the line end too, shown as `?`, and
/// every format character and line or paragraph separator (a bidi override, a
/// zero-width space, U+2028): a PR's names, paths and urls are its author's,
/// and one line must stay one line that reads as it is written.
fn plain(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() || hidden(c) { '?' } else { c })
        .collect()
}

/// Whether `c` is in the Unicode categories Cf, Zl or Zp.
///
/// shortcut: the list is Unicode 16's, written out; a character a later version
/// adds shows as itself until it is added here.
fn hidden(c: char) -> bool {
    matches!(c,
        '\u{AD}' | '\u{600}'..='\u{605}' | '\u{61C}' | '\u{6DD}' | '\u{70F}'
        | '\u{890}'..='\u{891}' | '\u{8E2}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}' | '\u{110BD}' | '\u{110CD}' | '\u{13430}'..='\u{1343F}'
        | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}' | '\u{E0001}'
        | '\u{E0020}'..='\u{E007F}')
}

/// A value out of a PR, as a line may show it: [`plain`], and cut at
/// [`MAX_SHOWN_CHARS`] characters with a `…`.
fn untrusted(text: &str) -> String {
    let shown = plain(text);
    match shown.char_indices().nth(MAX_SHOWN_CHARS) {
        Some((cut, _)) => format!("{}…", &shown[..cut]),
        None => shown,
    }
}

/// A refusal of `lvpm index check` as the line the user reads. Every value out
/// of the PR is [`untrusted`].
pub fn refusal_line(finding: &Finding) -> String {
    let u = untrusted;
    let (what, fix) = match finding {
        Finding::OutsideEntries { path } => (
            format!(
                "{}: only .toml files directly under packages/ may change",
                u(path)
            ),
            "remove this change".to_string(),
        ),
        Finding::WrongFileName { path, name } => (
            format!(
                "{}: the entry for {} must be packages/{}.toml",
                u(path),
                u(name),
                u(&name_key(name))
            ),
            "rename the file".to_string(),
        ),
        Finding::NameNotInstallable { path, name } => (
            format!("{}: {} is not a name lvpm can install", u(path), u(name)),
            "register another name".to_string(),
        ),
        Finding::TooManyVersions { versions } => (
            format!("this PR: it adds {versions} versions, more than {MAX_NEW_VERSIONS_PER_PR}"),
            format!("register at most {MAX_NEW_VERSIONS_PER_PR} versions per PR"),
        ),
        Finding::TooManyFiles { files } => (
            format!("this PR: it changes {files} entry files, more than {MAX_ENTRY_FILES_PER_PR}"),
            format!("change at most {MAX_ENTRY_FILES_PER_PR} entry files per PR"),
        ),
        Finding::OwnerChanged { name, old, new, .. } => (
            format!(
                "{0}: github changed from {1} to {2}, and {0} belongs to {1}",
                u(name),
                u(old),
                u(new)
            ),
            format!("publish from {}, or register another name", u(old)),
        ),
        Finding::VersionChanged {
            name,
            version,
            field,
            old,
            new,
            ..
        } => (
            format!(
                "{} {}: already listed with {field} {}, now {}",
                u(name),
                u(version),
                u(old),
                u(new)
            ),
            "listed versions cannot change; publish a new version".to_string(),
        ),
        Finding::VersionRemoved { name, version, .. } => (
            format!(
                "{} {}: listed versions cannot be removed",
                u(name),
                u(version)
            ),
            "restore it".to_string(),
        ),
        Finding::UrlNotReleaseFile {
            name,
            version,
            url,
            github,
            ..
        } => (
            format!(
                "{} {}: {} is not a release file of {}",
                u(name),
                u(version),
                u(url),
                u(github)
            ),
            format!("upload the package to a release of {}", u(github)),
        ),
        Finding::Unreadable { path, error } => (
            format!("{}: does not parse: {}", u(path), u(error)),
            "register with the lvpm release template instead of editing by hand".to_string(),
        ),
        Finding::FileDiffers {
            name,
            version,
            actual,
            entry,
            ..
        } => (
            format!(
                "{} {}: the file has sha256 {}, the entry says {}",
                u(name),
                u(version),
                u(actual),
                u(entry)
            ),
            REPLACED_FILE_FIX.to_string(),
        ),
        Finding::NotDownloaded {
            name,
            version,
            error,
            ..
        } => (
            format!(
                "{} {}: the file could not be downloaded: {}",
                u(name),
                u(version),
                u(error)
            ),
            "check the release is published and public, then push a new tag".to_string(),
        ),
        Finding::NotAPackage {
            name,
            version,
            error,
            ..
        } => (
            format!(
                "{} {}: the file is not a package lvpm reads: {}",
                u(name),
                u(version),
                u(error)
            ),
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
            format!(
                "{} {}: the package says {field} {}, the entry says {}",
                u(name),
                u(version),
                u(actual),
                u(entry)
            ),
            REPLACED_FILE_FIX.to_string(),
        ),
        Finding::NotAttested {
            name,
            version,
            reason,
            ..
        } => (
            format!(
                "{} {}: not attested ({})",
                u(name),
                u(version),
                not_attested(reason)
            ),
            "build with the lvpm release template, which attests the package".to_string(),
        ),
        Finding::AttestedByOther {
            name,
            version,
            subject,
            github,
            ..
        } => (
            format!(
                "{} {}: attested by {}, not https://github.com/{}",
                u(name),
                u(version),
                u(subject),
                u(github)
            ),
            format!("publish from {}", u(github)),
        ),
    };
    plain(&format!("refused {what}. fix: {fix}"))
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
/// refusal lines, and what the publisher does next. The lines come from the PR,
/// so they sit in one fenced code block whose fence is longer than any run of
/// backticks in them, and no more than [`MAX_LISTED_REFUSALS`] are listed.
pub fn pr_comment(refused: &[Finding]) -> String {
    let mut lines: Vec<String> = refused
        .iter()
        .take(MAX_LISTED_REFUSALS)
        .map(refusal_line)
        .collect();
    if refused.len() > MAX_LISTED_REFUSALS {
        lines.push(format!(
            "… and {} more",
            refused.len() - MAX_LISTED_REFUSALS
        ));
    }
    let longest_run = (lines.iter())
        .flat_map(|line| line.split(|c| c != '`').map(str::len))
        .max()
        .unwrap_or(0);
    let fence = "`".repeat((longest_run + 1).max(3));
    let lines = lines.join("\n");
    format!(
        "lvpm index check refused this PR:\n\n{fence}\n{lines}\n{fence}\n\nHow to publish to the \
         community index: {PUBLISH_DOCS}\n\nFix the cause, push a new tag, and close this PR."
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
                        error: "HTTP 404".into(),
                    }
                },
                "refused foo 1.1.0: the file could not be downloaded: HTTP 404. fix: check the release is published and public, then push a new tag".into(),
            ),
            (
                {
                    let (path, name, version) = at();
                    Finding::NotDownloaded {
                        path,
                        name,
                        version,
                        error: "too large".into(),
                    }
                },
                "refused foo 1.1.0: the file could not be downloaded: too large. fix: check the release is published and public, then push a new tag".into(),
            ),
            (
                Finding::VersionChanged {
                    path: path.into(),
                    name: name.into(),
                    version: "1.0.0".into(),
                    field: "version",
                    old: "1.0.0".into(),
                    new: "1.0.0.0".into(),
                },
                "refused foo 1.0.0: already listed with version 1.0.0, now 1.0.0.0. fix: listed versions cannot change; publish a new version".into(),
            ),
            (
                Finding::NameNotInstallable {
                    path: "packages/con.toml".into(),
                    name: "con".into(),
                },
                "refused packages/con.toml: con is not a name lvpm can install. fix: register another name".into(),
            ),
            (
                Finding::TooManyVersions { versions: 21 },
                "refused this PR: it adds 21 versions, more than 20. fix: register at most 20 versions per PR".into(),
            ),
            (
                Finding::TooManyFiles { files: 21 },
                "refused this PR: it changes 21 entry files, more than 20. fix: change at most 20 entry files per PR".into(),
            ),
            (
                Finding::Unreadable {
                    path: "packages/bar.toml".into(),
                    error: "lists no version".into(),
                },
                "refused packages/bar.toml: does not parse: lists no version. fix: register with the lvpm release template instead of editing by hand".into(),
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

    const COMMENT_END: &str = "\n\nHow to publish to the community index: https://github.com/ThatsTheEnd/lvpm/blob/main/docs/usage.md#publish-to-the-community-index\n\nFix the cause, push a new tag, and close this PR.";

    fn removed(name: &str) -> Finding {
        Finding::VersionRemoved {
            path: "packages/foo.toml".into(),
            name: name.into(),
            version: "1.0".into(),
        }
    }

    #[test]
    fn the_pr_comment_holds_the_lines_in_one_fenced_block_and_ends_with_what_to_do() {
        let refused: Vec<Finding> = pinned_check_lines().into_iter().map(|(f, _)| f).collect();

        let comment = pr_comment(&refused[..2]);

        assert_eq!(
            comment,
            format!(
                "lvpm index check refused this PR:\n\n```\n{}\n{}\n```{COMMENT_END}",
                "refused notes.md: only .toml files directly under packages/ may change. fix: remove this change",
                "refused packages/Foo2.toml: the entry for Foo2 must be packages/foo2.toml. fix: rename the file",
            )
        );
    }

    #[test]
    fn the_fence_is_longer_than_any_run_of_backticks_in_the_lines() {
        let rows = [("a``b", "```"), ("a```b", "````"), ("`x`````y``", "``````")];
        for (name, fence) in rows {
            let comment = pr_comment(&[removed(name)]);

            let line = refusal_line(&removed(name));
            assert_eq!(
                comment,
                format!(
                    "lvpm index check refused this PR:\n\n{fence}\n{line}\n{fence}{COMMENT_END}"
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn no_more_than_fifty_refusals_are_listed_and_the_rest_are_counted() {
        let many = |count: usize| -> Vec<Finding> {
            (0..count).map(|i| removed(&format!("p{i}"))).collect()
        };
        let listed = |comment: &str| {
            comment
                .lines()
                .filter(|l| l.starts_with("refused "))
                .count()
        };

        let fifty = pr_comment(&many(50));
        assert_eq!(listed(&fifty), 50);
        assert!(!fifty.contains("more\n"), "{fifty}");

        let fifty_one = pr_comment(&many(51));
        assert_eq!(listed(&fifty_one), 50);
        assert!(
            fifty_one.contains(
                "p49 1.0: listed versions cannot be removed. fix: restore it\n… and 1 more\n```\n"
            ),
            "{fifty_one}"
        );
        assert!(!fifty_one.contains("p50"), "{fifty_one}");

        assert!(pr_comment(&many(120)).contains("\n… and 70 more\n```\n"));
    }

    #[test]
    fn a_value_from_the_pr_is_cut_at_200_characters() {
        let long = "x".repeat(300);
        let cut = format!("{}…", "x".repeat(200));
        let exactly = "y".repeat(200);

        assert_eq!(
            refusal_line(&removed(&long)),
            format!("refused {cut} 1.0: listed versions cannot be removed. fix: restore it")
        );
        assert_eq!(
            refusal_line(&removed(&exactly)),
            format!("refused {exactly} 1.0: listed versions cannot be removed. fix: restore it")
        );
        let (mut renderer, out, _) = renderer();
        renderer.render(&Event::IndexVersionAccepted {
            name: long,
            version: "1".into(),
        });
        assert_eq!(out.text(), format!("accepted {cut} 1\n"));
        // Characters, not bytes: a cut never lands inside one.
        let wide = "é".repeat(300);
        assert_eq!(
            refusal_line(&removed(&wide)),
            format!(
                "refused {}… 1.0: listed versions cannot be removed. fix: restore it",
                "é".repeat(200)
            )
        );
    }

    #[test]
    fn invisible_and_reordering_characters_show_as_question_marks() {
        let name = "a\u{202e}b\u{2028}c\u{2029}d\u{200b}e\u{feff}f\u{2066}g\u{ad}h\u{e0041}";
        assert_eq!(
            refusal_line(&removed(name)),
            "refused a?b?c?d?e?f?g?h? 1.0: listed versions cannot be removed. fix: restore it"
        );
    }

    #[test]
    fn a_pr_comment_has_no_line_a_name_can_add() {
        let comment = pr_comment(&[removed("foo\n```\nHow to publish: https://evil.example\n")]);

        let lines: Vec<_> = comment.lines().collect();
        assert_eq!(
            lines[3],
            "refused foo?```?How to publish: https://evil.example? 1.0: listed versions cannot be removed. fix: restore it"
        );
        assert_eq!(lines[2], "````");
        assert_eq!(lines[4], "````");
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
