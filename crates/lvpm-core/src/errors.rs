//! Errors that carry what went wrong as data.
//!
//! The message of each is neutral: it says what failed and names no lvpm
//! command; it names a flag only when that flag's value is what is wrong. A
//! front end words the hint that follows, because only it knows what the user
//! can type. The GitHub faults are the exception: their texts carry the hint,
//! because it names `gh` and `GH_TOKEN`, not an lvpm command, so every front
//! end would word it the same.

use crate::events::SyncStep;
use crate::github::TokenSource;
use crate::lockfile::StaleReason;
use crate::manifest_edit::DependencyTable;
use crate::solve::Conflict;
use crate::timefmt::{rfc3339, time_until};
use std::path::PathBuf;

/// The operation's [`CancelToken`](crate::cancel::CancelToken) was cancelled
/// before the operation wrote a package file. Operations that fail with an
/// [`anyhow::Error`] carry it as the cause, for the caller to downcast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cancelled")]
pub struct Cancelled;

impl From<Cancelled> for LockError {
    fn from(_: Cancelled) -> LockError {
        LockError::Cancelled
    }
}

impl From<Cancelled> for SyncError {
    fn from(_: Cancelled) -> SyncError {
        SyncError::Cancelled
    }
}

impl From<Cancelled> for EditError {
    fn from(_: Cancelled) -> EditError {
        EditError::Cancelled
    }
}

// A cancel is one variant at every level, however many errors it passed
// through, so a front end matches on one shape. These three are written out
// because `#[from]` would wrap `LockError::Cancelled` instead.
impl From<LockError> for SyncError {
    fn from(cause: LockError) -> SyncError {
        match cause {
            LockError::Cancelled => SyncError::Cancelled,
            other => SyncError::Lock(other),
        }
    }
}

impl From<LockError> for EditError {
    fn from(cause: LockError) -> EditError {
        match cause {
            LockError::Cancelled => EditError::Cancelled,
            other => EditError::Lock(other),
        }
    }
}

impl From<SyncError> for EditError {
    fn from(cause: SyncError) -> EditError {
        match cause {
            SyncError::Cancelled => EditError::Cancelled,
            other => EditError::Sync(other),
        }
    }
}

/// Why `lock` wrote no lock, or `check` did not find one that fits.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// There is no `lvpm.toml` at or above `searched_from`.
    #[error("no lvpm.toml found in {} or above", .searched_from.display())]
    NoManifest { searched_from: PathBuf },
    /// The manifest does not say which LabVIEW to lock for.
    #[error("lvpm.toml has no labview")]
    NoLabview,
    /// The manifest's `labview` is not a minimum year such as `">=2025"`.
    #[error("labview = {value:?} in lvpm.toml is not a minimum LabVIEW year")]
    LabviewNotAYear { value: String },
    /// No set of versions satisfies the manifest.
    #[error("{0}")]
    Conflict(Conflict),
    /// Archives that could be neither read from the cache nor fetched, all of
    /// them: one run reports every one it could not get.
    #[error("{} archive(s) could not be fetched", .0.len())]
    ArchivesUnavailable(Vec<ArchiveFailure>),
    /// Versions the lock already holds whose archive no source has with the
    /// locked SHA-256, all of them. Locking again keeps a version's SHA-256:
    /// other bytes under it are never pinned in its place.
    #[error("{} locked archive(s) could not be had", .0.len())]
    LockedArchives(Vec<LockedArchiveProblem>),
    /// There is no lock where it should be.
    #[error("there is no lvpm.lock at {}", .path.display())]
    NoLock { path: PathBuf },
    /// The lock is there but is not a lock: `reason` is the cause, in its own
    /// words.
    #[error("{} cannot be read: {reason}", .path.display())]
    UnreadableLock { path: PathBuf, reason: String },
    /// The lock no longer represents the manifest.
    #[error("lvpm.lock is out of date")]
    Stale(StaleReason),
    /// An upgrade names a package that the lock does not hold.
    #[error("{name} is not in lvpm.lock")]
    NotLocked { name: String },
    /// An upgrade names a version that no source lists.
    #[error("no {name} {version} is in any source")]
    NoSuchVersion { name: String, version: String },
    /// An upgrade names a version that needs a newer LabVIEW than the
    /// project's.
    #[error(
        "{name} {version} requires LabVIEW {needs_year}; this project is locked for {project_year}"
    )]
    NeedsNewerLabview {
        name: String,
        version: String,
        needs_year: u32,
        project_year: u32,
    },
    /// The operation's token was cancelled; nothing was written.
    #[error("cancelled")]
    Cancelled,
    /// Anything else that went wrong while reading or writing: its own text.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// One archive that could not be had, and why, in the cause's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveFailure {
    pub name: String,
    pub version: String,
    pub reason: String,
}

