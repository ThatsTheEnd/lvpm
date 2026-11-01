//! GitHub dependencies in the lock's index.
//!
//! A dependency with a `github` line comes from its repo's releases and from
//! nowhere else, so each one first loses every entry of its name that a feed
//! listed. The feed's index lists every version without downloading any; a
//! repo only lists file names, so the entries the solver sees are made here,
//! from the bytes of the files themselves: the locked file, or else the newest
//! file that runs on the project's LabVIEW and the newer ones that do not, at
//! most `MAX_WALK` of them.
//!
//! shortcut: only the files read reach the solver, so a conflict that an older
//! release would solve fails, and `upgrade pkg@X` to a version that was not
//! read says no such version; upgrade to lazy per-version downloads when
//! someone hits it.

use super::env::OpEnv;
use super::lock::{Moving, github_lock_source, lv_gate};
use super::session::Downloads;
use crate::cache::Stored;
use crate::errors::{Cancelled, GitHubError, GitHubFault, LockError};
use crate::events::Event;
use crate::github::{Listing, ReleaseFile};
use crate::index::{self, Entry, Index};
use crate::lockfile::{Lockfile, constraint_text};
use crate::project::{Constraint, Project, same_name};
use crate::version::Version;
use anyhow::Context;
use std::path::Path;

/// The most files the walk down the releases downloads for one package: a repo
/// whose newest releases all need a newer LabVIEW than this many is told so by
/// the solver, which has the entries that were read.
const MAX_WALK: usize = 5;

/// One dependency with a `github` line.
struct Dependency<'a> {
    /// As the manifest writes it.
    name: &'a str,
    /// `owner/repo`.
    repo: &'a str,
    /// What `[dependencies]` and `[dev-dependencies]` ask of it.
    constraints: Vec<&'a Constraint>,
}

impl Dependency<'_> {
    fn admits(&self, version: &Version) -> bool {
        self.constraints
            .iter()
            .all(|constraint| constraint.admits(version))
    }

    fn fail(&self, fault: GitHubFault) -> LockError {
        let error = GitHubError {
            name: self.name.to_string(),
            repo: self.repo.to_string(),
            fault,
        };
        LockError::Other(error.into())
    }
}

/// `index`, with every GitHub dependency of `project` taken from its repo.
/// `previous` is the lock the run starts from and `moving` says which of its
/// versions may move; the releases are listed again for a package that may
/// move, and when the run refreshes.
pub(super) fn with_github(
    env: &OpEnv,
    downloads: &Downloads,
    project: &Project,
    previous: Option<&Lockfile>,
    moving: &Moving,
    mut index: Index,
) -> Result<Index, LockError> {
    if project.github.is_empty() {
        return Ok(index);
    }
    let gate = lv_gate(project.labview.as_deref().ok_or(LockError::NoLabview)?)?;
    for (name, repo) in &project.github {
        if env.cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        index.entries.retain(|entry| !same_name(&entry.name, name));
        let dependency = Dependency {
            name,
            repo,
            constraints: project
                .dependencies
                .iter()
                .chain(&project.dev_dependencies)
                .filter(|(entry, _)| same_name(entry, name))
                .map(|(_, constraint)| constraint)
                .collect(),
        };
        let locked = locked_version(previous, moving, &dependency);
        let entries = entries_of(env, downloads, &dependency, locked, gate, moving)?;
        index.entries.extend(entries);
    }
    Ok(index)
}

/// The version and SHA-256 the lock holds for `dependency`, when the run keeps
/// them: the lock took it from this repo, it is not moving, and its constraint
/// still admits the version.
fn locked_version(
    previous: Option<&Lockfile>,
    moving: &Moving,
    dependency: &Dependency,
) -> Option<(Version, String)> {
    let package = previous?
        .packages
        .iter()
        .find(|package| same_name(&package.name, dependency.name))?;
    let version = Version::parse(&package.version);
    let from_repo = package
        .source
        .eq_ignore_ascii_case(&github_lock_source(dependency.repo));
    let keeps = from_repo && !moving.moves(dependency.name) && dependency.admits(&version);
    keeps.then(|| (version, package.sha256.to_lowercase()))
}

