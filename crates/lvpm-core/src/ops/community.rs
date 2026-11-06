//! `lvpm index add`: list a package's file in a community index's entry file.
//! `lvpm index check`: the bot's verdict on a PR to that index.

use super::build::write_atomically;
use super::env::OpEnv;
use super::session::Downloads;
use crate::cache::sha256_hex;
use crate::community::{
    self, EntryFile, FileVersion, Finding, NewVersion, NotAttested, Tree, VersionListed,
};
use crate::errors::{Cancelled, IndexHashMismatch};
use crate::events::Event;
use crate::fetch::{MAX_INDEX_DOWNLOAD_BYTES, check_index_url};
use crate::github::MAX_ENTRY_BYTES;
use crate::index::{self, Entry};
use crate::project::{name_key, split_owner_repo};
use crate::verify::{Accepted, Policy, Refusal, decide};
use crate::version::Version;
use anyhow::{Context, Result, ensure};
use std::io::Read;
use std::path::Path;

/// Where the repo of an attested package is, as the attestation names it.
const ATTESTATION_SITE: &str = "https://github.com";

/// Why `lvpm index add` wrote nothing. The wording is the front end's; the
/// `Display` here is for a log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddRefused {
    #[error("--github is not owner/repo")]
    NotOwnerRepo,
    #[error("{file} is not a package: {error}")]
    NotAPackage { file: String, error: String },
    #[error("{name} {version}: --url is not a release file of {github} on {web}")]
    UrlNotInReleases {
        name: String,
        version: String,
        web: String,
        github: String,
    },
    #[error("{name} {version} belongs to {owner}, not {github}")]
    NotTheOwner {
        name: String,
        version: String,
        owner: String,
        github: String,
        entry_file: String,
    },
    #[error("{name} {version} is already listed")]
    AlreadyListed {
        name: String,
        version: String,
        entry_file: String,
    },
    #[error("{entry_file} does not parse: {error}")]
    EntryFileUnreadable { entry_file: String, error: String },
    #[error("{name} {version}: {entry_file} would exceed {limit} bytes")]
    EntryFileTooBig {
        name: String,
        version: String,
        entry_file: String,
        limit: u64,
    },
    #[error("{dir} is not a folder: {error}")]
    NotAFolder { dir: String, error: String },
}

/// `lvpm index add`: list `package` (a file at `url`, published from `github`)
/// in `<entries_dir>/<key>.toml`, creating the file for a new name. A refusal
/// is an [`AddRefused`] and writes nothing; the file is replaced whole or not
/// at all.
///
/// The url must be one the consumer accepts for this entry
/// ([`check_index_url`]), so a listed version is always installable.
pub fn add(env: &OpEnv, entries_dir: &Path, package: &Path, github: &str, url: &str) -> Result<()> {
    ensure!(split_owner_repo(github).is_some(), AddRefused::NotOwnerRepo);
    std::fs::read_dir(entries_dir).map_err(|why| AddRefused::NotAFolder {
        dir: entries_dir.display().to_string(),
        error: why.to_string(),
    })?;
    let (listed, sha256) = read_package(package)?;
    let key = name_key(&listed.name);
    let path = entries_dir.join(format!("{key}.toml"));
    let entry_file = shown_path(entries_dir, &key);
    let mut file = read_entry_file(&path, &key, &entry_file)?.unwrap_or_else(|| EntryFile {
        name: listed.name.clone(),
        github: github.to_string(),
        versions: Vec::new(),
    });
    ensure!(
        file.github.eq_ignore_ascii_case(github),
        AddRefused::NotTheOwner {
            name: listed.name.clone(),
            version: listed.version.to_string(),
            owner: file.github.clone(),
            github: github.to_string(),
            entry_file,
        }
    );
    let version = FileVersion::of_package(&listed, url, &sha256);
    file.add_version(version)
        .map_err(|VersionListed| AddRefused::AlreadyListed {
            name: listed.name.clone(),
            version: listed.version.to_string(),
            entry_file: entry_file.clone(),
        })?;
    check_url(&file, &listed, &env.github.web_base())?;
    let text = file.write();
    ensure!(
        text.len() as u64 <= MAX_ENTRY_BYTES,
        AddRefused::EntryFileTooBig {
            name: listed.name.clone(),
            version: listed.version.to_string(),
            entry_file,
            limit: MAX_ENTRY_BYTES,
        }
    );
    // What was built must read back, or the entry file would be one the index refuses.
    EntryFile::parse(&key, &text).map_err(|why| AddRefused::EntryFileUnreadable {
        entry_file: entry_file.clone(),
        error: why.to_string(),
    })?;
    write_atomically(&path, text.as_bytes())?;
    env.sink.emit(Event::IndexEntryAdded {
        name: listed.name,
        version: listed.version.to_string(),
        file: entry_file,
    });
    Ok(())
}

