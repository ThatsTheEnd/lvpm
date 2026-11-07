//! The community index's entry file, `packages/<key>.toml`: the name, the repo
//! that owns it, and one table per version, with everything lvpm needs to
//! resolve a version without downloading it.
//!
//! The file is untrusted input (any publisher can open a PR with one), so
//! [`EntryFile::parse`] checks every field and its errors name a key and a
//! version's position, never a value from the file. No network I/O here: the
//! file's text is handed in.

use crate::cache::sha256_hex;
use crate::fetch::check_index_url;
use crate::github::MAX_ENTRY_BYTES;
use crate::index::{Entry, IndexPin, Requirement, is_file_key, parse_lv_gate, parse_requires};
use crate::project::{name_key, split_owner_repo};
use crate::verify::NotChecked;
use crate::version::{Version, is_version};
use anyhow::Context;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

const KEY_NAME: &str = "name";
const KEY_GITHUB: &str = "github";
const KEY_VERSION: &str = "version";
const KEY_URL: &str = "url";
const KEY_SHA256: &str = "sha256";
const KEY_LABVIEW: &str = "labview";
const KEY_REQUIRES: &str = "requires";

const A_STRING: &str = "a string";
const A_LIST_OF_STRINGS: &str = "a list of strings";
const A_LIST_OF_TABLES: &str = "a list of tables";
const A_TABLE: &str = "a table";

/// Hex digits in a SHA-256.
const SHA256_HEX_DIGITS: usize = 64;

/// The most entry-file bytes [`Tree::read`] keeps in all. A file past it is
/// hashed and its body dropped, so it reads as one that is too big.
const MAX_TREE_BODY_BYTES: u64 = 64 * 1024 * 1024;

/// The most versions one PR may add; past it the PR is refused before any
/// download.
pub const MAX_NEW_VERSIONS_PER_PR: usize = 20;

/// The most entry files one PR may change or add.
pub const MAX_ENTRY_FILES_PER_PR: usize = 20;

/// Names Windows keeps for devices: a folder or file of one cannot be made.
const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// How much of an unknown key an error repeats.
const MAX_SHOWN_KEY_CHARS: usize = 32;

const TOP_LEVEL_KEYS: [&str; 3] = [KEY_NAME, KEY_GITHUB, KEY_VERSION];
const VERSION_KEYS: [&str; 5] = [KEY_VERSION, KEY_URL, KEY_SHA256, KEY_LABVIEW, KEY_REQUIRES];

/// One entry file: a package name, the repo that owns it, and its versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryFile {
    pub name: String,
    /// `owner/repo` that owns the name.
    pub github: String,
    pub versions: Vec<FileVersion>,
}

/// One `[[version]]` table. The text fields are kept as written; they were
/// checked on the way in, so [`EntryFile::entries`] reads them without
/// failing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    pub version: String,
    pub url: String,
    /// Lower-case hex.
    pub sha256: String,
    /// The package's LabVIEW gate, e.g. `>=20.0`; `None` when it has none.
    pub labview: Option<String>,
    /// The package's own requirements, e.g. `bar >=2`.
    pub requires: Vec<String>,
}

impl FileVersion {
    /// The first field (as `field`, old text, new text) where `now` differs from
    /// this listed version. Consumers trust every field, so any change counts,
    /// the version's own spelling too (`1.0.0` is `1.0.0.0` to lvpm, not to a
    /// reader of the index).
    fn first_change(&self, now: &FileVersion) -> Option<(&'static str, String, String)> {
        let gate = |v: &FileVersion| v.labview.clone().unwrap_or_else(|| "none".into());
        let requires = |v: &FileVersion| {
            if v.requires.is_empty() {
                "none".into()
            } else {
                v.requires.join(", ")
            }
        };
        [
            ("sha256", self.sha256.clone(), now.sha256.clone()),
            ("version", self.version.clone(), now.version.clone()),
            ("url", self.url.clone(), now.url.clone()),
            ("labview", gate(self), gate(now)),
            ("requires", requires(self), requires(now)),
        ]
        .into_iter()
        .find(|(_, old, new)| old != new)
    }
}

impl FileVersion {
    /// What an index lists of `package`, as lvpm reads it out of the file
    /// (`entry_from_bytes`): the file at `url` has `sha256`.
    pub fn of_package(package: &Entry, url: &str, sha256: &str) -> Self {
        FileVersion {
            version: package.version.to_string(),
            url: url.to_string(),
            sha256: sha256.to_string(),
            labview: package.lv_min.map(|min| format!(">={min:?}")),
            requires: package.requires.iter().map(requirement_text).collect(),
        }
    }
}

/// One requirement as an entry file writes it: `bar >=2`.
fn requirement_text(requirement: &Requirement) -> String {
    match &requirement.min {
        None => requirement.name.clone(),
        Some(min) => {
            let operator = if requirement.min_exclusive { ">" } else { ">=" };
            format!("{} {operator}{min}", requirement.name)
        }
    }
}

/// Why an entry file does not read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EntryFileError {
    /// The text is not TOML. Only the line is given: the parser's own message
    /// quotes the file.
    #[error("not valid TOML at line {line}")]
    Syntax { line: usize },
    /// `path` is a key (`github`) or a key in a version table (`version[2].sha256`,
    /// 1-based).
    #[error("{path}: {problem}")]
    Field { path: String, problem: Problem },
    /// No `[[version]]` table: a file that lists nothing is not an entry.
    #[error("lists no version")]
    NoVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    #[error("unknown key")]
    UnknownKey,
    #[error("missing")]
    Missing,
    #[error("must be {SHA256_HEX_DIGITS} hex digits")]
    NotSha256,
    #[error("does not match the file name")]
    NameMismatch,
    #[error("listed twice")]
    Duplicate,
    #[error("must be owner/repo")]
    NotOwnerRepo,
    #[error("cannot be read")]
    Unreadable,
    #[error("cannot be installed")]
    NotInstallable,
    #[error("must be {0}")]
    WrongType(&'static str),
}

/// The version is already in the file, by lvpm's reading of versions (`1.0.0.0`
/// is `1.0.0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("that version is already listed")]
pub struct VersionListed;

impl EntryFile {
    /// Reads the file `packages/<key>.toml`.
    pub fn parse(key: &str, text: &str) -> Result<Self, EntryFileError> {
        Self::parse_checking(Some(key), text)
    }

    /// [`parse`](Self::parse), except that with no `key` the name is not held
    /// to a file name: the caller compares it.
    fn parse_checking(key: Option<&str>, text: &str) -> Result<Self, EntryFileError> {
        let table: toml::Table = toml::from_str(text).map_err(|why| EntryFileError::Syntax {
            line: why.span().map_or(1, |span| line_at(text, span.start)),
        })?;
        let top = Scope::new(&table, String::new());
        top.reject_unknown(&TOP_LEVEL_KEYS)?;
        let name = top.required_str(KEY_NAME)?;
        if key.is_some_and(|key| name_key(name) != key) {
            return Err(top.error(KEY_NAME, Problem::NameMismatch));
        }
        // With no key the caller compares the name, and says so if it cannot be installed.
        if key.is_some() && !installable_name(name) {
            return Err(top.error(KEY_NAME, Problem::NotInstallable));
        }
        let mut file = EntryFile {
            name: name.to_string(),
            github: top.owner_repo(KEY_GITHUB)?.to_string(),
            versions: Vec::new(),
        };
        let tables = match table.get(KEY_VERSION) {
            None => &[][..],
            Some(toml::Value::Array(tables)) => tables,
            Some(_) => return Err(top.error(KEY_VERSION, Problem::WrongType(A_LIST_OF_TABLES))),
        };
        let mut seen = BTreeSet::new();
        for (i, value) in tables.iter().enumerate() {
            let prefix = format!("{KEY_VERSION}[{}]", i + 1);
            let toml::Value::Table(version) = value else {
                return Err(top.error(&prefix, Problem::WrongType(A_TABLE)));
            };
            let scope = Scope::new(version, format!("{prefix}."));
            scope.reject_unknown(&VERSION_KEYS)?;
            let version = scope.file_version()?;
            // Not `add_version`, which reads every listed version again.
            if !seen.insert(Version::parse(&version.version)) {
                return Err(scope.error(KEY_VERSION, Problem::Duplicate));
            }
            file.versions.push(version);
        }
        if file.versions.is_empty() {
            return Err(EntryFileError::NoVersion);
        }
        Ok(file)
    }

    /// The file as TOML, keys in the order of the spec; an empty `labview` or
    /// `requires` is left out.
    pub fn write(&self) -> String {
        let text = |value: &str| toml::Value::String(value.to_string());
        let versions = self.versions.iter().map(|v| {
            let mut table = toml::Table::new();
            table.insert(KEY_VERSION.into(), text(&v.version));
            table.insert(KEY_URL.into(), text(&v.url));
            table.insert(KEY_SHA256.into(), text(&v.sha256));
            if let Some(gate) = &v.labview {
                table.insert(KEY_LABVIEW.into(), text(gate));
            }
            if !v.requires.is_empty() {
                let items = v.requires.iter().map(|r| text(r)).collect();
                table.insert(KEY_REQUIRES.into(), toml::Value::Array(items));
            }
            toml::Value::Table(table)
        });
        let mut table = toml::Table::new();
        table.insert(KEY_NAME.into(), text(&self.name));
        table.insert(KEY_GITHUB.into(), text(&self.github));
        table.insert(KEY_VERSION.into(), toml::Value::Array(versions.collect()));
        toml::to_string(&table).expect("strings, lists and tables always serialise")
    }