/// The entries the solver gets for `dependency`: the locked file, from the
/// cache when it is there (nothing is listed or downloaded; a refresh looks at
/// the repo anyway), else the files the repo's releases hold.
fn entries_of(
    env: &OpEnv,
    downloads: &Downloads,
    dependency: &Dependency,
    locked: Option<(Version, String)>,
    gate: f64,
    moving: &Moving,
) -> Result<Vec<Entry>, LockError> {
    if !env.refresh
        && let Some((version, sha256)) = &locked
        && let Stored::Intact(bytes) = downloads.stored(sha256)
    {
        let file = format!("{}-{version}", dependency.name);
        let url = format!("cache:{sha256}");
        let entry = entry_of(dependency, &bytes, &url, &file, version)?;
        // A pin that no longer runs on the project's LabVIEW is looked for anew.
        if entry.fits_labview(gate) {
            return Ok(vec![entry]);
        }
    }

    let list = |refresh| {
        let (api, repo, name) = (&env.github.api, dependency.repo, dependency.name);
        downloads
            .releases(api, repo, name, refresh)
            .map_err(lock_error)
    };
    let mut listing = list(env.refresh || moving.moves(dependency.name))?;
    // A cached listing is as old as the lock that made it. When it lacks what
    // this run needs (the locked file, or a release the constraint admits),
    // GitHub is asked once more rather than a newer pin being given up for an
    // older release, or a release that exists being reported as missing.
    let enough = match &locked {
        Some((version, _)) => listing.files.iter().any(|file| file.version == *version),
        None => listing
            .files
            .iter()
            .any(|file| dependency.admits(&file.version)),
    };
    if listing.from_cache && !enough {
        listing = list(true)?;
    }
    let read = from_listing(env, downloads, dependency, &locked, gate, &listing);
    // A file uploaded again (`--clobber`) has a new asset id, so a cached
    // listing may name one that is gone: GitHub is asked once more.
    if listing.from_cache && is_not_found(&read) {
        return from_listing(env, downloads, dependency, &locked, gate, &list(true)?);
    }
    read
}

/// Whether `read` failed because GitHub has no such file.
fn is_not_found<T>(read: &Result<T, LockError>) -> bool {
    let Err(LockError::Other(why)) = read else {
        return false;
    };
    matches!(
        why.downcast_ref::<GitHubError>(),
        Some(GitHubError {
            fault: GitHubFault::NotFound { .. },
            ..
        })
    )
}

/// The entries of `dependency` in what `listing` holds: the locked file, if it
/// is still in a release, else the newest ones that fit; see [`walk`].
fn from_listing(
    env: &OpEnv,
    downloads: &Downloads,
    dependency: &Dependency,
    locked: &Option<(Version, String)>,
    gate: f64,
    listing: &Listing,
) -> Result<Vec<Entry>, LockError> {
    env.sink.emit(Event::GitHubListed {
        name: dependency.name.to_string(),
        repo: dependency.repo.to_string(),
        cached: listing.from_cache,
    });

    let mut admitted: Vec<&ReleaseFile> = listing
        .files
        .iter()
        .filter(|file| dependency.admits(&file.version))
        .collect();
    admitted.sort_by(|a, b| b.version.cmp(&a.version));
    if admitted.is_empty() {
        let skipped = listing.skipped;
        let fault = match listing.files.iter().map(|file| &file.version).max() {
            Some(newest) => GitHubFault::NoFittingVersion {
                constraint: dependency
                    .constraints
                    .iter()
                    .map(|constraint| constraint_text(constraint))
                    .collect::<Vec<_>>()
                    .join(" and "),
                newest: newest.raw.clone(),
                skipped,
            },
            None => GitHubFault::NoReleaseFile {
                name: dependency.name.to_string(),
                skipped,
            },
        };
        return Err(dependency.fail(fault));
    }

    let wanted = pin_first(admitted, locked.as_ref().map(|(version, _)| version));
    walk(&wanted, gate, |file| download(downloads, dependency, file))
}

/// `admitted` (newest first) with the locked file, when a release still holds
/// it, moved to the front: the walk then stops at the pin while it runs on the
/// project's LabVIEW, and goes on to the newest that does when it no longer does.
fn pin_first<'a>(admitted: Vec<&'a ReleaseFile>, locked: Option<&Version>) -> Vec<&'a ReleaseFile> {
    let (mut pinned, rest): (Vec<_>, Vec<_>) = admitted
        .into_iter()
        .partition(|file| locked == Some(&file.version));
    pinned.truncate(1);
    pinned.extend(rest);
    pinned
}

