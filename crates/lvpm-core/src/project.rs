//! The project manifest, `lvpm.toml`.
//!
//! ```toml
//! [project]
//! name = "NovaXC"
//! version = "0.1.0"
//! labview = ">=2025"                # the oldest LabVIEW the project is meant for
//!
//! [sources]                         # repositories beyond the public indexes
//! mirror = "http://host:8090/files" # a hosted index.vipr folder
//! team = "oci://ghcr.io/org/lvpm"   # an OCI registry prefix: asked only for packages pinned to it
//! defaults = true                   # false: only the sources listed here
//!
//! [dependencies]
//! delacor_lib_qmh = "7.1.2.1547"    # exactly this version
//! oglib_error = ">=6.0.1"           # the newest version satisfying the floor
//! jki_lib_caraya = "*"              # the newest version
//! zalign = { github = "Zuehlke/ZAlign", version = ">=1.0" }  # from that repo's releases
//! tools = { index = "mirror", version = "*" }  # from that [sources] entry, and no other
//!
//! [dev-dependencies]                # test and CI tooling, same syntax
//! jki_lib_unit_test = "*"
//!
//! [dependency-groups.ci]            # a named group, same syntax
//! ci_tools = "*"
//!
//! [nipm.dependencies]               # NI Package Manager packages, reported only
//! ni-daqmx-labview-support = "26.0"
//!
//! [policy]                          # what a GitHub release's attestation must say
//! require-signature = true          # refuse a release with no attestation
//! trusted-publishers = ["https://github.com/a/*"]  # and one from any other repository
//!
//! [application]                     # what `lvpm build --app` builds
//! project = "app/Main.lvproj"       # the LabVIEW project, relative to lvpm.toml
//! build-spec = "My Application"     # a build specification of that project
//! ```
//!
//! `labview` is a minimum: a venv binds to that version or a newer one, never
//! an older one.

use crate::errors::LockError;
use crate::manifest_edit::DependencyTable;
use crate::oci::{self, FeedUrl, UrlError};
use crate::verify::Policy;
use crate::version::Version;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

/// What a pasted repo URL starts with; `github` takes `owner/repo` without it.
const GITHUB_URL_PREFIX: &str = "https://github.com/";

/// The manifest's file name — what marks a directory as a project root.
pub const FILE_NAME: &str = "lvpm.toml";

/// What a dependency line asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    /// `"1.2.3.4"` — this version and no other.
    Exact(Version),
    /// `">=1.2"` — the newest version not below this one.
    AtLeast(Version),
    /// `"*"` — the newest version there is.
    Any,
}

impl Constraint {
    pub fn parse(s: &str) -> Result<Constraint> {
        let s = s.trim();
        Ok(match s {
            "" | "*" => Constraint::Any,
            _ if s.starts_with(">=") => Constraint::AtLeast(Version::parse(s[2..].trim())),
            _ if s.starts_with(['<', '>', '=', '~', '^']) => bail!(
                "unsupported version constraint {s:?} — use an exact version, \">=x.y\" or \"*\""
            ),
            _ => Constraint::Exact(Version::parse(s)),
        })
    }

    /// Whether `version` meets this constraint.
    pub fn admits(&self, version: &Version) -> bool {
        match self {
            Constraint::Exact(v) => version == v,
            Constraint::AtLeast(v) => version >= v,
            Constraint::Any => true,
        }
    }
}

/// A package as a command names it: `name`, or `name@constraint`. The
/// constraint is as written, not yet read: [`Constraint::parse`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageArg {
    pub name: String,
    pub constraint: Option<String>,
}

impl PackageArg {
    /// The constraint, read: [`Constraint::Any`] when none was written.
    pub fn read_constraint(&self) -> Result<Constraint> {
        self.constraint
            .as_deref()
            .map_or(Ok(Constraint::Any), Constraint::parse)
    }
}

/// What is wrong with a [`PackageArg`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackageArgError {
    /// No name: `""` or `@1.0`.
    #[error("{text:?} is not a package name or name@constraint")]
    EmptyName { text: String },
    /// `name@` with nothing after the `@`.
    #[error("a constraint is expected after \"@\" in \"{name}@\"")]
    EmptyConstraint { name: String },
}

impl PackageArg {
    /// Splits at the first `@`; white space around either part is dropped.
    pub fn parse(text: &str) -> Result<PackageArg, PackageArgError> {
        let (name, constraint) = match text.split_once('@') {
            Some((name, constraint)) => (name.trim(), Some(constraint.trim())),
            None => (text.trim(), None),
        };
        if name.is_empty() {
            return Err(PackageArgError::EmptyName {
                text: text.to_string(),
            });
        }
        if constraint == Some("") {
            return Err(PackageArgError::EmptyConstraint {
                name: name.to_string(),
            });
        }
        Ok(PackageArg {
            name: name.to_string(),
            constraint: constraint.map(str::to_string),
        })
    }
}

/// The year a manifest's `labview` names, `"2024"` for LabVIEW 2024. The one
/// reading of that value: locking, a headless install and a venv all use it.
pub fn labview_year(labview: &str) -> Result<u32, LockError> {
    match labview.trim().parse::<u32>() {
        Ok(year) if year >= 2000 => Ok(year),
        _ => Err(LockError::LabviewNotAYear {
            value: labview.to_string(),
        }),
    }
}

/// What a package name is compared by: ASCII lowercase, as VIPM compares them.
/// Letters outside ASCII are not folded, so every comparison of names agrees
/// with [`same_name`]. Names are never rewritten, only compared by their key.
pub fn name_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// Whether two package names are one package: their [`name_key`]s are equal.
pub fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Constraint::Exact(v) => write!(f, "@{}", v.raw),
            Constraint::AtLeast(v) => write!(f, " >={}", v.raw),
            Constraint::Any => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// `project.name`, when stated.
    pub name: Option<String>,
    /// `project.version`, when stated. Not acted on.
    pub version: Option<String>,
    /// `project.labview`, when stated, as the year its `">="` names: the
    /// oldest LabVIEW the project is meant for. A venv binds to this version or a newer one; a global
    /// install still takes its target from `--labview-version`.
    pub labview: Option<String>,
    /// `[sources]`, in file order: a name and an index URL as written.
    pub sources: Vec<(String, String)>,
    /// Whether the public indexes are consulted too (`sources.defaults`).
    pub default_sources: bool,
    /// `sources.min-age-days`: versions younger than this many days are held
    /// back. 0 when absent.
    pub min_age_days: u32,
    /// `[dependencies]`, in file order.
    pub dependencies: Vec<(String, Constraint)>,
    /// `[dev-dependencies]`, in file order: the same syntax as `[dependencies]`.
    pub dev_dependencies: Vec<(String, Constraint)>,
    /// `[dependency-groups.<name>]`, in file order: the same syntax as
    /// `[dependencies]`. Never `dev`, which is `[dev-dependencies]`.
    pub groups: Vec<(String, Vec<(String, Constraint)>)>,
    /// The `github = "owner/repo"` of every dependency that has one, in any
    /// table, by the name the line is written under.
    pub github: BTreeMap<String, String>,
    /// The `index = "<source>"` of every dependency that has one, in any table,
    /// by the name the line is written under: the `[sources]` name that
    /// package comes from and from no other.
    pub source_pins: BTreeMap<String, String>,
    /// The name, as written, of every dependency whose line says
    /// `hooks = "run"`, in any table.
    pub hooks_allowed: BTreeSet<String>,
    /// `[nipm.dependencies]` names, for reporting only.
    pub nipm: Vec<String>,
    /// `[distribution]`, as written: `lvpm build` reads it through
    /// `distribution::validate`, and `lvpm check` for the library it names.
    pub distribution: Option<toml::Table>,
    /// `[application]`, as written, with no unknown key: `lvpm build --app`
    /// reads it through `application::validate`.
    pub application: Option<toml::Table>,
    /// `[policy]`: what a GitHub release's attestation must say. Trust on
    /// first use when absent.
    pub policy: Policy,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            name: None,
            version: None,
            labview: None,
            sources: Vec::new(),
            default_sources: true,
            min_age_days: 0,
            dependencies: Vec::new(),
            dev_dependencies: Vec::new(),
            groups: Vec::new(),
            github: BTreeMap::new(),
            source_pins: BTreeMap::new(),
            hooks_allowed: BTreeSet::new(),
            nipm: Vec::new(),
            distribution: None,
            application: None,
            policy: Policy::default(),
        }
    }
}

