//! What lvpm does with a release's attestation: the `[policy]`, the outcome of
//! asking for the attestation, and the verdict. Pure: it reads no file, asks no
//! server and runs no program; whatever asks hands its answer in as an
//! [`Outcome`].
//!
//! ```toml
//! [policy]
//! require-signature = true                       # refuse a release with no attestation
//! trusted-publishers = ["https://github.com/a/*"]  # refuse an attestation from any other repository
//! ```
//!
//! A `[policy]` that asks for either key is said to *require* an attestation.
//! A release with an attestation gh rejects is refused whatever the policy says.

use serde::{Deserialize, Serialize};

/// The `[policy]` of a manifest. The default, no `[policy]` at all, is trust on
/// first use: nothing is required and any publisher is trusted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Policy {
    /// Refuse a release that has no attestation.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub require_signature: bool,
    /// URL patterns, as written, of the repositories whose attestations are
    /// trusted; empty trusts any. See [`pattern_matches`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub trusted_publishers: Vec<String>,
}

impl Policy {
    /// Whether this is no policy at all.
    pub fn is_default(&self) -> bool {
        *self == Policy::default()
    }

    /// Whether a release must carry an attestation.
    pub fn requires(&self) -> bool {
        self.require_signature || !self.trusted_publishers.is_empty()
    }

    /// Whether an attestation from the repository `subject` is trusted.
    fn trusts(&self, subject: &str) -> bool {
        self.trusted_publishers.is_empty()
            || (self.trusted_publishers.iter()).any(|pattern| pattern_matches(pattern, subject))
    }
}

/// Who built a release, as `gh attestation verify` reports it, verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attestation {
    /// The certificate's subject alternative name: the workflow's URL with its `@ref`.
    pub signer: String,
    /// The certificate's `sourceRepositoryURI`: the repository that ran it.
    pub subject: String,
}

/// What asking for a release's attestation came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The attestation exists and verifies.
    Attested(Attestation),
    /// GitHub holds no attestation for the archive.
    Unattested,
    /// The question could not be answered, for this reason.
    Unchecked(NotChecked),
    /// An attestation exists and does not verify.
    Invalid(Invalid),
}

/// Why lvpm could not check a release's attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum NotChecked {
    /// gh is not on the PATH.
    GhMissing,
    /// gh has no `attestation` command.
    GhTooOld,
    /// gh did not finish in time.
    GhTimedOut,
    /// gh ended on a signal, or its output did not arrive.
    GhInterrupted,
    /// gh could not be started or waited for: the system's own error text.
    GhFailed { error: String },
    /// lvpm has no GitHub token, and gh needs one.
    NoToken,
    /// GitHub's rate limit is used up; `anonymous` when no token was sent (60/h).
    RateLimited { anonymous: bool },
    /// GitHub did not accept the token (HTTP 401).
    TokenRejected,
    /// GitHub refused the lookup (HTTP 403), as for a repository the token cannot read.
    LookupRefused,
    /// GitHub answered with this other failing status.
    Http { status: u16 },
    /// GitHub could not be reached.
    Unreachable,
    /// GitHub's answer could not be read.
    Unreadable,
}

/// Why an attestation does not verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// gh rejected it; this is its stderr.
    Rejected { stderr: String },
    /// gh's output could not be read.
    UnreadableOutput,
    /// gh vouched for no subject with the archive's digest.
    OtherArchive,
}

/// Why a package is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The policy requires an attestation and the release has none.
    NoAttestation,
    /// The attestation is from a repository `trusted-publishers` does not list.
    UntrustedPublisher { subject: String, signer: String },
    /// The attestation does not verify.
    DoesNotVerify { cause: Invalid },
    /// The policy requires an attestation and lvpm could not look.
    CannotCheck { reason: NotChecked },
}

/// A package the policy lets through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accepted {
    /// Lock it with who built it.
    Signed(Attestation),
    /// Lock it with no signer: it has no attestation.
    Unsigned,
    /// Lock it with no signer and warn: lvpm could not check, for this reason.
    Unchecked(NotChecked),
}

/// What `policy` makes of `outcome`.
pub fn decide(outcome: Outcome, policy: &Policy) -> Result<Accepted, Refusal> {
    match outcome {
        Outcome::Invalid(cause) => Err(Refusal::DoesNotVerify { cause }),
        Outcome::Attested(attestation) if policy.trusts(&attestation.subject) => {
            Ok(Accepted::Signed(attestation))
        }
        Outcome::Attested(attestation) => Err(Refusal::UntrustedPublisher {
            subject: attestation.subject,
            signer: attestation.signer,
        }),
        Outcome::Unattested if policy.requires() => Err(Refusal::NoAttestation),
        Outcome::Unattested => Ok(Accepted::Unsigned),
        Outcome::Unchecked(reason) if policy.requires() => Err(Refusal::CannotCheck { reason }),
        Outcome::Unchecked(reason) => Ok(Accepted::Unchecked(reason)),
    }
}

/// Whether `pattern` matches the repository `subject`. ASCII case does not
/// count and a trailing `.git` on either is dropped. In the pattern `*` matches any
/// run, an empty one too, of characters other than `/`; every other character
/// stands for itself.
pub fn pattern_matches(pattern: &str, subject: &str) -> bool {
    let subject = subject.to_ascii_lowercase();
    let subject = subject.strip_suffix(".git").unwrap_or(&subject);
    let pattern = pattern.to_ascii_lowercase();
    let pattern = pattern.strip_suffix(".git").unwrap_or(&pattern);
    let pattern: Vec<char> = pattern.chars().collect();
    let subject: Vec<char> = subject.chars().collect();
    glob(&pattern, &subject)
}

