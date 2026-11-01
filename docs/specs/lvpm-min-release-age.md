<!-- spec-version: 14.0.1 -->
# Spec: A minimum release age for packages

**Format:** dev-team specs 14.0.1 · **Status:** gate passed; the four stakeholder answers were given by the user on 2026-10-31 · **Date:** 2026-10-31 ·
**Slice:** none yet; `/plan` decides whether it becomes one under #57

## Intent Description

A package version that was just published can be malicious. A maintainer
account is hijacked, or a release is tampered with, and the community usually
finds out within days. lvpm today picks the newest version that fits, so a
project resolves to that version on the day it appears.

After this change, a project can say in `lvpm.toml` that lvpm must not choose
a version younger than a given number of days:

```toml
[sources]
min-age-days = 14
```

When lvpm resolves a package, it skips every version whose release date is
less than `min-age-days` days ago and takes the newest version that remains.
This applies to direct and transitive dependencies. A version the user pins
exactly in `lvpm.toml` is a deliberate choice and is exempt. Without the key,
or with `0`, nothing changes.

This complements the first-source-wins rule (S13), which stops a *different*
source from shipping a higher version of a name. The minimum age addresses a
bad new version on the *same* source.

## Architecture Specification

**Where the date comes from.** Package directories (`.vipr`, `.ogpd`) carry
`Package.Release Date` per entry. The parser in `index.rs` (`parse_into`)
reads it into a new `Entry` field: the release time as Unix seconds, `None`
when absent or unreadable. Formats seen on 2026-10-31 in NI's and JKI's feeds:

| Shape                                | Count | Read as                         |
| ------------------------------------ | ----- | ------------------------------- |
| `Thu, 19 May 2016 15:46:24 -0500`    | 4381  | RFC 2822, signed offset         |
| `... 15:46:24 GMT`                   | 27    | offset 0                        |
| `... 15:46:24 0530` (no sign)        | 291   | positive offset (`+0530`)       |
| `???, 19 ??? 2016 ...`               | 3     | unreadable, so `None`           |

Entries from local folders, `lvpm cache add` imports and `.lvpkg` files have
no release date (`None`).

**Where the rule applies.** In the solver (`solve.rs`), beside the existing
LabVIEW gate (`gated`): a candidate is eligible when its release date is
`None`, or when `now - release >= min-age-days * 86400`. "Now" is the system
clock at resolution time. A root whose manifest constraint is an exact
version is not filtered. Floors (`>=`), `*` and every transitive
requirement (always a floor) are. A version named on `lvpm add/upgrade pkg@v`
is an exact pin to the solver for that run, so it is exempt too. A held-back
version never falls through to a later source (first source wins, S13).

**The manifest.** `project.rs` reads `[sources] min-age-days` as a
integer from 0 to 4294967295 into `Project`. Absent means `0` (off). Any other type or
a negative value is an error that names the key; the key is known, so it is
not silently ignored like unknown keys.

**The lock.** `lockfile::Inputs` records `min-age-days` when it is not `0`, so
locks written without the key keep their bytes. A different value from the
manifest makes the lock stale. The lock stores no dates and no clock reading:
time passing never makes a lock stale, and `lvpm sync` with a current lock
installs the locked versions as recorded; sync re-resolves only when there is
no current lock.

**Reporting.** When the rule holds back a newer version of a package, lock
output names the package, the held-back version and its release date, in the
human and `--json` output (a new `events.rs` event). When no eligible version
satisfies a constraint, resolution fails and the error names the package, the
constraint, the limit and the versions skipped with their release dates.

**Out of scope.** A per-user default (`lvpm config`), a CLI flag, a
per-package exclude list, hours or other units, and global installs outside a
project (they have no manifest).

**Constraints.**

- No new dependency for parsing one fixed date format.
- `vipm.toml` conversion is unchanged; VIPM has no such key.
- Document the key in `docs/manifest.md` under `[sources]`.

## Acceptance Criteria

1. **Off by default.** A manifest without `min-age-days`, or with `0`,
   resolves exactly as today, and its lock is byte-identical to one written
   before this change.
2. **Too-new version skipped.** With `min-age-days = 14`, `foo = ">=1.0"`, and
   a feed listing `foo` 1.0 (30 days old) and 1.1 (3 days old), the lock picks
   1.0. Pass: lock has `foo` 1.0.
3. **Boundary.** A version exactly 14 × 86400 seconds old is eligible; one
   second younger is not. A release date in the future is too new.
4. **Transitive.** A transitive requirement `bar>=2.0` with `bar` 2.0 (old)
   and 2.1 (new) resolves to 2.0.