/// A `[sources]` entry, once its URL is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Feed {
    /// A hosted index folder (`http://` or `https://`), listed as a whole.
    Hosted(String),
    /// A registry prefix (`oci://host/prefix`). A registry has no catalogue,
    /// so only the packages pinned to it with `index = "<source>"` are asked.
    Oci(FeedUrl),
}

impl Feed {
    /// What `Entry::source` holds for the entries this feed lists, and so what
    /// a source pin is compared with: the index URL of a hosted folder, the
    /// `oci://` URL of a registry prefix.
    pub fn owner(&self) -> String {
        match self {
            Feed::Hosted(folder) => crate::index::feed_url(folder),
            Feed::Oci(url) => url.to_string(),
        }
    }
}

/// The feed `url` names, for the `[sources]` entry `name`.
fn feed_of(name: &str, url: &str) -> Result<Feed> {
    if url.starts_with(oci::SCHEME) {
        return oci_feed(name, url).map(Feed::Oci);
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("[sources] {name}: {url:?} is not an http:// or https:// index folder");
    }
    Ok(Feed::Hosted(url.to_string()))
}

/// `oci://host/prefix`. A URL with `user:password@` is refused without a word
/// of it in the message: it would reach the output, and the lock.
fn oci_feed(name: &str, url: &str) -> Result<FeedUrl> {
    FeedUrl::parse(url).map_err(|why| match why {
        UrlError::Credentials { registry } => anyhow::anyhow!(
            "[sources] {name}: credentials do not belong in lvpm.toml; remove the \
             user:password@ part and run docker login {registry}"
        ),
        // Fixed text: it holds nothing of the URL.
        UrlError::Invalid(problem) => anyhow::anyhow!("[sources] {name}: {problem}"),
    })
}

impl Project {
    /// The `[sources]`, each with its feed. A path on disk is refused here
    /// rather than silently resolving nothing.
    pub fn resolved_sources(&self) -> Result<Vec<(String, Feed)>> {
        (self.sources.iter())
            .map(|(name, url)| Ok((name.clone(), feed_of(name, url)?)))
            .collect()
    }

    /// The `[sources]` name `name` is pinned to, in any table, whatever the
    /// case either is written in.
    pub fn source_pin_of(&self, name: &str) -> Option<&str> {
        stated_for(&self.source_pins, name).map(String::as_str)
    }

    /// Whether a dependency line says `hooks = "run"` for `name`, in any
    /// table, whatever the case either is written in.
    pub fn hooks_allowed_for(&self, name: &str) -> bool {
        self.hooks_allowed.iter().any(|n| same_name(n, name))
    }

    /// Every dependency table, with its lines: `[dependencies]`,
    /// `[dev-dependencies]`, then each group in file order. The one list of
    /// tables: whatever treats the tables alike goes through it.
    pub fn tables(&self) -> impl Iterator<Item = (DependencyTable, &[(String, Constraint)])> {
        [
            (DependencyTable::Dependencies, self.dependencies.as_slice()),
            (
                DependencyTable::DevDependencies,
                self.dev_dependencies.as_slice(),
            ),
        ]
        .into_iter()
        .chain(
            self.groups
                .iter()
                .map(|(name, entries)| (DependencyTable::Group(name.clone()), entries.as_slice())),
        )
    }
}

/// The file as TOML sees it. Tables stay tables so their order survives and
/// so a wrong value type is reported by key rather than as a shape mismatch.
#[derive(Deserialize, Default)]
#[serde(default)]
struct File {
    project: ProjectTable,
    sources: toml::Table,
    dependencies: toml::Table,
    #[serde(rename = "dev-dependencies")]
    dev_dependencies: toml::Table,
    #[serde(rename = "dependency-groups")]
    dependency_groups: toml::Table,
    nipm: NipmTable,
    distribution: Option<toml::Table>,
    /// Any value: `application = 1` is refused by key, not as a shape mismatch.
    application: Option<toml::Value>,
    /// Any value: `policy = 1` is refused by key, not as a shape mismatch.
    policy: Option<toml::Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ProjectTable {
    name: Option<String>,
    version: Option<String>,
    labview: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NipmTable {
    dependencies: toml::Table,
}

/// The nearest `lvpm.toml` at or above `start` — a project is wherever its
/// manifest is, the way cargo finds `Cargo.toml`.
pub fn find_manifest(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|d| d.join(FILE_NAME))
        .find(|p| p.is_file())
}

pub fn read(path: &Path) -> Result<Project> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading manifest {}", path.display()))?;
    parse(&text).with_context(|| format!("parsing manifest {}", path.display()))
}

const DEFAULTS: &str = "defaults";
const MIN_AGE_DAYS: &str = "min-age-days";
/// The keys of `[sources]` that are settings and never source names.
const SOURCE_SETTINGS: [&str; 2] = [DEFAULTS, MIN_AGE_DAYS];

pub(crate) fn parse(text: &str) -> Result<Project> {
    let f: File = toml::from_str(text)?;
    let mut p = Project {
        name: f.project.name,
        version: f.project.version,
        labview: f
            .project
            .labview
            .as_deref()
            .map(labview_minimum)
            .transpose()?,
        distribution: f.distribution,
        ..Project::default()
    };
    for (key, value) in f.sources {
        match (key.as_str(), value) {
            (DEFAULTS, toml::Value::Boolean(b)) => p.default_sources = b,
            // Before the string arm: `min-age-days = "14"` is not a source.
            (MIN_AGE_DAYS, value) => match value.as_integer().and_then(|n| u32::try_from(n).ok()) {
                Some(days) => p.min_age_days = days,
                None => bail!(
                    "[sources] min-age-days must be a whole number of days, 0 or more \
                         (got {value}), for example min-age-days = 14"
                ),
            },
            (_, toml::Value::String(s)) => {
                // Refused as the manifest is read, so that no command carries a
                // secret from lvpm.toml into the lock or its own output.
                if s.starts_with(oci::SCHEME) {
                    oci_feed(&key, &s)?;
                }
                p.sources.push((key, s))
            }
            (k, other) => {
                bail!(
                    "[sources] {k}: expected an index URL string, got {}",
                    other.type_str()
                )
            }
        }
    }
    let mut stated = Stated::default();
    p.dependencies = parse_dependencies("dependencies", f.dependencies, &mut stated)?;
    p.dev_dependencies = parse_dependencies("dev-dependencies", f.dev_dependencies, &mut stated)?;
    p.groups = parse_groups(f.dependency_groups, &mut stated)?;
    check_source_pins(&p.sources, &stated.index)?;
    p.github = (stated.github.into_iter())
        .map(|(name, (repo, _))| (name, repo))
        .collect();
    p.source_pins = (stated.index.into_iter())
        .map(|(name, (source, _))| (name, source))
        .collect();
    p.hooks_allowed = stated.hooks;
    p.nipm = f.nipm.dependencies.into_iter().map(|(k, _)| k).collect();
    p.policy = f.policy.map(parse_policy).transpose()?.unwrap_or_default();
    p.application = f.application.map(parse_application).transpose()?;
    Ok(p)
}

/// The keys of `[application]`; `application::validate` reads their values.
pub(crate) const APPLICATION_KEYS: [&str; 2] = ["project", "build-spec"];

/// `[application]` as a table with no key but its own two. What the values are
/// is `application::validate`'s to say, so `lvpm lock` works on a project
/// whose `.lvproj` is not there yet.
fn parse_application(value: toml::Value) -> Result<toml::Table> {
    let toml::Value::Table(table) = value else {
        bail!("application must be a table");
    };
    if let Some(key) = table
        .keys()
        .find(|k| !APPLICATION_KEYS.contains(&k.as_str()))
    {
        bail!(
            "[application] has unknown key \"{key}\"; expected {}",
            APPLICATION_KEYS.join(" or ")
        );
    }
    Ok(table)
}

/// `[policy]`: the two keys it knows, `require-signature` and
/// `trusted-publishers`. Any other key is refused, never ignored.
fn parse_policy(value: toml::Value) -> Result<Policy> {
    let toml::Value::Table(table) = value else {
        bail!("policy must be a table");
    };
    let mut policy = Policy::default();
    for (key, value) in table {
        match (key.as_str(), value) {
            ("require-signature", toml::Value::Boolean(b)) => policy.require_signature = b,
            ("require-signature", _) => bail!("[policy] require-signature must be true or false"),
            ("trusted-publishers", toml::Value::Array(patterns)) => {
                policy.trusted_publishers = patterns
                    .into_iter()
                    .map(trusted_publisher)
                    .collect::<Result<_>>()?;
            }
            ("trusted-publishers", _) => {
                bail!("[policy] trusted-publishers must be a list of URL patterns")
            }
            (key, _) => bail!(
                "[policy] has unknown key `{key}` (expected require-signature, trusted-publishers)"
            ),
        }
    }
    Ok(policy)
}