    /// Appends `version`, unless the file lists it already.
    pub fn add_version(&mut self, version: FileVersion) -> Result<(), VersionListed> {
        if self.listed(&version.version).is_some() {
            return Err(VersionListed);
        }
        self.versions.push(version);
        Ok(())
    }

    /// The version the file lists as `version`, by lvpm's reading of versions
    /// (`1.0.0.0` is `1.0.0`).
    pub fn listed(&self, version: &str) -> Option<&FileVersion> {
        let wanted = Version::parse(version);
        self.versions
            .iter()
            .find(|v| Version::parse(&v.version) == wanted)
    }

    /// The versions the file lists by lvpm's reading of them: one lookup for
    /// many asks of [`listed`](Self::listed).
    fn by_version(&self) -> BTreeMap<Version, &FileVersion> {
        let read = |v: &'_ FileVersion| Version::parse(&v.version);
        self.versions.iter().map(|v| (read(v), v)).collect()
    }

    /// One [`Entry`] per version, listed by `source` (`github-index:owner/repo`).
    pub fn entries(&self, source: &str) -> Vec<Entry> {
        self.versions
            .iter()
            .map(|v| self.entry(v, source))
            .collect()
    }

    /// `version` of this file, listed by `source`.
    fn entry(&self, v: &FileVersion, source: &str) -> Entry {
        Entry {
            name: self.name.clone(),
            version: Version::parse(&v.version),
            url: v.url.clone(),
            requires: v
                .requires
                .iter()
                .filter_map(|r| parse_requirement(r))
                .collect(),
            lv_min: v.labview.as_deref().and_then(parse_lv_gate),
            source: source.to_string(),
            index_pin: Some(IndexPin {
                sha256: v.sha256.clone(),
                publisher: self.github.clone(),
            }),
            ..Entry::default()
        }
    }
}

/// The 1-based line of `text` that byte `offset` is on; line 1 for an offset
/// that is not inside it or not on a character.
fn line_at(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(1, |before| before.matches('\n').count() + 1)
}

/// Whether lvpm can install a package of this name on any platform: not empty,
/// not ending in `.` (all dots too), and not a Windows device name once cut at
/// its first `.`.
pub(crate) fn installable_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default();
    !name.is_empty()
        && !name.ends_with('.')
        && !RESERVED_DEVICE_NAMES
            .iter()
            .any(|device| stem.eq_ignore_ascii_case(device))
}

/// The file name `packages/<key>.toml` an index lists `name` in: `None` when
/// that cannot be a file name, or lvpm could not install the package.
pub(crate) fn listable_key(name: &str) -> Option<String> {
    let key = name_key(name);
    (is_file_key(&key) && installable_name(name)).then_some(key)
}

/// The entries of `packages/<key>.toml` as `text`, with no source yet: what the
/// reader of the sources is handed to parse an index file, and labels the
/// entries itself.
pub fn read_entries(key: &str, text: &str) -> anyhow::Result<Vec<Entry>> {
    Ok(EntryFile::parse(key, text)?.entries(""))
}

/// One requirement, `bar >=2`: a name, and a version if there is an operator.
/// `parse_requires` reads leniently (`bar >=?` gets a version `?`), so what it
/// returns is checked here.
fn parse_requirement(item: &str) -> Option<Requirement> {
    let mut found = parse_requires(item);
    let requirement = found.pop().filter(|_| found.is_empty())?;
    let has_version = |v: &Version| v.raw.starts_with(|c: char| c.is_ascii_digit());
    (!requirement.name.is_empty() && requirement.min.as_ref().is_none_or(has_version))
        .then_some(requirement)
}

/// What `lvpm index check` finds wrong with a PR: one per refusal, with the
/// rule it breaks. The wording is the front end's; every finding names its
/// entry file's `path` (`packages/<key>.toml`). R1 to R4 and the unreadable
/// file need no network; the rest are found by reading the version's file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum Finding {
    /// R1: a file that is not `packages/<name>.toml` was added, changed,
    /// removed or replaced by a link.
    OutsideEntries { path: String },
    /// R1b: the file's `name` is not the one its file name says.
    WrongFileName { path: String, name: String },
    /// R2: the file names another `github` than the base's.
    OwnerChanged {
        path: String,
        name: String,
        old: String,
        new: String,
    },
    /// R3: a listed version has another `field` (`sha256`, `version`, `url`,
    /// `labview` or `requires`, the first that differs); `version` is as the
    /// base wrote it.
    VersionChanged {
        path: String,
        name: String,
        version: String,
        field: &'static str,
        old: String,
        new: String,
    },
    /// R3b: a listed version is gone.
    VersionRemoved {
        path: String,
        name: String,
        version: String,
    },
    /// R4: the version's `url` is not a release file of its `github`.
    UrlNotReleaseFile {
        path: String,
        name: String,
        version: String,
        url: String,
        github: String,
    },
    /// The file's `name` has a file key, but no package of that name can be
    /// installed on every platform.
    NameNotInstallable { path: String, name: String },
    /// The PR adds `versions` versions, more than [`MAX_NEW_VERSIONS_PER_PR`].
    TooManyVersions { versions: usize },
    /// The PR changes or adds `files` entry files, more than
    /// [`MAX_ENTRY_FILES_PER_PR`].
    TooManyFiles { files: usize },
    /// The file is too big, is not UTF-8, or does not read as an entry file.
    /// `error` never quotes the file.
    Unreadable { path: String, error: String },
    /// R5: the file at the version's `url` has SHA-256 `actual`, not `entry`.
    FileDiffers {
        path: String,
        name: String,
        version: String,
        actual: String,
        entry: String,
    },
    /// R5: the file at the version's `url` could not be downloaded.
    NotDownloaded {
        path: String,
        name: String,
        version: String,
        error: String,
    },
    /// The file the entry lists is not a package lvpm reads.
    NotAPackage {
        path: String,
        name: String,
        version: String,
        error: String,
    },
    /// R5b: the package says another `field` (`name`, `version`, `requires` or
    /// `labview`, the first that differs) than the entry; both as text.
    PackageDiffers {
        path: String,
        name: String,
        version: String,
        field: &'static str,
        actual: String,
        entry: String,
    },
    /// R6: nobody vouches that the publisher built the file.
    NotAttested {
        path: String,
        name: String,
        version: String,
        reason: NotAttested,
    },
    /// R6b: the file is attested by the repo at `subject`, not by `github`.
    AttestedByOther {
        path: String,
        name: String,
        version: String,
        subject: String,
        github: String,
    },
}

/// Why a file is not attested (R6). Every reason refuses: the bot fails closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotAttested {
    /// GitHub holds no attestation for the file.
    Missing,
    /// lvpm could not find out, for this reason.
    Unchecked(NotChecked),
    /// An attestation exists and does not verify.
    DoesNotVerify,
}

impl Finding {
    /// The entry file the finding is about; empty for a finding about the
    /// whole PR.
    pub fn path(&self) -> &str {
        match self {
            Finding::TooManyVersions { .. } | Finding::TooManyFiles { .. } => "",
            Finding::OutsideEntries { path }
            | Finding::WrongFileName { path, .. }
            | Finding::NameNotInstallable { path, .. }
            | Finding::OwnerChanged { path, .. }
            | Finding::VersionChanged { path, .. }
            | Finding::VersionRemoved { path, .. }
            | Finding::UrlNotReleaseFile { path, .. }
            | Finding::Unreadable { path, .. }
            | Finding::FileDiffers { path, .. }
            | Finding::NotDownloaded { path, .. }
            | Finding::NotAPackage { path, .. }
            | Finding::PackageDiffers { path, .. }
            | Finding::NotAttested { path, .. }
            | Finding::AttestedByOther { path, .. } => path,
        }
    }

    /// The version it is about, when it is about one.
    pub fn version(&self) -> Option<&str> {
        match self {
            Finding::OutsideEntries { .. }
            | Finding::WrongFileName { .. }
            | Finding::NameNotInstallable { .. }
            | Finding::TooManyVersions { .. }
            | Finding::TooManyFiles { .. }
            | Finding::OwnerChanged { .. }
            | Finding::Unreadable { .. } => None,
            Finding::VersionChanged { version, .. }
            | Finding::VersionRemoved { version, .. }
            | Finding::UrlNotReleaseFile { version, .. }
            | Finding::FileDiffers { version, .. }
            | Finding::NotDownloaded { version, .. }
            | Finding::NotAPackage { version, .. }
            | Finding::PackageDiffers { version, .. }
            | Finding::NotAttested { version, .. }
            | Finding::AttestedByOther { version, .. } => Some(version),
        }
    }
}

/// A version the head lists and the base does not, whose file passed the rules
/// that need no network: what the network rules check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVersion {
    pub path: String,
    pub name: String,
    /// The head's `github` for the name.
    pub github: String,
    pub version: FileVersion,
}

/// What [`diff`] makes of two checkouts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub findings: Vec<Finding>,
    pub new_versions: Vec<NewVersion>,
}

/// A checkout of the index repo, read as data: no code in it runs and no link
/// in it is followed.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    /// Every path (`/`-separated, relative to the checkout) that is not a
    /// folder with something in it.
    files: BTreeMap<String, Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    /// A regular file of no more than [`MAX_ENTRY_BYTES`]. `body` is kept only
    /// for an entry file, and only while the tree's bodies fit
    /// [`MAX_TREE_BODY_BYTES`].
    File {
        sha256: String,
        body: Option<Vec<u8>>,
    },
    /// A symbolic link, with where it points; never followed.
    Link(PathBuf),
    /// A device, pipe or socket; never opened.
    Special,
    /// A folder with nothing in it: what a submodule checks out as.
    EmptyFolder,
    /// A file past [`MAX_ENTRY_BYTES`]. Only the start was read, so it has no
    /// hash and is never equal to anything.
    TooBig,
    /// A name that is not UTF-8, or that reads as another name once shown. It
    /// is never equal to anything either.
    BadName,
}