/// What `lvpm index check` made of a PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The new versions that passed every rule.
    pub accepted: usize,
    /// Every refusal, by entry file, then version.
    pub refused: Vec<Finding>,
}

/// A version the PR adds that passed every rule, or a refusal.
enum Verdict {
    Accepted(NewVersion),
    Refused(Finding),
}

impl Verdict {
    /// Where it sorts: by entry file, then version as lvpm reads versions.
    fn place(&self) -> (String, Option<Version>) {
        let (path, version) = match self {
            Verdict::Accepted(new) => (new.path.as_str(), Some(new.version.version.as_str())),
            Verdict::Refused(finding) => (finding.path(), finding.version()),
        };
        (path.to_string(), version.map(Version::parse))
    }
}

/// `lvpm index check`: the verdict on the PR whose checkout is `head_dir`,
/// against `base_dir`, a checkout of the index it targets. One event per
/// verdict, by entry file, then version; a PR that changes nothing the rules
/// cover is said so.
///
/// The PR is data. Its files are read, never run, and no link in them is
/// followed. Each version it adds is downloaded from the owner's releases with
/// no token, up to [`MAX_INDEX_DOWNLOAD_BYTES`], and only if its url passed R4.
/// Every outcome of the attestation lookup but "built by the owner" refuses.
pub fn check(env: &OpEnv, base_dir: &Path, head_dir: &Path) -> Result<Checked> {
    let diff = community::diff(&Tree::read(base_dir)?, &Tree::read(head_dir)?);
    let web = env.github.web_base();
    let mut verdicts: Vec<Verdict> = diff.findings.into_iter().map(Verdict::Refused).collect();
    let mut downloads = None;
    for new in diff.new_versions {
        let found = match new.url_finding(&web) {
            Some(url) => vec![url],
            None => {
                if downloads.is_none() {
                    downloads = Some(Downloads::open(env)?);
                }
                let session = downloads.as_ref().expect("opened above");
                network_findings(session, &new)?
            }
        };
        match found.is_empty() {
            true => verdicts.push(Verdict::Accepted(new)),
            false => verdicts.extend(found.into_iter().map(Verdict::Refused)),
        }
    }
    verdicts.sort_by_key(Verdict::place);
    if verdicts.is_empty() {
        env.sink.emit(Event::IndexNothingToCheck);
    }
    let mut checked = Checked {
        accepted: 0,
        refused: Vec::new(),
    };
    for verdict in verdicts {
        match verdict {
            Verdict::Accepted(new) => {
                checked.accepted += 1;
                env.sink.emit(Event::IndexVersionAccepted {
                    name: new.name,
                    version: new.version.version,
                });
            }
            Verdict::Refused(finding) => {
                checked.refused.push(finding.clone());
                env.sink.emit(Event::IndexRefused { finding });
            }
        }
    }
    Ok(checked)
}