/// Why `sync` changed nothing, or stopped part-way.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// The manifest or the lock cannot be used as they are, or a new lock
    /// could not be made: [`LockError`]'s own words.
    #[error(transparent)]
    Lock(LockError),
    /// Locked archives that no source holds as the lock has them, all of them:
    /// nothing was changed.
    #[error("{} locked archive(s) could not be had", .0.len())]
    Archives(Vec<LockedArchiveProblem>),
    /// A change failed after others were made. `done` and `left` say which
    /// packages are on each side of it.
    #[error("sync stopped before it was done: {cause}")]
    Incomplete {
        done: Vec<SyncStep>,
        left: Vec<SyncStep>,
        cause: String,
    },
    /// The operation's token was cancelled; nothing was changed.
    #[error("cancelled")]
    Cancelled,
    /// Anything else that went wrong: its own text.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Why `add` or `remove` changed nothing in `lvpm.toml` and `lvpm.lock`, or
/// put both back as they were.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    /// The edit could not be locked: [`LockError`]'s own words.
    #[error(transparent)]
    Lock(LockError),
    /// The destination could not be synced to the new lock: [`SyncError`]'s own
    /// words. Both files are as they were.
    #[error(transparent)]
    Sync(SyncError),
    /// The operation's token was cancelled; both files are as they were.
    #[error("cancelled")]
    Cancelled,
    /// A package was written `name@` with nothing after the `@`.
    #[error("a constraint is expected after \"@\" in \"{name}@\"")]
    EmptyConstraint { name: String },
    /// `remove` names a package that is in neither dependency table.
    #[error("{name} is not in lvpm.toml")]
    NotADependency { name: String },
    /// `remove` names a package that is in the other dependency table.
    #[error("{name} is in [{}]", .found_in.key())]
    InOtherTable {
        name: String,
        found_in: DependencyTable,
    },
    /// Anything else that went wrong: its own text.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Why `init` wrote nothing.
#[derive(Debug, thiserror::Error)]
pub enum InitError {
    /// The folder `--project` names is not there.
    #[error("{} does not exist", .dir.display())]
    ProjectMissing { dir: PathBuf },
    /// The folder already has an `lvpm.toml`, which init never replaces.
    #[error("{} already exists", .path.display())]
    ManifestExists { path: PathBuf },
    /// `--labview-version` is not a LabVIEW year.
    #[error("--labview-version {value} is not a LabVIEW year (expected e.g. 2025)")]
    BadLabview { value: String },
    /// `--from` names a `.dragon` file; the file is not opened. `file` is as typed.
    #[error(
        "{file}: .dragon files are not supported and not planned (VIPM discontinued the format); \
         export a .vipc from VIPM instead"
    )]
    DragonFile { file: String },
    /// The `--from` file cannot be read or is not a usable `.vipc` or `vipm.toml`;
    /// `why` leads with the file as typed.
    #[error("{why}")]
    BadSource { why: String },
    /// Anything else that went wrong: its own text.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Why an install request could not be routed to a mode.
#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    /// There is no project to sync, and no package to install: no `lvpm.toml`
    /// at or above `searched_from`.
    #[error("give a package name")]
    NoPackage { searched_from: PathBuf },
    /// In a project a bare install is a sync and a named one an add, and a lock
    /// always holds the full set of packages, so these ask for something it
    /// cannot do. `adding`: it was a package that was named.
    #[error("not supported with a lock (lvpm.lock always holds the full set)")]
    NotWithLock {
        option: LockIncompatible,
        adding: bool,
    },
    /// Anything else that went wrong: its own text.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// What a request asks for that a sync from a lock cannot give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockIncompatible {
    /// Leaving out the dependencies of the packages.
    NoDeps,
    /// Reading the dependencies from another manifest.
    Manifest,
}

/// Why a GitHub repo's releases could not be had, in the words a user reads:
/// every text starts with the package and its repo.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("{name} (github.com/{repo}): {fault}")]
pub struct GitHubError {
    pub name: String,
    /// `owner/repo`.
    pub repo: String,
    pub fault: GitHubFault,
}

