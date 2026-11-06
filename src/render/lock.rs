//! The wording of `lvpm lock`: its events, and the errors it ends in. Core's
//! errors say what went wrong; the hint that ends each message, the name of
//! the manifest and the commands to type are the CLI's.

use super::{Renderer, Stream};
use lvpm_core::errors::{
    ArchiveFailure, ArchiveFault, AttestationRefusal, ListingFailure, ListingFault, LockError,
    LockedArchiveProblem, MovedTag, Origin, SignatureLookupFailure,
};
use lvpm_core::events::{Event, LockChange};
use lvpm_core::lockfile::{FORMAT_VERSION, StaleReason};
use lvpm_core::manifest_edit::DependencyTable;
use lvpm_core::project::same_name;
use lvpm_core::released;
use lvpm_core::solve::{AllHeldBack, Conflict, Fact};
use lvpm_core::verify::{Invalid, NotChecked, Refusal};

pub(super) fn render(r: &mut Renderer, event: &Event) {
    match event {
        Event::LockHashing {
            name,
            version,
            position,
            total,
        } => r.line(
            Stream::Err,
            &format!("hashing {name} {version} ({position}/{total})"),
        ),
        Event::GitHubListed { name, repo, cached } => r.line(
            Stream::Err,
            &format!(
                "{name}: listing releases of github.com/{repo}{}",
                if *cached { " (cached)" } else { "" }
            ),
        ),
        Event::RequirementUnresolved { name, required_by } => r.line(
            Stream::Err,
            &format!(
                "warning: {name}, required by {required_by}, is in no source: it is recorded \
                 as unresolved and not locked"
            ),
        ),
        Event::HeldBack {
            name,
            version,
            chosen,
            released,
            eligible,
            min_age_days,
        } => r.line(
            Stream::Err,
            &format!(
                "held back {name} {version} by min-age-days = {min_age_days} (released {}, \
                 eligible {}); using {chosen}",
                released::day(*released),
                released::first_whole_day_from(*eligible)
            ),
        ),
        Event::LockChanged { changes } => {
            for change in changes {
                r.line(Stream::Out, &change_line(change));
            }
        }
        Event::LockWritten { path, packages } => r.line(
            Stream::Out,
            &format!(
                "wrote {} ({packages} package{}); commit it with lvpm.toml",
                path.display(),
                if *packages == 1 { "" } else { "s" }
            ),
        ),
        Event::AttestationChecking { name, version } => r.line(
            Stream::Err,
            &format!("checking attestation of {name} {version}"),
        ),
        Event::AttestationsNotChecked { packages, reason } => {
            let named: Vec<String> = (packages.iter())
                .map(|package| format!("{} {}", package.name, package.version))
                .collect();
            let (said, hint) = not_checked(reason, None);
            r.line(
                Stream::Err,
                &format!(
                    "warning: did not check the attestation of {}: {said}; {hint}",
                    named.join(", ")
                ),
            )
        }
        Event::ManifestUpdated { name, from, to } => r.line(
            Stream::Out,
            &format!("lvpm.toml was updated: {name} \"{from}\" -> \"{to}\""),
        ),
        Event::LockUpToDate { path } => {
            r.line(Stream::Out, &format!("{} is up to date", path.display()))
        }
        Event::LockNothingNewer { path } => r.line(
            Stream::Out,
            &format!("no changes, nothing newer; {} is as it was", path.display()),
        ),
        Event::LockWouldChange { changes } => {
            r.line(Stream::Out, "lvpm.lock would be updated:");
            for change in changes {
                r.line(Stream::Out, &format!("  {}", change_line(change)));
            }
        }
        _ => unreachable!("routed here by Renderer::render: {event:?}"),
    }
}

/// Why a lock is out of date, as the clause that follows "lvpm.lock is out of
/// date:". It names the manifest line at fault.
pub fn stale_reason(reason: &StaleReason) -> String {
    match reason {
        StaleReason::FormatOutdated => {
            format!("the lock format is outdated (this lvpm writes format {FORMAT_VERSION})")
        }
        StaleReason::LabviewChanged => "labview changed".to_string(),
        StaleReason::SourcesChanged => "sources changed".to_string(),
        StaleReason::MinAgeChanged { from, to } => {
            format!("min-age-days changed ({from} -> {to})")
        }
        StaleReason::PolicyChanged => "[policy] changed since lvpm.lock was written".to_string(),
        StaleReason::DependencyAdded { name, table } => {
            format!("{} \"{name}\" was added", dependency(table))
        }
        StaleReason::DependencyRemoved { name, table } => {
            format!("{} \"{name}\" was removed", dependency(table))
        }
        StaleReason::ConstraintExcludesLocked {
            name,
            constraint,
            locked,
            table,
        } => {
            let prefix = match table {
                DependencyTable::Dependencies => String::new(),
                DependencyTable::DevDependencies => "dev dependency ".to_string(),
                DependencyTable::Group(_) => format!("[{}] ", table.key()),
            };
            format!("{prefix}\"{name}\" \"{constraint}\" excludes locked {locked}")
        }
    }
}

/// What a reason calls a dependency of `table`.
fn dependency(table: &DependencyTable) -> String {
    match table {
        DependencyTable::Dependencies => "dependency".to_string(),
        DependencyTable::DevDependencies => "dev dependency".to_string(),
        DependencyTable::Group(_) => format!("[{}] dependency", table.key()),
    }
}

fn change_line(change: &LockChange) -> String {
    match change {
        LockChange::Added { name, version } => format!("+ {name} {version}"),
        LockChange::Removed { name, version } => format!("- {name} {version}"),
        LockChange::Changed { name, from, to } => format!("~ {name} {from} -> {to}"),
    }
}

/// What `lvpm lock` prints when it fails: what happened, then the next thing
/// to do.
pub fn error(error: &LockError) -> String {
    error_for(error, "`lvpm lock`")
}