impl Node {
    /// Whether the node stands for something that was not read whole, so that
    /// two of them being equal says nothing.
    fn unread(&self) -> bool {
        matches!(self, Node::TooBig | Node::BadName)
    }

    /// Whether the node is no file: nothing an entry file can be.
    fn not_a_file(&self) -> bool {
        matches!(
            self,
            Node::Link(_) | Node::Special | Node::EmptyFolder | Node::BadName
        )
    }
}

/// Compare two checkouts of the index repo (`base`, then `head`) by the rules
/// that need no network: R1, R1b, R2, R3, R3b, and an entry file that does not
/// read. The versions the head adds to a file that passes R1b and R2 are in
/// [`Diff::new_versions`].
pub fn diff(base: &Tree, head: &Tree) -> Diff {
    let mut out = Diff::default();
    let mut entry_files = 0;
    let paths: BTreeSet<&String> = base.files.keys().chain(head.files.keys()).collect();
    for path in paths {
        let (before, after) = (base.files.get(path), head.files.get(path));
        if before == after && !before.is_some_and(Node::unread) {
            continue;
        }
        match entry_stem(path) {
            None => out
                .findings
                .push(Finding::OutsideEntries { path: path.clone() }),
            Some(stem) => {
                entry_files += usize::from(after.is_some());
                out.entry_changed(path, stem, before, after)
            }
        }
    }
    out.limit_the_work(entry_files);
    out
}

/// `x` of `packages/x.toml`: the only paths that can be an entry file.
fn entry_stem(path: &str) -> Option<&str> {
    let file = path.strip_prefix("packages/")?;
    file.strip_suffix(".toml")
        .filter(|stem| !file.contains('/') && !stem.is_empty())
}

/// The entry file a node holds: `None` for no file and for what is no file,
/// which holds nothing. The error is why the file does not read; it never
/// quotes the file.
fn read_entry(node: Option<&Node>) -> Result<Option<EntryFile>, String> {
    let Some(node) = node.filter(|node| !node.not_a_file()) else {
        return Ok(None);
    };
    match node {
        Node::File {
            body: Some(bytes), ..
        } => {
            let text = std::str::from_utf8(bytes).map_err(|_| "the file is not UTF-8")?;
            let file = EntryFile::parse_checking(None, text).map_err(|why| why.to_string())?;
            Ok(Some(file))
        }
        _ => Err(format!("the file is larger than {MAX_ENTRY_BYTES} bytes")),
    }
}

impl Diff {
    /// A PR that adds more versions, or changes more entry files, than the bot
    /// takes in one go is refused whole: none of its versions is downloaded.
    fn limit_the_work(&mut self, entry_files: usize) {
        let versions = self.new_versions.len();
        if versions > MAX_NEW_VERSIONS_PER_PR {
            self.findings.push(Finding::TooManyVersions { versions });
        }
        if entry_files > MAX_ENTRY_FILES_PER_PR {
            self.findings
                .push(Finding::TooManyFiles { files: entry_files });
        }
        if versions > MAX_NEW_VERSIONS_PER_PR || entry_files > MAX_ENTRY_FILES_PER_PR {
            self.new_versions.clear();
        }
    }

    /// The rules for the changed `packages/<stem>.toml` at `path`.
    fn entry_changed(
        &mut self,
        path: &str,
        stem: &str,
        before: Option<&Node>,
        after: Option<&Node>,
    ) {
        let path = || path.to_string();
        if after.is_some_and(Node::not_a_file) {
            self.findings.push(Finding::OutsideEntries { path: path() });
        }
        let (old, new) = match (read_entry(before), read_entry(after)) {
            (Ok(old), Ok(new)) => (old, new),
            (Err(error), _) | (Ok(_), Err(error)) => {
                self.findings.push(Finding::Unreadable {
                    path: path(),
                    error,
                });
                return;
            }
        };
        if let Some(new) = &new
            && (name_key(&new.name) != stem || !is_file_key(stem))
        {
            self.findings.push(Finding::WrongFileName {
                path: path(),
                name: new.name.clone(),
            });
            return;
        }
        if let Some(new) = &new
            && !installable_name(&new.name)
        {
            self.findings.push(Finding::NameNotInstallable {
                path: path(),
                name: new.name.clone(),
            });
            return;
        }
        let mut owner_changed = false;
        if let (Some(old), Some(new)) = (&old, &new)
            && !old.github.eq_ignore_ascii_case(&new.github)
        {
            owner_changed = true;
            self.findings.push(Finding::OwnerChanged {
                path: path(),
                name: old.name.clone(),
                old: old.github.clone(),
                new: new.github.clone(),
            });
        }
        let now_listed = new.as_ref().map(EntryFile::by_version);
        if let Some(old) = &old {
            for listed in &old.versions {
                let name = old.name.clone();
                let now = now_listed.as_ref();
                match now.and_then(|now| now.get(&Version::parse(&listed.version)).copied()) {
                    None => self.findings.push(Finding::VersionRemoved {
                        path: path(),
                        name,
                        version: listed.version.clone(),
                    }),
                    Some(now) => {
                        // R2 already refuses a new owner, whose URLs differ by construction.
                        let change = listed
                            .first_change(now)
                            .filter(|(field, ..)| !owner_changed || *field == "sha256");
                        if let Some((field, old, new)) = change {
                            self.findings.push(Finding::VersionChanged {
                                path: path(),
                                name,
                                version: listed.version.clone(),
                                field,
                                old,
                                new,
                            });
                        }
                    }
                }
            }
        }
        if let Some(new) = &new
            && !owner_changed
        {
            let was_listed = old.as_ref().map(EntryFile::by_version);
            for version in &new.versions {
                if was_listed
                    .as_ref()
                    .is_none_or(|was| !was.contains_key(&Version::parse(&version.version)))
                {
                    self.new_versions.push(NewVersion {
                        path: path(),
                        name: new.name.clone(),
                        github: new.github.clone(),
                        version: version.clone(),
                    });
                }
            }
        }
    }
}

impl Tree {
    /// The files under `dir`, without following a link. A top-level `.git` is
    /// skipped: it is the checkout's, not the repo's.
    pub fn read(dir: &Path) -> anyhow::Result<Tree> {
        Tree::read_keeping(dir, MAX_TREE_BODY_BYTES)
    }

    /// [`read`](Self::read), keeping no more than `budget` bytes of entry
    /// files.
    fn read_keeping(dir: &Path, mut budget: u64) -> anyhow::Result<Tree> {
        let mut tree = Tree::default();
        let mut folders = vec![(dir.to_path_buf(), String::new())];
        while let Some((folder, prefix)) = folders.pop() {
            let items = std::fs::read_dir(&folder)
                .with_context(|| format!("reading {}", folder.display()))?;
            let mut empty = true;
            for item in items {
                let item = item.with_context(|| format!("reading {}", folder.display()))?;
                if prefix.is_empty() && item.file_name() == ".git" {
                    continue;
                }
                empty = false;
                let path = format!("{prefix}{}", item.file_name().to_string_lossy());
                let on_disk = item.path();
                let kind = item.file_type()?;
                let node = if item.file_name().to_str().is_none() {
                    Node::BadName
                } else if kind.is_symlink() {
                    Node::Link(std::fs::read_link(&on_disk)?)
                } else if kind.is_dir() {
                    folders.push((on_disk, format!("{path}/")));
                    continue;
                } else if kind.is_file() {
                    read_file(&on_disk, entry_stem(&path).is_some(), &mut budget)
                        .with_context(|| format!("reading {}", on_disk.display()))?
                } else {
                    Node::Special
                };
                tree.insert(path, node);
            }
            if empty && !prefix.is_empty() {
                tree.insert(prefix.trim_end_matches('/').to_string(), Node::EmptyFolder);
            }
        }
        Ok(tree)
    }

    /// Two names that show as one path (a name that is not UTF-8 shows as lossy
    /// text) must not fold into one node: the path is refused.
    fn insert(&mut self, path: String, node: Node) {
        let node = if self.files.contains_key(&path) {
            Node::BadName
        } else {
            node
        };
        self.files.insert(path, node);
    }
}

/// The regular file at `path`: its SHA-256, and its bytes when `keep` and they
/// fit in `budget`, which they then take from. No more than
/// [`MAX_ENTRY_BYTES`] and one byte is read: a longer file is
/// [`Node::TooBig`], never compared by a hash of part of it.
fn read_file(path: &Path, keep: bool, budget: &mut u64) -> std::io::Result<Node> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let size = bytes.len() as u64;
    if size > MAX_ENTRY_BYTES {
        return Ok(Node::TooBig);
    }
    let keep = keep && size <= *budget;
    if keep {
        *budget -= size;
    }
    Ok(Node::File {
        sha256: sha256_hex(&bytes),
        body: keep.then_some(bytes),
    })
}

impl NewVersion {
    /// The entry a consumer would read for this version, listed by no source.
    pub fn entry(&self) -> Entry {
        let file = EntryFile {
            name: self.name.clone(),
            github: self.github.clone(),
            versions: Vec::new(),
        };
        file.entry(&self.version, "")
    }

    /// What a finding about this version names: its entry file, the package and
    /// the version as the entry file spells it.
    fn at(&self) -> (String, String, String) {
        (
            self.path.clone(),
            self.name.clone(),
            self.version.version.clone(),
        )
    }

