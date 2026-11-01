//! The solver: one set of versions that satisfies every constraint at once.
//!
//! Picking the newest version of each package one at a time can fail where a
//! valid set exists (`A = "*"`, `C = "1.0"`, and A 2.0 wants `C >= 2.0`). The
//! solver is PubGrub, run over an [`Index`]. It is pure: it reads nothing but
//! what it is handed, so the same inputs always give the same answer, whatever
//! is installed. Hashing the archives and writing `lvpm.lock` are a separate
//! step.
//!
//! Package keys are lowercased, as everywhere else in lvpm.

use crate::cancel::CancelToken;
use crate::index::{Entry, Floor, Index};
use crate::project::{Constraint, name_key as key};
use crate::released;
use crate::version::Version;
use pubgrub::{
    Dependencies, DependencyConstraints, DependencyProvider, DerivationTree, External,
    PackageResolutionStatistics, PubGrubError, Ranges,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::ops::Bound;

// ---- what the solver returns ----

/// The versions the solver chose: every direct and transitive package, once,
/// keyed by its lowercased name.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub packages: BTreeMap<String, Entry>,
    /// Requirements of the chosen packages that no source lists, by name then
    /// requirer. They are not a failure: the package is simply not locked.
    pub unresolved: Vec<Unresolved>,
}

/// A requirement that no source lists, and the package that has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub name: String,
    pub required_by: String,
}

/// Why `solve` returned no resolution.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SolveError {
    #[error("{0}")]
    Conflict(Conflict),
    #[error("the solver was cancelled")]
    Cancelled,
}

/// Solve `constraints` (a manifest's direct dependencies, in file order)
/// against `index`.
///
/// `prefs` are versions to keep when they still fit, such as the ones an
/// existing lock holds: a new upstream release then changes nothing. `first`
/// are packages to settle before the others, so that a preference of another
/// package gives way to what they need, never the other way round: an upgrade's
/// targets. `lv_gate` is the project's LabVIEW in the index's own units
/// (`lv_min`, so 24.0 for LabVIEW 2024): no package that needs a newer LabVIEW
/// is chosen. `None` applies no gate. `age` holds back every version released
/// less than its days ago, except one a manifest line pins exactly; `None`
/// holds nothing back. A cancelled `cancel` stops the solver.
pub fn solve(
    constraints: &[(String, Constraint)],
    index: &Index,
    prefs: &[(String, Version)],
    first: &[String],
    lv_gate: Option<f64>,
    age: Option<AgeLimit>,
    cancel: &CancelToken,
) -> Result<Resolution, SolveError> {
    let provider = Provider {
        catalog: Catalog::new(index),
        direct: constraints
            .iter()
            .map(|(name, constraint)| (key(name), range_of(constraint)))
            .collect(),
        pinned: constraints
            .iter()
            .filter_map(|(name, constraint)| match constraint {
                Constraint::Exact(version) => Some((key(name), version.clone())),
                Constraint::AtLeast(_) | Constraint::Any => None,
            })
            .collect(),
        prefs: prefs
            .iter()
            .map(|(name, version)| (key(name), version.clone()))
            .collect(),
        first: first.iter().map(|name| key(name)).collect(),
        lv_gate,
        age,
        cancel,
    };
    match pubgrub::resolve(&provider, Pkg::Root, root_version()) {
        Ok(selected) => {
            let packages: BTreeMap<String, Entry> = selected
                .into_iter()
                .filter_map(|(package, version)| match package {
                    Pkg::Root | Pkg::Extra(_) => None,
                    Pkg::Named(key) => provider
                        .catalog
                        .entry(&key, &version)
                        .map(|entry| (key, entry.clone())),
                })
                .collect();
            let unresolved = provider.unresolved_in(&packages);
            Ok(Resolution {
                packages,
                unresolved,
            })
        }
        Err(PubGrubError::NoSolution(tree)) => Err(SolveError::Conflict(Conflict::explain(
            &tree,
            &Names {
                catalog: &provider.catalog,
                manifest: constraints,
            },
        ))),
        // The provider fails only to say it was cancelled, from any of the
        // three places pubgrub lets it.
        Err(
            PubGrubError::ErrorRetrievingDependencies { .. }
            | PubGrubError::ErrorChoosingVersion { .. }
            | PubGrubError::ErrorInShouldCancel(_),
        ) => Err(SolveError::Cancelled),
    }
}

// ---- version mapping ----
//
// A manifest constraint, or a package's own requirement, is a set of versions.
// `Version`'s `Ord` is what PubGrub compares by, and it agrees with `Eq`:
// `1.0` and `1.0.0` are the same version, and a prerelease tail sorts below
// the release.

/// The versions a manifest line admits.
pub(crate) fn range_of(constraint: &Constraint) -> Ranges<Version> {
    match constraint {
        Constraint::Exact(version) => Ranges::singleton(version.clone()),
        Constraint::AtLeast(version) => Ranges::higher_than(version.clone()),
        Constraint::Any => Ranges::full(),
    }
}

/// The versions a package's requirement admits: its floor, or any version.
fn floor_range(floor: Option<&Floor>) -> Ranges<Version> {
    floor.map_or_else(Ranges::full, |floor| {
        let version = floor.version.clone();
        if floor.exclusive {
            Ranges::strictly_higher_than(version)
        } else {
            Ranges::higher_than(version)
        }
    })
}

// ---- the provider over an index ----

/// A package PubGrub reasons about. The manifest is the root: it depends on
/// the direct dependencies and is not itself a package.
///
/// A package the manifest names twice (in `[dependencies]` and in
/// `[dev-dependencies]`) gets every constraint, not just the last one. The root
/// depends on the package for the first, and on a stand-in for each later one:
/// `Extra(n)`, the `n`th manifest line, which asks for that package with its
/// own constraint. Both lines then stay separate facts if they cannot hold
/// together.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Pkg {
    Root,
    /// The manifest's `n`th constraint, standing in for the root.
    Extra(usize),
    Named(String),
}

impl fmt::Display for Pkg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pkg::Root | Pkg::Extra(_) => f.write_str("lvpm.toml"),
            Pkg::Named(key) => f.write_str(key),
        }
    }
}

fn root_version() -> Version {
    Version::parse("0")
}

/// Why a listed version cannot be chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Gated {
    pub(crate) needs_year: u32,
    pub(crate) project_year: u32,
}

impl fmt::Display for Gated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "needs LabVIEW {}, and the project's is {}",
            self.needs_year, self.project_year
        )
    }
}

/// How young a version may be: nothing released less than `days` days before
/// `now` (Unix seconds) is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgeLimit {
    pub now: i64,
    pub days: u32,
}

/// Whether a version released at `released` is younger than `limit` allows. A
/// limit of 0 days holds nothing back, whatever the date, and neither does a
/// missing date; a date in the future is too new.
fn too_new(released: Option<i64>, limit: &AgeLimit) -> bool {
    if limit.days == 0 {
        return false;
    }
    released.is_some_and(|released| limit.now < released::eligible(released, limit.days))
}

/// Why a listed version is not chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Unavailable {
    Labview(Gated),
    TooNew { released: i64, days: u32 },
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unavailable::Labview(gated) => fmt::Display::fmt(gated, f),
            Unavailable::TooNew { released, days } => write!(
                f,
                "released {}, held back by min-age-days = {days}",
                released::day(*released)
            ),
        }
    }
}

/// An index's entries grouped by package: newest first, one per version. Two
/// entries that are the same version (`1.0`, `1.0.0`) keep the one listed first.
struct Catalog<'a> {
    by_key: HashMap<String, Vec<&'a Entry>>,
}