/// [`error`] for a command that locks as part of what it does: `rerun` is what
/// to run again once the cause is dealt with, as "run {rerun} again" reads.
pub(super) fn error_for(error: &LockError, rerun: &str) -> String {
    match error {
        LockError::NoManifest { .. } => {
            format!("{error}\nnext: run `lvpm lock` in a project, a directory with an lvpm.toml")
        }
        LockError::NoLabview => format!(
            "{error}, so lvpm does not know which LabVIEW to lock for\n\
             next: add labview = \">=<year>\" under [project] in lvpm.toml (for example \
             labview = \">=2024\"), then run {rerun} again"
        ),
        LockError::LabviewNotAYear { .. } => format!(
            "{error}\n\
             next: write labview = \">=<year>\" under [project] in lvpm.toml (for example \
             labview = \">=2024\"), then run {rerun} again"
        ),
        LockError::Conflict(conflict) => conflict_text(conflict, rerun),
        LockError::ArchivesUnavailable(failures) => archives_text(failures, rerun),
        LockError::LockedArchives(problems) => locked_archives_text(problems, rerun),
        LockError::TagMoved(moved) => moved_tags_text(moved, rerun),
        LockError::Refused {
            refusals,
            trusted_publishers,
        } => refused_text(refusals, trusted_publishers),
        LockError::SignatureLookup(failures) => signature_lookups_text(failures),
        LockError::OciListing(failures) => listing_failures_text(failures),
        // `lvpm lock --check` fails with one line: a script reads it.
        LockError::NoLock { .. } => {
            "there is no lvpm.lock; run `lvpm lock` to create it".to_string()
        }
        LockError::UnreadableLock { reason, .. } => format!(
            "lvpm.lock cannot be read ({}); run `lvpm lock` to regenerate it",
            reason.lines().next().unwrap_or_default()
        ),
        LockError::Stale(reason) => format!(
            "lvpm.lock is out of date: {}; run `lvpm lock` to update it",
            stale_reason(reason)
        ),
        LockError::NotLocked { .. } => format!(
            "{error}\n\
             next: name a package that lvpm.lock lists, then run \
             `lvpm lock --upgrade-package <package>` again"
        ),
        LockError::NoSuchVersion { name, .. } => format!(
            "{error}\n\
             next: look for the versions there are with `lvpm search {name}`, then run \
             `lvpm lock --upgrade-package {name}@<version>` again"
        ),
        LockError::NeedsNewerLabview {
            name, project_year, ..
        } => format!(
            "{error}\n\
             next: pick a version that runs on LabVIEW {project_year}, or raise labview in \
             lvpm.toml, then run `lvpm lock --upgrade-package {name}@<version>` again"
        ),
        LockError::PinnedNotListed { package, .. } => format!(
            "{error}\n\
             next: list {package} in that source, or remove the index key from {package} in \
             lvpm.toml, then run {rerun} again"
        ),
        LockError::Cancelled => {
            format!("{error}; lvpm.lock is unchanged\nnext: run {rerun} again")
        }
        LockError::Other(cause) => format!("{cause:#}"),
    }
}

/// A conflict: a block for each package whose versions are all held back by
/// min-age-days, then, when something else is wrong too, the general report.
/// A `SourcePinned` fact is a note on a package, not a failure of its own: it
/// goes in the held-back block of its package, and is a line of the general
/// report only when the package has no such block.
fn conflict_text(conflict: &Conflict, rerun: &str) -> String {
    let pin_note = |package: &str| {
        (conflict.facts.iter()).find(|fact| {
            matches!(fact, Fact::SourcePinned { package: pinned, .. } if same_name(pinned, package))
        })
    };
    let mut blocks = Vec::new();
    let mut rest = Vec::new();
    for fact in &conflict.facts {
        match fact {
            Fact::AllHeldBack(held) => {
                blocks.push(all_held_back_text(held, pin_note(&held.package)))
            }
            Fact::SourcePinned { package, .. }
                if (conflict.facts.iter()).any(
                    |other| matches!(other, Fact::AllHeldBack(held) if same_name(&held.package, package)),
                ) => {}
            other => rest.push(other),
        }
    }
    if blocks.is_empty() || !rest.is_empty() {
        blocks.push(general_conflict_text(&rest, rerun));
    }
    blocks.join("\n")
}

/// Every version of `package` that `wanted` admits is held back: each with
/// its release and the day it is eligible, then `pinned` (the package's
/// `SourcePinned` fact, if it has one), and the way on.
fn all_held_back_text(held: &AllHeldBack, pinned: Option<&Fact>) -> String {
    let AllHeldBack {
        package,
        versions,
        wanted,
        required_by,
        days,
    } = held;
    let mut text = format!(
        "{package} {}: every version is held back by min-age-days = {days}",
        manifest_constraint(wanted)
    );
    let eligible: Vec<i64> = versions
        .iter()
        .map(|(_, released)| released::eligible_at(*released, *days))
        .collect();
    for ((version, released), eligible) in versions.iter().zip(&eligible) {
        text.push_str(&format!(
            "\n  {version} released {}, eligible {}",
            released::day(*released),
            released::first_whole_day_from(*eligible)
        ));
    }
    if let Some(note) = pinned {
        text.push_str(&format!("\n  {note}"));
    }
    let earliest = eligible
        .iter()
        .min()
        .map(|time| format!("wait until {}, ", released::first_whole_day_from(*time)))
        .unwrap_or_default();
    text.push_str(&format!(
        "\nnext: {earliest}lower min-age-days, or pin an exact version of {package} in lvpm.toml"
    ));
    if let Some(requirer) = required_by {
        text.push_str(&format!(" (it is required by {requirer})"));
    }
    text
}

fn general_conflict_text(facts: &[&Fact], rerun: &str) -> String {
    let unlisted: Vec<&str> = facts
        .iter()
        .filter_map(|fact| match fact {
            Fact::NotListed { package } => Some(package.as_str()),
            _ => None,
        })
        .collect();
    let only_unlisted = unlisted.len()
        == (facts.iter())
            .filter(|fact| !matches!(fact, Fact::OciExplicit { .. }))
            .count();
    let mut text = match only_unlisted {
        true => "cannot lock:".to_string(),
        false => "cannot lock: no set of versions satisfies lvpm.toml:".to_string(),
    };
    for fact in facts {
        text.push_str("\n  ");
        text.push_str(&fact_line(fact));
    }
    text.push_str("\nnext: ");
    match unlisted.is_empty() {
        true => text.push_str(&format!(
            "change one of these in lvpm.toml, then run {rerun} again"
        )),
        false => {
            let searches: Vec<String> = unlisted
                .iter()
                .map(|package| format!("`lvpm search {package}`"))
                .collect();
            text.push_str(&format!(
                "look for it with {}, or list a repository under [sources] in lvpm.toml, \
                 then run {rerun} again",
                searches.join(", ")
            ));
        }
    }
    text
}

/// A fact as one line of a report. What the manifest asks for is shown the way
/// the manifest line is written.
fn fact_line(fact: &Fact) -> String {
    match fact {
        Fact::Manifest { package, wanted } => {
            format!("lvpm.toml: {package} = \"{}\"", manifest_constraint(wanted))
        }
        Fact::Pinned { package, version } => format!("--upgrade-package {package}@{version}"),
        other => other.to_string(),
    }
}

/// A version set as a manifest constraint reads: `= 1.0` is `1.0`, `>= 2.0`
/// is `>=2.0`, `*` stays. Anything else is shown as the solver describes it.
fn manifest_constraint(wanted: &str) -> String {
    if let Some(exact) = wanted.strip_prefix("= ") {
        return exact.to_string();
    }
    match wanted.strip_prefix(">= ") {
        Some(floor) if !floor.contains(',') => format!(">={floor}"),
        _ => wanted.to_string(),
    }
}

/// An archive that could not be fetched, and why: the cause after the package,
/// or alone when it is a whole sentence about it.
pub(super) fn unfetched_line(name: &str, version: &str, reason: &str, in_full: bool) -> String {
    match in_full {
        true => reason.to_string(),
        false => format!("{name} {version}: {reason}"),
    }
}

fn archives_text(failures: &[ArchiveFailure], rerun: &str) -> String {
    let plural = if failures.len() == 1 { "" } else { "s" };
    let mut text = format!(
        "cannot lock: {} archive{plural} could not be fetched:",
        failures.len()
    );
    for failure in failures {
        text.push_str("\n  ");
        text.push_str(&unfetched_line(
            &failure.name,
            &failure.version,
            &failure.reason,
            failure.in_full,
        ));
    }
    text.push_str(&format!(
        "\nnext: make each source reachable, or add the archive with `lvpm cache add <file>`, \
         then run {rerun} again"
    ));
    text
}