/// R5 to R6b for one new version whose url passed R4: its file is downloaded
/// (checked against the entry's SHA-256 on the way in), read, and asked about.
/// A refusal is a finding; only a cancel is an error.
fn network_findings(downloads: &Downloads, new: &NewVersion) -> Result<Vec<Finding>> {
    let bytes = match downloads.fetch(&new.entry()) {
        Ok(bytes) => bytes,
        Err(why) if why.is::<Cancelled>() => return Err(why),
        Err(why) => {
            let finding = match why.downcast_ref::<IndexHashMismatch>() {
                Some(mismatch) => new.file_differs(&mismatch.actual),
                None => new.not_downloaded(format!("{why:#}")),
            };
            return Ok(vec![finding]);
        }
    };
    let package = match index::entry_from_bytes(Path::new(&new.version.url), &bytes) {
        Ok(package) => package,
        Err(why) => return Ok(vec![new.not_a_package(format!("{why:#}"))]),
    };
    let found = [
        new.package_finding(&package),
        attestation_finding(downloads, new),
    ];
    Ok(found.into_iter().flatten().collect())
}

/// R6 and R6b: S6's verdict on the downloaded file, under a policy that
/// requires an attestation from the version's `github` and no other.
fn attestation_finding(downloads: &Downloads, new: &NewVersion) -> Option<Finding> {
    let policy = Policy {
        require_signature: true,
        trusted_publishers: vec![format!("{ATTESTATION_SITE}/{}", new.github)],
    };
    let outcome = downloads.attestation(&new.github, &new.version.sha256);
    let not_attested = |reason| Some(new.not_attested(reason));
    match decide(outcome, &policy) {
        Ok(Accepted::Signed(_)) => None,
        // A policy that requires an attestation never lets these through; if
        // that changes, the bot must still refuse.
        Ok(Accepted::Unsigned) => not_attested(NotAttested::Missing),
        Ok(Accepted::Unchecked(reason)) => not_attested(NotAttested::Unchecked(reason)),
        Err(Refusal::NoAttestation) => not_attested(NotAttested::Missing),
        Err(Refusal::CannotCheck { reason }) => not_attested(NotAttested::Unchecked(reason)),
        Err(Refusal::DoesNotVerify { .. }) => not_attested(NotAttested::DoesNotVerify),
        Err(Refusal::UntrustedPublisher { subject, .. }) => Some(new.attested_by_other(subject)),
    }
}

/// The package as lvpm reads it, and the SHA-256 of its file. The file is read
/// up to the most an index entry's file may be.
fn read_package(path: &Path) -> Result<(Entry, String)> {
    let bytes = std::fs::File::open(path)
        .and_then(|file| read_up_to(file, MAX_INDEX_DOWNLOAD_BYTES))
        .map_err(|why| not_a_package(path, why.to_string()))?
        .ok_or_else(|| {
            let error = format!("the file is larger than {MAX_INDEX_DOWNLOAD_BYTES} bytes");
            not_a_package(path, error)
        })?;
    let entry = index::entry_from_bytes(path, &bytes)
        .map_err(|why| not_a_package(path, format!("{why:#}")))?;
    ensure!(
        index::is_file_key(&name_key(&entry.name)),
        not_a_package(
            path,
            format!("its name {:?} cannot be an entry file's name", entry.name)
        )
    );
    Ok((entry, sha256_hex(&bytes)))
}

fn not_a_package(path: &Path, error: String) -> AddRefused {
    AddRefused::NotAPackage {
        file: path.display().to_string(),
        error,
    }
}

/// The entry file at `path` (shown as `shown`): `None` when there is none.
fn read_entry_file(path: &Path, key: &str, shown: &str) -> Result<Option<EntryFile>> {
    let unreadable = |error: String| AddRefused::EntryFileUnreadable {
        entry_file: shown.to_string(),
        error,
    };
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(why) => return Err(why).with_context(|| format!("reading {shown}")),
    };
    let bytes = read_up_to(file, MAX_ENTRY_BYTES)
        .with_context(|| format!("reading {shown}"))?
        .ok_or_else(|| unreadable(format!("the file is larger than {MAX_ENTRY_BYTES} bytes")))?;
    let text = String::from_utf8(bytes).map_err(|_| unreadable("the file is not UTF-8".into()))?;
    let file = EntryFile::parse(key, &text).map_err(|why| unreadable(why.to_string()))?;
    Ok(Some(file))
}

