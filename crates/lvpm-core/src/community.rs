//! The community index's entry file, `packages/<key>.toml`: the name, the repo
//! that owns it, and one table per version, with everything lvpm needs to
//! resolve a version without downloading it.
//!
//! The file is untrusted input (any publisher can open a PR with one), so
//! [`EntryFile::parse`] checks every field and its errors name a key and a
//! version's position, never a value from the file. No network I/O here: the
//! file's text is handed in.

use crate::index::{Entry, IndexPin, Requirement, parse_lv_gate, parse_requires};
use crate::project::{name_key, split_owner_repo};
use crate::version::Version;

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
        let table: toml::Table = toml::from_str(text).map_err(|why| EntryFileError::Syntax {
            line: why
                .span()
                .map_or(1, |span| text[..span.start].matches('\n').count() + 1),
        })?;
        let top = Scope::new(&table, String::new());
        top.reject_unknown(&TOP_LEVEL_KEYS)?;
        let name = top.required_str(KEY_NAME)?;
        if name_key(name) != key {
            return Err(top.error(KEY_NAME, Problem::NameMismatch));
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
        for (i, value) in tables.iter().enumerate() {
            let prefix = format!("{KEY_VERSION}[{}]", i + 1);
            let toml::Value::Table(version) = value else {
                return Err(top.error(&prefix, Problem::WrongType(A_TABLE)));
            };
            let scope = Scope::new(version, format!("{prefix}."));
            scope.reject_unknown(&VERSION_KEYS)?;
            file.add_version(scope.file_version()?)
                .map_err(|VersionListed| scope.error(KEY_VERSION, Problem::Duplicate))?;
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
        let new = Version::parse(&version.version);
        if self
            .versions
            .iter()
            .any(|v| Version::parse(&v.version) == new)
        {
            return Err(VersionListed);
        }
        self.versions.push(version);
        Ok(())
    }

    /// One [`Entry`] per version, listed by `source` (`github-index:owner/repo`).
    pub fn entries(&self, source: &str) -> Vec<Entry> {
        self.versions
            .iter()
            .map(|v| Entry {
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
            })
            .collect()
    }
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
            version: self.required_str(KEY_VERSION)?.to_string(),
            url: self.required_str(KEY_URL)?.to_string(),
            sha256: self.sha256()?,
            labview: self.labview()?,
            requires: self.requires()?,
        })
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

    /// The `labview` key, if it has one: a gate with a floor, like `>=20.0`.
    fn labview(&self) -> Result<Option<String>, EntryFileError> {
        match self.optional_str(KEY_LABVIEW)? {
            Some(gate) if parse_lv_gate(gate).is_none() => {
                Err(self.error(KEY_LABVIEW, Problem::Unreadable))
            }
            gate => Ok(gate.map(str::to_string)),
        }
    }

    /// The `requires` key: each item one requirement, like `bar >=2`.
    fn requires(&self) -> Result<Vec<String>, EntryFileError> {
        let items = self.str_list(KEY_REQUIRES)?;
        if items.iter().all(|item| parse_requirement(item).is_some()) {
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
}