fn locked_archives_text(problems: &[LockedArchiveProblem], rerun: &str) -> String {
    let plural = if problems.len() == 1 { "" } else { "s" };
    let mut text = format!(
        "cannot lock: {} locked archive{plural} could not be had, and lvpm.lock was not changed:",
        problems.len()
    );
    for LockedArchiveProblem {
        name,
        version,
        sha256,
        fault,
    } in problems
    {
        text.push_str("\n  ");
        text.push_str(&match fault {
            ArchiveFault::NotFound => format!(
                "{name} {version}: no archive with SHA-256 {sha256} is in the cache or in any source"
            ),
            ArchiveFault::Unfetched { reason, in_full } => {
                unfetched_line(name, version, reason, *in_full)
            }
            ArchiveFault::DiffersFromLock { found_sha256 } => format!(
                "{name} {version}: the archive has SHA-256 {found_sha256} and lvpm.lock has \
                 {sha256}, so it was not locked again"
            ),
        });
    }
    text.push_str(&format!(
        "\nnext: add the archive lvpm.lock was made from with `lvpm cache add <file>`, or delete \
         lvpm.lock to lock the archives as they are now, then run {rerun} again"
    ));
    text
}

/// Every locked OCI version whose tag now names another manifest. Only an
/// upgrade accepts it, and the hint says which one.
fn moved_tags_text(moved: &[MovedTag], rerun: &str) -> String {
    let plural = if moved.len() == 1 { "" } else { "s" };
    let mut text = format!(
        "cannot lock: {} locked OCI version{plural} now {} another manifest, and nothing was written:",
        moved.len(),
        if moved.len() == 1 { "has" } else { "have" }
    );
    for MovedTag {
        name,
        version,
        locked,
        found,
    } in moved
    {
        text.push_str(&format!(
            "\n  {name} {version}: lvpm.lock has manifest {locked}, the registry now has {found}"
        ));
    }
    let accept: Vec<String> = (moved.iter())
        .map(|tag| format!("--upgrade-package {}@{}", tag.name, tag.version))
        .collect();
    text.push_str(&format!(
        "\nnext: if the tags moved on purpose, accept the new manifests with `lvpm lock {}` (or \
         `lvpm lock --upgrade` for every package); otherwise ask the publisher to put them back, \
         then run {rerun} again",
        accept.join(" ")
    ));
    text
}

/// Every package the policy refuses, each with what to do about it. No hint
/// says to turn the policy off.
fn refused_text(refusals: &[AttestationRefusal], trusted_publishers: &[String]) -> String {
    let plural = if refusals.len() == 1 { "" } else { "s" };
    let mut text = format!("lvpm refused {} package{plural}:", refusals.len());
    for refusal in refusals {
        let AttestationRefusal {
            name,
            version,
            origin,
            kind,
        } = refusal;
        let from = match origin {
            Origin::GitHub { repo } => format!("github.com/{repo}"),
            Origin::Oci { source } => source.clone(),
        };
        text.push_str("\n  ");
        text.push_str(&match kind {
            Refusal::NoAttestation => format!(
                "{name} {version} from {from} has no attestation\n    \
                 hint: ask the publisher to build with the lvpm release template"
            ),
            Refusal::UntrustedPublisher { subject, signer } => format!(
                "{name} {version} was attested by repository {subject} (workflow {signer})\n    \
                 which [policy] trusted-publishers does not list: {}\n    \
                 hint: add the repository to [policy] trusted-publishers, or depend on a release \
                 built by a trusted repository",
                trusted_publishers.join(", "),
                subject = super::printable(subject.clone()),
                signer = super::printable(signer.clone()),
            ),
            Refusal::DoesNotVerify { cause } => {
                let tampered = match origin {
                    Origin::GitHub { .. } => "archive",
                    Origin::Oci { .. } => "manifest",
                };
                format!(
                    "the attestation of {name} {version} from {from} does not verify:{}\n    \
                     hint: if gh's reason is a network error, try again; otherwise report this to \
                     the publisher, the {tampered} may have been tampered with. lvpm refuses it \
                     whatever [policy] says",
                    invalid(cause)
                )
            }
            Refusal::CannotCheck { reason } => {
                let repo = match origin {
                    Origin::GitHub { repo } => Some(repo.as_str()),
                    Origin::Oci { .. } => None,
                };
                let (said, hint) = not_checked(reason, repo);
                format!(
                    "cannot check the attestation of {name} {version}: {said}\n    hint: {hint}"
                )
            }
        });
    }
    text
}

/// Every package whose signature the registry could not be asked about.
fn signature_lookups_text(failures: &[SignatureLookupFailure]) -> String {
    let lines: Vec<String> = failures
        .iter()
        .map(|failure| {
            let SignatureLookupFailure {
                name,
                version,
                registry,
                cause,
            } = failure;
            format!(
                "{name} {version}: the signature lookup on registry {registry} failed: {cause}; \
                 the package was not locked"
            )
        })
        .collect();
    lines.join("\n")
}

/// Every pinned package an OCI source could not list: one line for each
/// registry whose login failed, ahead of one line for each other package.
fn listing_failures_text(failures: &[ListingFailure]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for failure in failures {
        if let ListingFault::Host(said) = &failure.fault
            && !lines.contains(said)
        {
            lines.push(said.clone());
        }
    }
    let per_package = (failures.iter()).filter(|f| !matches!(f.fault, ListingFault::Host(_)));
    lines.extend(per_package.map(|failure| match failure.fault {
        ListingFault::NotListed => format!(
            "{failure}; check the name, or publish it with lvpm publish --to {}",
            failure.feed
        ),
        _ => failure.to_string(),
    }));
    lines.join(
        "
",
    )
}

/// The end of a "does not verify:" line: lvpm's own reason after a space, or
/// the first 5 lines of gh's stderr, one to a line and indented four spaces.
fn invalid(cause: &Invalid) -> String {
    let stderr = match cause {
        Invalid::UnreadableOutput => return " unrecognised gh output".to_string(),
        Invalid::OtherArchive => return " gh's attestation does not match the archive".to_string(),
        Invalid::NoSourceRepository => {
            return " the bundle's certificate names no source repository, so gh was not run"
                .to_string();
        }
        Invalid::Rejected { stderr } => super::printable(strip_ansi(stderr)),
    };
    if stderr.trim().is_empty() {
        return " gh gave no reason".to_string();
    }
    (stderr.lines().take(5)).fold(String::new(), |text, line| format!("{text}\n    {line}"))
}

/// `text` without its ANSI escape sequences (`ESC [`, parameters, one final
/// byte), which gh colours its messages with.
fn strip_ansi(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for end in chars.by_ref() {
                if ('@'..='~').contains(&end) {
                    break;
                }
            }
        } else {
            plain.push(c);
        }
    }
    plain
}