/// One `trusted-publishers` entry: a string that starts with the GitHub URL,
/// in any case. What its `*` means is [`crate::verify::pattern_matches`]'s.
fn trusted_publisher(value: toml::Value) -> Result<String> {
    match value {
        toml::Value::String(pattern)
            if pattern.to_ascii_lowercase().starts_with(GITHUB_URL_PREFIX) =>
        {
            Ok(pattern)
        }
        toml::Value::String(pattern) => bail!(
            "[policy] trusted-publishers entry `{pattern}` must start with {GITHUB_URL_PREFIX}"
        ),
        _ => bail!("[policy] trusted-publishers must be a list of URL patterns"),
    }
}

/// `project.labview` as written, `">=2024"`, to the year it starts at, `"2024"`:
/// what [`Project::labview`] holds, so every reader of it sees a plain year.
pub(crate) fn labview_minimum(text: &str) -> Result<String> {
    let is_year = |s: &str| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit()) && s >= "2000";
    let text = text.trim();
    match text.strip_prefix(">=").map(str::trim) {
        Some(year) if is_year(year) => Ok(year.to_string()),
        _ if is_year(text) => {
            bail!("project.labview = {text:?} is a minimum: write labview = \">={text}\"")
        }
        _ => bail!(
            "project.labview = {text:?} is not a minimum LabVIEW year: \
             write it like labview = \">=2024\""
        ),
    }
}

/// The group the packages of `[dev-dependencies]` are in.
pub const DEV_GROUP: &str = "dev";

/// Whether `name` names the dev group, which is spelled any case.
pub fn is_dev_group(name: &str) -> bool {
    name.eq_ignore_ascii_case(DEV_GROUP)
}

/// Whether `name` can name a dependency group: letters, digits, `-` and `_`.
pub fn valid_group_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        bail!("group name {name:?} may use only letters, digits, - and _");
    }
    Ok(())
}

/// One dependency table's lines, in file order.
type Entries = Vec<(String, Constraint)>;

/// `[dependency-groups]`: each key a group name, each value that group's
/// dependency table, in file order. `dev` is `[dev-dependencies]`, not a group.
fn parse_groups(groups: toml::Table, stated: &mut Stated) -> Result<Vec<(String, Entries)>> {
    groups
        .into_iter()
        .map(|(name, value)| {
            valid_group_name(&name).context("[dependency-groups]")?;
            if is_dev_group(&name) {
                bail!(
                    "[dependency-groups.{name}] is not allowed: the group dev is [dev-dependencies]"
                );
            }
            let toml::Value::Table(entries) = value else {
                bail!(
                    "[dependency-groups] {name}: must be a table, got {}",
                    value.type_str()
                );
            };
            let entries =
                parse_dependencies(&format!("dependency-groups.{name}"), entries, stated)?;
            Ok((name, entries))
        })
        .collect()
}

/// What the dependency lines read so far say about where a package comes
/// from. One name says it once across the tables: a second table may repeat
/// it but not change it. `hooks` is no such statement: a line in any table
/// that says `hooks = "run"` allows them.
#[derive(Default)]
struct Stated {
    /// Name to its repo and the table that line is in.
    github: BTreeMap<String, (String, String)>,
    /// Name to the source it is pinned to and the table that line is in.
    index: BTreeMap<String, (String, String)>,
    /// Names whose line says `hooks = "run"`, as written.
    hooks: BTreeSet<String>,
}

/// What `name` was stated as, whatever the case it was written in.
fn stated_for<'a, V>(stated: &'a BTreeMap<String, V>, name: &str) -> Option<&'a V> {
    (stated.iter()).find_map(|(key, value)| same_name(key, name).then_some(value))
}

impl Stated {
    /// `name = { github = repo }` in `table`.
    fn state_github(&mut self, table: &str, name: &str, repo: String) -> Result<()> {
        if let Some((_, pinned_in)) = stated_for(&self.index, name) {
            bail!(
                "[{table}] {name}: github and index cannot be used together; {name} has an index in [{pinned_in}], and a github dependency comes only from its repo"
            );
        }
        match stated_for(&self.github, name) {
            Some((other, _)) if !other.eq_ignore_ascii_case(&repo) => bail!(
                "[{table}] {name}: github = {repo:?} differs from {other:?} in the other dependency table"
            ),
            Some(_) => {}
            None => {
                self.github
                    .insert(name.to_string(), (repo, table.to_string()));
            }
        }
        Ok(())
    }

    /// `name = { index = source }` in `table`.
    fn state_index(&mut self, table: &str, name: &str, source: String) -> Result<()> {
        if let Some((_, github_in)) = stated_for(&self.github, name) {
            bail!(
                "[{table}] {name}: github and index cannot be used together; {name} has github in [{github_in}], and a github dependency comes only from its repo"
            );
        }
        match stated_for(&self.index, name) {
            Some((other, other_table)) if *other != source => bail!(
                "[{table}] {name}: index = {source:?} differs from {other:?} in [{other_table}]"
            ),
            Some(_) => {}
            None => {
                self.index
                    .insert(name.to_string(), (source, table.to_string()));
            }
        }
        Ok(())
    }
}

/// One dependency table, in file order. `table` names it in error messages;
/// the `github` repo and the `index` source of a line that has them, and the
/// name of a line that says `hooks = "run"`, go to `stated`.
fn parse_dependencies(
    table: &str,
    entries: toml::Table,
    stated: &mut Stated,
) -> Result<Vec<(String, Constraint)>> {
    entries
        .into_iter()
        .map(|(name, value)| {
            let (constraint, keys) = dependency(table, &name, value)?;
            if let Some(repo) = keys.github {
                stated.state_github(table, &name, repo)?;
            }
            if let Some(source) = keys.index {
                stated.state_index(table, &name, source)?;
            }
            if keys.hooks {
                stated.hooks.insert(name.clone());
            }
            Ok((name, constraint))
        })
        .collect()
}

/// A source pin names a `[sources]` entry: `defaults` and `min-age-days` are
/// keys of that table but never entries, so they are refused like any other
/// name it does not have.
fn check_source_pins(
    sources: &[(String, String)],
    pins: &BTreeMap<String, (String, String)>,
) -> Result<()> {
    for (name, (source, table)) in pins {
        // A string `defaults = "<url>"` lands in `sources`; it is still a setting.
        let is_setting = SOURCE_SETTINGS.contains(&source.as_str());
        if !is_setting && sources.iter().any(|(known, _)| known == source) {
            continue;
        }
        let listed: Vec<&str> = (sources.iter())
            .map(|(known, _)| known.as_str())
            .filter(|known| !SOURCE_SETTINGS.contains(known))
            .collect();
        let known = match listed.is_empty() {
            true => "lists no index".to_string(),
            false => format!("has {}", listed.join(", ")),
        };
        let setting = match is_setting {
            true => format!(
                "; {} are settings, not sources",
                SOURCE_SETTINGS.join(" and ")
            ),
            false => String::new(),
        };
        bail!(
            "[{table}] {name}: index = {source:?} is not a [sources] name; [sources] {known}{setting}"
        );
    }
    Ok(())
}

/// The optional keys of a table-form dependency line besides `version`.
#[derive(Default)]
struct LineKeys {
    github: Option<String>,
    index: Option<String>,
    /// `hooks = "run"`.
    hooks: bool,
}

/// One dependency line: a version string, or a table with `version` and,
/// optionally, `github`, `index` or `hooks`.
fn dependency(table: &str, name: &str, value: toml::Value) -> Result<(Constraint, LineKeys)> {
    let (version, keys) = match value {
        toml::Value::String(version) => (version, LineKeys::default()),
        toml::Value::Table(fields) => dependency_table(table, name, fields)?,
        other => bail!(
            "[{table}] {name}: expected a quoted version or a table, got {}",
            other.type_str()
        ),
    };
    let constraint = Constraint::parse(&version).with_context(|| format!("[{table}] {name}"))?;
    Ok((constraint, keys))
}

/// The keys a table-form dependency line may have.
const LINE_KEYS: [&str; 4] = ["version", "github", "index", "hooks"];

/// The one value `hooks` may have.
const HOOKS_RUN: &str = "run";