/// What `file` holds, or `None` when it holds more than `limit` bytes: no more
/// than `limit` and one byte are read.
fn read_up_to(file: std::fs::File, limit: u64) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

/// The newest version of `file` is a release file of its publisher, on the
/// site at `web`: the rule the consumer applies to it.
fn check_url(file: &EntryFile, listed: &Entry, web: &str) -> Result<(), AddRefused> {
    let added = file.entries("").pop().expect("a version was just added");
    check_index_url(&added, web).map_err(|_| AddRefused::UrlNotInReleases {
        name: listed.name.clone(),
        version: listed.version.to_string(),
        web: web.to_string(),
        github: file.github.clone(),
    })
}

/// `<entries_dir>/<key>.toml` as the folder was given, with `/` on every
/// platform.
fn shown_path(entries_dir: &Path, key: &str) -> String {
    let dir = entries_dir.display().to_string();
    format!("{}/{key}.toml", dir.trim_end_matches(['/', '\\']))
}

#[cfg(test)]
mod tests {
    use super::super::testing::{self, Recorded};
    use super::*;
    use crate::lvpkg;
    use std::path::PathBuf;

    const OWNER: &str = "someone/Serializer";
    const URL: &str =
        "https://github.com/someone/Serializer/releases/download/1.2.1/foo-1.2.1.lvpkg";

    /// A scratch folder holding an empty `packages/` and the packages built
    /// for a test.
    struct Index {
        dir: PathBuf,
    }

    impl Index {
        fn new(tag: &str) -> Index {
            let dir = testing::scratch_dir(tag);
            std::fs::create_dir(dir.join("packages")).unwrap();
            Index { dir }
        }

        fn packages(&self) -> PathBuf {
            self.dir.join("packages")
        }

        fn entry_file(&self) -> PathBuf {
            self.packages().join("foo.toml")
        }

        /// The package file of `foo` at `version`, which requires `bar >=2.0`.
        fn package(&self, version: &str) -> PathBuf {
            let meta = lvpkg::Meta {
                name: "foo".into(),
                version: version.into(),
                labview: ">=2024".into(),
                vendor: "acme".into(),
                display_name: "Foo".into(),
                install_dir: "vi.lib/acme/foo".into(),
                dependencies: vec![("bar".into(), ">=2.0".into())],
            };
            let files = vec![("a.vi".to_string(), b"a".to_vec())];
            let path = self.dir.join(format!("foo-{version}.lvpkg"));
            std::fs::write(&path, lvpkg::write(&meta, &files, None).unwrap()).unwrap();
            path
        }

        /// A `.vip` whose spec says `name`, version 1.0 and `extra` lines.
        fn vip(&self, name: &str, extra: &str) -> PathBuf {
            let spec = format!(
                "[Package]
Name=\"{name}\"
Version=\"1.0\"
{extra}
"
            );
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            zip.start_file("spec", zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut zip, spec.as_bytes()).unwrap();
            let path = self.dir.join("odd.vip");
            std::fs::write(&path, zip.finish().unwrap().into_inner()).unwrap();
            path
        }