/// What lvpm says of a reason it could not check an attestation, and what to
/// do about it. `repo` is the repository the package is from, when it is one
/// package that is asked about.
fn not_checked(reason: &NotChecked, repo: Option<&str>) -> (String, String) {
    const AUTH: &str = "run `gh auth login` or set GH_TOKEN";
    const LATER: &str = "try again later";
    let (said, hint) = match reason {
        NotChecked::GhMissing => (
            "gh is not installed",
            "install the GitHub CLI 2.49 or newer: https://cli.github.com",
        ),
        NotChecked::GhTooOld => (
            "gh is too old to verify attestations",
            "upgrade the GitHub CLI to 2.49 or newer",
        ),
        NotChecked::GhTimedOut => ("gh timed out", LATER),
        NotChecked::GhInterrupted => ("gh was interrupted", LATER),
        NotChecked::GhFailed { error } => {
            let said = super::printable(format!("gh could not be run: {error}"));
            return (said, LATER.to_string());
        }
        NotChecked::NoToken => ("lvpm has no GitHub token", AUTH),
        NotChecked::RateLimited { anonymous: true } => {
            ("GitHub rate limit reached (60/h without a token)", AUTH)
        }
        NotChecked::RateLimited { anonymous: false } => ("GitHub rate limit reached", LATER),
        NotChecked::TokenRejected => ("GitHub did not accept the token (HTTP 401)", AUTH),
        NotChecked::LookupRefused => {
            let repo = repo.unwrap_or("the repository");
            return (
                "GitHub refused the attestation lookup (HTTP 403)".to_string(),
                format!("check that your token can read {repo}"),
            );
        }
        NotChecked::Http { status } => {
            return (format!("GitHub returned HTTP {status}"), LATER.to_string());
        }
        NotChecked::Unreachable => (
            "could not reach GitHub",
            "check your network connection and try again",
        ),
        NotChecked::Unreadable => ("GitHub returned an unreadable answer", LATER),
    };
    (said.to_string(), hint.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::tests::renderer;
    use super::*;
    use lvpm_core::events::PackageRef;
    use lvpm_core::oci::FeedUrl;
    use lvpm_core::released::SECS_PER_DAY;
    use std::path::PathBuf;

    #[test]
    fn an_archive_that_is_not_the_locked_one_is_named_with_both_hashes_and_the_way_out() {
        let text = error(&LockError::LockedArchives(vec![LockedArchiveProblem {
            name: "alpha".into(),
            version: "2.0".into(),
            sha256: "aaa".into(),
            fault: ArchiveFault::DiffersFromLock {
                found_sha256: "bbb".into(),
            },
        }]));
        assert_eq!(
            text,
            "cannot lock: 1 locked archive could not be had, and lvpm.lock was not changed:\n\
             \x20 alpha 2.0: the archive has SHA-256 bbb and lvpm.lock has aaa, so it was not \
             locked again\n\
             next: add the archive lvpm.lock was made from with `lvpm cache add <file>`, or \
             delete lvpm.lock to lock the archives as they are now, then run `lvpm lock` again"
        );
    }

    #[test]
    fn a_moved_tag_names_both_digests_and_the_upgrade_that_accepts_it() {
        let moved = |name: &str| MovedTag {
            name: name.into(),
            version: "1.1.0".into(),
            locked: "sha256:aaa".into(),
            found: "sha256:bbb".into(),
        };
        assert_eq!(
            error(&LockError::TagMoved(vec![moved("alpha")])),
            "cannot lock: 1 locked OCI version now has another manifest, and nothing was \
             written:\n\
             \x20 alpha 1.1.0: lvpm.lock has manifest sha256:aaa, the registry now has sha256:bbb\n\
             next: if the tags moved on purpose, accept the new manifests with `lvpm lock \
             --upgrade-package alpha@1.1.0` (or `lvpm lock --upgrade` for every package); \
             otherwise ask the publisher to put them back, then run `lvpm lock` again"
        );
        let both = error(&LockError::TagMoved(vec![moved("alpha"), moved("beta")]));
        assert!(
            both.contains("2 locked OCI versions now have another manifest"),
            "{both}"
        );
        assert!(
            both.contains("--upgrade-package alpha@1.1.0 --upgrade-package beta@1.1.0"),
            "{both}"
        );
    }

    fn listing(package: &str, fault: ListingFault) -> ListingFailure {
        ListingFailure {
            package: package.to_string(),
            source: "team".to_string(),
            feed: FeedUrl::parse("oci://r:1/org").unwrap(),
            fault,
        }
    }

    #[test]
    fn pinned_failures_are_one_line_each_in_the_order_given() {
        let text = error(&LockError::OciListing(vec![
            listing("alpha", ListingFault::NotListed),
            listing("zeta", ListingFault::Unreachable("closed".into())),
        ]));
        assert_eq!(
            text,
            "alpha: source \"team\" (oci://r:1/org) does not list it; check the name, or \
             publish it with lvpm publish --to oci://r:1/org\n\
             zeta: source \"team\" (oci://r:1/org) is unreachable: closed"
        );
        let refused = error(&LockError::OciListing(vec![listing(
            "gamma",
            ListingFault::Refused,
        )]));
        assert_eq!(
            refused,
            "gamma: registry r:1 refused the lookup for source \"team\"; the package is missing \
             or your login cannot read it; run docker login r:1"
        );
    }

    #[test]
    fn a_registry_whose_login_failed_is_one_line_ahead_of_the_other_packages() {
        let said = "registry r:1 refused the login from auths; run docker login r:1";
        let host = ListingFault::Host(said.into());
        let text = error(&LockError::OciListing(vec![
            listing("alpha", host.clone()),
            listing("beta", ListingFault::NotListed),
            listing("zeta", host),
        ]));
        assert_eq!(
            text,
            "registry r:1 refused the login from auths; run docker login r:1\n\
             beta: source \"team\" (oci://r:1/org) does not list it; check the name, or \
             publish it with lvpm publish --to oci://r:1/org"
        );
    }

    #[test]
    fn a_listing_of_releases_is_a_line_on_stderr_that_says_when_it_came_from_the_cache() {
        let (mut renderer, out, err) = renderer();
        for cached in [false, true] {
            renderer.render(&Event::GitHubListed {
                name: "pkg".into(),
                repo: "o/r".into(),
                cached,
            });
        }
        assert_eq!(
            err.text(),
            "pkg: listing releases of github.com/o/r\n\
             pkg: listing releases of github.com/o/r (cached)\n"
        );
        assert_eq!(out.text(), "");
    }

    #[test]
    fn a_dry_run_lists_how_the_lock_would_change_on_stdout() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::LockWouldChange {
            changes: vec![LockChange::Added {
                name: "gamma".into(),
                version: "1.0".into(),
            }],
        });
        assert_eq!(out.text(), "lvpm.lock would be updated:\n  + gamma 1.0\n");
        assert_eq!(err.text(), "");
    }

    #[test]
    fn a_held_back_version_is_one_line_on_stderr_with_its_dates_and_the_one_used() {
        let (mut renderer, out, err) = renderer();
        let released = released::parse("Wed, 28 Oct 2026 11:46:49 GMT").expect("a date");
        renderer.render(&Event::HeldBack {
            name: "alpha".into(),
            version: "1.1".into(),
            chosen: "1.0".into(),
            released,
            eligible: released::eligible_at(released, 14),
            min_age_days: 14,
        });
        assert_eq!(
            err.text(),
            "held back alpha 1.1 by min-age-days = 14 (released 2026-10-28, eligible \
             2026-11-12); using 1.0\n"
        );
        assert_eq!(out.text(), "");
    }

    #[test]
    fn progress_and_warnings_go_to_stderr_and_the_result_to_stdout() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::LockHashing {
            name: "alpha".into(),
            version: "2.0".into(),
            position: 3,
            total: 48,
        });
        renderer.render(&Event::RequirementUnresolved {
            name: "ni_lib_missing".into(),
            required_by: "beta".into(),
        });
        renderer.render(&Event::LockChanged {
            changes: vec![
                LockChange::Added {
                    name: "alpha".into(),
                    version: "2.0".into(),
                },
                LockChange::Changed {
                    name: "beta".into(),
                    from: "1.0".into(),
                    to: "1.1".into(),
                },
                LockChange::Removed {
                    name: "gamma".into(),
                    version: "1.5".into(),
                },
            ],
        });
        renderer.render(&Event::LockWritten {
            path: PathBuf::from("lvpm.lock"),
            packages: 2,
        });
        assert_eq!(
            err.text(),
            "hashing alpha 2.0 (3/48)\n\
             warning: ni_lib_missing, required by beta, is in no source: it is recorded as \
             unresolved and not locked\n"
        );
        assert_eq!(
            out.text(),
            "+ alpha 2.0\n~ beta 1.0 -> 1.1\n- gamma 1.5\n\
             wrote lvpm.lock (2 packages); commit it with lvpm.toml\n"
        );
    }

    #[test]
    fn a_lock_left_alone_says_it_is_up_to_date() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::LockUpToDate {
            path: PathBuf::from("lvpm.lock"),
        });
        assert_eq!(out.text(), "lvpm.lock is up to date\n");
        assert_eq!(err.text(), "");
    }

    #[test]
    fn an_upgrade_with_nothing_newer_says_so_on_stdout() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::LockNothingNewer {
            path: PathBuf::from("lvpm.lock"),
        });
        assert_eq!(
            out.text(),
            "no changes, nothing newer; lvpm.lock is as it was\n"
        );
        assert_eq!(err.text(), "");
    }

    #[test]
    fn a_rewritten_constraint_is_a_line_on_stdout_with_both_constraints() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::ManifestUpdated {
            name: "beta".into(),
            from: ">=1.0".into(),
            to: ">=0.9".into(),
        });
        assert_eq!(
            out.text(),
            "lvpm.toml was updated: beta \">=1.0\" -> \">=0.9\"\n"
        );
        assert_eq!(err.text(), "");
    }

    #[test]
    fn a_refused_upgrade_says_why_and_ends_with_the_command_to_try_again() {
        for (error, why, again) in [
            (
                LockError::NotLocked {
                    name: "zeta".into(),
                },
                "zeta is not in lvpm.lock",
                "`lvpm lock --upgrade-package <package>` again",
            ),
            (
                LockError::NoSuchVersion {
                    name: "beta".into(),
                    version: "9.9".into(),
                },
                "no beta 9.9 is in any source",
                "`lvpm lock --upgrade-package beta@<version>` again",
            ),
            (
                LockError::NeedsNewerLabview {
                    name: "beta".into(),
                    version: "2.0".into(),
                    needs_year: 2026,
                    project_year: 2024,
                },
                "beta 2.0 requires LabVIEW 2026; this project is locked for 2024",
                "`lvpm lock --upgrade-package beta@<version>` again",
            ),
        ] {
            let text = super::error(&error);
            assert!(text.starts_with(why), "{text}");
            let last = text.lines().last().unwrap();
            assert!(
                last.starts_with("next: ") && last.ends_with(again),
                "{text}"
            );
        }
    }

    #[test]
    fn every_stale_reason_names_the_line_at_fault() {
        let named = |name: &str| name.to_string();
        let ci = || DependencyTable::Group("ci".to_string());
        let excludes = |table| StaleReason::ConstraintExcludesLocked {
            name: named("beta"),
            constraint: named("2.0"),
            locked: named("1.0"),
            table,
        };
        for (reason, text) in [
            (
                StaleReason::DependencyAdded {
                    name: named("gamma"),
                    table: DependencyTable::Dependencies,
                },
                "dependency \"gamma\" was added",
            ),
            (
                StaleReason::DependencyRemoved {
                    name: named("beta"),
                    table: DependencyTable::Dependencies,
                },
                "dependency \"beta\" was removed",
            ),
            (
                StaleReason::DependencyAdded {
                    name: named("caraya"),
                    table: DependencyTable::DevDependencies,
                },
                "dev dependency \"caraya\" was added",
            ),
            (
                StaleReason::DependencyRemoved {
                    name: named("caraya"),
                    table: DependencyTable::DevDependencies,
                },
                "dev dependency \"caraya\" was removed",
            ),
            (
                excludes(DependencyTable::Dependencies),
                "\"beta\" \"2.0\" excludes locked 1.0",
            ),
            (
                excludes(DependencyTable::DevDependencies),
                "dev dependency \"beta\" \"2.0\" excludes locked 1.0",
            ),
            (
                StaleReason::DependencyAdded {
                    name: named("beta"),
                    table: ci(),
                },
                "[dependency-groups.ci] dependency \"beta\" was added",
            ),
            (
                StaleReason::DependencyRemoved {
                    name: named("ci_tools"),
                    table: ci(),
                },
                "[dependency-groups.ci] dependency \"ci_tools\" was removed",
            ),
            (
                excludes(ci()),
                "[dependency-groups.ci] \"beta\" \"2.0\" excludes locked 1.0",
            ),
            (StaleReason::LabviewChanged, "labview changed"),
            (StaleReason::SourcesChanged, "sources changed"),
            (
                StaleReason::MinAgeChanged { from: 14, to: 7 },
                "min-age-days changed (14 -> 7)",
            ),
            (
                StaleReason::MinAgeChanged { from: 0, to: 14 },
                "min-age-days changed (0 -> 14)",
            ),
            (
                StaleReason::PolicyChanged,
                "[policy] changed since lvpm.lock was written",
            ),
            (
                StaleReason::FormatOutdated,
                "the lock format is outdated (this lvpm writes format 2)",
            ),
        ] {
            assert_eq!(stale_reason(&reason), text, "{reason:?}");
        }
    }

    #[test]
    fn a_failed_check_is_one_line_that_ends_with_the_command_to_run() {
        let path = PathBuf::from("work/lvpm.lock");
        for (error, expected) in [
            (
                LockError::Stale(StaleReason::DependencyAdded {
                    name: "gamma".into(),
                    table: DependencyTable::Dependencies,
                }),
                "lvpm.lock is out of date: dependency \"gamma\" was added; \
                 run `lvpm lock` to update it",
            ),
            (
                LockError::Stale(StaleReason::MinAgeChanged { from: 14, to: 7 }),
                "lvpm.lock is out of date: min-age-days changed (14 -> 7); \
                 run `lvpm lock` to update it",
            ),
            (
                LockError::NoLock { path: path.clone() },
                "there is no lvpm.lock; run `lvpm lock` to create it",
            ),
            (
                LockError::UnreadableLock {
                    path,
                    reason: "parsing lvpm.lock: TOML parse error at line 1\n  |\n1 | not toml"
                        .into(),
                },
                "lvpm.lock cannot be read (parsing lvpm.lock: TOML parse error at line 1); \
                 run `lvpm lock` to regenerate it",
            ),
        ] {
            assert_eq!(super::error(&error), expected);
        }
    }

    fn manifest(package: &str, wanted: &str) -> Fact {
        Fact::Manifest {
            package: package.into(),
            wanted: wanted.into(),
        }
    }

    #[test]
    fn a_conflict_names_the_manifest_lines_as_they_are_written() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![
                manifest("A", "*"),
                manifest("C", "= 1.0"),
                manifest("D", ">= 2.0"),
                Fact::Requires {
                    package: "A".into(),
                    versions: "= 2.0".into(),
                    needs: "C".into(),
                    wanted: ">= 2.0".into(),
                },
            ],
        }));
        assert_eq!(
            text,
            "cannot lock: no set of versions satisfies lvpm.toml:\n\
             \x20 lvpm.toml: A = \"*\"\n\
             \x20 lvpm.toml: C = \"1.0\"\n\
             \x20 lvpm.toml: D = \">=2.0\"\n\
             \x20 A = 2.0 requires C >= 2.0\n\
             next: change one of these in lvpm.toml, then run `lvpm lock` again"
        );
    }

    #[test]
    fn a_pinned_version_is_named_as_the_flag_that_asked_for_it() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![
                manifest("alpha", "= 1.0"),
                Fact::Pinned {
                    package: "beta".into(),
                    version: "2.0".into(),
                },
            ],
        }));
        assert!(
            text.contains("\n  lvpm.toml: alpha = \"1.0\"\n  --upgrade-package beta@2.0\n"),
            "{text}"
        );
    }

    /// 2023-11-14T22:13:20Z.
    const NOW: i64 = 1_700_000_000;

    fn all_held(requirer: Option<&str>, days: u32) -> Fact {
        Fact::AllHeldBack(AllHeldBack {
            package: "alpha".into(),
            versions: vec![
                ("1.2".into(), NOW - 2 * SECS_PER_DAY),
                ("1.1".into(), NOW - 3 * SECS_PER_DAY),
            ],
            wanted: ">= 1.1".into(),
            required_by: requirer.map(String::from),
            days,
        })
    }

    fn alpha_is_pinned() -> Fact {
        Fact::SourcePinned {
            package: "Alpha".into(),
            source: "second".into(),
            feed: "http://second.example/index.vipr".into(),
        }
    }

    #[test]
    fn a_source_pin_beside_a_held_back_package_is_a_note_in_its_block_not_a_second_report() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![all_held(None, 14), alpha_is_pinned()],
        }));
        assert_eq!(
            text,
            "alpha >=1.1: every version is held back by min-age-days = 14\n\
             \x20 1.2 released 2023-11-12, eligible 2023-11-27\n\
             \x20 1.1 released 2023-11-11, eligible 2023-11-26\n\
             \x20 Alpha is pinned to \"second\" (http://second.example/index.vipr); \
             other sources are ignored for a pinned package\n\
             next: wait until 2023-11-26, lower min-age-days, or pin an exact version of alpha \
             in lvpm.toml"
        );
        assert_eq!(text.matches("next:").count(), 1, "{text}");
    }

    #[test]
    fn a_source_pin_beside_a_labview_gate_is_one_line_of_the_general_report() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![
                manifest("Alpha", "*"),
                Fact::NeedsLabview {
                    package: "Alpha".into(),
                    versions: "= 2.0".into(),
                    from: "http://second.example/index.vipr".into(),
                    needs_year: 2026,
                    project_year: 2024,
                },
                alpha_is_pinned(),
            ],
        }));
        assert_eq!(
            text,
            "cannot lock: no set of versions satisfies lvpm.toml:\n\
             \x20 lvpm.toml: Alpha = \"*\"\n\
             \x20 Alpha = 2.0 from http://second.example/index.vipr needs LabVIEW 2026, \
             and the project's is 2024\n\
             \x20 Alpha is pinned to \"second\" (http://second.example/index.vipr); \
             other sources are ignored for a pinned package\n\
             next: change one of these in lvpm.toml, then run `lvpm lock` again"
        );
    }

    #[test]
    fn a_pin_note_for_one_package_does_not_join_the_held_back_block_of_another() {
        let other = Fact::SourcePinned {
            package: "beta".into(),
            source: "second".into(),
            feed: "http://second.example/index.vipr".into(),
        };
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![all_held(None, 14), other],
        }));
        assert!(!text.contains("\n  Alpha is pinned"), "{text}");
        assert!(text.contains("\n  beta is pinned to \"second\""), "{text}");
    }

    #[test]
    fn a_pinned_package_its_source_does_not_list_says_what_to_change() {
        let text = error(&LockError::PinnedNotListed {
            package: "alpha".into(),
            source_name: "second".into(),
            feed: "http://second.example/index.vipr".into(),
        });
        assert_eq!(
            text,
            "alpha is pinned to \"second\" (http://second.example/index.vipr), which does not \
             list it; other sources are ignored for a pinned package\n\
             next: list alpha in that source, or remove the index key from alpha in lvpm.toml, \
             then run `lvpm lock` again"
        );
    }

    #[test]
    fn versions_all_held_back_are_listed_newest_first_with_the_day_each_is_eligible() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![all_held(None, 14)],
        }));
        assert_eq!(
            text,
            "alpha >=1.1: every version is held back by min-age-days = 14\n\
             \x20 1.2 released 2023-11-12, eligible 2023-11-27\n\
             \x20 1.1 released 2023-11-11, eligible 2023-11-26\n\
             next: wait until 2023-11-26, lower min-age-days, or pin an exact version of alpha \
             in lvpm.toml"
        );
    }

    #[test]
    fn a_held_back_requirement_says_who_requires_it() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![all_held(Some("beta"), 14)],
        }));
        assert!(
            text.ends_with("in lvpm.toml (it is required by beta)"),
            "{text}"
        );
    }

    #[test]
    fn the_longest_limit_renders_without_overflowing() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![all_held(None, u32::MAX)],
        }));
        assert!(
            text.starts_with(
                "alpha >=1.1: every version is held back by min-age-days = 4294967295\n"
            ),
            "{text}"
        );
        // The release plus u32::MAX days is year 11 761 244, not a wrapped or capped day.
        assert!(
            text.contains(
                "\n  1.2 released 2023-11-12, eligible 11761244-12-01\n  \
                 1.1 released 2023-11-11, eligible 11761244-11-30\n"
            ),
            "{text}"
        );
        assert!(
            text.ends_with(
                "\nnext: wait until 11761244-11-30, lower min-age-days, or pin an exact \
                 version of alpha in lvpm.toml"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_package_that_is_not_all_held_back_shows_its_young_version_among_the_facts() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![
                manifest("a", "*"),
                Fact::VersionHeldBack {
                    package: "a".into(),
                    version: "1.1".into(),
                    released: NOW - 3 * SECS_PER_DAY,
                    days: 14,
                },
            ],
        }));
        assert_eq!(
            text,
            "cannot lock: no set of versions satisfies lvpm.toml:\n\
             \x20 lvpm.toml: a = \"*\"\n\
             \x20 a 1.1 is held back by min-age-days = 14 (released 2023-11-11)\n\
             next: change one of these in lvpm.toml, then run `lvpm lock` again"
        );
    }

    #[test]
    fn what_is_wrong_besides_the_held_back_versions_follows_in_the_general_report() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![
                all_held(None, 14),
                Fact::NotListed {
                    package: "zeta".into(),
                },
            ],
        }));
        assert!(
            text.starts_with("alpha >=1.1: every version is held back"),
            "{text}"
        );
        assert!(
            text.contains("\ncannot lock:\n  zeta is in no configured source\nnext: look for it"),
            "{text}"
        );
    }

    #[test]
    fn the_oci_note_follows_a_package_in_no_source_and_keeps_the_short_heading() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![
                Fact::NotListed {
                    package: "zeta".into(),
                },
                Fact::OciExplicit {
                    source: "team".into(),
                },
            ],
        }));
        assert!(
            text.starts_with(concat!(
                "cannot lock:\n",
                "  zeta is in no configured source\n",
                "  note: OCI source \"team\" is explicit; pin with index = \"team\"\n",
                "next: look for it"
            )),
            "{text}"
        );
    }

    #[test]
    fn a_package_in_no_source_comes_with_the_search_to_run() {
        let text = error(&LockError::Conflict(Conflict {
            facts: vec![
                Fact::NotListed {
                    package: "zeta".into(),
                },
                Fact::NotListed {
                    package: "omega".into(),
                },
            ],
        }));
        assert_eq!(
            text,
            "cannot lock:\n\
             \x20 zeta is in no configured source\n\
             \x20 omega is in no configured source\n\
             next: look for it with `lvpm search zeta`, `lvpm search omega`, or list a repository \
             under [sources] in lvpm.toml, then run `lvpm lock` again"
        );
    }

    #[test]
    fn every_unfetchable_archive_is_in_one_message() {
        let text = error(&LockError::ArchivesUnavailable(vec![
            ArchiveFailure {
                name: "alpha".into(),
                version: "2.0".into(),
                reason: "downloading http://h/a.vip: refused".into(),
                in_full: false,
            },
            ArchiveFailure {
                name: "gamma".into(),
                version: "1.5".into(),
                reason: "reading g.vip".into(),
                in_full: false,
            },
        ]));
        assert!(
            text.starts_with("cannot lock: 2 archives could not be fetched:\n"),
            "{text}"
        );
        assert!(
            text.contains("\n  alpha 2.0: downloading http://h/a.vip: refused\n"),
            "{text}"
        );
        assert!(text.contains("\n  gamma 1.5: reading g.vip\n"), "{text}");
        assert!(text.ends_with("then run `lvpm lock` again"), "{text}");
    }

    #[test]
    fn a_whole_sentence_about_an_archive_is_shown_as_it_is() {
        let text = error(&LockError::ArchivesUnavailable(vec![ArchiveFailure {
            name: "alpha".into(),
            version: "1.1.0".into(),
            reason: "cannot fetch alpha 1.1.0 by digest from source \"team\": refused".into(),
            in_full: true,
        }]));
        assert!(
            text.contains("\n  cannot fetch alpha 1.1.0 by digest from source \"team\": refused\n"),
            "{text}"
        );
    }

    #[test]
    fn a_manifest_without_labview_is_told_what_to_add() {
        let text = error(&LockError::NoLabview);
        assert!(
            text.contains("add labview = \">=<year>\" under [project] in lvpm.toml"),
            "{text}"
        );
        assert!(text.ends_with("then run `lvpm lock` again"), "{text}");
    }

    #[test]
    fn every_error_ends_with_the_next_command_to_run() {
        for error in [
            LockError::NoManifest {
                searched_from: PathBuf::from("here"),
            },
            LockError::NoLabview,
            LockError::LabviewNotAYear {
                value: "soon".into(),
            },
            LockError::Cancelled,
        ] {
            let text = super::error(&error);
            let last = text.lines().last().unwrap();
            assert!(
                last.starts_with("next: ") && last.contains("`lvpm lock`"),
                "{text}"
            );
        }
    }

    fn refusal(name: &str, version: &str, repo: &str, kind: Refusal) -> AttestationRefusal {
        AttestationRefusal {
            name: name.into(),
            version: version.into(),
            origin: Origin::GitHub { repo: repo.into() },
            kind,
        }
    }

    fn refused(refusals: Vec<AttestationRefusal>) -> String {
        error(&LockError::Refused {
            refusals,
            trusted_publishers: vec!["https://github.com/a/*".into()],
        })
    }

    #[test]
    fn two_refused_packages_are_one_block_with_a_hint_each() {
        let text = refused(vec![
            refusal("serializer", "1.2.0", "b/x", Refusal::NoAttestation),
            refusal(
                "helper",
                "0.3.0",
                "b/x",
                Refusal::UntrustedPublisher {
                    subject: "https://github.com/b/x".into(),
                    signer: "https://github.com/b/x/.github/workflows/release.yml@refs/tags/v0.3.0"
                        .into(),
                },
            ),
        ]);
        assert_eq!(
            text,
            "lvpm refused 2 packages:\n\
             \x20 serializer 1.2.0 from github.com/b/x has no attestation\n\
             \x20   hint: ask the publisher to build with the lvpm release template\n\
             \x20 helper 0.3.0 was attested by repository https://github.com/b/x (workflow \
             https://github.com/b/x/.github/workflows/release.yml@refs/tags/v0.3.0)\n\
             \x20   which [policy] trusted-publishers does not list: https://github.com/a/*\n\
             \x20   hint: add the repository to [policy] trusted-publishers, or depend on a \
             release built by a trusted repository"
        );
    }

    #[test]
    fn an_oci_package_is_said_to_be_from_its_source_and_never_from_github() {
        let oci = |kind| AttestationRefusal {
            name: "alpha".into(),
            version: "1.0.0".into(),
            origin: Origin::Oci {
                source: "oci://reg:5000/team".into(),
            },
            kind,
        };
        let from = "alpha 1.0.0 from oci://reg:5000/team";
        let cannot = Refusal::CannotCheck {
            reason: NotChecked::LookupRefused,
        };
        let table = [
            (Refusal::NoAttestation, format!("{from} has no attestation")),
            (
                Refusal::DoesNotVerify {
                    cause: Invalid::NoSourceRepository,
                },
                format!(
                    "the attestation of {from} does not verify: the bundle's certificate names \
                     no source repository, so gh was not run"
                ),
            ),
            (
                cannot,
                "cannot check the attestation of alpha 1.0.0: GitHub refused the attestation \
                 lookup (HTTP 403)\n    hint: check that your token can read the repository"
                    .to_string(),
            ),
        ];
        for (kind, line) in table {
            let text = refused(vec![oci(kind)]);
            assert!(text.contains(&line), "{line:?} is not in: {text}");
            assert!(!text.contains("github.com/"), "{text}");
        }
    }

    #[test]
    fn a_failed_verification_blames_the_archive_of_a_github_package_and_the_manifest_of_an_oci_one()
    {
        let does_not_verify = |origin| {
            refused(vec![AttestationRefusal {
                name: "alpha".into(),
                version: "1.0.0".into(),
                origin,
                kind: Refusal::DoesNotVerify {
                    cause: Invalid::OtherArchive,
                },
            }])
        };
        let github = does_not_verify(Origin::GitHub { repo: "o/r".into() });
        assert!(
            github.contains("the archive may have been tampered with"),
            "{github}"
        );
        let oci = does_not_verify(Origin::Oci {
            source: "oci://reg:5000/team".into(),
        });
        assert!(
            oci.contains("the manifest may have been tampered with"),
            "{oci}"
        );
        assert!(!oci.contains("archive may"), "{oci}");
    }

    #[test]
    fn a_failed_signature_lookup_is_a_line_for_each_package_that_says_it_was_not_locked() {
        let failure = |name: &str, cause: &str| SignatureLookupFailure {
            name: name.into(),
            version: "1.0.0".into(),
            registry: "reg:5000".into(),
            cause: cause.into(),
        };
        let text = error(&LockError::SignatureLookup(vec![
            failure(
                "alpha",
                "registry reg:5000 answered 500 to GET team/alpha/referrers",
            ),
            failure("beta", "registry reg:5000 is unreachable: refused"),
        ]));
        assert_eq!(
            text,
            "alpha 1.0.0: the signature lookup on registry reg:5000 failed: registry reg:5000 \
             answered 500 to GET team/alpha/referrers; the package was not locked\n\
             beta 1.0.0: the signature lookup on registry reg:5000 failed: registry reg:5000 is \
             unreachable: refused; the package was not locked"
        );
    }

    #[test]
    fn the_header_says_package_for_one_and_packages_for_more() {
        let text = refused(vec![refusal("p", "1.0.0", "o/r", Refusal::NoAttestation)]);
        assert!(text.starts_with("lvpm refused 1 package:\n"), "{text}");
    }

    #[test]
    fn a_cannot_check_refusal_carries_the_hint_for_its_cause_and_none_turns_the_policy_off() {
        const AUTH: &str = "run `gh auth login` or set GH_TOKEN";
        const LATER: &str = "try again later";
        let table = [
            (
                NotChecked::GhMissing,
                "gh is not installed",
                "install the GitHub CLI 2.49 or newer: https://cli.github.com",
            ),
            (
                NotChecked::GhTooOld,
                "gh is too old to verify attestations",
                "upgrade the GitHub CLI to 2.49 or newer",
            ),
            (NotChecked::NoToken, "lvpm has no GitHub token", AUTH),
            (
                NotChecked::RateLimited { anonymous: true },
                "GitHub rate limit reached (60/h without a token)",
                AUTH,
            ),
            (
                NotChecked::TokenRejected,
                "GitHub did not accept the token (HTTP 401)",
                AUTH,
            ),
            (
                NotChecked::Unreachable,
                "could not reach GitHub",
                "check your network connection and try again",
            ),
            (
                NotChecked::LookupRefused,
                "GitHub refused the attestation lookup (HTTP 403)",
                "check that your token can read o/r",
            ),
            (
                NotChecked::RateLimited { anonymous: false },
                "GitHub rate limit reached",
                LATER,
            ),
            (
                NotChecked::Http { status: 500 },
                "GitHub returned HTTP 500",
                LATER,
            ),
            (
                NotChecked::Unreadable,
                "GitHub returned an unreadable answer",
                LATER,
            ),
            (
                NotChecked::GhFailed {
                    error: "access denied".into(),
                },
                "gh could not be run: access denied",
                LATER,
            ),
            (NotChecked::GhTimedOut, "gh timed out", LATER),
            (NotChecked::GhInterrupted, "gh was interrupted", LATER),
        ];
        for (reason, said, hint) in table {
            let text = refused(vec![refusal(
                "p",
                "1.0.0",
                "o/r",
                Refusal::CannotCheck { reason },
            )]);
            assert_eq!(
                text,
                format!(
                    "lvpm refused 1 package:\n  \
                     cannot check the attestation of p 1.0.0: {said}\n    hint: {hint}"
                )
            );
            assert!(!text.contains("policy"), "{text}");
        }
    }

    #[test]
    fn gh_stderr_is_stripped_of_colour_cut_to_5_lines_and_indented_4() {
        let stderr: String = (1..=8)
            .map(|n| match n % 2 {
                0 => format!("\x1b[31;1mline {n}\x1b[0m\n"),
                _ => format!("line {n}\n"),
            })
            .collect();
        let text = refused(vec![refusal(
            "p",
            "1.0.0",
            "o/r",
            Refusal::DoesNotVerify {
                cause: Invalid::Rejected { stderr },
            },
        )]);
        assert_eq!(
            text,
            "lvpm refused 1 package:\n\
             \x20 the attestation of p 1.0.0 from github.com/o/r does not verify:\n\
             \x20   line 1\n    line 2\n    line 3\n    line 4\n    line 5\n\
             \x20   hint: if gh's reason is a network error, try again; otherwise report this to the \
             publisher, the archive may have been tampered with. lvpm refuses it whatever \
             [policy] says"
        );
    }

    #[test]
    fn what_lvpm_itself_found_wrong_with_ghs_output_stands_on_the_line() {
        for (cause, own) in [
            (Invalid::UnreadableOutput, "unrecognised gh output"),
            (
                Invalid::OtherArchive,
                "gh's attestation does not match the archive",
            ),
            (
                Invalid::Rejected {
                    stderr: " \n".into(),
                },
                "gh gave no reason",
            ),
        ] {
            let text = refused(vec![refusal(
                "p",
                "1.0.0",
                "o/r",
                Refusal::DoesNotVerify { cause },
            )]);
            assert!(
                text.contains(&format!(
                    "  the attestation of p 1.0.0 from github.com/o/r does not verify: {own}\n    hint: if gh's"
                )),
                "{text}"
            );
        }
    }

    #[test]
    fn control_characters_from_gh_and_certificates_do_not_reach_the_terminal() {
        let text = refused(vec![
            refusal(
                "p",
                "1.0.0",
                "o/r",
                Refusal::DoesNotVerify {
                    cause: Invalid::Rejected {
                        stderr: "\x1b]0;title\x07bad\rsig\n".into(),
                    },
                },
            ),
            refusal(
                "q",
                "1.0.0",
                "o/q",
                Refusal::UntrustedPublisher {
                    subject: "https://github.com/b/\x1b]8;;x\x07q".into(),
                    signer: "w\x07".into(),
                },
            ),
        ]);
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{text:?}"
        );
        assert!(text.contains("    ?]0;title?bad?sig\n"), "{text:?}");
    }

    #[test]
    fn checking_is_a_progress_line_on_stderr() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::AttestationChecking {
            name: "p".into(),
            version: "1.0.0".into(),
        });
        assert_eq!(err.text(), "checking attestation of p 1.0.0\n");
        assert_eq!(out.text(), "");
    }

    #[test]
    fn unchecked_packages_collapse_to_one_warning_per_reason_with_its_hint() {
        let (mut renderer, out, err) = renderer();
        let package = |name: &str, version: &str| PackageRef {
            name: name.to_string(),
            version: version.to_string(),
        };
        for (packages, reason) in [
            (
                vec![package("a", "1.0.0"), package("b", "2.0.0")],
                NotChecked::GhMissing,
            ),
            (vec![package("c", "3.0.0")], NotChecked::GhTimedOut),
        ] {
            renderer.render(&Event::AttestationsNotChecked { packages, reason });
        }
        assert_eq!(
            err.text(),
            "warning: did not check the attestation of a 1.0.0, b 2.0.0: gh is not installed; \
             install the GitHub CLI 2.49 or newer: https://cli.github.com\n\
             warning: did not check the attestation of c 3.0.0: gh timed out; try again later\n"
        );
        assert_eq!(out.text(), "");
    }
}