5. **Exact pin exempt.** `foo = "1.1"` with 1.1 three days old locks 1.1.
6. **No eligible version.** `foo = ">=1.1"` where every version the source
   lists at or above the floor is younger than the limit (no fall-through to a
   later source): lock fails, and the error names
   `foo`, `>=1.1`, the 14-day limit and 1.1 with its release date.
7. **Undated is allowed.** A feed entry with no or an unreadable
   `Package.Release Date`, and any local, cached or `.lvpkg` package, is
   eligible.
8. **Date parsing.** Each of the four shapes in the table above parses as
   stated; the sign-less `0530` reads as `+0530`.
9. **Held-back notice.** In case 2, lock output names `foo`, 1.1 and its
   release date; `--json` carries the same as an event.
10. **Bad value.** `min-age-days = -1`, `"14"`, `14.5` or `4294967296` is an error naming
    `min-age-days`.
11. **Lock staleness.** Changing `min-age-days` from 14 to 7 (or adding or
    removing it) makes `lvpm.lock` stale; leaving it unchanged while days
    pass does not.
12. **Docs.** `docs/manifest.md` lists `min-age-days` in the `[sources]`
    table with its meaning, the exact-pin exemption and the undated rule.

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| release date | The `Package.Release Date` a package directory lists for one package version | `verified` | NI and JKI feeds, 2026-10-31; user accepted |
| minimum release age (`min-age-days`) | The number of whole days that must have passed since a version's release date before lvpm may choose it | `verified` | user, 2026-10-31 |
| eligible version | A version the minimum release age does not exclude: old enough, undated, or pinned exactly as a root | `verified` | user answers, 2026-10-31 |
| exact pin | A `[dependencies]` or `[dev-dependencies]` constraint naming one version, such as `"1.2.3"` | `verified` | docs/manifest.md |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| Key name, place and unit | `requires-stakeholder-input` | human | `[sources] min-age-days = 14`, integer days, beside `defaults` |
| Entry with no or unreadable release date | `requires-stakeholder-input` | human | Allowed (fail open). Risk accepted: an attacker who controls a feed entry can omit the date; 173 JKI entries have none |
| Exact pin younger than the limit | `requires-stakeholder-input` | human | Allowed; the limit applies to floors, `*` and transitive picks |
| Per-user default or CLI flag | `requires-stakeholder-input` | human | Not in scope; `lvpm.toml` only |
| Behaviour when too new: older version vs failure | `inferable` | inference | The request says "don't install or upgrade to" newer ones, so the newest eligible version is chosen; failure only when none satisfies the constraint |
| Transitive dependencies covered | `inferable` | inference | A malicious release can enter through any dependency; transitive requirements are already floors |
| Local, cache and `.lvpkg` packages | `inferable` | inference | They carry no release date, so the undated rule (allowed) covers them; they are also inputs the user adds on purpose (S13) |
| Sign-less offset `0530` | `inferable` | inference | Negative offsets always carry `-` in the feeds, so a missing sign is a dropped `+`; at day granularity a wrong reading costs at most a day |
| Clock source | `inferable` | inference | System clock at resolution time, as `cache.rs` `now()` does |
| Lock records the setting, not dates | `inferable` | inference | Same pattern as `[sources]` order making a lock stale (S13); a lock must be a function of its inputs, with no timestamps |
| Time passing and `lvpm sync` | `inferable` | inference | The lock is the record and sync never re-resolves; the rule takes effect at the next `lvpm lock` or `lvpm add` |
| Invalid value | `inferable` | inference | Manifest rejects badly typed dependency values rather than misreading them; same for a known key |
| Held-back notice | `inferable` | inference | A silently older pick would look like a resolver bug; existing events report resolution outcomes |
| `min-age-days` placed outside `[sources]` | `inferable` | inference | Treated like any unknown top-level key (ignored), as today; the docs show it under `[sources]` |
| Global install outside a project | `inferable` | inference | No manifest, so no setting; out of scope |
| Trust in the release date | `inferable` | inference | The date is what the feed lists; the docs say the limit protects only as far as that date is trusted |
| `lvpm upgrade pkg@v` with a young `v` | `inferable` | inference | The explicit version is exempt for that run (it is an exact pin to the solver); `upgrade` writes a floor, so a later re-lock applies the limit again. Documented, not changed |

**Completeness sweep.** Entities: the `min-age-days` setting, the release
date. Setting: create/update/delete is editing `lvpm.toml` (criteria 1, 10,
11); read in criteria 2–7. Release date: read-only by design, from the feed
(criterion 8). Authentication and authorization: not applicable, no new
network access or identity. Audit/logging: the held-back notice (criterion 9).
Error handling: criteria 6 and 10.

## Consistency Gate

- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion
- [x] Architecture constrains without over-engineering
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged — inferable with rationale or resolved by human