/// Whether `pattern` matches all of `text`. `matched[j]` says whether the
/// pattern so far matches `text[..j]`, so the cost is pattern length times
/// text length however many stars there are.
fn glob(pattern: &[char], text: &[char]) -> bool {
    let mut matched = vec![false; text.len() + 1];
    matched[0] = true;
    for &p in pattern {
        let mut next = vec![false; text.len() + 1];
        for j in 0..=text.len() {
            next[j] = match p {
                // An empty run, or one more non-`/` character on a run.
                '*' => matched[j] || (j > 0 && text[j - 1] != '/' && next[j - 1]),
                _ => j > 0 && text[j - 1] == p && matched[j - 1],
            };
        }
        matched = next;
    }
    matched[text.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attestation(repo: &str) -> Attestation {
        Attestation {
            signer: format!("https://github.com/{repo}/.github/workflows/release.yml@refs/tags/v1"),
            subject: format!("https://github.com/{repo}"),
        }
    }

    fn attested(repo: &str) -> Outcome {
        Outcome::Attested(attestation(repo))
    }

    fn policy(require_signature: bool, trusted: &[&str]) -> Policy {
        Policy {
            require_signature,
            trusted_publishers: trusted
                .iter()
                .map(|p| format!("https://github.com/{p}"))
                .collect(),
        }
    }

    #[test]
    fn the_policy_decides() {
        let none = policy(false, &[]);
        let required = policy(true, &[]);
        let trusted = policy(false, &["a/*"]);
        let signed = |repo: &str| Ok(Accepted::Signed(attestation(repo)));
        let untrusted = Err(Refusal::UntrustedPublisher {
            subject: "https://github.com/b/x".into(),
            signer: "https://github.com/b/x/.github/workflows/release.yml@refs/tags/v1".into(),
        });
        let unchecked = || Outcome::Unchecked(NotChecked::GhMissing);
        let cannot = Err(Refusal::CannotCheck {
            reason: NotChecked::GhMissing,
        });
        let rejected = || Invalid::Rejected {
            stderr: "verification failed".into(),
        };
        let invalid = || Outcome::Invalid(rejected());
        let forged = Err(Refusal::DoesNotVerify { cause: rejected() });

        let rows = [
            (attested("a/x"), &none, signed("a/x")),
            (attested("a/x"), &required, signed("a/x")),
            (attested("A/x"), &trusted, signed("A/x")),
            (attested("b/x"), &trusted, untrusted),
            (Outcome::Unattested, &none, Ok(Accepted::Unsigned)),
            (Outcome::Unattested, &required, Err(Refusal::NoAttestation)),
            (Outcome::Unattested, &trusted, Err(Refusal::NoAttestation)),
            (
                unchecked(),
                &none,
                Ok(Accepted::Unchecked(NotChecked::GhMissing)),
            ),
            (unchecked(), &required, cannot.clone()),
            (unchecked(), &trusted, cannot),
            (invalid(), &none, forged.clone()),
            (invalid(), &required, forged.clone()),
            (invalid(), &trusted, forged),
            (attested("b/x"), &required, signed("b/x")),
        ];
        for (outcome, policy, verdict) in rows {
            assert_eq!(
                decide(outcome.clone(), policy),
                verdict,
                "{outcome:?} under {policy:?}"
            );
        }
    }

    #[test]
    fn a_policy_requires_an_attestation_when_it_asks_for_a_signature_or_names_publishers() {
        assert!(!policy(false, &[]).requires());
        assert!(policy(true, &[]).requires());
        assert!(policy(false, &["a/*"]).requires());
        assert!(policy(false, &[]).is_default());
        assert!(!policy(true, &[]).is_default());
    }

    #[test]
    fn trusted_publishers_patterns() {
        for (pattern, matches, subject) in [
            ("https://github.com/a/*", true, "https://github.com/a/x"),
            ("https://github.com/a/*", true, "https://GitHub.com/A/X"),
            ("https://github.com/a/*", true, "https://github.com/a/x.git"),
            ("https://github.com/a/*", false, "https://github.com/a/x/y"),
            ("https://github.com/a/*", false, "https://github.com/ab/x"),
            ("https://github.com/a/x*", true, "https://github.com/a/x"),
            (
                "https://github.com/a/*-*",
                true,
                "https://github.com/a/lv-lib",
            ),
            (
                "https://github.com/a/x.y",
                false,
                "https://github.com/a/xzy",
            ),
            ("https://github.com/a/x.git", true, "https://github.com/a/x"),
            ("https://github.com/a/**", true, "https://github.com/a/x"),
            ("https://github.com/a/", false, "https://github.com/a/x"),
            ("https://github.com/*/x", false, "https://github.com/a/b/x"),
            // The Kelvin sign lowercases to `k` under Unicode, not under ASCII.
            (
                "https://github.com/a/k*",
                false,
                "https://github.com/a/\u{212A}x",
            ),
        ] {
            assert_eq!(
                pattern_matches(pattern, subject),
                matches,
                "{pattern} on {subject}, expected {matches}"
            );
        }
    }

    #[test]
    fn many_stars_on_a_subject_they_do_not_match_answer_at_once() {
        let pattern = format!("https://github.com/{}b/x", "*a".repeat(12));
        let subject = format!("https://github.com/{}/x", "a".repeat(100));
        let started = std::time::Instant::now();
        assert!(!pattern_matches(&pattern, &subject));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
