//! The attestation pass of locking: who built each GitHub package, and whether
//! the `[policy]` lets it be locked.
//!
//! Only a GitHub release is asked: a feed, a local file and vipm.io/NI have
//! nothing to ask and cost no call. A package the lock already holds keeps the
//! signer it was locked with and is only decided again, so an edit to
//! `[policy]` is enforced with no network; one locked with no signer is asked
//! again only when the policy now requires one.

use super::env::OpEnv;
use super::lock::kept_of;
use super::session::Downloads;
use crate::errors::{AttestationRefusal, Cancelled, LockError};
use crate::events::Event;
use crate::index;
use crate::lockfile::Lockfile;
use crate::solve::Resolution;
use crate::verify::{Accepted, Attestation, Outcome, Policy, decide};
use std::collections::HashMap;

/// What the pass found out.
pub(super) struct Attested {
    /// Who built each package that has an attestation, by lowercased name.
    pub signers: HashMap<String, Attestation>,
    /// One [`Event::AttestationsNotChecked`] per distinct reason: packages the
    /// policy let through that lvpm could not check. Said after the lock is written.
    pub not_checked: Vec<Event>,
}

/// Decide every GitHub package of `resolution` (its archives hashed in
/// `hashes`) under `policy`. Every package the policy refuses is collected, and
/// they are reported together.
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
    let mut unchecked: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for (key, entry) in &resolution.packages {
        let Some(repo) = index::github_repo(entry) else {
            continue;
        };
        let kept = kept_of(previous, key, entry);
        let stored = kept.and_then(|locked| {
            Some(Attestation {
                signer: locked.signer.clone()?,
                subject: locked.subject.clone()?,
            })
        });
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
                downloads.attestation(entry, &hashes[key])
            }
        };
        let package = (entry.name.clone(), entry.version.to_string());
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
                name: package.0,
                version: package.1,
                repo: repo.to_string(),
                kind,
            }),
        }
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