/// What went wrong with a GitHub repo, without the package it was asked for.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum GitHubFault {
    /// Two releases hold the same version of the package.
    #[error(
        "version {version} is in two releases, {first_tag} and {second_tag}; delete one of the files"
    )]
    TwoReleases {
        version: String,
        first_tag: String,
        second_tag: String,
    },
    /// The rate limit is used up. `token_source` is where the token that was
    /// sent came from, if one was; `reset` is when the limit resets, in Unix
    /// seconds, and `now` is when the answer came, so the text can say how long.
    #[error("{}", rate_limited(*.token_source, *.reset, *.now))]
    RateLimited {
        token_source: Option<TokenSource>,
        reset: Option<i64>,
        now: i64,
    },
    /// The repo is not there, or not visible to the caller.
    #[error("{}", not_found(*.token_source))]
    NotFound { token_source: Option<TokenSource> },
    /// GitHub did not accept the token that was sent.
    #[error("GitHub rejected the token from {token_source}; unset it or run gh auth login")]
    TokenRejected { token_source: TokenSource },
    /// A 401 or 403 that is not a rate limit; `message` is GitHub's own.
    #[error("GitHub refused access ({message})")]
    Refused { message: String },
    /// Releases hold the package, none at a version the constraint admits.
    /// `newest` is the newest version a release holds; `skipped` counts the
    /// drafts and pre-releases left out.
    #[error("no release has a version {constraint}; the newest is {newest}{}", skipped_note(*.skipped))]
    NoFittingVersion {
        constraint: String,
        newest: String,
        skipped: usize,
    },
    /// No release holds a file named for the package, `name`-<version>.lvpkg
    /// or .vip; `skipped` counts the drafts and pre-releases left out.
    #[error("no release holds {name}-<version>.lvpkg or .vip{}", skipped_note(*.skipped))]
    NoReleaseFile { name: String, skipped: usize },
    /// The file holds another package than its name says: `expected` is
    /// `<name> <version>` as the file name gives them, `found` the same as the
    /// package inside gives them.
    #[error(
        "{file} holds {found}, not {expected}; the file name must be <name>-<version> of the package inside"
    )]
    ContentsDiffer {
        // Boxed, so that a fault stays small enough for a `Result` (clippy's
        // `result_large_err`).
        file: Box<str>,
        expected: Box<str>,
        found: Box<str>,
    },
    /// The releases could not be read: the network, any other failing status,
    /// or a body that is not what the API sends. `cause` is the failure's own
    /// words.
    #[error("cannot read the releases; check your network and try again: {cause}")]
    Unreadable { cause: String },
}

/// What to do when GitHub does not know who is asking.
const SIGN_IN: &str = "run gh auth login or set GH_TOKEN";

fn rate_limited(token_source: Option<TokenSource>, reset: Option<i64>, now: i64) -> String {
    let until = reset.map_or(String::new(), |reset| {
        format!(
            " until {} ({})",
            rfc3339(reset),
            time_until(reset.saturating_sub(now))
        )
    });
    match token_source {
        Some(source) => {
            format!("GitHub's rate limit for the token from {source} is used up{until}")
        }
        None => format!("GitHub's rate limit is used up{until}; {SIGN_IN}"),
    }
}

/// What follows a "no release" text when drafts and pre-releases were left out.
fn skipped_note(skipped: usize) -> String {
    match skipped {
        0 => String::new(),
        n => format!(" ({n} pre-releases and drafts skipped)"),
    }
}

fn not_found(token_source: Option<TokenSource>) -> String {
    match token_source {
        Some(source) => format!("not found, or the token from {source} cannot see it"),
        None => format!("not found. If it is private, {SIGN_IN}"),
    }
}

/// A locked archive that sync could not get as the lock has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedArchiveProblem {
    pub name: String,
    pub version: String,
    /// The SHA-256 the lock records for it.
    pub sha256: String,
    pub fault: ArchiveFault,
}