/// The `version`, `github`, `index` and `hooks` of a table-form dependency
/// line. Any key not known yet is refused, never ignored.
fn dependency_table(
    table: &str,
    name: &str,
    mut fields: toml::Table,
) -> Result<(String, LineKeys)> {
    let at = format!("[{table}] {name}");
    if fields.contains_key("index") && fields.contains_key("github") {
        bail!(
            "{at}: github and index cannot be used together; a github dependency comes only from its repo"
        );
    }
    if let Some(key) = fields.keys().find(|k| !LINE_KEYS.contains(&k.as_str())) {
        let (last, rest) = LINE_KEYS.split_last().expect("LINE_KEYS is not empty");
        bail!(
            "{at}: unknown key {key:?}; expected {} or {last}",
            rest.join(", ")
        );
    }
    let Some(version) = fields.remove("version") else {
        bail!(
            "{at}: a table needs version, e.g. {name} = {{ github = \"owner/repo\", version = \"*\" }}"
        );
    };
    let toml::Value::String(version) = version else {
        bail!(
            "{at}: expected a quoted version, got {}",
            version.type_str()
        );
    };
    let github = fields
        .remove("github")
        .map(|value| github_repo(&at, value))
        .transpose()?;
    let index = fields
        .remove("index")
        .map(|value| match value {
            toml::Value::String(source) => Ok(source),
            other => bail!(
                "{at}: expected index to be a quoted [sources] name, got {}",
                other.type_str()
            ),
        })
        .transpose()?;
    let hooks = (fields.remove("hooks"))
        .map(|value| hooks_run(&at, value))
        .transpose()?
        .is_some();
    Ok((
        version,
        LineKeys {
            github,
            index,
            hooks,
        },
    ))
}

/// `value` as the one `hooks` value there is, or a refusal naming what it was.
fn hooks_run(at: &str, value: toml::Value) -> Result<()> {
    let got = match value {
        toml::Value::String(s) if s == HOOKS_RUN => return Ok(()),
        toml::Value::String(s) => format!("{s:?}"),
        other => other.type_str().to_string(),
    };
    bail!("{at}: hooks must be \"{HOOKS_RUN}\" (delete the key to keep hooks off), got {got}")
}