    /// R4: the version's `url` must be a release file of its `github` on the
    /// site at `web`.
    pub fn url_finding(&self, web: &str) -> Option<Finding> {
        check_index_url(&self.entry(), web)
            .err()
            .map(|_| Finding::UrlNotReleaseFile {
                path: self.path.clone(),
                name: self.name.clone(),
                version: self.version.version.clone(),
                url: self.version.url.clone(),
                github: self.github.clone(),
            })
    }

    /// R5: the file at the version's `url` has SHA-256 `actual`.
    pub fn file_differs(&self, actual: &str) -> Finding {
        let (path, name, version) = self.at();
        Finding::FileDiffers {
            path,
            name,
            version,
            actual: actual.to_string(),
            entry: self.version.sha256.clone(),
        }
    }

    /// R5: the file could not be downloaded, for `error`.
    pub fn not_downloaded(&self, error: String) -> Finding {
        let (path, name, version) = self.at();
        Finding::NotDownloaded {
            path,
            name,
            version,
            error,
        }
    }

    /// The file is no package lvpm reads, for `error`.
    pub fn not_a_package(&self, error: String) -> Finding {
        let (path, name, version) = self.at();
        Finding::NotAPackage {
            path,
            name,
            version,
            error,
        }
    }

    /// R5b: the first of name, version, requires and labview that `package`
    /// (the file, as lvpm reads it) says differently from the entry. Versions
    /// are compared as lvpm reads them, requirements in any order, and the
    /// gate as the number it is.
    pub fn package_finding(&self, package: &Entry) -> Option<Finding> {
        let entry = self.entry();
        let said = FileVersion::of_package(package, &self.version.url, &self.version.sha256);
        let none = || "none".to_string();
        let joined = |list: &[String]| match list.is_empty() {
            true => none(),
            false => list.join(", "),
        };
        let sorted = |requirements: &[Requirement]| {
            let mut texts: Vec<String> = requirements.iter().map(requirement_text).collect();
            texts.sort();
            texts
        };
        let (field, actual, listed) = [
            (
                entry.name != package.name,
                "name",
                package.name.clone(),
                entry.name.clone(),
            ),
            (
                entry.version != package.version,
                "version",
                said.version.clone(),
                self.version.version.clone(),
            ),
            (
                sorted(&entry.requires) != sorted(&package.requires),
                "requires",
                joined(&said.requires),
                joined(&self.version.requires),
            ),
            (
                entry.lv_min != package.lv_min,
                "labview",
                said.labview.clone().unwrap_or_else(none),
                self.version.labview.clone().unwrap_or_else(none),
            ),
        ]
        .into_iter()
        .find_map(|(differs, field, actual, listed)| differs.then_some((field, actual, listed)))?;
        let (path, name, version) = self.at();
        Some(Finding::PackageDiffers {
            path,
            name,
            version,
            field,
            actual,
            entry: listed,
        })
    }

    /// R6: the file is not attested, for `reason`.
    pub fn not_attested(&self, reason: NotAttested) -> Finding {
        let (path, name, version) = self.at();
        Finding::NotAttested {
            path,
            name,
            version,
            reason,
        }
    }

    /// R6b: the file is attested by the repo at `subject`.
    pub fn attested_by_other(&self, subject: String) -> Finding {
        let (path, name, version) = self.at();
        Finding::AttestedByOther {
            path,
            name,
            version,
            subject,
            github: self.github.clone(),
        }
    }
}

/// A table of the file, with the prefix its keys carry in an error.
struct Scope<'a> {
    table: &'a toml::Table,
    prefix: String,
}

impl<'a> Scope<'a> {
    fn new(table: &'a toml::Table, prefix: String) -> Self {
        Scope { table, prefix }
    }

    fn error(&self, key: &str, problem: Problem) -> EntryFileError {
        EntryFileError::Field {
            path: format!("{}{key}", self.prefix),
            problem,
        }
    }

    /// A key not in `known` is refused. It is untrusted text, so it is cut and
    /// escaped before it reaches an error.
    fn reject_unknown(&self, known: &[&str]) -> Result<(), EntryFileError> {
        match self.table.keys().find(|key| !known.contains(&key.as_str())) {
            None => Ok(()),
            Some(key) => {
                let shown: String = key
                    .chars()
                    .take(MAX_SHOWN_KEY_CHARS)
                    .flat_map(char::escape_debug)
                    .collect();
                Err(self.error(&shown, Problem::UnknownKey))
            }
        }
    }