impl<'a> Catalog<'a> {
    fn new(index: &'a Index) -> Self {
        let mut by_key: HashMap<String, Vec<&Entry>> = HashMap::new();
        for entry in &index.entries {
            by_key.entry(key(&entry.name)).or_default().push(entry);
        }
        for versions in by_key.values_mut() {
            versions.sort_by(|a, b| b.version.cmp(&a.version));
            versions.dedup_by(|later, earlier| later.version == earlier.version);
        }
        Catalog { by_key }
    }

    fn versions(&self, key: &str) -> &[&'a Entry] {
        self.by_key.get(key).map_or(&[], Vec::as_slice)
    }

    fn entry(&self, key: &str, version: &Version) -> Option<&'a Entry> {
        self.versions(key)
            .iter()
            .find(|entry| entry.version == *version)
            .copied()
    }
}

/// The one way the provider fails: its token was cancelled.
#[derive(Debug, thiserror::Error)]
#[error("the solver was cancelled")]
struct Stopped;

struct Provider<'a> {
    catalog: Catalog<'a>,
    /// The manifest's constraints, by lowercased name, in file order.
    direct: Vec<(String, Ranges<Version>)>,
    prefs: HashMap<String, Version>,
    /// Lowercased names of the packages decided before the rest.
    first: HashSet<String>,
    lv_gate: Option<f64>,
    age: Option<AgeLimit>,
    /// The versions the manifest's own lines pin exactly (lowercased name,
    /// version): exempt from `age`. A feed entry's requirements never pin.
    pinned: Vec<(String, Version)>,
    cancel: &'a CancelToken,
}

impl Provider<'_> {
    /// The requirements of the chosen packages that no source lists: they are
    /// left out of the solve and reported instead.
    fn unresolved_in(&self, chosen: &BTreeMap<String, Entry>) -> Vec<Unresolved> {
        let mut unresolved: Vec<Unresolved> = chosen
            .values()
            .flat_map(|entry| {
                entry
                    .requires
                    .iter()
                    .filter(|requirement| self.is_unlisted(&requirement.name))
                    .map(|requirement| Unresolved {
                        name: requirement.name.clone(),
                        required_by: entry.name.clone(),
                    })
            })
            .collect();
        unresolved.sort_by_key(|u| (key(&u.name), key(&u.required_by)));
        // Spellings that differ only in case are one requirement; the first
        // spelling in file order stays.
        unresolved.dedup_by_key(|u| (key(&u.name), key(&u.required_by)));
        unresolved
    }

    fn is_unlisted(&self, name: &str) -> bool {
        self.catalog.versions(&key(name)).is_empty()
    }

    /// Why `entry` cannot be chosen: the LabVIEW it needs, else its age.
    fn unavailable(&self, entry: &Entry) -> Option<Unavailable> {
        if let Some(gated) = self.lv_gate.and_then(|gate| gated(entry, gate)) {
            return Some(Unavailable::Labview(gated));
        }
        let limit = self.age.as_ref()?;
        let released = entry.released?;
        let name = key(&entry.name);
        let pinned = self
            .pinned
            .iter()
            .any(|(pinned, version)| *pinned == name && *version == entry.version);
        (!pinned && too_new(Some(released), limit)).then_some(Unavailable::TooNew {
            released,
            days: limit.days,
        })
    }
}

/// The LabVIEW year `entry` needs, when that is newer than the project's
/// (`gate`, in the index's own units).
pub(crate) fn gated(entry: &Entry, gate: f64) -> Option<Gated> {
    let needs = entry.lv_min?;
    (gate + 1e-9 < needs).then(|| Gated {
        needs_year: lv_year(needs),
        project_year: lv_year(gate),
    })
}

/// LabVIEW's marketing year from its internal version (26.0 is 2026).
fn lv_year(internal: f64) -> u32 {
    2000 + internal.trunc() as u32
}

impl DependencyProvider for Provider<'_> {
    type P = Pkg;
    type V = Version;
    type VS = Ranges<Version>;
    type Priority = bool;
    type M = Unavailable;
    type Err = Stopped;

    /// The packages of `first` are decided first, and when one of them
    /// conflicts with a package decided later, that one is what gives way.
    fn prioritize(
        &self,
        package: &Pkg,
        _range: &Ranges<Version>,
        _conflicts: &PackageResolutionStatistics,
    ) -> bool {
        matches!(package, Pkg::Named(key) if self.first.contains(key))
    }

    fn should_cancel(&self) -> Result<(), Stopped> {
        match self.cancel.is_cancelled() {
            true => Err(Stopped),
            false => Ok(()),
        }
    }

    fn choose_version(
        &self,
        package: &Pkg,
        range: &Ranges<Version>,
    ) -> Result<Option<Version>, Stopped> {
        let Pkg::Named(key) = package else {
            // The root and its stand-ins have one version. Once a conflict
            // rules it out there is none left to choose.
            return Ok(range.contains(&root_version()).then(root_version));
        };
        let preferred = self
            .prefs
            .get(key)
            .filter(|version| range.contains(version))
            .and_then(|version| self.catalog.entry(key, version));
        let chosen = preferred.or_else(|| {
            self.catalog
                .versions(key)
                .iter()
                .find(|entry| range.contains(&entry.version))
                .copied()
        });
        Ok(chosen.map(|entry| entry.version.clone()))
    }

    fn get_dependencies(
        &self,
        package: &Pkg,
        version: &Version,
    ) -> Result<Dependencies<Pkg, Ranges<Version>, Unavailable>, Stopped> {
        let key = match package {
            Pkg::Named(key) => key,
            Pkg::Root => {
                let mut named = HashSet::new();
                return Ok(Dependencies::Available(
                    self.direct
                        .iter()
                        .enumerate()
                        .map(|(line, (key, range))| match named.insert(key) {
                            true => (Pkg::Named(key.clone()), range.clone()),
                            false => (Pkg::Extra(line), Ranges::full()),
                        })
                        .collect(),
                ));
            }
            Pkg::Extra(line) => {
                let (key, range) = &self.direct[*line];
                return Ok(Dependencies::Available(
                    [(Pkg::Named(key.clone()), range.clone())]
                        .into_iter()
                        .collect(),
                ));
            }
        };
        // Pubgrub only asks about versions `choose_version` returned, so the
        // entry is there; a missing one is a package with no requirements.
        let Some(entry) = self.catalog.entry(key, version) else {
            return Ok(Dependencies::Available(DependencyConstraints::default()));
        };
        if let Some(unavailable) = self.unavailable(entry) {
            return Ok(Dependencies::Unavailable(unavailable));
        }
        Ok(Dependencies::Available(
            entry
                .requires
                .iter()
                .filter(|requirement| !self.is_unlisted(&requirement.name))
                .map(|requirement| {
                    (
                        Pkg::Named(self::key(&requirement.name)),
                        floor_range(requirement.floor().as_ref()),
                    )
                })
                .collect(),
        ))
    }
}

// ---- the conflict report ----
//
// PubGrub explains a failure as a tree of derived statements. A reader needs
// the facts at its leaves: what the manifest asked for, what each package
// requires, what no source lists. They are kept as data, in the manifest's own
// spelling of each name, and `Display` is the only place they become prose.

/// No set of versions satisfies the constraints: the facts that rule it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub facts: Vec<Fact>,
}