/// `value` as `owner/repo`: a string of two non-empty parts around one `/`,
/// made of what GitHub names allow, so it is safe in a URL path and a message.
fn github_repo(at: &str, value: toml::Value) -> Result<String> {
    let part = |s: &str| {
        !s.is_empty()
            && !s.chars().all(|c| c == '.')
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if let toml::Value::String(repo) = &value
        && matches!(*repo.split('/').collect::<Vec<_>>(), [owner, name] if part(owner) && part(name))
    {
        return Ok(repo.clone());
    }
    let (shown, url_hint) = match value.as_str() {
        Some(text) if text.starts_with(GITHUB_URL_PREFIX) => {
            (format!("{text:?}"), format!("; drop {GITHUB_URL_PREFIX:?}"))
        }
        Some(text) => (format!("{text:?}"), String::new()),
        None => (value.to_string(), String::new()),
    };
    bail!(
        "{at}: github = {shown} is not \"owner/repo\", e.g. github = \"LabVIEW-Open-Source/Serializer\"{url_hint}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_compared_by_their_ascii_lowercase_and_nothing_else() {
        assert_eq!(name_key("JKI_Lib_Caraya"), "jki_lib_caraya");
        // Not folded: a name that differs by a non-ASCII letter's case is another name.
        assert_ne!(name_key("Ärger"), name_key("ärger"));
        for (a, b) in [
            ("Beta", "beta"),
            ("Ärger", "ärger"),
            ("a", "b"),
            ("ß", "SS"),
        ] {
            assert_eq!(same_name(a, b), name_key(a) == name_key(b), "{a} {b}");
        }
    }

    #[test]
    fn a_package_arg_is_a_name_and_an_optional_constraint_and_neither_may_be_empty() {
        let arg = |text| PackageArg::parse(text).unwrap();
        assert_eq!(arg(" foo ").constraint, None);
        assert_eq!(arg("foo@>=1.0").constraint.as_deref(), Some(">=1.0"));
        assert_eq!(arg(" foo @ 1.0 "), arg("foo@1.0"));
        for text in ["", "  ", "@1.0", " @1.0"] {
            assert_eq!(
                PackageArg::parse(text),
                Err(PackageArgError::EmptyName { text: text.into() }),
                "{text:?}"
            );
        }
        for text in ["foo@", "foo@  "] {
            assert_eq!(
                PackageArg::parse(text),
                Err(PackageArgError::EmptyConstraint { name: "foo".into() }),
                "{text:?}"
            );
        }
    }

    #[test]
    fn labview_is_a_minimum_year_and_every_other_form_is_refused_by_key() {
        let labview = |value: &str| parse(&format!("[project]\nlabview = {value:?}\n"));
        assert_eq!(labview(">=2024").unwrap().labview.as_deref(), Some("2024"));
        assert_eq!(
            labview(" >= 2024 ").unwrap().labview.as_deref(),
            Some("2024")
        );
        let plain = labview("2024").unwrap_err().to_string();
        assert!(
            plain.contains("project.labview") && plain.contains("\">=2024\""),
            "{plain}"
        );
        for value in [
            ">2024", "^2024", "<=2024", ">=24", ">=2024.1", ">=", "", ">=1999", "soon",
        ] {
            let why = labview(value).unwrap_err().to_string();
            assert!(why.contains("project.labview"), "{value:?}: {why}");
        }
    }

    #[test]
    fn reads_every_table_in_file_order() {
        let p = parse(
            r#"
[project]
name = "NovaXC"
version = "0.1.0"
labview = ">=2025"

[sources]
mirror = "http://host:8090/files"
backup = "https://host/other"

[dependencies]
oglib_array = "6.0.1.20"
lava_lib_tree_control_api = "1.0.1-1"   # odd but real
oglib_error = ">=6.0.1"
jki_lib_caraya = "*"

[nipm]

[nipm.dependencies]
ni-daqmx-labview-support = "26.0.0.49434-0+f282"
"#,
        )
        .unwrap();
        assert_eq!(p.name.as_deref(), Some("NovaXC"));
        assert_eq!(p.labview.as_deref(), Some("2025"));
        assert_eq!(
            p.sources,
            vec![
                ("mirror".into(), "http://host:8090/files".into()),
                ("backup".into(), "https://host/other".into())
            ]
        );
        assert!(p.default_sources);
        let names: Vec<&str> = p.dependencies.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "oglib_array",
                "lava_lib_tree_control_api",
                "oglib_error",
                "jki_lib_caraya"
            ]
        );
        assert_eq!(
            p.dependencies[0].1,
            Constraint::Exact(Version::parse("6.0.1.20"))
        );
        assert_eq!(
            p.dependencies[1].1,
            Constraint::Exact(Version::parse("1.0.1-1"))
        );
        assert_eq!(
            p.dependencies[2].1,
            Constraint::AtLeast(Version::parse("6.0.1"))
        );
        assert_eq!(p.dependencies[3].1, Constraint::Any);
        assert_eq!(p.nipm, ["ni-daqmx-labview-support"]);
    }

    #[test]
    fn a_constraint_admits_its_own_bound() {
        let v = |s: &str| Version::parse(s);
        assert!(Constraint::AtLeast(v("2.0")).admits(&v("2.0")));
        assert!(!Constraint::AtLeast(v("2.0")).admits(&v("1.9")));
        assert!(Constraint::Exact(v("1.0")).admits(&v("1.0")));
        assert!(!Constraint::Exact(v("1.0")).admits(&v("1.1")));
        assert!(Constraint::Any.admits(&v("0.1")));
        let arg = |s: &str| PackageArg::parse(s).unwrap().read_constraint();
        assert_eq!(arg("a").unwrap(), Constraint::Any);
        assert!(arg("a@<2").is_err());
    }

    #[test]
    fn constraints_parse_the_three_forms_and_refuse_the_rest() {
        let v = |s: &str| Version::parse(s);
        assert_eq!(
            Constraint::parse("6.0.1.20").unwrap(),
            Constraint::Exact(v("6.0.1.20"))
        );
        assert_eq!(
            Constraint::parse(">=6.0.1").unwrap(),
            Constraint::AtLeast(v("6.0.1"))
        );
        assert_eq!(
            Constraint::parse(">= 6.0.1 ").unwrap(),
            Constraint::AtLeast(v("6.0.1"))
        );
        assert_eq!(Constraint::parse("*").unwrap(), Constraint::Any);
        assert_eq!(Constraint::parse("").unwrap(), Constraint::Any);
        assert!(Constraint::parse("~1.2").is_err());
        assert!(Constraint::parse("<2").is_err());
        assert!(Constraint::parse("=1.2").is_err());
        assert_eq!(Constraint::parse("1.2").unwrap().to_string(), "@1.2");
        assert_eq!(Constraint::parse(">=1.2").unwrap().to_string(), " >=1.2");
        assert_eq!(Constraint::parse("*").unwrap().to_string(), "");
    }

    #[test]
    fn rejects_what_it_cannot_read() {
        assert!(parse("[dependencies]\noglib_array\n").is_err(), "not TOML");
        assert!(
            parse("[dependencies]\noglib_array = 6.0\n").is_err(),
            "unquoted version is a float"
        );
        assert!(
            parse("[dependencies]\noglib_array = [\"6.0\"]\n").is_err(),
            "wrong type"
        );
        assert!(parse("[sources]\nlocal = 3\n").is_err());
    }

    #[test]
    fn sources_defaults_switch_and_missing_tables_are_fine() {
        let p = parse("[project]\nname = \"x\"\n").unwrap();
        assert_eq!(p.dependencies, vec![]);
        assert!(p.default_sources);
        let p = parse("[sources]\ndefaults = false\n").unwrap();
        assert!(!p.default_sources);
        assert!(p.sources.is_empty());
    }

    #[test]
    fn min_age_days_is_zero_when_absent_and_a_whole_number_of_days_otherwise() {
        let days = |sources: &str| {
            parse(&format!("[sources]\n{sources}"))
                .unwrap()
                .min_age_days
        };
        assert_eq!(days(""), 0);
        assert_eq!(days("min-age-days = 0\n"), 0);
        assert_eq!(days("min-age-days = 14\n"), 14);
        assert_eq!(days("min-age-days = 4294967295\n"), u32::MAX);
        assert_eq!(parse("").unwrap().min_age_days, 0, "no [sources] at all");
    }

    #[test]
    fn min_age_days_is_not_a_source_and_does_not_disturb_the_others() {
        let p = parse("[sources]\nmirror = \"http://h/x\"\nmin-age-days = 14\ndefaults = false\n")
            .unwrap();
        assert_eq!(p.sources, vec![("mirror".into(), "http://h/x".into())]);
        assert!(!p.default_sources);
        assert_eq!(p.min_age_days, 14);
    }

    #[test]
    fn min_age_days_outside_sources_is_ignored_like_any_unknown_key() {
        let p = parse("min-age-days = 14\n\n[project]\nname = \"x\"\n").unwrap();
        assert_eq!(p.min_age_days, 0);
    }

    #[test]
    fn min_age_days_that_is_not_a_whole_number_of_days_is_refused_with_the_value_as_written() {
        for value in ["-1", "\"14\"", "14.0", "14.5", "[14]", "true", "4294967296"] {
            let why = parse(&format!("[sources]\nmin-age-days = {value}\n"))
                .unwrap_err()
                .to_string();
            assert_eq!(
                why,
                format!(
                    "[sources] min-age-days must be a whole number of days, 0 or more \
                     (got {value}), for example min-age-days = 14"
                ),
                "{value}"
            );
        }
    }

    #[test]
    fn min_age_days_beyond_any_integer_is_refused_naming_the_key() {
        let why = parse("[sources]\nmin-age-days = 99999999999999999999\n")
            .unwrap_err()
            .to_string();
        assert!(why.contains("min-age-days"), "{why}");
    }

    #[test]
    fn sources_must_be_hosted_index_folders() {
        let p = parse(
            "[sources]\nnet = \"http://h/x\"\nsecure = \"https://h/y\"\nteam = \"oci://r.example:5000/org/lvpm\"\n",
        )
        .unwrap();
        let feeds = p.resolved_sources().unwrap();
        assert_eq!(
            feeds[..2],
            [
                ("net".to_string(), Feed::Hosted("http://h/x".into())),
                ("secure".to_string(), Feed::Hosted("https://h/y".into())),
            ]
        );
        let (name, Feed::Oci(url)) = &feeds[2] else {
            panic!("an oci:// source is an OCI feed: {feeds:?}");
        };
        assert_eq!((name.as_str(), url.registry()), ("team", "r.example:5000"));
        assert_eq!(url.prefix(), "org/lvpm");

        for refused in ["./Dependencies", "ftp://h/x"] {
            let folder = parse(&format!("[sources]\nlocal = \"{refused}\"\n")).unwrap();
            assert!(
                folder.resolved_sources().is_err(),
                "{refused} is not a source"
            );
        }
    }

    #[test]
    fn a_source_pin_compares_with_the_owner_of_its_feed() {
        let hosted = Feed::Hosted("http://h/x".into());
        assert_eq!(hosted.owner(), "http://h/x/index.vipr");
        let oci = Feed::Oci(FeedUrl::parse("oci://r:5000/org/lvpm").unwrap());
        assert_eq!(oci.owner(), "oci://r:5000/org/lvpm");
    }

    #[test]
    fn credentials_in_an_oci_source_are_refused_without_echoing_them() {
        let user = "u1-4d2e8b-user";
        let secret = "s1-7f3a9c-secret";
        for url in [
            format!("oci://{user}:{secret}@r:5000/team"),
            format!("oci://{user}:{secret}@r:5000/team/"),
            // A password may hold a slash.
            format!("oci://{user}:{secret}/x@r:5000/team"),
            format!("oci://{user}@r:5000/team"),
        ] {
            let why = parse(&format!("[sources]\nteam = \"{url}\"\n"))
                .unwrap_err()
                .to_string();
            assert_eq!(
                why,
                "[sources] team: credentials do not belong in lvpm.toml; remove the \
                 user:password@ part and run docker login r:5000",
                "{url}"
            );
        }
        // A tail that is no registry is not echoed either, so the message
        // cannot carry the secret.
        let why = parse(&format!(
            "[sources]\nteam = \"oci://{user}:{secret}@/x@y/z\"\n"
        ))
        .unwrap_err()
        .to_string();
        assert!(!why.contains(user) && !why.contains(secret), "{why}");
        assert!(why.starts_with("[sources] team: "), "{why}");
    }

    #[test]
    fn an_oci_source_that_is_no_registry_prefix_is_refused() {
        for url in ["oci://r:5000", "oci://r:5000/", "oci://r:5000/UPPER"] {
            let why = parse(&format!("[sources]\nteam = \"{url}\"\n"))
                .unwrap_err()
                .to_string();
            assert!(why.starts_with("[sources] team: "), "{url}: {why}");
        }
    }

    #[test]
    fn dev_dependencies_use_the_dependency_syntax() {
        let p = parse(
            "[dependencies]\nalpha = \"1.0\"\n\n[dev-dependencies]\ncaraya = \"*\"\nunit = \">=2.1\"\n",
        )
        .unwrap();
        assert_eq!(p.dependencies.len(), 1, "the tables stay apart");
        assert_eq!(
            p.dev_dependencies,
            vec![
                ("caraya".to_string(), Constraint::Any),
                (
                    "unit".to_string(),
                    Constraint::AtLeast(Version::parse("2.1"))
                )
            ]
        );
        let none = parse("[project]\nname = \"x\"\n").unwrap();
        assert!(none.dev_dependencies.is_empty());
    }

    #[test]
    fn a_table_line_records_its_github_repo_in_either_dependency_table() {
        let p = parse(
            "[dependencies]\nserializer = { github = \"LabVIEW-Open-Source/Serializer\", version = \">=1.2\" }\nplain = \"1.0\"\n\n\
             [dev-dependencies]\ncaraya = { version = \"*\", github = \"JKISoftware/Caraya\" }\n",
        )
        .unwrap();
        assert_eq!(
            p.dependencies,
            vec![
                (
                    "serializer".to_string(),
                    Constraint::AtLeast(Version::parse("1.2"))
                ),
                (
                    "plain".to_string(),
                    Constraint::Exact(Version::parse("1.0"))
                )
            ]
        );
        assert_eq!(
            p.dev_dependencies,
            vec![("caraya".to_string(), Constraint::Any)]
        );
        assert_eq!(
            p.github.into_iter().collect::<Vec<_>>(),
            [
                ("caraya".to_string(), "JKISoftware/Caraya".to_string()),
                (
                    "serializer".to_string(),
                    "LabVIEW-Open-Source/Serializer".to_string()
                )
            ],
            "the short form has no repo"
        );
    }

    #[test]
    fn a_name_in_both_tables_has_one_github_repo() {
        let both = |dev: &str| {
            parse(&format!(
                "[dependencies]\nCaraya = {{ github = \"o/r\", version = \"*\" }}\n\n[dev-dependencies]\ncaraya = {dev}\n"
            ))
        };
        let same = both("{ github = \"o/r\", version = \"1.0\" }").unwrap();
        assert_eq!(same.github.len(), 1);
        both("\"1.0\"").unwrap();

        let error = both("{ github = \"x/y\", version = \"1.0\" }").unwrap_err();
        assert_eq!(
            error.to_string(),
            "[dev-dependencies] caraya: github = \"x/y\" differs from \"o/r\" in the other dependency table"
        );
    }

    #[test]
    fn a_table_with_only_a_version_is_the_short_form() {
        let short = parse("[dependencies]\na = \">=1.2\"\nb = \"*\"\nc = \"1.0\"\n").unwrap();
        let table = parse(
            "[dependencies]\na = { version = \">=1.2\" }\nb = { version = \"*\" }\nc = { version = \"1.0\" }\n",
        )
        .unwrap();
        assert_eq!(table, short);
        assert!(table.github.is_empty());
    }

    /// What reading `[<table>]` with this one line says, in full.
    fn refusal(table: &str, line: &str) -> String {
        format!("{:#}", parse(&format!("[{table}]\n{line}\n")).unwrap_err())
    }

    #[test]
    fn a_hooks_value_other_than_the_string_run_is_refused_with_its_value_or_type() {
        let said = |got: &str| {
            format!(
                "[dependencies] pkg: hooks must be \"run\" (delete the key to keep hooks off), got {got}"
            )
        };
        for (value, got) in [
            ("\"yes\"", "\"yes\""),
            ("\"Run\"", "\"Run\""),
            ("\"\"", "\"\""),
            ("true", "boolean"),
            ("1", "integer"),
            ("[]", "array"),
        ] {
            let line = format!("pkg = {{ version = \"*\", hooks = {value} }}");
            assert_eq!(refusal("dependencies", &line), said(got), "{line}");
        }
    }

    #[test]
    fn a_hooks_refusal_names_the_table_it_is_in() {
        assert_eq!(
            refusal(
                "dev-dependencies",
                "pkg = { version = \"*\", hooks = \"skip\" }"
            ),
            "[dev-dependencies] pkg: hooks must be \"run\" (delete the key to keep hooks off), got \"skip\""
        );
    }

    #[test]
    fn a_table_lvpm_cannot_use_is_refused_by_table_and_name_with_its_reason() {
        let github_and_index = "[dependencies] pkg: github and index cannot be used together; a github dependency comes only from its repo";
        let a_table_needs_version = "[dependencies] pkg: a table needs version, e.g. pkg = { github = \"owner/repo\", version = \"*\" }";
        for (line, said) in [
            (
                "pkg = { github = \"o/r\", index = \"internal\", version = \"*\" }",
                github_and_index,
            ),
            (
                "pkg = { index = \"internal\", github = \"o/r\" }",
                github_and_index,
            ),
            (
                "pkg = { github = \"o/r\", version = \"*\", foo = \"x\" }",
                "[dependencies] pkg: unknown key \"foo\"; expected version, github, index or hooks",
            ),
            ("pkg = { github = \"o/r\" }", a_table_needs_version),
            ("pkg = {}", a_table_needs_version),
            (
                "pkg = { github = \"o/r\", version = 5 }",
                "[dependencies] pkg: expected a quoted version, got integer",
            ),
            (
                "pkg = 5",
                "[dependencies] pkg: expected a quoted version or a table, got integer",
            ),
        ] {
            assert_eq!(refusal("dependencies", line), said, "{line}");
        }
    }

    #[test]
    fn a_github_value_that_is_not_owner_slash_repo_is_refused_with_the_value() {
        let is_not = |value: &str| {
            format!(
                "[dependencies] pkg: github = {value} is not \"owner/repo\", e.g. github = \"LabVIEW-Open-Source/Serializer\""
            )
        };
        for (value, said) in [
            ("\"o\"", is_not("\"o\"")),
            ("\"o/r/x\"", is_not("\"o/r/x\"")),
            ("\"/r\"", is_not("\"/r\"")),
            ("\"o/\"", is_not("\"o/\"")),
            ("\"/\"", is_not("\"/\"")),
            ("\"../x\"", is_not("\"../x\"")),
            ("\"o/r?x#y\"", is_not("\"o/r?x#y\"")),
            ("\"o/r s\"", is_not("\"o/r s\"")),
            ("\"\"", is_not("\"\"")),
            ("5", is_not("5")),
            (
                "\"https://github.com/o/r\"",
                format!(
                    "{}; drop \"https://github.com/\"",
                    is_not("\"https://github.com/o/r\"")
                ),
            ),
        ] {
            let line = format!("pkg = {{ github = {value}, version = \"*\" }}");
            assert_eq!(refusal("dependencies", &line), said, "{line}");
        }
    }

    #[test]
    fn a_dev_dependency_is_refused_under_its_own_table_name() {
        assert_eq!(
            refusal(
                "dev-dependencies",
                "pkg = { github = \"o/r\", index = \"x\", version = \"*\" }"
            ),
            "[dev-dependencies] pkg: github and index cannot be used together; a github dependency comes only from its repo"
        );
    }

    #[test]
    fn a_bad_constraint_in_a_table_names_the_entry_like_the_short_form() {
        let why = refusal(
            "dependencies",
            "pkg = { github = \"o/r\", version = \"~1.2\" }",
        );
        assert_eq!(
            why,
            refusal("dependencies", "pkg = \"~1.2\""),
            "the same error as the short form"
        );
        assert!(
            why.starts_with("[dependencies] pkg: unsupported version constraint"),
            "{why}"
        );
    }

    #[test]
    fn an_unsupported_dev_dependency_constraint_names_the_entry_and_the_constraint() {
        let why = format!(
            "{:#}",
            parse("[dev-dependencies]\ncaraya = \"~1.0\"\n").unwrap_err()
        );
        assert!(why.contains("[dev-dependencies] caraya"), "{why}");
        assert!(why.contains("~1.0"), "{why}");
        assert!(parse("[dev-dependencies]\ncaraya = 1.0\n").is_err());
    }

    /// `groups` as `(name, [entry names])`, to compare by what they hold.
    fn group_entries(p: &Project) -> Vec<(&str, Vec<&str>)> {
        p.groups
            .iter()
            .map(|(group, entries)| {
                (
                    group.as_str(),
                    entries.iter().map(|(name, _)| name.as_str()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn dependency_groups_are_read_in_file_order_beside_the_other_tables() {
        let p = parse(
            "[dependencies]\nalpha = \"1.0\"\n\n[dependency-groups.zeta]\nz_tool = \">=2.0\"\n\n\
             [dev-dependencies]\ncaraya = \"*\"\n\n[dependency-groups.ci-1_a]\nb_tool = \"*\"\na_tool = \"1.0\"\n",
        )
        .unwrap();
        assert_eq!(
            group_entries(&p),
            [
                ("zeta", vec!["z_tool"]),
                ("ci-1_a", vec!["b_tool", "a_tool"])
            ]
        );
        assert_eq!(
            p.groups[0].1,
            vec![(
                "z_tool".to_string(),
                Constraint::AtLeast(Version::parse("2.0"))
            )]
        );
        assert_eq!(p.dependencies.len(), 1, "the tables stay apart");
        assert_eq!(p.dev_dependencies.len(), 1, "the tables stay apart");
        assert!(
            parse("[project]\nname = \"x\"\n")
                .unwrap()
                .groups
                .is_empty()
        );
    }

    #[test]
    fn tables_lists_dependencies_then_dev_then_each_group_in_file_order() {
        let p = parse(
            "[dependency-groups.zeta]
z = \"*\"

[dev-dependencies]
d = \"*\"

             [dependencies]
a = \"*\"

[dependency-groups.ci]
c = \"*\"
e = \"*\"
",
        )
        .unwrap();
        let tables: Vec<(String, usize)> = p
            .tables()
            .map(|(table, entries)| (table.key(), entries.len()))
            .collect();
        assert_eq!(
            tables,
            [
                ("dependencies".to_string(), 1),
                ("dev-dependencies".to_string(), 1),
                ("dependency-groups.zeta".to_string(), 1),
                ("dependency-groups.ci".to_string(), 2),
            ]
        );
    }

    #[test]
    fn an_empty_group_is_a_group() {
        let p = parse("[dependency-groups.ci]\n").unwrap();
        assert_eq!(group_entries(&p), [("ci", vec![])]);
    }

    #[test]
    fn a_group_named_dev_in_any_case_is_refused_pointing_at_dev_dependencies() {
        for name in ["dev", "Dev", "DEV"] {
            let why = format!(
                "{:#}",
                parse(&format!("[dependency-groups.{name}]\ncaraya = \"*\"\n")).unwrap_err()
            );
            assert_eq!(
                why,
                format!(
                    "[dependency-groups.{name}] is not allowed: the group dev is [dev-dependencies]"
                )
            );
        }
        assert!(
            parse("[dependency-groups.devs]\n").is_ok(),
            "only dev itself"
        );
    }

    #[test]
    fn a_dependency_groups_value_that_is_not_a_table_is_refused_by_key() {
        let why = format!(
            "{:#}",
            parse("[dependency-groups]\nci = \"ci_tools\"\n").unwrap_err()
        );
        assert_eq!(why, "[dependency-groups] ci: must be a table, got string");
    }

    #[test]
    fn a_group_name_outside_letters_digits_dash_and_underscore_is_refused() {
        for name in ["", "a.b", "a b", "a/b", "ä"] {
            assert!(valid_group_name(name).is_err(), "{name:?}");
        }
        for name in ["ci", "CI", "a-b_c9", "9"] {
            assert!(valid_group_name(name).is_ok(), "{name:?}");
        }
        let why = format!(
            "{:#}",
            parse("[dependency-groups.\"a.b\"]\nbeta = \"*\"\n").unwrap_err()
        );
        assert!(why.contains("[dependency-groups]"), "{why}");
        assert!(why.contains("\"a.b\""), "{why}");
        assert!(why.contains("letters, digits, - and _"), "{why}");
    }

    #[test]
    fn a_bad_entry_in_a_group_names_the_group_and_the_entry() {
        let why = refusal("dependency-groups.ci", "pkg = \"~1.0\"");
        assert!(
            why.starts_with("[dependency-groups.ci] pkg: unsupported version constraint"),
            "{why}"
        );
    }

    #[test]
    fn a_name_in_a_group_and_in_dependencies_has_one_github_repo() {
        let both = |group: &str| {
            parse(&format!(
                "[dependencies]\ncaraya = {{ github = \"o/r\", version = \"*\" }}\n\n[dependency-groups.ci]\nCaraya = {group}\n"
            ))
        };
        both("{ github = \"o/r\", version = \"1.0\" }").unwrap();
        both("\"1.0\"").unwrap();
        let why = both("{ github = \"x/y\", version = \"1.0\" }")
            .unwrap_err()
            .to_string();
        assert_eq!(
            why,
            "[dependency-groups.ci] Caraya: github = \"x/y\" differs from \"o/r\" in the other dependency table"
        );
    }

    #[test]
    fn a_policy_table_is_read() {
        let p = parse(
            "[policy]\nrequire-signature = true\ntrusted-publishers = [\"https://github.com/a/*\"]\n",
        )
        .unwrap();
        assert!(p.policy.require_signature);
        assert_eq!(p.policy.trusted_publishers, ["https://github.com/a/*"]);
    }

    #[test]
    fn no_policy_means_trust_on_first_use() {
        assert!(
            parse("[project]\nname = \"x\"\n")
                .unwrap()
                .policy
                .is_default()
        );
        assert!(parse("[policy]\n").unwrap().policy.is_default());
    }

    #[test]
    fn a_bad_policy_is_refused_naming_the_key() {
        for (policy, key) in [
            ("[policy]\nrequire-signature = \"yes\"", "require-signature"),
            ("[policy]\ntrusted-publisher = []", "`trusted-publisher`"),
            (
                "[policy]\ntrusted-publishers = \"https://github.com/a/*\"",
                "trusted-publishers",
            ),
            ("[policy]\ntrusted-publishers = [1]", "trusted-publishers"),
            (
                "[policy]\ntrusted-publishers = [\"\"]",
                "trusted-publishers",
            ),
            (
                "[policy]\ntrusted-publishers = [\"github.com/a/*\"]",
                "trusted-publishers",
            ),
            ("policy = 1", "policy"),
        ] {
            let why = parse(policy).unwrap_err().to_string();
            assert!(why.contains(key), "{policy}: {why}");
        }
    }

    #[test]
    fn a_bad_policy_says_what_to_write() {
        let why = |text| parse(text).unwrap_err().to_string();
        assert_eq!(
            why("[policy]\ntrusted-publisher = []"),
            "[policy] has unknown key `trusted-publisher` (expected require-signature, trusted-publishers)"
        );
        assert_eq!(
            why("[policy]\ntrusted-publishers = [\"github.com/a/*\"]"),
            "[policy] trusted-publishers entry `github.com/a/*` must start with https://github.com/"
        );
        assert_eq!(why("policy = 1"), "policy must be a table");
        assert_eq!(
            why("[policy]\nrequire-signature = 1"),
            "[policy] require-signature must be true or false"
        );
        assert_eq!(
            why("[policy]\ntrusted-publishers = 1"),
            "[policy] trusted-publishers must be a list of URL patterns"
        );
    }

    #[test]
    fn an_application_table_is_kept_as_written_and_is_none_without_one() {
        let p = parse(
            "[application]
project = \"a/B.lvproj\"
build-spec = 5
",
        )
        .unwrap();
        let table = p.application.expect("a table");
        assert_eq!(table["project"].as_str(), Some("a/B.lvproj"));
        // What its values are is `application::validate`'s to say.
        assert_eq!(table["build-spec"].as_integer(), Some(5));
        assert_eq!(
            parse(
                "[project]
name = \"x\"
"
            )
            .unwrap()
            .application,
            None
        );
        assert_eq!(
            parse(
                "[application]
"
            )
            .unwrap()
            .application,
            Some(toml::Table::new())
        );
    }

    #[test]
    fn an_application_table_with_another_key_or_no_table_is_refused() {
        let why = |text| parse(text).unwrap_err().to_string();
        assert_eq!(
            why("[application]
target = \"RT\"
"),
            "[application] has unknown key \"target\"; expected project or build-spec"
        );
        assert_eq!(
            why("[application]
project = \"a.lvproj\"
Build-Spec = \"s\"
"),
            "[application] has unknown key \"Build-Spec\"; expected project or build-spec"
        );
        assert_eq!(why("application = 1"), "application must be a table");
    }

    #[test]
    fn a_pattern_starts_with_the_github_url_in_any_case() {
        let p = parse("[policy]\ntrusted-publishers = [\"HTTPS://GitHub.com/A/*\"]\n").unwrap();
        assert_eq!(p.policy.trusted_publishers, ["HTTPS://GitHub.com/A/*"]);
    }

    /// A manifest whose `[sources]` are `first` and `second`, then `tables`.
    fn with_two_sources(tables: &str) -> Result<Project> {
        parse(&format!(
            "[sources]\nfirst = \"http://h/a\"\nsecond = \"http://h/b\"\n\n{tables}"
        ))
    }

    #[test]
    fn a_source_pin_is_read_from_a_line_in_every_kind_of_table() {
        let p = with_two_sources(
            "[dependencies]\nalpha = { version = \">=1.0\", index = \"first\" }\n\n\
             [dev-dependencies]\nCaraya = { index = \"second\", version = \"*\" }\n\n\
             [dependency-groups.ci]\nci_tools = { index = \"first\", version = \"2.0\" }\nplain = \"1.0\"\n",
        )
        .unwrap();
        assert_eq!(
            p.source_pins.into_iter().collect::<Vec<_>>(),
            [
                ("Caraya".to_string(), "second".to_string()),
                ("alpha".to_string(), "first".to_string()),
                ("ci_tools".to_string(), "first".to_string()),
            ],
            "by the name as written; the short form has no pin"
        );
    }

    #[test]
    fn hooks_run_is_read_from_a_line_in_every_kind_of_table() {
        let p = parse(
            "[dependencies]\nalpha = { version = \"1.0\", hooks = \"run\" }\nplain = \"1.0\"\n\n\
             [dev-dependencies]\nBeta = { hooks = \"run\", version = \"*\" }\n\n\
             [dependency-groups.ci]\ngamma = { version = \"2.0\", hooks = \"run\" }\n",
        )
        .unwrap();
        assert_eq!(
            p.hooks_allowed.into_iter().collect::<Vec<_>>(),
            ["Beta", "alpha", "gamma"],
            "by the name as written; the short form allows nothing"
        );
    }

    #[test]
    fn hooks_run_next_to_github_or_index_keeps_both() {
        let p = with_two_sources(
            "[dependencies]\na = { github = \"o/r\", version = \"*\", hooks = \"run\" }\n\
             b = { index = \"first\", version = \"1.0\", hooks = \"run\" }\n",
        )
        .unwrap();
        assert_eq!(p.github.get("a").map(String::as_str), Some("o/r"));
        assert_eq!(p.source_pin_of("b"), Some("first"));
        assert!(p.hooks_allowed_for("a"), "hooks lost on the github line");
        assert!(p.hooks_allowed_for("b"), "hooks lost on the index line");
    }

    #[test]
    fn hooks_allowed_for_matches_the_name_in_any_case() {
        let p = parse("[dependencies]\nJKI_Lib = { version = \"*\", hooks = \"run\" }\n").unwrap();
        assert!(p.hooks_allowed_for("jki_lib"), "lower case");
        assert!(p.hooks_allowed_for("JKI_LIB"), "upper case");
        assert!(!p.hooks_allowed_for("other"));
    }

    #[test]
    fn hooks_run_on_one_of_two_lines_for_a_name_allows_it() {
        let p = parse(
            "[dependencies]
a = \"1.0\"

[dev-dependencies]
A = { version = \"1.0\", hooks = \"run\" }
",
        )
        .unwrap();
        assert!(p.hooks_allowed_for("a"));
    }

    #[test]
    fn a_manifest_without_hooks_allows_none() {
        let p = parse("[dependencies]\na = \"1.0\"\nb = { version = \"*\" }\n").unwrap();
        assert!(p.hooks_allowed.is_empty());
        assert!(!p.hooks_allowed_for("a"));
    }

    #[test]
    fn a_pinned_line_reads_like_an_unpinned_one_but_for_the_pin() {
        let pinned = with_two_sources(
            "[dependencies]\na = { version = \">=1.2\", index = \"first\" }\nb = { index = \"second\", version = \"1.0\" }\n",
        )
        .unwrap();
        let plain = with_two_sources("[dependencies]\na = \">=1.2\"\nb = \"1.0\"\n").unwrap();
        assert_eq!(
            Project {
                source_pins: BTreeMap::new(),
                ..pinned
            },
            plain
        );
    }

    #[test]
    fn the_short_form_and_a_table_without_index_pin_nothing() {
        let p = with_two_sources(
            "[dependencies]\na = \"1.0\"\nb = { version = \"*\" }\nc = { github = \"o/r\", version = \"*\" }\n",
        )
        .unwrap();
        assert!(p.source_pins.is_empty());
    }

    #[test]
    fn a_pin_naming_a_source_that_is_not_listed_is_refused_naming_the_line_and_the_sources() {
        let unknown = |line: &str| {
            format!(
                "{:#}",
                with_two_sources(&format!("[dependencies]\n{line}\n")).unwrap_err()
            )
        };
        let said = "[dependencies] alpha: index = \"nope\" is not a [sources] name; [sources] has first, second";
        assert_eq!(
            unknown("alpha = { index = \"nope\", version = \"*\" }"),
            said
        );
        // The group's own name, as written, is in the text.
        let in_group = with_two_sources(
            "[dependency-groups.ci]\nalpha = { index = \"nope\", version = \"*\" }\n",
        )
        .unwrap_err()
        .to_string();
        assert!(
            in_group.starts_with("[dependency-groups.ci] alpha: index = \"nope\""),
            "{in_group}"
        );
        // A source name is spelled exactly: `First` is not `first`.
        assert!(
            unknown("alpha = { index = \"First\", version = \"*\" }")
                .contains("index = \"First\" is not")
        );
    }

    #[test]
    fn a_pin_in_a_manifest_with_no_sources_says_it_lists_no_index() {
        let why = refusal(
            "dependencies",
            "alpha = { index = \"nope\", version = \"*\" }",
        );
        assert_eq!(
            why,
            "[dependencies] alpha: index = \"nope\" is not a [sources] name; [sources] lists no index"
        );
        let defaults_only = parse("[sources]\ndefaults = false\n\n[dependencies]\nalpha = { index = \"nope\", version = \"*\" }\n")
            .unwrap_err()
            .to_string();
        assert!(
            defaults_only.ends_with("[sources] lists no index"),
            "{defaults_only}"
        );
    }

    #[test]
    fn a_pin_naming_a_setting_of_sources_says_it_is_not_a_source() {
        for key in ["defaults", "min-age-days"] {
            let why = with_two_sources(&format!(
                "[dependencies]\nalpha = {{ index = \"{key}\", version = \"*\" }}\n"
            ))
            .unwrap_err()
            .to_string();
            assert_eq!(
                why,
                format!(
                    "[dependencies] alpha: index = \"{key}\" is not a [sources] name; [sources] has first, second; defaults and min-age-days are settings, not sources"
                )
            );
        }
    }

    #[test]
    fn a_pin_naming_defaults_is_refused_even_when_defaults_holds_a_url() {
        let why = parse(
            "[sources]
defaults = \"https://example.com/\"

[dependencies]
alpha = { index = \"defaults\", version = \"*\" }
",
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            why,
            "[dependencies] alpha: index = \"defaults\" is not a [sources] name; [sources] lists no index; defaults and min-age-days are settings, not sources"
        );
    }

    #[test]
    fn a_name_with_github_in_one_table_and_an_index_in_another_is_refused_naming_both() {
        let both = |first: &str, second: &str| {
            with_two_sources(&format!(
                "[dependencies]\nalpha = {first}\n\n[dev-dependencies]\nAlpha = {second}\n"
            ))
            .unwrap_err()
            .to_string()
        };
        let github = "{ github = \"o/r\", version = \"*\" }";
        let index = "{ index = \"first\", version = \"*\" }";
        assert_eq!(
            both(github, index),
            "[dev-dependencies] Alpha: github and index cannot be used together; Alpha has github in [dependencies], and a github dependency comes only from its repo"
        );
        assert_eq!(
            both(index, github),
            "[dev-dependencies] Alpha: github and index cannot be used together; Alpha has an index in [dependencies], and a github dependency comes only from its repo"
        );
    }

    #[test]
    fn a_pin_that_is_not_a_string_is_refused_naming_the_line() {
        for (value, kind) in [
            ("5", "integer"),
            ("true", "boolean"),
            ("[\"first\"]", "array"),
        ] {
            let why = with_two_sources(&format!(
                "[dependencies]\nalpha = {{ index = {value}, version = \"*\" }}\n"
            ))
            .unwrap_err()
            .to_string();
            assert_eq!(
                why,
                format!(
                    "[dependencies] alpha: expected index to be a quoted [sources] name, got {kind}"
                )
            );
        }
    }

    #[test]
    fn one_name_pinned_to_two_sources_in_two_tables_is_refused_naming_both() {
        let both = |dev: &str| {
            with_two_sources(&format!(
                "[dependencies]\nalpha = {{ index = \"first\", version = \"*\" }}\n\n[dev-dependencies]\nAlpha = {dev}\n"
            ))
        };
        assert_eq!(
            both("{ index = \"second\", version = \"*\" }")
                .unwrap_err()
                .to_string(),
            "[dev-dependencies] Alpha: index = \"second\" differs from \"first\" in [dependencies]"
        );
        let same = both("{ index = \"first\", version = \"1.0\" }").unwrap();
        assert_eq!(
            same.source_pins.len(),
            1,
            "one pin, whatever the case of the name"
        );
        both("\"1.0\"").unwrap();
    }

    #[test]
    fn a_name_pinned_in_a_group_and_in_dependencies_has_one_source() {
        let why = with_two_sources(
            "[dependency-groups.a]\npkg = { index = \"first\", version = \"*\" }\n\n\
             [dependency-groups.b]\npkg = { index = \"second\", version = \"*\" }\n",
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            why,
            "[dependency-groups.b] pkg: index = \"second\" differs from \"first\" in [dependency-groups.a]"
        );
    }

    #[test]
    fn a_name_in_two_groups_has_one_github_repo() {
        let why = parse(
            "[dependency-groups.a]\npkg = { github = \"o/r\", version = \"*\" }\n\n\
             [dependency-groups.b]\npkg = { github = \"x/y\", version = \"*\" }\n",
        )
        .unwrap_err()
        .to_string();
        assert!(why.starts_with("[dependency-groups.b] pkg:"), "{why}");
    }
}