        /// Everything under `packages/`, by name, with its bytes.
        fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
            let mut files: Vec<_> = std::fs::read_dir(self.packages())
                .unwrap()
                .map(|e| e.unwrap())
                .map(|e| {
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        std::fs::read(e.path()).unwrap(),
                    )
                })
                .collect();
            files.sort();
            files
        }

        fn add(&self, package: &Path, github: &str, url: &str) -> Result<()> {
            let env = testing::env(&self.dir);
            add(&env, &self.packages(), package, github, url)
        }

        /// The refusal of an add, which wrote nothing.
        fn refused(&self, package: &Path, github: &str, url: &str) -> AddRefused {
            let before = self.snapshot();
            let why = self.add(package, github, url).unwrap_err();
            assert_eq!(self.snapshot(), before, "a refusal wrote something");
            why.downcast::<AddRefused>().expect("a refusal")
        }

        fn entry(&self) -> EntryFile {
            let text = std::fs::read_to_string(self.entry_file()).unwrap();
            EntryFile::parse("foo", &text).unwrap()
        }
    }

    #[test]
    fn a_first_version_creates_the_entry_file_with_what_the_package_says() {
        let index = Index::new("add-first");
        let package = index.package("1.2.1.16");
        let sink = Recorded::default();
        let env = testing::env_on(&sink, &index.dir);

        add(&env, &index.packages(), &package, OWNER, URL).unwrap();

        let file = index.entry();
        assert_eq!(file.github, OWNER);
        let [version] = file.versions.as_slice() else {
            panic!("{file:?}");
        };
        assert_eq!(version.version, "1.2.1.16");
        assert_eq!(version.url, URL);
        assert_eq!(
            version.sha256,
            sha256_hex(&std::fs::read(&package).unwrap())
        );
        assert_eq!(version.labview.as_deref(), Some(">=24.0"));
        assert_eq!(version.requires, ["bar >=2.0"]);
        let packages = index.packages().display().to_string();
        assert_eq!(
            sink.events(),
            [Event::IndexEntryAdded {
                name: "foo".into(),
                version: "1.2.1.16".into(),
                file: format!("{packages}/foo.toml"),
            }]
        );
    }

    #[test]
    fn a_new_version_is_appended_and_the_old_one_stays() {
        let index = Index::new("add-second");
        index.add(&index.package("1.0"), OWNER, URL).unwrap();
        let first = index.entry().versions[0].clone();
        let url = URL.replace("1.2.1", "1.1");

        index
            .add(&index.package("1.1"), "someone/serializer", &url)
            .unwrap();

        let file = index.entry();
        assert_eq!(file.versions.len(), 2);
        assert_eq!(file.versions[0], first);
        assert_eq!(file.versions[1].version, "1.1");
        assert_eq!(file.github, OWNER, "the owner's own spelling stays");
    }

    #[test]
    fn another_repo_is_refused_naming_the_owner_and_nothing_is_written() {
        let index = Index::new("add-owner");
        index.add(&index.package("1.0"), OWNER, URL).unwrap();
        let url = "https://github.com/other/Fork/releases/download/1.1/foo-1.1.lvpkg";

        let why = index.refused(&index.package("1.1"), "other/Fork", url);

        let packages = index.packages().display().to_string();
        assert_eq!(
            why,
            AddRefused::NotTheOwner {
                name: "foo".into(),
                version: "1.1".into(),
                owner: OWNER.into(),
                github: "other/Fork".into(),
                entry_file: format!("{packages}/foo.toml"),
            }
        );
    }

    #[test]
    fn a_listed_version_is_refused_even_spelled_another_way() {
        let index = Index::new("add-listed");
        index.add(&index.package("1.0"), OWNER, URL).unwrap();

        let why = index.refused(&index.package("1.0.0.0"), OWNER, URL);

        assert!(matches!(why, AddRefused::AlreadyListed { .. }), "{why:?}");
    }

    #[test]
    fn a_url_the_consumer_would_refuse_is_refused_and_nothing_is_written() {
        let index = Index::new("add-url");
        let package = index.package("1.0");
        let release = "https://github.com/someone/Serializer/releases/download";
        let urls = [
            "https://example.com/x.lvpkg".to_string(),
            format!("{release}/1.0/../../../../other/Fork/releases/download/1/x.lvpkg"),
            "https://github.com/other/Fork/releases/download/1.0/foo.lvpkg".to_string(),
            format!("{release}/1.0/foo.lvpkg?token=1"),
            format!("{release}/1.0/sub/foo.lvpkg"),
            format!("{release}/1.0%2f..%2f..%2fother/foo.lvpkg"),
            "C:\\Users\\x\\foo.lvpkg".to_string(),
            "cache:abc".to_string(),
            "oci://ghcr.io/x/foo:1.0".to_string(),
        ];
        for url in urls {
            let why = index.refused(&package, OWNER, &url);
            assert_eq!(
                why,
                AddRefused::UrlNotInReleases {
                    name: "foo".into(),
                    version: "1.0".into(),
                    web: "https://github.com".into(),
                    github: OWNER.into()
                },
                "{url}"
            );
        }
    }

    #[test]
    fn the_url_is_checked_against_the_web_base_of_the_api() {
        let index = Index::new("add-web");
        let mut env = testing::env(&index.dir);
        env.github.api = "http://127.0.0.1:1".into();
        let package = index.package("1.0");

        let why = add(&env, &index.packages(), &package, OWNER, URL).unwrap_err();
        assert_eq!(
            why.downcast::<AddRefused>().unwrap(),
            AddRefused::UrlNotInReleases {
                name: "foo".into(),
                version: "1.0".into(),
                web: "http://127.0.0.1:1".into(),
                github: OWNER.into()
            }
        );

        let url = "http://127.0.0.1:1/someone/Serializer/releases/download/1.0/foo.lvpkg";
        add(&env, &index.packages(), &package, OWNER, url).unwrap();
    }

    #[test]
    fn a_file_that_is_not_a_package_is_refused_naming_it() {
        let index = Index::new("add-notes");
        let notes = index.dir.join("notes.txt");
        std::fs::write(&notes, "not a zip").unwrap();
        let missing = index.dir.join("missing.lvpkg");

        for file in [&notes, &missing] {
            let why = index.refused(file, OWNER, URL);
            let AddRefused::NotAPackage { file: shown, .. } = &why else {
                panic!("{why:?}");
            };
            assert_eq!(shown, &file.display().to_string());
        }
    }

    #[test]
    fn a_package_file_over_the_limit_is_refused() {
        let index = Index::new("add-big");
        let big = index.dir.join("big.lvpkg");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(MAX_INDEX_DOWNLOAD_BYTES + 1).unwrap();

        let why = index.refused(&big, OWNER, URL);

        let AddRefused::NotAPackage { error, .. } = &why else {
            panic!("{why:?}");
        };
        assert!(error.contains("larger than"), "{error}");
    }

    #[test]
    fn an_entry_file_that_does_not_read_is_refused_and_left_as_it_was() {
        let index = Index::new("add-unreadable");
        let package = index.package("1.0");
        // A listing that reads, but is padded past the limit.
        let too_big = format!(
            "name = \"foo\"
github = \"{OWNER}\"
{}",
            " ".repeat(MAX_ENTRY_BYTES as usize)
        );
        let texts: [(&str, Vec<u8>); 3] = [
            ("not TOML", b"name = [".to_vec()),
            ("a listing for another name", b"name = \"bar\"\n".to_vec()),
            ("over the limit", too_big.into_bytes()),
        ];
        for (what, text) in texts {
            std::fs::write(index.entry_file(), &text).unwrap();

            let why = index.refused(&package, OWNER, URL);

            assert!(
                matches!(why, AddRefused::EntryFileUnreadable { .. }),
                "{what}: {why:?}"
            );
        }
    }

    #[test]
    fn a_github_that_is_not_owner_slash_repo_is_refused() {
        let index = Index::new("add-github");
        let package = index.package("1.0");
        for github in ["someone", "/r", "o/", "o/r/x", "../r", "o/.."] {
            assert_eq!(
                index.refused(&package, github, URL),
                AddRefused::NotOwnerRepo,
                "{github}"
            );
        }
    }

    #[test]
    fn a_package_whose_name_cannot_be_a_file_name_is_refused() {
        let index = Index::new("add-name");
        for name in ["../evil", "a/b", r"a\b", "sp ace", ""] {
            let why = index.refused(&index.vip(name, ""), OWNER, URL);

            let AddRefused::NotAPackage { error, .. } = &why else {
                panic!("{name:?}: {why:?}");
            };
            assert!(error.contains("name"), "{name:?}: {error}");
        }
        assert!(!index.dir.join("evil.toml").exists());
    }

    #[test]
    fn a_requirement_the_entry_file_cannot_hold_is_refused_before_anything_is_written() {
        let index = Index::new("add-requires");
        let package = index.vip(
            "foo",
            "[Dependencies]
Requires=\"bar>=?\"",
        );

        let why = index.refused(&package, OWNER, URL);

        let AddRefused::EntryFileUnreadable { entry_file, error } = &why else {
            panic!("{why:?}");
        };
        assert!(entry_file.ends_with("/foo.toml"), "{entry_file}");
        assert!(error.contains("requires"), "{error}");
    }

    #[test]
    fn an_entry_file_that_would_pass_the_limit_is_refused_and_left_as_it_was() {
        let index = Index::new("add-grown");
        // One listed version whose requirements fill the file to just under the limit.
        let listing = |requirements: usize| {
            let version = FileVersion {
                version: "0.0.1".into(),
                url: URL.into(),
                sha256: "0".repeat(64),
                labview: None,
                requires: vec!["bar >=2.0".to_string(); requirements],
            };
            EntryFile {
                name: "foo".into(),
                github: OWNER.into(),
                versions: vec![version],
            }
        };
        let (one, two) = (listing(1).write().len(), listing(2).write().len());
        let fill = (MAX_ENTRY_BYTES as usize - 100 - one) / (two - one) + 1;
        let file = listing(fill);
        assert!(file.write().len() < MAX_ENTRY_BYTES as usize);
        std::fs::write(index.entry_file(), file.write()).unwrap();

        let why = index.refused(&index.package("1.2.1"), OWNER, URL);

        let packages = index.packages().display().to_string();
        assert_eq!(
            why,
            AddRefused::EntryFileTooBig {
                name: "foo".into(),
                version: "1.2.1".into(),
                entry_file: format!("{packages}/foo.toml"),
                limit: MAX_ENTRY_BYTES,
            }
        );
    }

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// A checkout of an index: the files written under a scratch folder.
    struct Checkout(PathBuf);

    impl Checkout {
        fn new(dir: &Path, name: &str) -> Checkout {
            let path = dir.join(name);
            std::fs::create_dir_all(path.join("packages")).unwrap();
            Checkout(path)
        }

        fn write(&self, path: &str, text: &str) {
            std::fs::write(self.0.join(path), text).unwrap();
        }

        /// `packages/foo.toml`, owned by `a/x`, listing `versions` with `sha256`.
        fn list_foo(&self, versions: &[&str], sha256: &str) {
            let listed = |version: &&str| FileVersion {
                version: version.to_string(),
                url: format!("https://github.com/a/x/releases/download/{version}/foo.lvpkg"),
                sha256: sha256.into(),
                labview: None,
                requires: Vec::new(),
            };
            let file = EntryFile {
                name: "foo".into(),
                github: "a/x".into(),
                versions: versions.iter().map(listed).collect(),
            };
            self.write("packages/foo.toml", &file.write());
        }
    }

    fn checked(base: &Checkout, head: &Checkout) -> (Checked, Vec<Event>) {
        let sink = Recorded::default();
        let env = testing::env_on(&sink, &base.0);
        let checked = check(&env, &base.0, &head.0).unwrap();
        (checked, sink.events())
    }

    #[test]
    fn a_pr_that_changes_nothing_is_nothing_to_check() {
        let dir = testing::scratch_dir("check-nothing");
        let (base, head) = (Checkout::new(&dir, "base"), Checkout::new(&dir, "head"));
        base.list_foo(&["1.0.0"], SHA_A);
        head.list_foo(&["1.0.0"], SHA_A);

        let (checked, events) = checked(&base, &head);

        assert_eq!(events, [Event::IndexNothingToCheck]);
        assert_eq!((checked.accepted, checked.refused), (0, vec![]));
    }

    #[test]
    fn refusals_are_one_event_each_by_entry_file_then_version_and_none_is_downloaded() {
        let dir = testing::scratch_dir("check-order");
        let (base, head) = (Checkout::new(&dir, "base"), Checkout::new(&dir, "head"));
        base.list_foo(&["1.0.0", "1.10.0", "1.2.0"], SHA_A);
        head.list_foo(&["1.0.0", "1.10.0", "1.2.0"], SHA_B);
        head.write("notes.md", "hello");

        let (checked, events) = checked(&base, &head);

        let said: Vec<(String, Option<String>)> = events
            .iter()
            .map(|event| match event {
                Event::IndexRefused { finding } => (
                    finding.path().to_string(),
                    finding.version().map(str::to_string),
                ),
                other => panic!("{other:?}"),
            })
            .collect();
        let foo = "packages/foo.toml".to_string();
        assert_eq!(
            said,
            [
                ("notes.md".to_string(), None),
                (foo.clone(), Some("1.0.0".into())),
                (foo.clone(), Some("1.2.0".into())),
                (foo, Some("1.10.0".into())),
            ],
            "versions sort as lvpm reads them, not as text"
        );
        assert_eq!(checked.refused.len(), 4);
        assert_eq!(checked.accepted, 0);
    }

    #[test]
    fn a_url_that_is_not_the_owners_is_refused_before_anything_is_downloaded() {
        let dir = testing::scratch_dir("check-url");
        let (base, head) = (Checkout::new(&dir, "base"), Checkout::new(&dir, "head"));
        base.list_foo(&["1.0.0"], SHA_A);
        head.write(
            "packages/foo.toml",
            &format!(
                "name = \"foo\"\ngithub = \"a/x\"\n\
                 [[version]]\nversion = \"1.0.0\"\n\
                 url = \"https://github.com/a/x/releases/download/1.0.0/foo.lvpkg\"\n\
                 sha256 = \"{SHA_A}\"\n\
                 [[version]]\nversion = \"1.1.0\"\nurl = \"https://example.com/foo.lvpkg\"\n\
                 sha256 = \"{SHA_B}\"\n"
            ),
        );

        // The default environment's GitHub is unreachable: a download would be a finding of its own.
        let (_, events) = checked(&base, &head);

        let [Event::IndexRefused { finding }] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert!(
            matches!(finding, Finding::UrlNotReleaseFile { version, .. } if version == "1.1.0"),
            "{finding:?}"
        );
    }

    #[test]
    fn a_checkout_that_is_not_a_folder_is_an_error() {
        let dir = testing::scratch_dir("check-missing");
        let head = Checkout::new(&dir, "head");
        let env = testing::env(&dir);

        let why = check(&env, &dir.join("no-base"), &head.0).unwrap_err();

        assert!(why.to_string().contains("no-base"), "{why:#}");
    }

    #[test]
    fn a_packages_folder_that_is_not_a_folder_is_refused_naming_it() {
        let index = Index::new("add-nofolder");
        let package = index.package("1.0");
        let env = testing::env(&index.dir);
        let file = index.dir.join("a-file");
        std::fs::write(&file, "x").unwrap();
        for dir in [index.dir.join("missing"), file] {
            let why = add(&env, &dir, &package, OWNER, URL).unwrap_err();

            let AddRefused::NotAFolder { dir: shown, error } = why.downcast().expect("a refusal")
            else {
                panic!("not NotAFolder");
            };
            assert_eq!(shown, dir.display().to_string());
            assert!(!error.is_empty());
        }
    }
}
