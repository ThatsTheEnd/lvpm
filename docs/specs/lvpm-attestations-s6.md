<!-- spec-version: 2 -->
# Spec: attestations and consumer policy (S6)

Issue: [ThatsTheEnd/lvpm#65](https://github.com/ThatsTheEnd/lvpm/issues/65)

## Intent Description

Today a GitHub dependency is trusted on first use: the lock pins its SHA-256
but says nothing about who built it. After this slice, a release built by
lvpm's CI template carries a GitHub artifact attestation. lvpm checks that
attestation when it locks and writes the signer into `lvpm.lock`.

A project that wants more than trust on first use sets a `[policy]` table.
`require-signature = true` refuses a GitHub release with no attestation.
`trusted-publishers` refuses a release attested by a repository it does not
list, for example a fork's release of the same package. The consumer's policy
decides trust, never the feed (distribution-design §5).

uv has no counterpart. It pins hashes and ignores PyPI's PEP 740
attestations. npm checks provenance itself, but only in
`npm audit signatures`. lvpm checks at lock time.

## Architecture Specification

- **Verifier: the `gh` CLI** (decision Q1). The new
  `crates/lvpm-core/src/verify.rs` does two things:
  1. It asks GitHub's API whether the archive has an attestation:
     `GET /repos/{owner}/{repo}/attestations/sha256:{digest}`, with lvpm's
     existing client and token. A 404 or an empty list means the archive is
     **unattested**. This step needs no `gh`.
  2. When an attestation exists, it runs
     `gh attestation verify <file> --repo owner/repo --format json` on the
     archive in the cache. From the JSON it takes the certificate's source
     repository URI and its signer identity (the SAN, the workflow URL with
     its ref).

  The outcome is one of four:
  - **attested**: signer and source repository;
  - **unattested**;
  - **unchecked**: `gh` is missing or not logged in, with the reason;
  - **invalid**: `gh` exited non-zero on an attestation that exists.

  Running `gh` goes through a runner stored on `GitHub`. Its default reports
  gh as unavailable, so tests never start `gh`; the CLI and the TUI install
  the real one.
- **Policy: `[policy]` in `lvpm.toml`**, parsed in `project.rs` into
  `Project::policy`:
  - `require-signature`: a bool, default false.
  - `trusted-publishers`: a list of repository URL patterns, default empty,
    which means any signer. In a pattern, `*` matches any run of characters
    except `/`. So `https://github.com/hexagon/*` matches every repository of
    `hexagon`. Matching ignores case.

  An unknown key in `[policy]` is an error, like the other tables.
- **Where it applies** (decision Q3): only packages from a GitHub source
  (`github = "owner/repo"`). Feed, vipm.io/NI and local packages are not
  checked, whatever the policy says. S7b widens this.
- **When**: `lock` checks each GitHub package after it has the bytes, in
  `ops/lock.rs` where it hashes them. `sync` checks whenever it locks. An
  up-to-date lock is not checked again: its SHA-256 pins the bytes, and the
  signer was checked when the lock was written. `[policy]` joins the lock's
  `Inputs`, so changing the policy makes the lock stale and forces a new
  check.
- **Refusals**: the lock is not written, and the error names the package,
  its repository and the reason:
  - **invalid**: always refused, whatever the policy.
  - **unattested**: refused when `require-signature = true` or `trusted-publishers` is non-empty (decision Q10).
  - **unchecked**: refused when the policy requires an attestation (as above);
    the hint depends on the cause. When the policy is off
    (decision Q2), lvpm warns that it did not check the attestation and
    records no signer.
  - **attested, but the source repository matches no `trusted-publishers`
    pattern**: refused. The error shows the signer.
- **Lock**: `LockedPackage` gains `signer` (the SAN) and `subject` (the
  source repository URI). Both are optional and are not written when empty.
  `lockfile::FORMAT_VERSION` stays the same, because older locks read with
  the fields absent.
- **CI template** (`.github/workflow-templates/lvpm-release.yml`): it adds
  the `id-token: write` and `attestations: write` permissions, and an
  `actions/attest-build-provenance@v2` step on `dist/*.lvpkg` before the
  upload.
- **Docs**: `docs/usage.md` gains a `[policy]` section. The comment block in
  `lockfile.rs` shows the new fields.

## Acceptance Criteria

1. A GitHub release whose archive carries a valid attestation locks, and its
   `lvpm.lock` entry shows `signer` and `subject`. Unit test: a faked
   verifier against the lock builder. Demo: a real release built by the
   template.
2. With `require-signature = true`, an unattested GitHub release is refused.
   The error names the package and says it has no attestation, and no lock
   is written.
3. With `trusted-publishers = ["https://github.com/a/*"]`, a release attested
   by `https://github.com/b/x` is refused. The error names the signer. A
   release attested by `https://github.com/a/x` locks. With the same list
   and `require-signature` off, an unattested release is refused (Q10).
4. An attestation that `gh` rejects is refused even with no `[policy]`.
5. With the policy off and `gh` unavailable, the lock is written without a
   signer and a warning is shown. When the policy requires an attestation,
   the same case is an error whose hint names the fix for its cause.
6. Feed and local packages lock as before under `require-signature = true`.
7. Changing `[policy]` makes `lvpm lock --check` report a stale lock.
8. `[policy]` with an unknown key, or with a wrong type, fails to parse and
   names the key.
9. The template's YAML has the two permissions and the attest step. A
   cucumber or text check covers this, as for the template today.

## Glossary

| Term | Meaning | Status |
|---|---|---|
| attestation | a Sigstore-signed SLSA provenance statement GitHub stores for an artifact digest | verified |
| signer | the certificate's SubjectAlternativeName: the workflow that signed, with its ref | verified |
| subject (in the lock) | the attestation's source repository URI | verified (human chose lock fields via the design doc) |
| trusted publisher | a repository URL pattern in `[policy] trusted-publishers` | verified |

## Ambiguity Log

| # | Question | Class | Resolution |
|---|---|---|---|
| Q1 | Verify with the gh CLI or a Rust Sigstore library? | requires-stakeholder-input | human, 2026-11-02: **gh CLI**. Adds no crates, and gh is already part of the credential chain. |
| Q2 | With the policy off, gh missing or not logged in? | requires-stakeholder-input | human: **skip with a warning**, record no signer. With require-signature on, it is an error. |
| Q3 | Does require-signature apply to non-GitHub sources? | requires-stakeholder-input | human: **GitHub dependencies only**. S7b widens it. |
| Q4 | How do we tell "unattested" from "verification failed"? gh exits 1 for both. | inferable | Ask the attestations API first, then run gh only when one exists. This does not depend on gh's message text. |
| Q5 | What does `trusted-publishers` match against? | inferable | The certificate's source repository URI. That is what tells a fork from its upstream, and the design doc's example pattern is a repo URL. |
| Q6 | Is an invalid attestation refused with the policy off? | inferable | Yes. A present-but-forged attestation is a stronger signal than none, and the design says lvpm "verifies" it. |
| Q7 | Does sync re-verify an up-to-date lock? | inferable | No. The SHA-256 pins the bytes, and the policy is part of `Inputs`, so a policy change makes the lock stale. This matches the issue's "on lock and sync": sync verifies whenever it locks. |
| Q8 | Pattern syntax | inferable | `*` matches within one path segment and matching ignores case. That is enough for the design doc's `https://github.com/hexagon/*`. |
| Q9 | Bump the lock format version? | inferable | No. The new fields are optional, and serde reads old locks. |
| Q10 | Does `trusted-publishers` alone refuse unattested releases? | requires-stakeholder-input | human, 2026-11-02: **yes**. A non-empty list means every GitHub package needs an attestation from a listed repo; otherwise skipping the attestation would bypass the allowlist. |
| Q11 | Unchecked vs invalid | inferable | Unchecked: gh is not found, times out, is older than 2.49 (no `attestation`), lvpm has no token, or the API lookup fails with anything but 404. Invalid: gh ran with a token and exited non-zero, its JSON can't be read, or its subject digest isn't the archive's SHA-256. |

Completeness sweep: CRUD does not apply (policy is read-only config). Auth
uses the existing GitHub token and `gh` login. Errors are listed under
"Refusals". No new logging beyond the warning in Q2.

## Consistency Gate

- [x] Intent is unambiguous
- [x] Every intent behaviour maps to a criterion (signer→1, require→2, trusted→3)
- [x] Architecture constrained to policy, lock, verifier, lock wiring, rendering and the template (the Files list in the plan; a `deviation`)
- [x] Names consistent (signer, subject, attested/unattested/unchecked/invalid)
- [x] No contradictions
- [x] Every ambiguity logged

Verdict: **pass**.