/// The entries of `files`, newest first, each made by `entry_of`, until one
/// runs on LabVIEW `gate` or the walk's bound is reached. The ones that do not
/// run on it stay in, so that the solver can say what they need.
fn walk(
    files: &[&ReleaseFile],
    gate: f64,
    mut entry_of: impl FnMut(&ReleaseFile) -> Result<Entry, LockError>,
) -> Result<Vec<Entry>, LockError> {
    let mut entries = Vec::new();
    for file in files.iter().take(MAX_WALK) {
        let entry = entry_of(file)?;
        let fits = entry.fits_labview(gate);
        entries.push(entry);
        if fits {
            break;
        }
    }
    Ok(entries)
}

/// The entry for `file`, made from its bytes.
fn download(
    downloads: &Downloads,
    dependency: &Dependency,
    file: &ReleaseFile,
) -> Result<Entry, LockError> {
    let asked = listed(dependency.name, dependency.repo, file);
    let bytes = downloads.fetch(&asked).map_err(lock_error)?;
    entry_of(dependency, &bytes, &file.url, &file.file, &file.version)
}

/// The entry a release file is before its bytes are read: where to get it, and
/// nothing the file itself says.
pub(super) fn listed(name: &str, repo: &str, file: &ReleaseFile) -> Entry {
    Entry {
        name: name.to_string(),
        version: file.version.clone(),
        url: file.url.clone(),
        md5: None,
        display_name: None,
        requires: Vec::new(),
        lv_min: None,
        released: None,
        source: index::github_source(repo),
    }
}

/// The entry for the package in `bytes`, which `file` (at `url`) names as
/// `dependency` at `version`. The package inside must be that one.
///
/// Its MD5 is set from the bytes, so that fetching the entry again is a cache
/// hit, and its version is the file name's spelling, which the cache keys on.
fn entry_of(
    dependency: &Dependency,
    bytes: &[u8],
    url: &str,
    file: &str,
    version: &Version,
) -> Result<Entry, LockError> {
    let mut entry = index::entry_from_bytes(Path::new(url), bytes).with_context(|| {
        format!(
            "{} (github.com/{}): {file} is not a package lvpm can read",
            dependency.name, dependency.repo
        )
    })?;
    if !same_name(&entry.name, dependency.name) || entry.version != *version {
        return Err(dependency.fail(GitHubFault::ContentsDiffer {
            file: file.into(),
            expected: format!("{} {version}", dependency.name).into(),
            found: format!("{} {}", entry.name, entry.version).into(),
        }));
    }
    entry.version = version.clone();
    entry.md5 = Some(index::md5_hex(bytes));
    entry.source = index::github_source(dependency.repo);
    Ok(entry)
}