/// What is wrong with a locked archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveFault {
    /// Neither the cache nor any source has an archive with the locked
    /// SHA-256.
    NotFound,
    /// A source lists it and could not be read or fetched from, in the cause's
    /// own words: sync tells the user this instead of "not found".
    Unfetched { reason: String },
    /// An archive of that name and version was found, and its bytes are not
    /// the locked ones: it is never installed.
    DiffersFromLock { found_sha256: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_is_one_variant_at_every_level_and_says_only_cancelled() {
        assert!(matches!(
            SyncError::from(LockError::Cancelled),
            SyncError::Cancelled
        ));
        assert!(matches!(SyncError::from(Cancelled), SyncError::Cancelled));
        assert!(matches!(
            EditError::from(LockError::Cancelled),
            EditError::Cancelled
        ));
        assert!(matches!(
            EditError::from(SyncError::Cancelled),
            EditError::Cancelled
        ));
        assert!(matches!(
            EditError::from(SyncError::from(LockError::Cancelled)),
            EditError::Cancelled
        ));
        assert!(matches!(EditError::from(Cancelled), EditError::Cancelled));
        for text in [
            LockError::Cancelled.to_string(),
            SyncError::Cancelled.to_string(),
            EditError::Cancelled.to_string(),
        ] {
            assert_eq!(text, "cancelled");
        }
    }

    fn github(fault: GitHubFault) -> String {
        GitHubError {
            name: "pkg".into(),
            repo: "o/r".into(),
            fault,
        }
        .to_string()
    }

    #[test]
    fn every_github_text_starts_with_the_package_and_its_repo_and_says_what_to_do() {
        let (reset, now) = (Some(1_793_455_500), 1_793_455_500 - 720);
        let table = [
            (
                GitHubFault::RateLimited {
                    token_source: None,
                    reset,
                    now,
                },
                "GitHub's rate limit is used up until 2026-10-31T14:05:00Z (in 12 min); run gh auth login or set GH_TOKEN",
            ),
            (
                GitHubFault::RateLimited {
                    token_source: Some(TokenSource::GhToken),
                    reset,
                    now,
                },
                "GitHub's rate limit for the token from GH_TOKEN is used up until 2026-10-31T14:05:00Z (in 12 min)",
            ),
            (
                GitHubFault::RateLimited {
                    token_source: None,
                    reset: None,
                    now,
                },
                "GitHub's rate limit is used up; run gh auth login or set GH_TOKEN",
            ),
            (
                GitHubFault::RateLimited {
                    token_source: Some(TokenSource::GhAuthToken),
                    reset: None,
                    now,
                },
                "GitHub's rate limit for the token from gh auth token is used up",
            ),
            (
                GitHubFault::NotFound { token_source: None },
                "not found. If it is private, run gh auth login or set GH_TOKEN",
            ),
            (
                GitHubFault::NotFound {
                    token_source: Some(TokenSource::GitHubToken),
                },
                "not found, or the token from GITHUB_TOKEN cannot see it",
            ),
            (
                GitHubFault::TokenRejected {
                    token_source: TokenSource::GhToken,
                },
                "GitHub rejected the token from GH_TOKEN; unset it or run gh auth login",
            ),
            (
                GitHubFault::Refused {
                    message: "Resource not accessible".into(),
                },
                "GitHub refused access (Resource not accessible)",
            ),
            (
                GitHubFault::NoFittingVersion {
                    constraint: ">=1.2.2".into(),
                    newest: "1.2.1.16".into(),
                    skipped: 0,
                },
                "no release has a version >=1.2.2; the newest is 1.2.1.16",
            ),
            (
                GitHubFault::NoFittingVersion {
                    constraint: "*".into(),
                    newest: "1.0".into(),
                    skipped: 2,
                },
                "no release has a version *; the newest is 1.0 (2 pre-releases and drafts skipped)",
            ),
            (
                GitHubFault::NoReleaseFile {
                    name: "pkg".into(),
                    skipped: 0,
                },
                "no release holds pkg-<version>.lvpkg or .vip",
            ),
            (
                GitHubFault::NoReleaseFile {
                    name: "pkg".into(),
                    skipped: 1,
                },
                "no release holds pkg-<version>.lvpkg or .vip (1 pre-releases and drafts skipped)",
            ),
            (
                GitHubFault::ContentsDiffer {
                    file: "pkg-1.0.lvpkg".into(),
                    expected: "pkg 1.0".into(),
                    found: "other 1.1".into(),
                },
                "pkg-1.0.lvpkg holds other 1.1, not pkg 1.0; the file name must be <name>-<version> of the package inside",
            ),
            (
                GitHubFault::Unreadable {
                    cause: "HTTP 502".into(),
                },
                "cannot read the releases; check your network and try again: HTTP 502",
            ),
        ];
        for (fault, text) in table {
            assert_eq!(github(fault), format!("pkg (github.com/o/r): {text}"));
        }
    }

    #[test]
    fn any_other_lock_error_keeps_its_wrapper() {
        assert!(matches!(
            EditError::from(LockError::NoLabview),
            EditError::Lock(LockError::NoLabview)
        ));
        assert!(matches!(
            SyncError::from(LockError::NoLabview),
            SyncError::Lock(LockError::NoLabview)
        ));
    }
}
