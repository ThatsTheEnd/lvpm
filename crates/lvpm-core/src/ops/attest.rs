//! The attestation pass of locking: who built each GitHub or OCI package, and
//! whether the `[policy]` lets it be locked.
//!
//! Only a GitHub release and an OCI package are asked: a feed, a local file
//! and vipm.io/NI have nothing to ask and cost no call. A package the lock already holds keeps the
//! signer it was locked with and is only decided again, so an edit to
//! `[policy]` is enforced with no network; one locked with no signer is asked
//! again only when the policy now requires one.

use super::env::OpEnv;
use super::lock::kept_of;
use super::session::Downloads;
use crate::errors::{AttestationRefusal, Cancelled, LockError, Origin, SignatureLookupFailure};
use crate::events::{Event, PackageRef};
use crate::index::{self, Entry};
use crate::lockfile::Lockfile;
use crate::oci;
use crate::solve::Resolution;
use crate::verify::{Accepted, Attestation, NotChecked, Outcome, Policy, decide};
use anyhow::Result;
use std::collections::HashMap;

/// What the pass found out.
pub(super) struct Attested {
    /// Who built each package that has an attestation, by lowercased name.
    pub signers: HashMap<String, Attestation>,
    /// One [`Event::AttestationsNotChecked`] per distinct reason: packages the
    /// policy let through that lvpm could not check. Said after the lock is
    /// written, or after what a dry run would change.
    pub not_checked: Vec<Event>,
}

/// Who is asked about `entry`'s attestation, by where it comes from; `None`
/// when nobody is.
fn origin_of(entry: &Entry) -> Option<Origin> {
    if let Some(repo) = index::attesting_repo(entry) {
        return Some(Origin::GitHub {
            repo: repo.to_string(),
        });
    }
    index::oci_reference(entry).map(|_| Origin::Oci {
        source: entry.source.clone(),
    })
}

/// Ask for the attestation of `entry` (its archive hashes to `sha`) of the one
/// `origin` has. Only an OCI registry can fail to answer.
fn ask(downloads: &Downloads, origin: &Origin, entry: &Entry, sha: &str) -> Result<Outcome> {
    match origin {
        Origin::GitHub { repo } => Ok(downloads.attestation(repo, sha)),
        Origin::Oci { .. } => downloads.oci_attestation(entry),
    }
}

/// Decide every GitHub and OCI package of `resolution` (its archives hashed in
/// `hashes`) under `policy`. Every package the policy refuses is collected, and
/// they are reported together; so is every package whose registry could not be
/// asked, which comes first: there is no answer to decide.
pub(super) fn attest_archives(
    env: &OpEnv,
    resolution: &Resolution,
    hashes: &HashMap<String, String>,
    previous: Option<&Lockfile>,
    policy: &Policy,
) -> Result<Attested, LockError> {
    let mut downloads = None;
    let mut signers = HashMap::new();
    let mut refusals = Vec::new();
    let mut lookups = Vec::new();
    let mut unchecked: Vec<(NotChecked, Vec<PackageRef>)> = Vec::new();
    for (key, entry) in &resolution.packages {
        let Some(origin) = origin_of(entry) else {
            continue;
        };
        let kept = kept_of(previous, key, entry);
        let stored = kept.and_then(|locked| locked.attestation());
        let outcome = match (kept, stored) {
            (_, Some(attestation)) => Outcome::Attested(attestation),
            (Some(_), None) if !policy.requires() => continue,
            _ => {
                if env.cancel.is_cancelled() {
                    return Err(Cancelled.into());
                }
                env.sink.emit(Event::AttestationChecking {
                    name: entry.name.clone(),
                    version: entry.version.to_string(),
                });
                if downloads.is_none() {
                    downloads = Some(Downloads::open(env)?);
                }
                let downloads = downloads.as_ref().expect("opened above");
                match ask(downloads, &origin, entry, &hashes[key]) {
                    Ok(outcome) => outcome,
                    // Only a registry's failure is a lookup that failed.
                    Err(why) => {
                        let failed = why.downcast::<oci::Error>()?;
                        lookups.push(SignatureLookupFailure {
                            name: entry.name.clone(),
                            version: entry.version.to_string(),
                            registry: failed.registry().to_string(),
                            cause: failed.to_string(),
                        });
                        continue;
                    }
                }
            }
        };
        let package = PackageRef {
            name: entry.name.clone(),
            version: entry.version.to_string(),
        };
        match decide(outcome, policy) {
            Ok(Accepted::Signed(attestation)) => {
                signers.insert(key.clone(), attestation);
            }
            Ok(Accepted::Unsigned) => {}
            Ok(Accepted::Unchecked(reason)) => {
                match unchecked.iter_mut().find(|(said, _)| *said == reason) {
                    Some((_, packages)) => packages.push(package),
                    None => unchecked.push((reason, vec![package])),
                }
            }
            Err(kind) => refusals.push(AttestationRefusal {
                name: package.name,
                version: package.version,
                origin,
                kind,
            }),
        }
    }
    if !lookups.is_empty() {
        return Err(LockError::SignatureLookup(lookups));
    }
    if !refusals.is_empty() {
        return Err(LockError::Refused {
            refusals,
            trusted_publishers: policy.trusted_publishers.clone(),
        });
    }
    let not_checked = (unchecked.into_iter())
        .map(|(reason, packages)| Event::AttestationsNotChecked { packages, reason })
        .collect();
    Ok(Attested {
        signers,
        not_checked,
    })
}