/// One fact a conflict rests on. `wanted` and `versions` are version sets as
/// a constraint reads (`= 1.0`, `>= 2.0`, `*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    /// `lvpm.toml` asks for `package` within `wanted`.
    Manifest { package: String, wanted: String },
    /// An upgrade asked for `package` at exactly `version`, which the
    /// manifest does not say. The solver never produces it: `ops::lock` swaps
    /// it in for the `Manifest` fact of a version it pinned.
    Pinned { package: String, version: String },
    /// `package` at `versions` requires `needs` within `wanted`.
    Requires {
        package: String,
        versions: String,
        needs: String,
        wanted: String,
    },
    /// `package` at `versions`, listed by source `from`, needs a newer
    /// LabVIEW than the project's.
    NeedsLabview {
        package: String,
        versions: String,
        from: String,
        needs_year: u32,
        project_year: u32,
    },
    /// Every version of a package that the manifest or a requirement admits
    /// and the project's LabVIEW runs is younger than the age limit.
    TooNew(AllHeldBack),
    /// `package` at `version`, released at `released` (Unix seconds), is
    /// younger than `days` days. Of a package whose other versions fail for
    /// another reason too, so that not every version is held back: see
    /// `TooNew`.
    VersionTooNew {
        package: String,
        version: String,
        released: i64,
        days: u32,
    },
    /// No version source `from` lists of `package` is within `wanted`. A
    /// later source is never asked (ADR 4).
    NoVersion {
        package: String,
        wanted: String,
        from: String,
    },
    /// No source lists `package` at all.
    NotListed { package: String },
}

/// Every version of `package` that `wanted` admits and the project's LabVIEW
/// runs was released less than `days` days ago: `versions` are those, newest
/// first, with their release times (Unix seconds). `required_by` is the
/// package whose requirement asks for it, when the manifest does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllHeldBack {
    pub package: String,
    pub versions: Vec<(String, i64)>,
    pub wanted: String,
    pub required_by: Option<String>,
    pub days: u32,
}

impl Fact {
    /// Where the fact sits in a report: what was asked, then why it fails.
    fn rank(&self) -> u8 {
        match self {
            Fact::Manifest { .. } | Fact::Pinned { .. } => 0,
            Fact::Requires { .. } => 1,
            Fact::NeedsLabview { .. } | Fact::TooNew(_) | Fact::VersionTooNew { .. } => 2,
            Fact::NoVersion { .. } => 3,
            Fact::NotListed { .. } => 4,
        }
    }
}

impl fmt::Display for Fact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fact::Manifest { package, wanted } => {
                write!(f, "lvpm.toml requires {package} {wanted}")
            }
            Fact::Pinned { package, version } => {
                write!(f, "the upgrade chose {package} {version}")
            }
            Fact::Requires {
                package,
                versions,
                needs,
                wanted,
            } => write!(f, "{package} {versions} requires {needs} {wanted}"),
            Fact::NeedsLabview {
                package,
                versions,
                from,
                needs_year,
                project_year,
            } => write!(
                f,
                "{package} {versions} from {from} needs LabVIEW {needs_year}, and the project's is {project_year}"
            ),
            Fact::TooNew(AllHeldBack {
                package,
                versions,
                days,
                ..
            }) => {
                let held: Vec<&str> = versions
                    .iter()
                    .map(|(version, _)| version.as_str())
                    .collect();
                write!(
                    f,
                    "{package} {} held back by min-age-days = {days}",
                    held.join(", ")
                )
            }
            Fact::VersionTooNew {
                package,
                version,
                released,
                days,
            } => write!(
                f,
                "{package} {version} is held back by min-age-days = {days} (released {})",
                released::day(*released)
            ),
            Fact::NoVersion {
                package,
                wanted,
                from,
            } => write!(
                f,
                "no version of {package} {wanted} is available from {from}"
            ),
            Fact::NotListed { package } => write!(f, "{package} is in no configured source"),
        }
    }
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no set of versions satisfies the constraints:")?;
        self.facts
            .iter()
            .try_for_each(|fact| write!(f, "\n  {fact}"))
    }
}

/// How to spell a package: as the manifest does when it names it, else as the
/// index does.
struct Names<'a> {
    catalog: &'a Catalog<'a>,
    manifest: &'a [(String, Constraint)],
}

impl Names<'_> {
    fn of(&self, package: &Pkg) -> String {
        let Pkg::Named(key) = package else {
            return package.to_string();
        };
        let in_manifest = self
            .manifest
            .iter()
            .find(|(name, _)| self::key(name) == *key);
        let in_index = self.catalog.versions(key).first();
        match (in_manifest, in_index) {
            (Some((name, _)), _) => name.clone(),
            (None, Some(entry)) => entry.name.clone(),
            (None, None) => key.clone(),
        }
    }

    /// The one source the package's versions come from.
    fn source(&self, package: &Pkg) -> String {
        let Pkg::Named(key) = package else {
            return String::new();
        };
        let first = self.catalog.versions(key).first();
        first.map_or_else(String::new, |entry| entry.source.clone())
    }
}

type Tree = DerivationTree<Pkg, Ranges<Version>, Unavailable>;

impl Conflict {
    fn explain(tree: &Tree, names: &Names) -> Conflict {
        let mut facts = Vec::new();
        let mut held = Vec::new();
        collect_facts(tree, names, &mut HashSet::new(), &mut facts, &mut held);
        let mut facts = merge_held_back(facts, held);
        facts.sort_by_key(Fact::rank);
        Conflict { facts }
    }
}

/// A version the age limit holds back; pubgrub reports one per version.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TooNewVersion {
    package: String,
    version: String,
    released: i64,
    days: u32,
}

/// The held-back versions as facts: one `TooNew` per package every version of
/// which is held back, in place of one per version, and none of the facts it
/// makes redundant: what the manifest or a requirer asked for (kept in the
/// fact), the versions no source lists in that range, and the versions that
/// need a newer LabVIEW. The same goes for the packages that require such a
/// package and nothing else: they fail only because it does.
///
/// A held-back package that requires something does not fail by its age alone
/// (its older version may fail on that requirement), and a requirer with a
/// requirement that is not held back fails for that too. Their facts stay as
/// they are, for the general report, and the held-back versions of such a
/// package are one `VersionTooNew` each.
fn merge_held_back(facts: Vec<Fact>, held: Vec<TooNewVersion>) -> Vec<Fact> {
    let mut packages: Vec<&str> = Vec::new();
    for version in &held {
        if !packages.contains(&version.package.as_str()) {
            packages.push(&version.package);
        }
    }
    let all_held: Vec<&str> = packages
        .iter()
        .copied()
        .filter(|package| requirements(&facts, package).next().is_none())
        .collect();
    let mut failing = all_held.clone();
    while let Some(requirer) = facts.iter().find_map(|fact| match fact {
        Fact::Requires { package, .. }
            if !failing.contains(&package.as_str())
                && !packages.contains(&package.as_str())
                && requirements(&facts, package).all(|needs| failing.contains(&needs)) =>
        {
            Some(package.as_str())
        }
        _ => None,
    }) {
        failing.push(requirer);
    }
    let concerns_held = |fact: &Fact| match fact {
        Fact::NeedsLabview { package, .. } => all_held.contains(&package.as_str()),
        Fact::Manifest { package, .. } | Fact::Requires { package, .. } => {
            failing.contains(&package.as_str())
        }
        // What is left of a held-back package once its listed versions are ruled out.
        Fact::NoVersion { package, .. } => {
            failing.contains(&package.as_str()) || packages.contains(&package.as_str())
        }
        Fact::Pinned { .. }
        | Fact::NotListed { .. }
        | Fact::TooNew(_)
        | Fact::VersionTooNew { .. } => false,
    };
    let mut merged: Vec<Fact> = all_held
        .iter()
        .map(|package| Fact::TooNew(all_held_back(package, &facts, &held)))
        .collect();
    for package in packages
        .iter()
        .filter(|package| !all_held.contains(package))
    {
        merged.extend(
            newest_first(package, &held)
                .into_iter()
                .map(|held| Fact::VersionTooNew {
                    package: held.package.clone(),
                    version: held.version.clone(),
                    released: held.released,
                    days: held.days,
                }),
        );
    }
    merged.extend(facts.iter().filter(|fact| !concerns_held(fact)).cloned());
    merged
}