/// A cancel is told apart from a failure.
fn lock_error(why: anyhow::Error) -> LockError {
    match why.is::<Cancelled>() {
        true => Cancelled.into(),
        false => why.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn file(version: &str) -> ReleaseFile {
        ReleaseFile {
            version: Version::parse(version),
            url: format!("http://api/assets/{version}"),
            tag: format!("v{version}"),
            file: format!("pkg-{version}.vip"),
        }
    }

    fn entry(file: &ReleaseFile, lv_min: Option<f64>) -> Entry {
        Entry {
            name: "pkg".into(),
            version: file.version.clone(),
            url: file.url.clone(),
            md5: None,
            display_name: None,
            requires: Vec::new(),
            lv_min,
            released: None,
            source: index::github_source("o/r"),
        }
    }

    /// What `walk` made of `versions` (newest first) when each needs the
    /// LabVIEW in `needs`, for a project on LabVIEW 24: the versions it made
    /// entries for.
    fn walked(versions: &[&str], needs: &[f64]) -> Vec<String> {
        walked_with_pin(versions, None, needs)
    }

    /// `walked`, with the lock pinning `pin`; `needs` is in walk order.
    fn walked_with_pin(versions: &[&str], pin: Option<&str>, needs: &[f64]) -> Vec<String> {
        let files: Vec<ReleaseFile> = versions.iter().map(|version| file(version)).collect();
        let pin = pin.map(Version::parse);
        let refs = pin_first(files.iter().collect(), pin.as_ref());
        let asked = RefCell::new(0);
        let entries = walk(&refs, 24.0, |file| {
            let at = *asked.borrow();
            *asked.borrow_mut() += 1;
            Ok(entry(file, Some(needs[at])))
        })
        .unwrap();
        assert_eq!(entries.len(), *asked.borrow(), "one download per entry");
        entries
            .iter()
            .map(|entry| entry.version.raw.clone())
            .collect()
    }

    fn lock_from(source: &str) -> Lockfile {
        Lockfile {
            version: crate::lockfile::FORMAT_VERSION,
            labview: "2024".into(),
            inputs: crate::lockfile::Inputs {
                defaults: false,
                sources: Vec::new(),
                dependencies: Default::default(),
                dev_dependencies: Default::default(),
                github: Default::default(),
                min_age_days: 0,
            },
            packages: vec![crate::lockfile::LockedPackage {
                name: "pkg".into(),
                version: "1.0".into(),
                sha256: "AB".into(),
                md5: None,
                source: source.into(),
                requires: Vec::new(),
                dev: false,
            }],
            unresolved: Vec::new(),
        }
    }

    #[test]
    fn a_pin_is_kept_only_when_the_lock_took_it_from_the_same_repo() {
        let constraint = Constraint::Any;
        let dependency = Dependency {
            name: "pkg",
            repo: "o/r",
            constraints: vec![&constraint],
        };
        let pinned =
            |source: &str| locked_version(Some(&lock_from(source)), &Moving::Nothing, &dependency);

        assert_eq!(
            pinned("https://github.com/o/r"),
            Some((Version::parse("1.0"), "ab".to_string()))
        );
        assert!(pinned("https://github.com/O/R").is_some(), "in any case");
        for other in [
            "https://github.com/fork/r",
            "local",
            "http://feed/index.vipr",
        ] {
            assert_eq!(pinned(other), None, "{other}");
        }
    }

    #[test]
    fn only_a_file_that_github_does_not_have_is_not_found() {
        let github = |fault| {
            LockError::Other(
                GitHubError {
                    name: "pkg".into(),
                    repo: "o/r".into(),
                    fault,
                }
                .into(),
            )
        };
        let not_found = github(GitHubFault::NotFound { token_source: None });
        let other = github(GitHubFault::Unreadable { cause: "x".into() });

        assert!(is_not_found::<()>(&Err(not_found)));
        assert!(!is_not_found::<()>(&Err(other)));
        assert!(!is_not_found::<()>(&Err(LockError::NoLabview)));
        assert!(!is_not_found(&Ok(())));
    }

    #[test]
    fn the_walk_stops_at_the_first_file_that_runs_on_the_projects_labview() {
        assert_eq!(walked(&["3", "2", "1"], &[20.0, 20.0, 20.0]), ["3"]);
        // A file that needs a newer LabVIEW is skipped, and stays in the index
        // so that the solver can say what it needs.
        assert_eq!(walked(&["3", "2", "1"], &[25.0, 24.0, 20.0]), ["3", "2"]);
        assert_eq!(
            walked(&["3", "2", "1"], &[25.0, 26.0, 20.0]),
            ["3", "2", "1"]
        );
        // None fits: every one read is kept.
        assert_eq!(walked(&["2", "1"], &[25.0, 25.0]), ["2", "1"]);
    }

    #[test]
    fn a_pin_that_runs_on_the_projects_labview_is_the_only_file_read() {
        assert_eq!(
            walked_with_pin(&["2.0", "1.0"], Some("1.0"), &[24.0]),
            ["1.0"]
        );
    }

    #[test]
    fn a_pin_that_no_longer_runs_on_the_projects_labview_gives_way_to_the_newest_that_does() {
        assert_eq!(
            walked_with_pin(&["3.0", "2.0", "1.0"], Some("1.0"), &[25.0, 26.0, 24.0]),
            ["1.0", "3.0", "2.0"]
        );
    }

    #[test]
    fn the_walk_reads_no_more_files_than_its_bound() {
        let versions: Vec<String> = (1..=MAX_WALK + 3).rev().map(|n| n.to_string()).collect();
        let refs: Vec<&str> = versions.iter().map(String::as_str).collect();
        let walked = walked(&refs, &vec![30.0; versions.len()]);
        assert_eq!(walked.len(), MAX_WALK);
        assert_eq!(walked[0], versions[0], "the newest come first");
    }

    #[test]
    fn a_file_that_cannot_be_read_ends_the_walk() {
        let files = [file("2"), file("1")];
        let refs: Vec<&ReleaseFile> = files.iter().collect();
        let mut asked = 0;
        let outcome = walk(&refs, 24.0, |_| {
            asked += 1;
            Err(LockError::NoLabview)
        });
        assert!(matches!(outcome, Err(LockError::NoLabview)));
        assert_eq!(asked, 1);
    }
}