    fn required_str(&self, key: &str) -> Result<&'a str, EntryFileError> {
        match self.table.get(key) {
            Some(toml::Value::String(text)) => Ok(text),
            Some(_) => Err(self.error(key, Problem::WrongType(A_STRING))),
            None => Err(self.error(key, Problem::Missing)),
        }
    }

    fn owner_repo(&self, key: &str) -> Result<&'a str, EntryFileError> {
        let text = self.required_str(key)?;
        split_owner_repo(text)
            .map(|_| text)
            .ok_or_else(|| self.error(key, Problem::NotOwnerRepo))
    }

    fn file_version(&self) -> Result<FileVersion, EntryFileError> {
        Ok(FileVersion {
            version: self.version()?,
            url: self.required_str(KEY_URL)?.to_string(),
            sha256: self.sha256()?,
            labview: self.labview()?,
            requires: self.requires()?,
        })
    }

    /// The `version` key: a whole version, as lvpm reads them.
    fn version(&self) -> Result<String, EntryFileError> {
        let version = self.required_str(KEY_VERSION)?;
        match is_version(version) {
            true => Ok(version.to_string()),
            false => Err(self.error(KEY_VERSION, Problem::Unreadable)),
        }
    }

    /// The `sha256` key, as lower-case hex.
    fn sha256(&self) -> Result<String, EntryFileError> {
        let hex = self.required_str(KEY_SHA256)?;
        if hex.len() == SHA256_HEX_DIGITS && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(hex.to_ascii_lowercase())
        } else {
            Err(self.error(KEY_SHA256, Problem::NotSha256))
        }
    }

    /// The `labview` key, if it has one: the gate as lvpm writes it, like
    /// `>=20.0`, and nothing after it.
    fn labview(&self) -> Result<Option<String>, EntryFileError> {
        let written =
            |gate: &str| parse_lv_gate(gate).is_some_and(|min| format!(">={min:?}") == gate);
        match self.optional_str(KEY_LABVIEW)? {
            Some(gate) if !written(gate) => Err(self.error(KEY_LABVIEW, Problem::Unreadable)),
            gate => Ok(gate.map(str::to_string)),
        }
    }

    /// The `requires` key: each item one requirement, like `bar >=2`.
    fn requires(&self) -> Result<Vec<String>, EntryFileError> {
        let items = self.str_list(KEY_REQUIRES)?;
        let installable = |item: &String| {
            parse_requirement(item).is_some_and(|r| listable_key(&r.name).is_some())
        };
        if items.iter().all(installable) {
            Ok(items)
        } else {
            Err(self.error(KEY_REQUIRES, Problem::Unreadable))
        }
    }

    fn optional_str(&self, key: &str) -> Result<Option<&'a str>, EntryFileError> {
        match self.table.get(key) {
            None => Ok(None),
            Some(toml::Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(self.error(key, Problem::WrongType(A_STRING))),
        }
    }

    fn str_list(&self, key: &str) -> Result<Vec<String>, EntryFileError> {
        let wrong = || self.error(key, Problem::WrongType(A_LIST_OF_STRINGS));
        match self.table.get(key) {
            None => Ok(Vec::new()),
            Some(toml::Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_string).ok_or_else(wrong))
                .collect(),
            Some(_) => Err(wrong()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn file(extra: &str) -> String {
        format!(
            r#"name = "foo"
github = "a/x"
[[version]]
version = "1.0.0"
url = "https://github.com/a/x/releases/download/1.0.0/foo-1.0.0.lvpkg"
sha256 = "{SHA_A}"
[[version]]
version = "1.1.0"
url = "https://github.com/a/x/releases/download/1.1.0/foo-1.1.0.lvpkg"
sha256 = "{SHA_B}"
requires = ["bar >=2"]
labview = ">=20.0"
{extra}"#
        )
    }

    #[test]
    fn an_entry_file_becomes_one_entry_per_version() {
        let entries = EntryFile::parse("foo", &file(""))
            .unwrap()
            .entries("github-index:o/r");

        assert_eq!(entries.len(), 2);
        let pins: Vec<_> = entries
            .iter()
            .map(|e| {
                (
                    e.name.as_str(),
                    e.source.as_str(),
                    e.index_pin.clone().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            pins,
            [
                (
                    "foo",
                    "github-index:o/r",
                    IndexPin {
                        sha256: SHA_A.into(),
                        publisher: "a/x".into()
                    }
                ),
                (
                    "foo",
                    "github-index:o/r",
                    IndexPin {
                        sha256: SHA_B.into(),
                        publisher: "a/x".into()
                    }
                ),
            ]
        );
        assert_eq!(entries[1].version, Version::parse("1.1.0"));
        let requires: Vec<_> = entries[1]
            .requires
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(requires, ["bar"]);
        assert_eq!(entries[1].requires[0].min, Some(Version::parse("2")));
        assert_eq!(entries[1].lv_min, Some(20.0));
        assert!(entries[0].requires.is_empty() && entries[0].lv_min.is_none());
    }

    #[test]
    fn an_entry_file_that_is_not_valid_does_not_parse() {
        let sha = format!("sha256 = \"{SHA_A}\"");
        let rows: [(&str, String, &str); 8] = [
            (
                "an unknown top-level key",
                format!(
                    "own = 1
{}",
                    file("")
                ),
                "own",
            ),
            (
                "a version without sha256",
                file("").replace(&sha, ""),
                "sha256",
            ),
            (
                "a sha256 that is not 64 hex digits",
                file("").replace(SHA_A, "abc"),
                "sha256",
            ),
            (
                "name bar",
                file("").replace("name = \"foo\"", "name = \"bar\""),
                "name",
            ),
            (
                "version 1.0.0 listed twice",
                file("").replace("\"1.1.0\"", "\"1.0.0\""),
                "version",
            ),
            ("github a", file("").replace("a/x\"", "a\""), "github"),
            (
                "labview twenty",
                file("").replace(">=20.0", "twenty"),
                "labview",
            ),
            (
                "requires bar >=?",
                file("").replace("bar >=2", "bar >=?"),
                "requires",
            ),
        ];
        for (change, text, key) in rows {
            let why = EntryFile::parse("foo", &text).expect_err(change);
            assert!(why.to_string().contains(key), "{change}: {why}");
        }
    }

    #[test]
    fn write_then_read_gives_the_same_entries() {
        let version = FileVersion {
            version: "1.0.0".into(),
            url: "https://github.com/a/x/releases/download/1.0.0/foo-1.0.0.lvpkg".into(),
            sha256: SHA_A.into(),
            labview: None,
            requires: Vec::new(),
        };
        let written = EntryFile {
            name: "foo".into(),
            github: "a/x".into(),
            versions: vec![version],
        };

        let text = written.write();

        assert_eq!(EntryFile::parse("foo", &text).unwrap(), written);
        assert!(
            !text.contains(KEY_LABVIEW) && !text.contains(KEY_REQUIRES),
            "{text}"
        );
    }

    #[test]
    fn a_key_of_the_wrong_type_is_named_with_its_version() {
        let head = "name = \"foo\"
github = \"a/x\"
";
        let rows = [
            (
                "name = 1
github = \"a/x\"
"
                .to_string(),
                "name: must be a string",
            ),
            (
                format!(
                    "{head}version = \"1.0\"
"
                ),
                "version: must be a list of tables",
            ),
            (
                format!(
                    "{head}version = [1]
"
                ),
                "version[1]: must be a table",
            ),
        ];
        for (text, expected) in rows {
            let why = EntryFile::parse("foo", &text).expect_err(expected);
            assert_eq!(why.to_string(), expected);
        }
        let text = file("").replace("\">=20.0\"", "20");
        let why = EntryFile::parse("foo", &text).unwrap_err();
        assert_eq!(why.to_string(), "version[2].labview: must be a string");
    }

    #[test]
    fn an_error_names_the_version_by_position_and_never_quotes_the_file() {
        let secret = "hunter2-do-not-echo";
        let bad_sha = file("").replace(SHA_B, secret);
        let why = EntryFile::parse("foo", &bad_sha).unwrap_err();
        assert_eq!(why.to_string(), "version[2].sha256: must be 64 hex digits");

        let long_key = format!("{secret}\n{}", "k".repeat(100));
        let why = EntryFile::parse(
            "foo",
            &format!("\"{}\" = 1\n{}", long_key.escape_debug(), file("")),
        )
        .unwrap_err();
        let shown = why.to_string();
        assert!(
            shown.ends_with(": unknown key") && shown.len() < 80,
            "{shown}"
        );
        assert!(!shown.contains('\n'), "{shown}");

        let why = EntryFile::parse("foo", &format!("name = \"foo\"\n{secret} ===\n")).unwrap_err();
        assert_eq!(why, EntryFileError::Syntax { line: 2 });
        assert!(!why.to_string().contains(secret));
    }

    #[test]
    fn versions_are_the_same_when_lvpm_reads_them_alike() {
        let text = file("").replace("\"1.1.0\"", "\"1.0.0.0\"");
        let why = EntryFile::parse("foo", &text).unwrap_err();
        assert_eq!(why.to_string(), "version[2].version: listed twice");

        let mut parsed = EntryFile::parse("foo", &file("")).unwrap();
        let new = |version: &str| FileVersion {
            version: version.into(),
            url: "u".into(),
            sha256: SHA_A.into(),
            labview: None,
            requires: Vec::new(),
        };
        assert_eq!(parsed.add_version(new("1.0.0.0")), Err(VersionListed));
        assert_eq!(parsed.add_version(new("1.2.0")), Ok(()));
        assert_eq!(parsed.versions.len(), 3);
    }

    #[test]
    fn write_orders_the_keys_and_reads_back_whole() {
        let parsed = EntryFile::parse("foo", &file("")).unwrap();

        let text = parsed.write();

        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {text}"))
        };
        let second = text.rfind("[[version]]").unwrap();
        let order = [
            at("name"),
            at("github"),
            at("[[version]]"),
            at("version = \"1.0.0\""),
            second,
        ];
        assert!(order.is_sorted(), "{text}");
        let keys = ["version =", "url =", "sha256 =", "labview =", "requires ="];
        let positions: Vec<usize> = keys
            .iter()
            .map(|k| second + text[second..].find(k).unwrap())
            .collect();
        assert!(positions.is_sorted(), "{text}");
        assert_eq!(EntryFile::parse("foo", &text).unwrap(), parsed);
    }

    #[test]
    fn a_name_is_matched_to_its_file_key_by_case() {
        let text = file("").replace("name = \"foo\"", "name = \"Foo\"");
        assert_eq!(EntryFile::parse("foo", &text).unwrap().name, "Foo");
    }

    #[test]
    fn an_upper_case_sha256_is_read_as_lower_case() {
        let text = file("").replace(SHA_A, &SHA_A.to_uppercase());
        let parsed = EntryFile::parse("foo", &text).unwrap();
        assert_eq!(parsed.versions[0].sha256, SHA_A);
    }

    #[test]
    fn owner_and_repo_follow_githubs_names() {
        for bad in ["a", "/x", "a/", "a/x/y", "../x", "a/..", "a/x?y", "a b/x"] {
            let text = file("").replace("a/x\"", &format!("{bad}\""));
            let why = EntryFile::parse("foo", &text).unwrap_err();
            assert_eq!(why.to_string(), "github: must be owner/repo", "{bad}");
        }
    }

    #[test]
    fn a_package_is_listed_as_the_entry_it_is_read_back_as() {
        let package = Entry {
            name: "foo".into(),
            version: Version::parse("1.2.1.16"),
            requires: parse_requires("bar>=2.0,baz>1.5,qux"),
            lv_min: parse_lv_gate("LabVIEW>=24.0"),
            ..Entry::default()
        };

        let listed = FileVersion::of_package(&package, "u", SHA_A);

        assert_eq!(listed.version, "1.2.1.16");
        assert_eq!(listed.labview.as_deref(), Some(">=24.0"));
        assert_eq!(listed.requires, ["bar >=2.0", "baz >1.5", "qux"]);
        let file = EntryFile {
            name: "foo".into(),
            github: "a/x".into(),
            versions: vec![listed],
        };
        let back = EntryFile::parse("foo", &file.write()).unwrap().entries("");
        assert_eq!(back[0].lv_min, Some(24.0));
        let floors: Vec<_> = back[0]
            .requires
            .iter()
            .map(|r| (r.name.as_str(), r.min_exclusive, r.min.is_some()))
            .collect();
        assert_eq!(
            floors,
            [
                ("bar", false, true),
                ("baz", true, true),
                ("qux", false, false)
            ]
        );
    }

    #[test]
    fn a_requirement_is_one_name_with_an_optional_readable_floor() {
        for bad in ["", "bar >=", "bar >=?", ">=2", "a >=1, b >=1"] {
            let text = file("").replace("bar >=2", bad);
            let why = EntryFile::parse("foo", &text).unwrap_err();
            assert_eq!(
                why.to_string(),
                "version[2].requires: cannot be read",
                "{bad:?}"
            );
        }
        let text = file("").replace("\"bar >=2\"", "\"bar\", \"baz >1.5\"");
        let entries = EntryFile::parse("foo", &text)
            .unwrap()
            .entries("github-index:o/r");
        let floors: Vec<_> = entries[1]
            .requires
            .iter()
            .map(|r| (r.name.as_str(), r.min_exclusive))
            .collect();
        assert_eq!(floors, [("bar", false), ("baz", true)]);
    }

    const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const WEB: &str = "https://github.com";

    fn entry_text(name: &str, github: &str, versions: &[(&str, &str)]) -> String {
        let mut text = format!("name = \"{name}\"\ngithub = \"{github}\"\n");
        for (version, sha) in versions {
            text.push_str(&format!(
                "[[version]]\nversion = \"{version}\"\n\
                 url = \"{WEB}/{github}/releases/download/{version}/{name}.lvpkg\"\n\
                 sha256 = \"{sha}\"\n"
            ));
        }
        text
    }

    impl Tree {
        fn put(&mut self, path: &str, bytes: &[u8]) {
            let body = entry_stem(path).map(|_| bytes.to_vec());
            let sha256 = crate::cache::sha256_hex(bytes);
            self.files.insert(path.into(), Node::File { sha256, body });
        }

        fn put_entry(&mut self, path: &str, name: &str, github: &str, versions: &[(&str, &str)]) {
            self.put(path, entry_text(name, github, versions).as_bytes());
        }

        fn too_big(&mut self, path: &str) {
            self.files.insert(path.into(), Node::TooBig);
        }

        fn link(&mut self, path: &str) {
            self.files
                .insert(path.into(), Node::Link("elsewhere".into()));
        }

        fn remove(&mut self, path: &str) {
            self.files.remove(path);
        }
    }

    /// `packages/foo.toml` of `a/x` with 1.0.0 and 1.0.1.
    fn base() -> Tree {
        let mut tree = Tree::default();
        tree.put_entry(
            "packages/foo.toml",
            "foo",
            "a/x",
            &[("1.0.0", SHA_A), ("1.0.1", SHA_B)],
        );
        tree.put("README.md", b"an index");
        tree
    }

    /// The finding as the scenarios write it: rule, then its fields.
    fn shown(finding: &Finding) -> String {
        match finding {
            Finding::OutsideEntries { path } => format!("R1 {path}"),
            Finding::WrongFileName { path, name } => format!("R1b {path} {name}"),
            Finding::OwnerChanged { name, old, new, .. } => format!("R2 {name} {old} {new}"),
            Finding::VersionChanged {
                name,
                version,
                field,
                old,
                new,
                ..
            } if *field == "sha256" => format!("R3 {name} {version} {old} {new}"),
            Finding::VersionChanged {
                name,
                version,
                field,
                old,
                new,
                ..
            } => format!("R3 {name} {version} {field} {old} -> {new}"),
            Finding::VersionRemoved { name, version, .. } => format!("R3b {name} {version}"),
            Finding::UrlNotReleaseFile { name, version, .. } => format!("R4 {name} {version}"),
            Finding::Unreadable { path, .. } => format!("parse {path}"),
            other => panic!("a rule that needs the network: {other:?}"),
        }
    }

    fn found(head: &Tree) -> Vec<String> {
        diff(&base(), head).findings.iter().map(shown).collect()
    }

    #[test]
    fn a_pure_rule_refuses() {
        type Change = fn(&mut Tree);
        let rows: [(&str, Change, Vec<String>); 16] = [
            (
                "adds notes.md",
                |t| t.put("notes.md", b"hi"),
                vec!["R1 notes.md".into()],
            ),
            (
                "adds packages/sub/x.toml",
                |t| t.put("packages/sub/x.toml", b"hi"),
                vec!["R1 packages/sub/x.toml".into()],
            ),
            (
                "adds packages/x.txt",
                |t| t.put("packages/x.txt", b"hi"),
                vec!["R1 packages/x.txt".into()],
            ),
            (
                "adds packages/bar.toml as a symlink",
                |t| t.link("packages/bar.toml"),
                vec!["R1 packages/bar.toml".into()],
            ),
            (
                "adds packages/Foo2.toml for name foo2",
                |t| t.put_entry("packages/Foo2.toml", "foo2", "b/y", &[("1.0.0", SHA_A)]),
                vec!["R1b packages/Foo2.toml foo2".into()],
            ),
            (
                "changes foo's github to b/x",
                |t| {
                    t.put_entry(
                        "packages/foo.toml",
                        "foo",
                        "b/x",
                        &[("1.0.0", SHA_A), ("1.0.1", SHA_B)],
                    );
                },
                vec!["R2 foo a/x b/x".into()],
            ),
            (
                "changes foo's github to A/X",
                |t| {
                    let text = entry_text("foo", "a/x", &[("1.0.0", SHA_A), ("1.0.1", SHA_B)])
                        .replacen("github = \"a/x\"", "github = \"A/X\"", 1);
                    t.put("packages/foo.toml", text.as_bytes());
                },
                vec![],
            ),
            (
                "changes foo 1.0.0's sha256",
                |t| {
                    t.put_entry(
                        "packages/foo.toml",
                        "foo",
                        "a/x",
                        &[("1.0.0", SHA_C), ("1.0.1", SHA_B)],
                    );
                },
                vec![format!("R3 foo 1.0.0 {SHA_A} {SHA_C}")],
            ),
            (
                "writes foo 1.0.0 as 1.0.0.0 with another sha256",
                |t| {
                    t.put_entry(
                        "packages/foo.toml",
                        "foo",
                        "a/x",
                        &[("1.0.0.0", SHA_C), ("1.0.1", SHA_B)],
                    );
                },
                vec![format!("R3 foo 1.0.0 {SHA_A} {SHA_C}")],
            ),
            (
                "writes foo 1.0.0 as 1.0.0.0, same sha256",
                |t| {
                    let text = entry_text("foo", "a/x", &[("1.0.0", SHA_A), ("1.0.1", SHA_B)])
                        .replacen("version = \"1.0.0\"", "version = \"1.0.0.0\"", 1);
                    t.put("packages/foo.toml", text.as_bytes());
                },
                vec!["R3 foo 1.0.0 version 1.0.0 -> 1.0.0.0".into()],
            ),
            (
                "adds packages/bar.toml with no version",
                |t| {
                    t.put(
                        "packages/bar.toml",
                        b"name = \"bar\"
github = \"b/y\"
",
                    )
                },
                vec!["parse packages/bar.toml".into()],
            ),
            (
                "adds a requirement to foo 1.0.0, same sha256",
                |t| {
                    let text = entry_text("foo", "a/x", &[("1.0.0", SHA_A), ("1.0.1", SHA_B)])
                        .replacen(
                            &format!("sha256 = \"{SHA_A}\"\n"),
                            &format!("sha256 = \"{SHA_A}\"\nrequires = [\"bar >=2\"]\n"),
                            1,
                        );
                    t.put("packages/foo.toml", text.as_bytes());
                },
                vec!["R3 foo 1.0.0 requires none -> bar >=2".into()],
            ),
            (
                "points foo 1.0.0 at another release file, same sha256",
                |t| {
                    let text = entry_text("foo", "a/x", &[("1.0.0", SHA_A), ("1.0.1", SHA_B)])
                        .replacen("/1.0.0/foo.lvpkg", "/1.0.0/other.lvpkg", 1);
                    t.put("packages/foo.toml", text.as_bytes());
                },
                vec![format!(
                    "R3 foo 1.0.0 url {WEB}/a/x/releases/download/1.0.0/foo.lvpkg -> \
                     {WEB}/a/x/releases/download/1.0.0/other.lvpkg"
                )],
            ),
            (
                "removes foo 1.0.0",
                |t| t.put_entry("packages/foo.toml", "foo", "a/x", &[("1.0.1", SHA_B)]),
                vec!["R3b foo 1.0.0".into()],
            ),
            (
                "deletes packages/foo.toml",
                |t| t.remove("packages/foo.toml"),
                vec!["R3b foo 1.0.0".into(), "R3b foo 1.0.1".into()],
            ),
            (
                "adds an entry file of 2 MiB",
                |t| t.too_big("packages/big.toml"),
                vec!["parse packages/big.toml".into()],
            ),
        ];
        for (change, apply, expected) in rows {
            let mut head = base();
            apply(&mut head);
            assert_eq!(found(&head), expected, "{change}");
        }
    }

    #[test]
    fn a_file_that_does_not_read_is_one_finding_that_quotes_nothing() {
        let secret = "hunter2-do-not-echo";
        let mut head = base();
        head.put("packages/bar.toml", format!("{secret} ===").as_bytes());
        head.put("packages/baz.toml", &[0xff, 0xfe]);

        let found = diff(&base(), &head).findings;

        let paths: Vec<_> = found
            .iter()
            .map(|f| match f {
                Finding::Unreadable { path, error } => {
                    assert!(!error.contains(secret), "{error}");
                    path.as_str()
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(paths, ["packages/bar.toml", "packages/baz.toml"]);
    }

    #[test]
    fn a_file_swapped_for_a_link_is_refused_and_its_versions_are_gone() {
        let mut head = base();
        head.link("packages/foo.toml");

        assert_eq!(
            found(&head),
            ["R1 packages/foo.toml", "R3b foo 1.0.0", "R3b foo 1.0.1"]
        );
    }

    #[test]
    fn a_head_equal_to_the_base_has_nothing_to_check() {
        assert_eq!(diff(&base(), &base()), Diff::default());
    }

    #[test]
    fn a_changed_or_removed_file_outside_the_entries_is_refused() {
        let mut head = base();
        head.put("README.md", b"an index, edited");
        assert_eq!(found(&head), ["R1 README.md"]);

        head.remove("README.md");
        assert_eq!(found(&head), ["R1 README.md"]);
    }

    #[test]
    fn a_file_the_pr_leaves_alone_is_not_read() {
        let mut main = base();
        main.put("packages/old.toml", b"not toml ===");

        assert_eq!(diff(&main, &main.clone()), Diff::default());
    }

    #[test]
    fn a_name_that_cannot_be_a_file_name_is_refused_even_when_the_file_is_named_for_it() {
        let mut head = base();
        head.put_entry("packages/a b.toml", "a b", "b/y", &[("1.0.0", SHA_A)]);

        assert_eq!(found(&head), ["R1b packages/a b.toml a b"]);
    }

    #[test]
    fn several_new_versions_are_each_checked() {
        let mut head = base();
        head.put_entry(
            "packages/foo.toml",
            "foo",
            "a/x",
            &[
                ("1.0.0", SHA_A),
                ("1.0.1", SHA_B),
                ("1.1.0", SHA_C),
                ("1.2.0", SHA_C),
            ],
        );

        let found = diff(&base(), &head);

        assert_eq!(found.findings, []);
        let versions: Vec<_> = found
            .new_versions
            .iter()
            .map(|n| {
                (
                    n.name.as_str(),
                    n.version.version.as_str(),
                    n.github.as_str(),
                )
            })
            .collect();
        assert_eq!(versions, [("foo", "1.1.0", "a/x"), ("foo", "1.2.0", "a/x")]);
        assert_eq!(found.new_versions[0].path, "packages/foo.toml");
    }

    #[test]
    fn a_new_name_is_all_new_versions() {
        let mut head = base();
        head.put_entry("packages/bar.toml", "bar", "b/y", &[("1.0.0", SHA_A)]);

        let found = diff(&base(), &head);

        assert_eq!(found.findings, []);
        assert_eq!(found.new_versions.len(), 1);
        assert_eq!(found.new_versions[0].github, "b/y");
    }

    #[test]
    fn the_versions_of_a_refused_file_are_not_checked() {
        let mut head = base();
        head.put_entry(
            "packages/foo.toml",
            "foo",
            "b/x",
            &[("1.0.0", SHA_A), ("1.1.0", SHA_C)],
        );
        head.put_entry("packages/Bar.toml", "bar", "b/y", &[("1.0.0", SHA_A)]);

        let found = diff(&base(), &head);

        assert_eq!(found.new_versions, []);
        assert_eq!(found.findings.len(), 3, "{:?}", found.findings);
    }

    #[test]
    fn a_url_is_a_release_file_of_the_owner_or_it_is_refused() {
        let rows = [
            ("{web}/a/x/releases/download/1.1.0/foo.lvpkg", true),
            (
                "https://example.com/a/x/releases/download/1.1.0/foo.lvpkg",
                false,
            ),
            ("{web}/a/xy/releases/download/1.1.0/foo.lvpkg", false),
            (
                "{web}/a/x/releases/download/1.1.0/../../../b/y/foo.lvpkg",
                false,
            ),
            (
                "{web}/a/x/releases/download/1.1.0%2f..%2f..%2fb/foo.lvpkg",
                false,
            ),
            ("{web}/a/x/releases/download/1.1.0/sub/foo.lvpkg", false),
            ("{web}/A/X/releases/download/1.1.0/foo.lvpkg", true),
        ];
        for (url, accepted) in rows {
            let url = url.replace("{web}", WEB);
            let new = NewVersion {
                path: "packages/foo.toml".into(),
                name: "foo".into(),
                github: "a/x".into(),
                version: FileVersion {
                    version: "1.1.0".into(),
                    url: url.clone(),
                    sha256: SHA_A.into(),
                    labview: None,
                    requires: Vec::new(),
                },
            };

            let finding = new.url_finding(WEB);

            assert_eq!(finding.is_none(), accepted, "{url}");
            if !accepted {
                assert_eq!(
                    finding,
                    Some(Finding::UrlNotReleaseFile {
                        path: "packages/foo.toml".into(),
                        name: "foo".into(),
                        version: "1.1.0".into(),
                        url,
                        github: "a/x".into(),
                    })
                );
            }
        }
    }

    /// The new version `1.1.0` of foo, listed with `requires` and `labview`.
    fn listed(version: &str, labview: Option<&str>, requires: &[&str]) -> NewVersion {
        NewVersion {
            path: "packages/foo.toml".into(),
            name: "foo".into(),
            github: "a/x".into(),
            version: FileVersion {
                version: version.into(),
                url: format!("{WEB}/a/x/releases/download/{version}/foo.lvpkg"),
                sha256: SHA_A.into(),
                labview: labview.map(str::to_string),
                requires: requires.iter().map(|r| r.to_string()).collect(),
            },
        }
    }

    /// What the file says of itself.
    fn package(name: &str, version: &str, labview: Option<&str>, requires: &str) -> Entry {
        Entry {
            name: name.into(),
            version: Version::parse(version),
            requires: parse_requires(requires),
            lv_min: labview.and_then(parse_lv_gate),
            ..Entry::default()
        }
    }

    #[test]
    fn a_package_that_says_what_the_entry_says_is_accepted_however_it_is_spelled() {
        let new = listed("1.1.0", Some(">=20.0"), &["baz >1.5", "bar >=2"]);
        // The version as lvpm reads it, the requirements in another order, and the gate as a number.
        let same = package("foo", "1.1.0.0", Some("LabVIEW>=20"), "bar>=2,baz>1.5");

        assert_eq!(new.package_finding(&same), None);
    }

    #[test]
    fn a_package_that_says_another_field_is_refused_naming_both_texts() {
        let new = listed("1.1.0", Some(">=20.0"), &["bar >=2"]);
        let said = |name, version, labview, requires| {
            let found = new.package_finding(&package(name, version, labview, requires));
            let Some(Finding::PackageDiffers {
                field,
                actual,
                entry,
                ..
            }) = found
            else {
                panic!("{found:?}");
            };
            (field, actual, entry)
        };
        let row = |field, actual: &str, entry: &str| (field, actual.to_string(), entry.to_string());

        let ok = ("foo", "1.1.0", Some("LabVIEW>=20.0"), "bar>=2");
        assert_eq!(said("bar", ok.1, ok.2, ok.3), row("name", "bar", "foo"));
        assert_eq!(
            said(ok.0, "1.1.1", ok.2, ok.3),
            row("version", "1.1.1", "1.1.0")
        );
        assert_eq!(
            said(ok.0, ok.1, ok.2, "bar>=3,qux"),
            row("requires", "bar >=3, qux", "bar >=2")
        );
        assert_eq!(
            said(ok.0, ok.1, None, ok.3),
            row("labview", "none", ">=20.0")
        );
        assert_eq!(
            said(ok.0, ok.1, Some("LabVIEW>=21.0"), ok.3),
            row("labview", ">=21.0", ">=20.0")
        );
    }

    #[test]
    fn the_first_field_that_differs_is_the_one_named() {
        let new = listed("1.1.0", None, &[]);

        let found = new.package_finding(&package("bar", "9", Some("LabVIEW>=21"), "x"));

        assert!(
            matches!(found, Some(Finding::PackageDiffers { field: "name", .. })),
            "{found:?}"
        );
    }

    /// Whether the platform let the test make a link (Windows needs a
    /// privilege for it).
    fn make_link(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(target, link);
        made.is_ok()
    }

    #[test]
    fn a_checkout_is_read_without_following_links_or_keeping_more_than_entries() {
        let dir = crate::ops::testing::scratch_dir("community-tree");
        let write = |path: &str, bytes: &[u8]| {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write("packages/foo.toml", b"name = \"foo\"");
        write(
            "packages/big.toml",
            &vec![b' '; MAX_ENTRY_BYTES as usize + 1],
        );
        write("packages/sub/x.toml", b"x");
        write(".git/config", b"x");
        write("README.md", b"an index");
        let linked = make_link(&dir.join("README.md"), &dir.join("packages/link.toml"));

        let tree = Tree::read(&dir).unwrap();

        let paths: Vec<_> = tree.files.keys().map(String::as_str).collect();
        assert!(!paths.iter().any(|p| p.starts_with(".git")), "{paths:?}");
        assert!(paths.contains(&"packages/sub/x.toml"), "{paths:?}");
        let body = |path: &str| match &tree.files[path] {
            Node::File { body, .. } => body.clone(),
            other => panic!("{path}: {other:?}"),
        };
        assert_eq!(body("packages/foo.toml"), Some(b"name = \"foo\"".to_vec()));
        assert_eq!(tree.files["packages/big.toml"], Node::TooBig);
        assert_eq!(body("README.md"), None);
        if linked {
            assert!(matches!(tree.files["packages/link.toml"], Node::Link(_)));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn checkout(tag: &str) -> (PathBuf, impl Fn(&str, &[u8])) {
        let dir = crate::ops::testing::scratch_dir(tag);
        let root = dir.clone();
        let write = move |path: &str, bytes: &[u8]| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        (dir, write)
    }

    #[test]
    fn a_file_past_the_cap_is_never_compared_by_a_hash_of_part_of_it() {
        let (dir, write) = checkout("community-big");
        write("README.md", &vec![b'a'; MAX_ENTRY_BYTES as usize + 1]);
        write(
            "packages/big.toml",
            &vec![b'a'; MAX_ENTRY_BYTES as usize + 1],
        );
        let tree = Tree::read(&dir).unwrap();
        assert_eq!(tree.files["README.md"], Node::TooBig);
        assert_eq!(tree.files["packages/big.toml"], Node::TooBig);

        // The same file in the base and the head is still refused: nothing says it is the same.
        let found = diff(&tree, &tree.clone()).findings;

        let paths: Vec<_> = found.iter().map(shown).collect();
        assert_eq!(paths, ["R1 README.md", "parse packages/big.toml"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_of_exactly_the_cap_is_read_whole() {
        let (dir, write) = checkout("community-cap");
        write("README.md", &vec![b'a'; MAX_ENTRY_BYTES as usize]);

        let tree = Tree::read(&dir).unwrap();

        assert!(matches!(tree.files["README.md"], Node::File { .. }));
        assert_eq!(diff(&tree, &tree.clone()), Diff::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn entry_bodies_past_the_budget_are_hashed_and_dropped() {
        let (dir, write) = checkout("community-budget");
        let text = entry_text("foo", "a/x", &[("1.0.0", SHA_A)]);
        write("packages/foo.toml", text.as_bytes());
        write("packages/bar.toml", text.replace("foo", "bar").as_bytes());
        let budget = text.len() as u64;

        let tree = Tree::read_keeping(&dir, budget).unwrap();

        let kept: Vec<bool> = tree
            .files
            .values()
            .map(|node| match node {
                Node::File { body, .. } => body.is_some(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(kept.iter().filter(|kept| **kept).count(), 1, "{kept:?}");
        // Unchanged, it needs no body; changed, it reads as one that is too big.
        assert_eq!(diff(&tree, &tree.clone()), Diff::default());
        let mut head = tree.clone();
        for node in head.files.values_mut() {
            if let Node::File { sha256, .. } = node {
                *sha256 = "changed".into();
            }
        }
        let found = diff(&tree, &head).findings;
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            matches!(&found[0], Finding::Unreadable { error, .. } if error.contains("larger than")),
            "{found:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_folder_is_a_node_and_is_refused_where_it_stands() {
        let (dir, write) = checkout("community-empty");
        write(
            "packages/foo.toml",
            entry_text("foo", "a/x", &[("1.0.0", SHA_A)]).as_bytes(),
        );
        std::fs::create_dir_all(dir.join("vendor/lib")).unwrap();
        std::fs::create_dir_all(dir.join("packages/bar.toml")).unwrap();
        let head = Tree::read(&dir).unwrap();

        let keys: Vec<_> = head.files.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["packages/bar.toml", "packages/foo.toml", "vendor/lib"]
        );
        assert_eq!(head.files["vendor/lib"], Node::EmptyFolder);
        let mut base = head.clone();
        base.files.remove("vendor/lib");
        base.files.remove("packages/bar.toml");

        let found: Vec<_> = diff(&base, &head).findings.iter().map(shown).collect();

        assert_eq!(found, ["R1 packages/bar.toml", "R1 vendor/lib"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_names_that_show_alike_never_fold_into_one_node() {
        let mut tree = Tree::default();
        let file = Node::File {
            sha256: "x".into(),
            body: None,
        };
        tree.insert("a\u{fffd}".into(), file.clone());
        tree.insert("a\u{fffd}".into(), file);
        assert_eq!(tree.files["a\u{fffd}"], Node::BadName);

        // Nor is one equal to itself: it was never read.
        let found: Vec<_> = diff(&tree, &tree.clone())
            .findings
            .iter()
            .map(shown)
            .collect();
        assert_eq!(found, ["R1 a\u{fffd}"]);
        let mut entry_path = Tree::default();
        entry_path
            .files
            .insert("packages/a\u{fffd}.toml".into(), Node::BadName);
        let found: Vec<_> = diff(&Tree::default(), &entry_path)
            .findings
            .iter()
            .map(shown)
            .collect();
        assert_eq!(found, ["R1 packages/a\u{fffd}.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_name_that_is_not_utf8_is_read_as_a_bad_name() {
        use std::os::unix::ffi::OsStrExt;
        let (dir, _) = checkout("community-badname");
        let name = std::ffi::OsStr::from_bytes(b"x\xff");
        std::fs::write(dir.join(name), b"x").unwrap();

        let tree = Tree::read(&dir).unwrap();

        assert_eq!(tree.files["x\u{fffd}"], Node::BadName);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_entry_file_that_lists_no_version_does_not_parse() {
        let text = "name = \"foo\"\ngithub = \"a/x\"\n";
        let why = EntryFile::parse("foo", text).unwrap_err();
        assert_eq!(why.to_string(), "lists no version");
        let mut head = base();
        head.put(
            "packages/bar.toml",
            b"name = \"bar\"\ngithub = \"b/y\"\nversion = []\n",
        );
        assert_eq!(found(&head), ["parse packages/bar.toml"]);
    }

    #[test]
    fn a_name_lvpm_cannot_install_is_not_read() {
        let bad = [
            "", ".", "..", "foo.", "con", "CON", "con.txt", "Aux.x", "nul", "prn", "com1", "COM9",
            "lpt1.a.b", "lpt9",
        ];
        for name in bad {
            assert!(!installable_name(name), "{name:?}");
        }
        for name in [
            "foo", "console", "com0", "com10", "lpt", "a.con", "foo.bar", "x-1_y",
        ] {
            assert!(installable_name(name), "{name:?}");
        }
        let text = file("").replace("name = \"foo\"", "name = \"con\"");
        let why = EntryFile::parse("con", &text).unwrap_err();
        assert_eq!(why.to_string(), "name: cannot be installed");
    }

    #[test]
    fn a_readable_file_with_a_name_lvpm_cannot_install_is_refused_for_it() {
        for (name, path) in [
            ("con", "packages/con.toml"),
            ("Foo.", "packages/foo..toml"),
            ("aux.x", "packages/aux.x.toml"),
        ] {
            let mut head = base();
            head.put_entry(path, name, "b/y", &[("1.0.0", SHA_A)]);

            let found = diff(&base(), &head);

            assert_eq!(
                found.findings,
                [Finding::NameNotInstallable {
                    path: path.into(),
                    name: name.into()
                }],
                "{name}"
            );
            assert_eq!(found.new_versions, [], "{name}");
        }
    }

    #[test]
    fn a_version_a_gate_or_a_requirement_lvpm_never_writes_does_not_parse() {
        let rows = [
            (
                "a version of letters",
                "\"1.1.0\"",
                "\"abc\"",
                "version[2].version",
            ),
            (
                "an empty version",
                "\"1.1.0\"",
                "\"\"",
                "version[2].version",
            ),
            (
                "a version with a space",
                "\"1.1.0\"",
                "\"1.1.0 \"",
                "version[2].version",
            ),
            (
                "a gate with junk after it",
                "\">=20.0\"",
                "\">=20.0 junk\"",
                "version[2].labview",
            ),
            (
                "a gate with a name",
                "\">=20.0\"",
                "\"LabVIEW>=20.0\"",
                "version[2].labview",
            ),
            (
                "a gate without a point",
                "\">=20.0\"",
                "\">=20\"",
                "version[2].labview",
            ),
            (
                "a gate written 20.00",
                "\">=20.0\"",
                "\">=20.00\"",
                "version[2].labview",
            ),
            (
                "a gate that is no floor",
                "\">=20.0\"",
                "\"<=20.0\"",
                "version[2].labview",
            ),
            (
                "a requirement with a slash",
                "\"bar >=2\"",
                "\"a/b >=2\"",
                "version[2].requires",
            ),
            (
                "a requirement with a space",
                "\"bar >=2\"",
                "\"a b >=2\"",
                "version[2].requires",
            ),
            (
                "a requirement that is a device",
                "\"bar >=2\"",
                "\"con >=2\"",
                "version[2].requires",
            ),
            (
                "a requirement of dots",
                "\"bar >=2\"",
                "\"..\"",
                "version[2].requires",
            ),
        ];
        for (what, old, new, key) in rows {
            let text = file("").replace(old, new);
            let why = EntryFile::parse("foo", &text).expect_err(what);
            assert_eq!(why.to_string(), format!("{key}: cannot be read"), "{what}");
        }
        let text = file("").replace("\"bar >=2\"", "\"bar\", \"Baz_1.x >1.5\"");
        assert!(EntryFile::parse("foo", &text).is_ok());
    }

    #[test]
    fn a_syntax_error_offset_off_a_character_is_line_one_not_a_panic() {
        assert_eq!(line_at("a\nb\nc", 4), 3);
        assert_eq!(line_at("é\nb", 1), 1);
        assert_eq!(line_at("a", 9), 1);
    }

    fn many_versions(count: usize) -> Tree {
        let versions: Vec<(String, &str)> =
            (0..count).map(|i| (format!("2.0.{i}"), SHA_C)).collect();
        let versions: Vec<(&str, &str)> = versions.iter().map(|(v, s)| (v.as_str(), *s)).collect();
        let mut head = base();
        head.put_entry("packages/many.toml", "many", "b/y", &versions);
        head
    }

    #[test]
    fn a_pr_that_adds_too_many_versions_is_refused_whole_before_any_download() {
        let at_the_limit = diff(&base(), &many_versions(MAX_NEW_VERSIONS_PER_PR));
        assert_eq!(at_the_limit.findings, []);
        assert_eq!(at_the_limit.new_versions.len(), MAX_NEW_VERSIONS_PER_PR);

        let over = diff(&base(), &many_versions(MAX_NEW_VERSIONS_PER_PR + 1));

        assert_eq!(
            over.findings,
            [Finding::TooManyVersions {
                versions: MAX_NEW_VERSIONS_PER_PR + 1
            }]
        );
        assert_eq!(over.new_versions, []);
    }

    #[test]
    fn a_pr_that_changes_too_many_entry_files_is_refused_whole_before_any_download() {
        let files = |count: usize| {
            let mut head = base();
            for i in 0..count {
                let name = format!("p{i}");
                let path = format!("packages/{name}.toml");
                head.put_entry(&path, &name, "b/y", &[("1.0.0", SHA_A)]);
            }
            head
        };
        let at_the_limit = diff(&base(), &files(MAX_ENTRY_FILES_PER_PR));
        assert_eq!(at_the_limit.findings, []);

        let over = diff(&base(), &files(MAX_ENTRY_FILES_PER_PR + 1));

        // Each file brings a version, so the versions are over as well.
        assert_eq!(
            over.findings,
            [
                Finding::TooManyVersions {
                    versions: MAX_ENTRY_FILES_PER_PR + 1
                },
                Finding::TooManyFiles {
                    files: MAX_ENTRY_FILES_PER_PR + 1
                }
            ]
        );
        assert_eq!(over.new_versions, []);
        let mut deleting = base();
        for i in 0..=MAX_ENTRY_FILES_PER_PR {
            deleting.put_entry(
                &format!("packages/p{i}.toml"),
                &format!("p{i}"),
                "b/y",
                &[("1.0.0", SHA_A)],
            );
        }
        assert_eq!(
            diff(&deleting, &base())
                .findings
                .iter()
                .filter(|f| matches!(f, Finding::TooManyFiles { .. }))
                .count(),
            0,
            "a deleted file is not a changed or added one"
        );
    }
}