/// The packages `package` requires, by name.
fn requirements<'a>(facts: &'a [Fact], package: &'a str) -> impl Iterator<Item = &'a str> {
    facts.iter().filter_map(move |fact| match fact {
        Fact::Requires {
            package: requirer,
            needs,
            ..
        } if requirer == package => Some(needs.as_str()),
        _ => None,
    })
}

/// The held-back versions of `package`, newest first.
fn newest_first<'a>(package: &str, held: &'a [TooNewVersion]) -> Vec<&'a TooNewVersion> {
    let mut versions: Vec<_> = held.iter().filter(|held| held.package == package).collect();
    versions.sort_by(|a, b| Version::parse(&b.version).cmp(&Version::parse(&a.version)));
    versions
}

/// What `package`, every version of which is held back, comes to: the versions
/// newest first, and who asked for it.
fn all_held_back(package: &str, facts: &[Fact], held: &[TooNewVersion]) -> AllHeldBack {
    let versions = newest_first(package, held);
    let by_manifest = facts.iter().find_map(|fact| match fact {
        Fact::Manifest {
            package: asked,
            wanted,
        } if asked == package => Some((wanted, None)),
        _ => None,
    });
    let by_requirer = facts.iter().find_map(|fact| match fact {
        Fact::Requires {
            package: requirer,
            needs,
            wanted,
            ..
        } if needs == package => Some((wanted, Some(requirer.clone()))),
        _ => None,
    });
    // Whatever pubgrub tried was asked for by something in the tree; `*` only
    // stands in for a question nothing asks.
    let (wanted, required_by) = by_manifest.or(by_requirer).map_or_else(
        || ("*".to_string(), None),
        |(wanted, requirer)| (wanted.clone(), requirer),
    );
    AllHeldBack {
        package: package.to_string(),
        versions: versions
            .iter()
            .map(|held| (held.version.clone(), held.released))
            .collect(),
        wanted,
        required_by,
        days: versions.first().map_or(0, |held| held.days),
    }
}

/// The external incompatibilities at the tree's leaves, once each. A derived
/// node that appears more than once in the tree (`shared_id`) is walked the
/// first time only: its leaves are already collected.
fn collect_facts(
    tree: &Tree,
    names: &Names,
    walked: &mut HashSet<usize>,
    facts: &mut Vec<Fact>,
    held: &mut Vec<TooNewVersion>,
) {
    let fact = match tree {
        DerivationTree::Derived(derived) => {
            if derived.shared_id.is_some_and(|id| !walked.insert(id)) {
                return;
            }
            collect_facts(&derived.cause1, names, walked, facts, held);
            collect_facts(&derived.cause2, names, walked, facts, held);
            return;
        }
        DerivationTree::External(External::NotRoot(..)) => return,
        // The manifest and its stand-ins are not packages anyone can lack.
        DerivationTree::External(External::NoVersions(Pkg::Root | Pkg::Extra(_), _)) => return,
        DerivationTree::External(External::NoVersions(package, wanted)) => match package {
            Pkg::Named(key) if names.catalog.versions(key).is_empty() => Fact::NotListed {
                package: names.of(package),
            },
            _ => Fact::NoVersion {
                package: names.of(package),
                wanted: describe(wanted),
                from: names.source(package),
            },
        },
        // The root's hand-off to a stand-in says nothing the stand-in's own
        // line does not.
        DerivationTree::External(External::FromDependencyOf(_, _, Pkg::Extra(_), _)) => return,
        DerivationTree::External(External::FromDependencyOf(package, versions, needs, wanted)) => {
            match package {
                Pkg::Root | Pkg::Extra(_) => Fact::Manifest {
                    package: names.of(needs),
                    wanted: describe(wanted),
                },
                Pkg::Named(_) => Fact::Requires {
                    package: names.of(package),
                    versions: subject(versions),
                    needs: names.of(needs),
                    wanted: describe(wanted),
                },
            }
        }
        DerivationTree::External(External::Custom(
            package,
            versions,
            Unavailable::Labview(gated),
        )) => Fact::NeedsLabview {
            package: names.of(package),
            versions: subject(versions),
            from: names.source(package),
            needs_year: gated.needs_year,
            project_year: gated.project_year,
        },
        // One per version; `merge_held_back` makes them facts.
        DerivationTree::External(External::Custom(
            package,
            versions,
            Unavailable::TooNew { released, days },
        )) => {
            let version = TooNewVersion {
                package: names.of(package),
                version: subject(versions),
                released: *released,
                days: *days,
            };
            if !held.contains(&version) {
                held.push(version);
            }
            return;
        }
    };
    if !facts.contains(&fact) {
        facts.push(fact);
    }
}

/// A version set as a constraint reads: `*`, `= 1.0`, `>= 2.0`, `>= 1.0, < 2.0`,
/// and `or` between the pieces of a set that has gaps.
fn describe(set: &Ranges<Version>) -> String {
    let piece = |(lower, upper): (&Bound<Version>, &Bound<Version>)| match (lower, upper) {
        (Bound::Included(a), Bound::Included(b)) if a == b => format!("= {a}"),
        (Bound::Unbounded, Bound::Unbounded) => "*".to_string(),
        _ => [
            match lower {
                Bound::Included(v) => Some(format!(">= {v}")),
                Bound::Excluded(v) => Some(format!("> {v}")),
                Bound::Unbounded => None,
            },
            match upper {
                Bound::Included(v) => Some(format!("<= {v}")),
                Bound::Excluded(v) => Some(format!("< {v}")),
                Bound::Unbounded => None,
            },
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", "),
    };
    set.iter().map(piece).collect::<Vec<_>>().join(" or ")
}

