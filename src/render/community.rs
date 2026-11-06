//! The wording of the community index commands: `lvpm index add`'s result and
//! its refusals.

use super::{Renderer, Stream};
use lvpm_core::events::Event;
use lvpm_core::ops::community::AddRefused;

pub(super) fn render(r: &mut Renderer, event: &Event) {
    match event {
        Event::IndexEntryAdded {
            name,
            version,
            file,
        } => r.line(Stream::Out, &format!("added {name} {version} to {file}")),
        _ => unreachable!("routed here by Renderer::render: {event:?}"),
    }
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

    #[test]
    fn an_error_that_is_not_a_refusal_is_passed_on() {
        let why = add_error(anyhow::anyhow!("disk full"));
        assert_eq!(why.to_string(), "disk full");
    }
}