/// The versions a statement is about: `2.0` for one version, else a constraint.
fn subject(set: &Ranges<Version>) -> String {
    set.as_singleton()
        .map_or_else(|| describe(set), ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Requirement;
    use std::cmp::Ordering;

    fn entry(name: &str, version: &str, requires: &[(&str, &str)]) -> Entry {
        Entry {
            name: name.to_string(),
            version: Version::parse(version),
            url: String::new(),
            md5: None,
            display_name: None,
            requires: requires
                .iter()
                .map(|(name, floor)| Requirement {
                    name: name.to_string(),
                    min: Some(Version::parse(floor)),
                    min_exclusive: false,
                })
                .collect(),
            lv_min: None,
            source: String::new(),
            released: None,
        }
    }

    fn manifest(lines: &[(&str, &str)]) -> Vec<(String, Constraint)> {
        lines
            .iter()
            .map(|(name, constraint)| {
                (
                    name.to_string(),
                    Constraint::parse(constraint).expect("a constraint"),
                )
            })
            .collect()
    }

    fn chosen(resolution: &Resolution) -> Vec<(&str, &str)> {
        resolution
            .packages
            .iter()
            .map(|(key, entry)| (key.as_str(), entry.version.raw.as_str()))
            .collect()
    }

    #[test]
    fn ord_agrees_with_eq_for_trailing_zeros_and_prerelease_tails() {
        let samples = [
            "1.0",
            "1.0.0",
            "1.0.0.0",
            "1.0.1",
            "1.0.0alpha1",
            "1.0alpha2",
            "1.0.0alpha1-1",
            "2.3-1",
            "2.3",
        ]
        .map(Version::parse);
        for a in &samples {
            for b in &samples {
                assert_eq!(
                    a == b,
                    a.cmp(b) == Ordering::Equal,
                    "{a} and {b}: Eq and Ord disagree"
                );
                assert_eq!(a.cmp(b), b.cmp(a).reverse(), "{a} and {b}: Ord is lopsided");
            }
        }
        assert_eq!(Version::parse("1.0"), Version::parse("1.0.0"));
        assert!(Version::parse("1.0.0alpha1") < Version::parse("1.0.0"));
        assert!(Version::parse("1.0.0alpha1") > Version::parse("0.9"));
    }

    #[test]
    fn a_constraint_maps_to_the_versions_it_admits() {
        let exact = range_of(&Constraint::Exact(Version::parse("1.0")));
        assert!(exact.contains(&Version::parse("1.0.0")));
        assert!(!exact.contains(&Version::parse("1.0.1")));
        let floor = range_of(&Constraint::AtLeast(Version::parse("1.2")));
        assert!(floor.contains(&Version::parse("1.2")));
        assert!(floor.contains(&Version::parse("2.0")));
        assert!(!floor.contains(&Version::parse("1.1.9")));
        assert!(range_of(&Constraint::Any).contains(&Version::parse("0.0.1")));
        assert_eq!(floor_range(None), Ranges::full());
    }

    #[test]
    fn a_strict_floor_excludes_its_own_version() {
        let strict = Floor {
            version: Version::parse("1.2"),
            exclusive: true,
        };
        let range = floor_range(Some(&strict));
        assert!(!range.contains(&Version::parse("1.2")));
        assert!(!range.contains(&Version::parse("1.2.0")));
        assert!(range.contains(&Version::parse("1.2.1")));
        assert!(
            floor_range(Some(&Floor::at_least(Version::parse("1.2")))).contains(&strict.version)
        );
    }

    #[test]
    fn a_requirement_that_needs_something_above_the_only_listed_version_conflicts() {
        let mut a = entry("a", "1.0", &[]);
        a.requires = vec![Requirement {
            name: "c".into(),
            min: Some(Version::parse("1.0")),
            min_exclusive: true,
        }];
        let index = Index {
            entries: vec![a, entry("c", "1.0", &[])],
        };
        assert!(matches!(
            run(&[("a", "*")], &index, &[], None),
            Err(SolveError::Conflict(_))
        ));
    }

    #[test]
    fn two_entries_for_the_same_version_count_once() {
        let index = Index {
            entries: vec![entry("a", "1.0", &[]), entry("a", "1.0.0", &[])],
        };
        let resolution = run(&[("a", "*")], &index, &[], None).expect("solves");
        assert_eq!(chosen(&resolution), [("a", "1.0")]);
    }

    #[test]
    fn two_prereleases_of_one_release_are_two_entries_with_their_own_archives() {
        let mut first = entry("foo", "0.1.0alpha1-1", &[]);
        first.url = "http://example/alpha1.vip".into();
        first.md5 = Some("md5-of-alpha1".into());
        let mut second = entry("foo", "0.1.0alpha2-1", &[]);
        second.url = "http://example/alpha2.vip".into();
        second.md5 = Some("md5-of-alpha2".into());
        let index = Index {
            entries: vec![first, second],
        };
        let resolution = run(&[("foo", "0.1.0alpha2-1")], &index, &[], None).expect("solves");
        let picked = &resolution.packages["foo"];
        assert_eq!(picked.version.raw, "0.1.0alpha2-1");
        assert_eq!(picked.url, "http://example/alpha2.vip");
        assert_eq!(picked.md5.as_deref(), Some("md5-of-alpha2"));
    }

    #[test]
    fn a_release_is_chosen_over_its_own_prerelease_revision() {
        let index = Index {
            entries: vec![
                entry("foo", "1.0.0alpha1-1", &[]),
                entry("foo", "1.0.0", &[]),
            ],
        };
        let resolution = run(&[("foo", "*")], &index, &[], None).expect("solves");
        assert_eq!(chosen(&resolution), [("foo", "1.0.0")]);
        assert_eq!(
            index
                .best("foo", None, None)
                .map(|e| e.version.raw.as_str()),
            Some("1.0.0")
        );
    }

    #[test]
    fn a_preference_that_no_longer_fits_is_dropped() {
        let index = Index {
            entries: vec![entry("a", "1.0", &[]), entry("a", "2.0", &[])],
        };
        let prefs = [("A".to_string(), Version::parse("1.0"))];
        let resolution = run(&[("a", ">=1.5")], &index, &prefs, None).expect("solves");
        assert_eq!(chosen(&resolution), [("a", "2.0")]);
    }

    #[test]
    fn a_package_reached_only_through_others_moves_to_its_newest_and_takes_a_held_sibling_along() {
        // d needs e, which needs x, and s; x 2.0 needs s 2.0; the lock holds s
        // 1.0, and the upgrade is of x alone (AC-U2: whatever no longer
        // satisfies the new set moves with it).
        let index = Index {
            entries: vec![
                entry("d", "1.0", &[("e", "1.0"), ("s", "1.0")]),
                entry("e", "1.0", &[("x", "1.0")]),
                entry("x", "1.0", &[]),
                entry("x", "2.0", &[("s", "2.0")]),
                entry("s", "1.0", &[]),
                entry("s", "2.0", &[]),
            ],
        };
        let held = [("d", "1.0"), ("e", "1.0"), ("s", "1.0")]
            .map(|(name, version)| (name.to_string(), Version::parse(version)));
        let resolution = solve(
            &manifest(&[("d", "*")]),
            &index,
            &held,
            &["X".to_string()],
            None,
            None,
            &CancelToken::new(),
        )
        .expect("solves");
        assert_eq!(
            chosen(&resolution),
            [("d", "1.0"), ("e", "1.0"), ("s", "2.0"), ("x", "2.0")]
        );
    }

    #[test]
    fn version_sets_read_as_constraints() {
        let v = Version::parse;
        assert_eq!(describe(&Ranges::full()), "*");
        assert_eq!(describe(&Ranges::singleton(v("1.0"))), "= 1.0");
        assert_eq!(describe(&Ranges::higher_than(v("2.0"))), ">= 2.0");
        assert_eq!(describe(&Ranges::strictly_higher_than(v("2.0"))), "> 2.0");
        assert_eq!(
            describe(&Ranges::between(v("1.0"), v("2.0"))),
            ">= 1.0, < 2.0"
        );
        assert_eq!(
            describe(&Ranges::singleton(v("2.0")).complement()),
            "< 2.0 or > 2.0"
        );
        assert_eq!(subject(&Ranges::singleton(v("2.0"))), "2.0");
        assert_eq!(subject(&Ranges::higher_than(v("1.0"))), ">= 1.0");
    }

    #[test]
    fn a_conflict_lists_what_the_manifest_asked_and_what_a_package_requires() {
        let index = Index {
            entries: vec![entry("A", "2.0", &[("C", "2.0")]), entry("C", "1.0", &[])],
        };
        let Err(SolveError::Conflict(conflict)) =
            run(&[("A", "*"), ("C", "1.0")], &index, &[], None)
        else {
            panic!("expected a conflict");
        };
        assert!(conflict.facts.contains(&Fact::Manifest {
            package: "A".into(),
            wanted: "*".into()
        }));
        assert!(conflict.facts.contains(&Fact::Manifest {
            package: "C".into(),
            wanted: "= 1.0".into()
        }));
        assert!(conflict.facts.contains(&Fact::Requires {
            package: "A".into(),
            versions: "2.0".into(),
            needs: "C".into(),
            wanted: ">= 2.0".into()
        }));
        let report = conflict.to_string();
        for line in ["lvpm.toml requires C = 1.0", "A 2.0 requires C >= 2.0"] {
            assert!(report.contains(line), "{line:?} is not in:\n{report}");
        }
        // the manifest's lines come first, each fact once
        assert!(matches!(conflict.facts[0], Fact::Manifest { .. }));
        let mut once = conflict.facts.clone();
        once.dedup();
        assert_eq!(once, conflict.facts);
    }

    #[test]
    fn a_gate_that_rules_out_every_version_is_named() {
        let mut gated = entry("alpha", "2.0", &[]);
        gated.lv_min = Some(26.0);
        gated.source = "hosted".into();
        let index = Index {
            entries: vec![gated],
        };
        let Err(SolveError::Conflict(conflict)) = run(&[("alpha", "*")], &index, &[], Some(24.0))
        else {
            panic!("expected a conflict");
        };
        assert!(conflict.facts.contains(&Fact::NeedsLabview {
            package: "alpha".into(),
            versions: "2.0".into(),
            from: "hosted".into(),
            needs_year: 2026,
            project_year: 2024
        }));
    }

    #[test]
    fn a_version_miss_names_the_source_of_the_package() {
        let mut listed = entry("alpha", "1.0", &[]);
        listed.source = "http://h/first/index.vipr".into();
        let index = Index {
            entries: vec![listed],
        };
        let Err(SolveError::Conflict(conflict)) = run(&[("alpha", ">=2.0")], &index, &[], None)
        else {
            panic!("expected a conflict");
        };
        assert!(
            conflict
                .to_string()
                .contains("no version of alpha >= 2.0 is available from http://h/first/index.vipr"),
            "{conflict}"
        );
    }

    #[test]
    fn a_direct_dependency_no_source_lists_is_not_listed() {
        let index = Index { entries: vec![] };
        let Err(SolveError::Conflict(conflict)) = run(&[("Zeta", "*")], &index, &[], None) else {
            panic!("expected a conflict");
        };
        assert!(conflict.facts.contains(&Fact::NotListed {
            package: "Zeta".into()
        }));
        assert!(
            conflict
                .to_string()
                .contains("Zeta is in no configured source")
        );
    }

    #[test]
    fn unlisted_requirements_are_reported_once_per_requirer_and_never_block() {
        let index = Index {
            entries: vec![
                entry(
                    "beta",
                    "1.0",
                    &[("Ni_Lib_Missing", "1.0"), ("other_missing", "1.0")],
                ),
                entry("gamma", "1.0", &[("ni_lib_missing", "2.0")]),
            ],
        };
        let resolution = run(&[("beta", "*"), ("gamma", "*")], &index, &[], None).expect("solves");
        assert_eq!(chosen(&resolution), [("beta", "1.0"), ("gamma", "1.0")]);
        assert_eq!(
            resolution.unresolved,
            [
                Unresolved {
                    name: "Ni_Lib_Missing".into(),
                    required_by: "beta".into()
                },
                Unresolved {
                    name: "ni_lib_missing".into(),
                    required_by: "gamma".into()
                },
                Unresolved {
                    name: "other_missing".into(),
                    required_by: "beta".into()
                },
            ]
        );
    }

    #[test]
    fn requirements_that_differ_only_in_case_are_one_unresolved_entry() {
        let index = Index {
            entries: vec![entry("beta", "1.0", &[("Foo", "1.0"), ("foo", "2.0")])],
        };
        let resolution = run(&[("beta", "*")], &index, &[], None).expect("solves");
        assert_eq!(
            resolution.unresolved,
            [Unresolved {
                name: "Foo".into(),
                required_by: "beta".into()
            }]
        );
    }

    #[test]
    fn a_shared_subtree_is_walked_once() {
        use pubgrub::Derived;
        use std::sync::Arc;

        let index = Index { entries: vec![] };
        let manifest = manifest(&[("a", "*")]);
        let names = Names {
            catalog: &Catalog::new(&index),
            manifest: &manifest,
        };
        let a = || Pkg::Named("a".into());
        let mut tree = Tree::External(External::FromDependencyOf(
            Pkg::Root,
            Ranges::full(),
            a(),
            Ranges::full(),
        ));
        // Each level uses the level below as both causes: 2^64 walks without
        // memoisation, 64 with it.
        for id in 0..64 {
            let below = Arc::new(tree);
            tree = Tree::Derived(Derived {
                terms: Default::default(),
                shared_id: Some(id),
                cause1: below.clone(),
                cause2: below,
            });
        }
        let conflict = Conflict::explain(&tree, &names);
        assert_eq!(
            conflict.facts,
            [Fact::Manifest {
                package: "a".into(),
                wanted: "*".into()
            }]
        );
    }

    #[test]
    fn a_requirement_of_a_version_that_was_not_chosen_is_not_reported() {
        let index = Index {
            entries: vec![
                entry("a", "2.0", &[("gone", "1.0"), ("c", "2.0")]),
                entry("a", "1.0", &[]),
                entry("c", "1.0", &[]),
            ],
        };
        let resolution = run(&[("a", "*"), ("c", "1.0")], &index, &[], None).expect("solves");
        assert_eq!(chosen(&resolution), [("a", "1.0"), ("c", "1.0")]);
        assert!(resolution.unresolved.is_empty());
    }

    #[test]
    fn a_cancelled_token_stops_the_solver() {
        let index = Index {
            entries: vec![entry("a", "1.0", &[])],
        };
        let token = CancelToken::new();
        token.cancel();
        let result = solve(
            &manifest(&[("a", "*")]),
            &index,
            &[],
            &[],
            None,
            None,
            &token,
        );
        assert!(matches!(result, Err(SolveError::Cancelled)));
    }

    #[test]
    fn a_package_the_manifest_names_twice_must_satisfy_both_lines() {
        let index = Index {
            entries: vec![
                entry("gamma", "1.0", &[]),
                entry("gamma", "1.5", &[]),
                entry("gamma", "2.0", &[]),
            ],
        };
        // Neither line overwrites the other, whichever comes last, and
        // however the name is spelled.
        for lines in [
            [("gamma", ">=1.0"), ("gamma", "1.5")],
            [("gamma", "1.5"), ("gamma", ">=1.0")],
            [("Gamma", "1.5"), ("gamma", "*")],
        ] {
            let resolution = run(&lines, &index, &[], None).expect("solves");
            assert_eq!(chosen(&resolution), [("gamma", "1.5")], "{lines:?}");
        }
    }

    #[test]
    fn two_lines_for_one_package_that_cannot_both_hold_are_both_in_the_conflict() {
        let index = Index {
            entries: vec![entry("gamma", "1.0", &[]), entry("gamma", "1.5", &[])],
        };
        let Err(SolveError::Conflict(conflict)) =
            run(&[("gamma", "1.0"), ("gamma", "1.5")], &index, &[], None)
        else {
            panic!("expected a conflict");
        };
        for wanted in ["= 1.0", "= 1.5"] {
            assert!(
                conflict.facts.contains(&Fact::Manifest {
                    package: "gamma".into(),
                    wanted: wanted.into()
                }),
                "{wanted} is not in:\n{conflict}"
            );
        }
        assert_eq!(conflict.facts.len(), 2, "{conflict}");
    }

    // ---- the age limit ----

    const NOW: i64 = 1_700_000_000;
    const DAY: i64 = 86_400;
    const LIMIT_DAYS: u32 = 14;
    const LIMIT: Option<AgeLimit> = Some(AgeLimit {
        now: NOW,
        days: LIMIT_DAYS,
    });

    /// An entry released `days_ago` days before `NOW`.
    fn released(name: &str, version: &str, days_ago: i64) -> Entry {
        Entry {
            released: Some(NOW - days_ago * DAY),
            ..entry(name, version, &[])
        }
    }

    fn requiring(mut entry: Entry, name: &str, floor: &str) -> Entry {
        entry.requires.push(Requirement {
            name: name.to_string(),
            min: Some(Version::parse(floor)),
            min_exclusive: false,
        });
        entry
    }

    fn held_back_facts(result: Result<Resolution, SolveError>) -> Vec<Fact> {
        match result {
            Err(SolveError::Conflict(conflict)) => conflict.facts,
            other => panic!("expected a conflict, got {other:?}"),
        }
    }

    #[test]
    fn a_version_exactly_the_limit_old_is_eligible_and_one_second_younger_is_not() {
        let limit = LIMIT.unwrap();
        let age = i64::from(LIMIT_DAYS) * DAY;
        assert!(!too_new(Some(NOW - age), &limit));
        assert!(too_new(Some(NOW - age + 1), &limit));
        assert!(!too_new(Some(NOW - age - 1), &limit));
    }

    #[test]
    fn a_release_date_in_the_future_is_too_new() {
        assert!(too_new(Some(NOW + 1), &LIMIT.unwrap()));
    }

    #[test]
    fn a_limit_of_zero_days_holds_nothing_back_not_even_the_future() {
        let off = AgeLimit { now: NOW, days: 0 };
        assert!(!too_new(Some(NOW), &off));
        assert!(!too_new(Some(NOW + DAY), &off));
        assert!(!too_new(None, &off));
    }

    #[test]
    fn a_version_with_no_release_date_is_eligible() {
        assert!(!too_new(None, &LIMIT.unwrap()));
    }

    #[test]
    fn the_largest_limit_does_not_overflow() {
        let huge = AgeLimit {
            now: i64::MAX,
            days: u32::MAX,
        };
        assert!(!too_new(Some(0), &huge));
        assert!(too_new(Some(0), &AgeLimit { now: NOW, ..huge }));
        assert!(too_new(Some(NOW), &AgeLimit { now: NOW, ..huge }));
        // The day it becomes eligible does not fit an i64: it saturates, and a
        // clock just short of the end of time is still before it.
        let near_the_end = AgeLimit {
            now: i64::MAX - 1,
            days: u32::MAX,
        };
        assert!(too_new(Some(i64::MAX - 10), &near_the_end));
    }

    #[test]
    fn a_version_younger_than_the_limit_is_held_back_for_the_newest_old_enough() {
        let index = Index {
            entries: vec![released("a", "1.0", 30), released("a", "1.1", 3)],
        };
        let resolution = run_aged(&[("a", ">=1.0")], &index, &[], None, LIMIT).expect("solves");
        assert_eq!(chosen(&resolution), [("a", "1.0")]);
        let unlimited = run_aged(&[("a", ">=1.0")], &index, &[], None, None).expect("solves");
        assert_eq!(chosen(&unlimited), [("a", "1.1")]);
    }

    #[test]
    fn a_locked_version_that_is_now_too_new_gives_way_to_an_older_one() {
        let index = Index {
            entries: vec![released("a", "1.0", 30), released("a", "1.1", 10)],
        };
        let prefs = [("a".to_string(), Version::parse("1.1"))];
        let resolution = run_aged(&[("a", "*")], &index, &prefs, None, LIMIT).expect("solves");
        assert_eq!(chosen(&resolution), [("a", "1.0")]);
    }

    #[test]
    fn a_transitive_requirement_is_held_to_the_limit_too() {
        let index = Index {
            entries: vec![
                requiring(released("a", "1.0", 30), "b", "2.0"),
                released("b", "2.0", 30),
                released("b", "2.1", 3),
            ],
        };
        let resolution = run_aged(&[("a", "*")], &index, &[], None, LIMIT).expect("solves");
        assert_eq!(chosen(&resolution), [("a", "1.0"), ("b", "2.0")]);
    }

    #[test]
    fn a_version_the_manifest_pins_exactly_is_exempt_and_only_that_version() {
        let index = Index {
            entries: vec![
                released("a", "1.0", 30),
                released("a", "1.1", 3),
                released("a", "1.2", 2),
            ],
        };
        let pinned = run_aged(&[("a", "1.1")], &index, &[], None, LIMIT).expect("solves");
        assert_eq!(chosen(&pinned), [("a", "1.1")]);
        // A second line for the package does not take the pin's exemption away.
        let both = run_aged(&[("a", "1.1"), ("a", ">=1.0")], &index, &[], None, LIMIT);
        assert_eq!(chosen(&both.expect("solves")), [("a", "1.1")]);
        // A floor is not a pin: `>=1.1` admits only versions that are too new.
        let floor = held_back_facts(run_aged(&[("a", ">=1.1")], &index, &[], None, LIMIT));
        assert_eq!(
            floor,
            [Fact::TooNew(AllHeldBack {
                package: "a".into(),
                versions: vec![("1.2".into(), NOW - 2 * DAY), ("1.1".into(), NOW - 3 * DAY)],
                wanted: ">= 1.1".into(),
                required_by: None,
                days: LIMIT_DAYS,
            })]
        );
    }

    #[test]
    fn an_exact_pin_exempts_the_pinned_package_but_not_what_it_requires() {
        let index = Index {
            entries: vec![
                requiring(released("a", "1.1", 3), "b", "2.0"),
                released("b", "2.0", 30),
                released("b", "2.1", 3),
            ],
        };
        let resolution = run_aged(&[("a", "1.1")], &index, &[], None, LIMIT).expect("solves");
        assert_eq!(chosen(&resolution), [("a", "1.1"), ("b", "2.0")]);
    }

    #[test]
    fn every_version_held_back_is_one_fact_listing_them_newest_first() {
        let index = Index {
            entries: vec![
                released("a", "1.0", 30),
                released("a", "1.1", 3),
                released("a", "1.2", 2),
            ],
        };
        let facts = held_back_facts(run_aged(&[("a", ">=1.1")], &index, &[], None, LIMIT));
        assert_eq!(
            facts,
            [Fact::TooNew(AllHeldBack {
                package: "a".into(),
                versions: vec![("1.2".into(), NOW - 2 * DAY), ("1.1".into(), NOW - 3 * DAY)],
                wanted: ">= 1.1".into(),
                required_by: None,
                days: LIMIT_DAYS,
            })]
        );
    }

    #[test]
    fn a_requirement_with_no_version_old_enough_names_who_asked() {
        let index = Index {
            entries: vec![
                requiring(released("a", "1.0", 30), "b", "2.0"),
                released("b", "2.0", 3),
            ],
        };
        let facts = held_back_facts(run_aged(&[("a", "*")], &index, &[], None, LIMIT));
        let [
            Fact::TooNew(AllHeldBack {
                package,
                wanted,
                required_by,
                ..
            }),
        ] = &facts[..]
        else {
            panic!("expected one TooNew fact, got {facts:?}");
        };
        assert_eq!(
            (package.as_str(), wanted.as_str(), required_by.as_deref()),
            ("b", ">= 2.0", Some("a"))
        );
    }

    #[test]
    fn a_version_the_project_labview_cannot_run_is_not_among_those_held_back() {
        let mut gated = released("a", "3.0", 30);
        gated.lv_min = Some(26.0);
        let index = Index {
            entries: vec![released("a", "1.1", 3), gated],
        };
        let facts = held_back_facts(run_aged(&[("a", ">=1.1")], &index, &[], Some(24.0), LIMIT));
        let [Fact::TooNew(AllHeldBack { versions, .. })] = &facts[..] else {
            panic!("expected only a TooNew fact, got {facts:?}");
        };
        assert_eq!(versions, &[("1.1".to_string(), NOW - 3 * DAY)]);
    }

    #[test]
    fn each_reason_a_version_is_unavailable_says_what_it_is() {
        let too_new = Unavailable::TooNew {
            released: NOW,
            days: LIMIT_DAYS,
        };
        assert_eq!(
            too_new.to_string(),
            "released 2023-11-14, held back by min-age-days = 14"
        );
        let gated = Unavailable::Labview(Gated {
            needs_year: 2026,
            project_year: 2024,
        });
        assert_eq!(
            gated.to_string(),
            "needs LabVIEW 2026, and the project's is 2024"
        );
    }

    fn too_new_version(package: &str, version: &str, days_ago: i64) -> TooNewVersion {
        TooNewVersion {
            package: package.into(),
            version: version.into(),
            released: NOW - days_ago * DAY,
            days: LIMIT_DAYS,
        }
    }

    fn manifest_fact(package: &str, wanted: &str) -> Fact {
        Fact::Manifest {
            package: package.into(),
            wanted: wanted.into(),
        }
    }

    fn requires_fact(package: &str, versions: &str, needs: &str, wanted: &str) -> Fact {
        Fact::Requires {
            package: package.into(),
            versions: versions.into(),
            needs: needs.into(),
            wanted: wanted.into(),
        }
    }

    /// Every one of `versions` (with the days ago it was released) of `package` held back.
    fn all_held_back_of(package: &str, versions: &[(&str, i64)], wanted: &str) -> AllHeldBack {
        AllHeldBack {
            package: package.into(),
            versions: versions
                .iter()
                .map(|(version, days_ago)| (version.to_string(), NOW - days_ago * DAY))
                .collect(),
            wanted: wanted.into(),
            required_by: None,
            days: LIMIT_DAYS,
        }
    }

    fn all_held_back_for(requirer: &str, held: AllHeldBack) -> Fact {
        Fact::TooNew(AllHeldBack {
            required_by: Some(requirer.into()),
            ..held
        })
    }

    fn version_too_new(package: &str, version: &str, days_ago: i64) -> Fact {
        Fact::VersionTooNew {
            package: package.into(),
            version: version.into(),
            released: NOW - days_ago * DAY,
            days: LIMIT_DAYS,
        }
    }

    #[test]
    fn a_held_back_package_leaves_the_facts_about_other_packages_alone() {
        let unrelated = [
            manifest_fact("c", "= 1.0"),
            Fact::NotListed {
                package: "z".into(),
            },
        ];
        let facts = vec![
            manifest_fact("a", ">= 1.0"),
            unrelated[0].clone(),
            unrelated[1].clone(),
        ];
        let held = vec![
            too_new_version("a", "1.1", 3),
            too_new_version("a", "1.2", 2),
        ];
        assert_eq!(
            merge_held_back(facts, held),
            [
                Fact::TooNew(all_held_back_of("a", &[("1.2", 2), ("1.1", 3)], ">= 1.0")),
                unrelated[0].clone(),
                unrelated[1].clone(),
            ]
        );
    }

    #[test]
    fn a_package_whose_older_version_needs_something_held_back_is_not_all_held_back() {
        // a 1.1 is too new; a 1.0 is old enough but needs b >= 2.0, and b 2.0
        // is too new: "every version of a is held back" would be false.
        let index = Index {
            entries: vec![
                released("a", "1.1", 3),
                requiring(released("a", "1.0", 30), "b", "2.0"),
                released("b", "2.0", 3),
            ],
        };
        let facts = held_back_facts(run_aged(&[("a", "*")], &index, &[], None, LIMIT));
        assert_eq!(
            facts,
            [
                manifest_fact("a", "*"),
                requires_fact("a", "1.0", "b", ">= 2.0"),
                all_held_back_for("a", all_held_back_of("b", &[("2.0", 3)], ">= 2.0")),
                version_too_new("a", "1.1", 3),
            ]
        );
    }

    #[test]
    fn a_package_whose_older_version_needs_what_cannot_be_had_is_not_all_held_back() {
        // a 1.0 needs c >= 2.0, and the manifest pins c 1.0.
        let index = Index {
            entries: vec![
                released("a", "1.1", 3),
                requiring(released("a", "1.0", 30), "c", "2.0"),
                released("c", "1.0", 30),
            ],
        };
        let facts = held_back_facts(run_aged(
            &[("a", "*"), ("c", "1.0")],
            &index,
            &[],
            None,
            LIMIT,
        ));
        assert_eq!(
            facts,
            [
                manifest_fact("a", "*"),
                manifest_fact("c", "= 1.0"),
                requires_fact("a", "1.0", "c", ">= 2.0"),
                version_too_new("a", "1.1", 3),
            ]
        );
        assert!(
            Conflict { facts }
                .to_string()
                .contains("a 1.1 is held back by min-age-days = 14 (released 2023-11-11)")
        );
    }

    #[test]
    fn a_requirer_that_needs_only_held_back_packages_fails_only_because_of_them() {
        let facts = vec![
            manifest_fact("a", "*"),
            requires_fact("a", "1.0", "b", ">= 2.0"),
            requires_fact("a", "1.0", "d", ">= 1.0"),
        ];
        let held = vec![
            too_new_version("b", "2.0", 3),
            too_new_version("d", "1.0", 3),
        ];
        assert_eq!(
            merge_held_back(facts, held),
            [
                all_held_back_for("a", all_held_back_of("b", &[("2.0", 3)], ">= 2.0")),
                all_held_back_for("a", all_held_back_of("d", &[("1.0", 3)], ">= 1.0")),
            ]
        );
    }

    #[test]
    fn a_requirer_with_a_requirement_that_is_not_held_back_keeps_its_own_facts() {
        let facts = vec![
            manifest_fact("a", "*"),
            requires_fact("a", "1.0", "b", ">= 2.0"),
            requires_fact("a", "1.0", "c", ">= 2.0"),
            Fact::NoVersion {
                package: "c".into(),
                wanted: ">= 2.0".into(),
                from: String::new(),
            },
        ];
        let merged = merge_held_back(facts.clone(), vec![too_new_version("b", "2.0", 3)]);
        assert_eq!(
            merged,
            [
                vec![all_held_back_for(
                    "a",
                    all_held_back_of("b", &[("2.0", 3)], ">= 2.0")
                )],
                facts
            ]
            .concat()
        );
    }

    #[test]
    fn a_version_exactly_the_limit_old_is_chosen_by_the_solver() {
        let index = Index {
            entries: vec![
                released("a", "1.0", 30),
                released("a", "1.1", i64::from(LIMIT_DAYS)),
                released("a", "1.2", i64::from(LIMIT_DAYS) - 1),
            ],
        };
        let resolution = run_aged(&[("a", ">=1.0")], &index, &[], None, LIMIT).expect("solves");
        assert_eq!(chosen(&resolution), [("a", "1.1")]);
    }

    #[test]
    fn the_project_labview_wins_over_an_exact_pin_that_is_too_new() {
        let mut gated = released("a", "1.1", 3);
        gated.lv_min = Some(26.0);
        let index = Index {
            entries: vec![released("a", "1.0", 30), gated],
        };
        let Err(SolveError::Conflict(conflict)) =
            run_aged(&[("a", "1.1")], &index, &[], Some(24.0), LIMIT)
        else {
            panic!("expected a conflict");
        };
        assert!(
            conflict.facts.contains(&Fact::NeedsLabview {
                package: "a".into(),
                versions: "1.1".into(),
                from: String::new(),
                needs_year: 2026,
                project_year: 2024,
            }),
            "{conflict}"
        );
    }

    fn run(
        lines: &[(&str, &str)],
        index: &Index,
        prefs: &[(String, Version)],
        lv_gate: Option<f64>,
    ) -> Result<Resolution, SolveError> {
        run_aged(lines, index, prefs, lv_gate, None)
    }

    fn run_aged(
        lines: &[(&str, &str)],
        index: &Index,
        prefs: &[(String, Version)],
        lv_gate: Option<f64>,
        age: Option<AgeLimit>,
    ) -> Result<Resolution, SolveError> {
        solve(
            &manifest(lines),
            index,
            prefs,
            &[],
            lv_gate,
            age,
            &CancelToken::new(),
        )
    }
}
