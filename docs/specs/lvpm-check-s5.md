<!-- spec-version: 14.0.1 -->
# Spec: `lvpm check` (slice S5)

**Format:** dev-team specs 14.0.1 · **Status:** approved 2026-11-02 · **Date:** 2026-11-02 ·
**Slice:** [S5 in build-slices.md](../vision/build-slices.md#slice-s5-lvpm-check) · **Issue:** [#63](https://github.com/ThatsTheEnd/lvpm/issues/63) ·
**Builds on:** S1 (`lvlib.rs`, `[project] labview` as a minimum), S11 (dependency groups)

## Intent Description

A library author finds out that a VI links to a file that is not there, or
into a package the manifest does not declare, only when someone else opens
it in LabVIEW. `lvpm check [DIR]` finds this earlier, without starting
LabVIEW. It reads every VI's linker table and save version and names every
VI that breaks a rule, together with the link at fault. It exits non-zero
when there is at least one finding.

The rules: every VI is saved at or below the project's minimum LabVIEW and
has its compiled code separated. Every link resolves to a file that exists
at exactly that path. Every link outside the project goes into LabVIEW's own
`vi.lib`, or into a package the manifest declares directly. The library's
own VIs may use `[dependencies]` only. Other VIs under DIR, such as tests,
may also use `[dev-dependencies]` and dependency groups. This is cargo's
rule (a crate sees only its direct dependencies, and only tests see
dev-dependencies), applied to LabVIEW links.

Demo: break a link in pilot A; `lvpm check` names the VI and the link.

## Architecture Specification

**Placement ([ADR 5](../adr/0005-core-and-cli-split.md)).**

| Part | Where |
|---|---|
| RSRC block walker, linker-table reader, `PTH0` codec, save version, separate-compiled flag, `.llb` member list | `crates/lvpm-core/src/linker.rs` (new) |
| `PTH0` encode/decode | moves from `viserver.rs` into `linker.rs`; `viserver.rs` calls it |
| Library membership | `crates/lvpm-core/src/lvlib.rs` (exists, reused as it is) |
| Operation | `crates/lvpm-core/src/ops/check.rs` returns the findings; findings are data, not events, unless a progress event is needed for long walks |
| CLI | `src/main.rs` (`check [DIR]`), `src/render/check.rs`, `src/render/mod.rs` (and `events.rs` only if an event is added) |
| Fixtures | `tests/fixtures/check/`, saved with the local LabVIEW 2026 |
| Scenarios | `tests/features/build-slices/s5-lvpm-check.feature`, added to `FEATURE_FILES` |

**What the files say (observed 2026-11-02 in the pilot `.vip`s and the cached
`oglib_boolean`, see PROVENANCE).**

- RSRC header: `"RSRC\r\n"`, u16 format, type, creator, u32 info offset at 16, u32 data offset at 24. The block list sits in the info section. Each block is a 4-character type with sections, and each section's data is `u32 size | bytes` from the data offset.
- `vers` holds the save version: the first byte is BCD major (`0x13` = LabVIEW 2013, `0x20` = 2020, `0x26` = 2026).
- `LIvi`, `LIfp` and `LIbd` hold the links as `PTH0` records: `"PTH0" | u32 len | u16 kind | u16 levels | pascal components`. Kind 0 is absolute or symbolic (the first component is `<vilib>`, `<userlib>`, `<instrlib>`, …). Kind 1 is relative to the VI **file**, and an empty component means "up one level". Kind 2 is "not a path" and is skipped.
- A link may run through an `.llb` (`<vilib>/Utility/error.llb/Trim Whitespace.vi`). It resolves when the `.llb` exists and lists that member.
- The separate-compiled-code flag lives in `LVSR`. Its bit is found by comparing two fixtures that are the same except for that setting, and is recorded in `linker.rs` with that evidence.

**Resolution (closes the prototype's gaps).**

- Exact path: a link resolves only when the file (or `.llb` member) exists. The prototype's "some ancestor exists" rule is not used.
- `<vilib>` and the other symbolic roots resolve against the bound installation first, then against each venv package's `<pkg>/1/…` tree, in LabVIEW's order. The installation is the one the venv is bound to (`.lvpm/venv.json`). LabVIEW is never started.
- Attribution: a resolved file that a venv install record (`install::list_installed`) lists belongs to that package. A `vi.lib` file that no record lists is LabVIEW's own and always allowed.
- An undecodable linker block fails as `linker table not readable (block, offset)`, never as "no links".

**Inputs.** The minimum LabVIEW is `Project::labview` (a year). The library
is `[distribution] top-level-library`; its members come from `lvlib::members`.
The walk covers `.vi`, `.vit`, `.ctl` and `.ctt` under DIR and skips the venv
folder and dot-folders. DIR defaults to the project root.

**Out of scope:** running inside `build` (S5b); `lvpm tree`; decoding the old
OpenG `PTH0` flavour (follow-up issue, once a sample exists).

## Acceptance Criteria

1. A fixture VI with a link to a missing file fails `lvpm check`, and the output names the VI and the link path.
2. A fixture library VI that links into an installed venv package missing from `[dependencies]` fails, naming the VI, the link and the package. It passes once the package is in `[dependencies]`.
3. A link into a package that is only a transitive dependency fails the same way as criterion 2.
4. A library member that links into a package listed only in `[dev-dependencies]` or a group fails. A non-member VI under DIR (a test) that links into the same package passes.
5. A fixture saved in LabVIEW 2026 fails under `labview = ">=2025"`, naming the VI and both versions. It passes under `">=2026"`.
6. A fixture saved without separated compiled code fails, naming the VI.
7. A link through an `.llb` resolves only when the `.llb` lists that member.
8. Exactly one finding per broken rule per VI and link. All findings are listed before the exit, and the exit code is non-zero when there is any finding and 0 when there is none.
9. An undecodable linker block is reported as "not readable" with the VI named, never as a pass.
10. `lvpm check` starts no LabVIEW: it runs with no LabVIEW process and no VI Server. Where the bound installation is absent, links into its `vi.lib` are counted as "not checked: N" and do not fail.
11. Pilot A (pinned checkout, `@pilot-source`) passes. Pilot B passes with `labview_open_source_lib_data_manipulation` in `[dependencies]` and fails naming it without.
12. Without a synced venv, `lvpm check` says to run `lvpm sync` and exits non-zero. It does not report every external link as undeclared.
13. `viserver.rs` uses the `PTH0` codec from `linker.rs`; its existing `PTH0` tests pass unchanged.
14. docs/usage.md lists `lvpm check`; PROVENANCE.md records where the linker format knowledge came from.

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| linker table | The `LIvi`/`LIfp`/`LIbd` blocks of a VI, listing the files it links to as `PTH0` records | `verified` | spike on pilot VIs; user approved the approach |
| external link | A link that resolves outside DIR | `verified` | slice text |
| library member | A file `lvlib::members` lists for `[distribution] top-level-library` | `verified` | S1, user decision |
| LabVIEW's own | A `vi.lib` file of the bound installation that no venv install record lists | `verified` | user decision |
| direct dependency | A package named in the manifest table the VI may use (see criterion 4), not one reached only through the lock | `verified` | user decision |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| Where fixture VIs come from (PROVENANCE) | `requires-stakeholder-input` | human | Saved with the local LabVIEW 2026, committed under `tests/fixtures/check/`; our own work. uv commits hand-built wheel fixtures the same way. |
| How a link is attributed to a package | `requires-stakeholder-input` | human | Venv install records; needs a synced venv, like `uv pip check`. |
| Transitive links | `requires-stakeholder-input` | human | Fail: direct only, cargo's rule. |
| One PR or split | `requires-stakeholder-input` | human | One PR. |
| May check read the installation's vi.lib | `requires-stakeholder-input` | human | Yes, read but never start, like cargo needing the sysroot on disk; absent install → "not checked: N". |
| Which VIs are held to `[dependencies]` only | `requires-stakeholder-input` | human | Library members; other VIs may also use dev and group dependencies, as with cargo's `src/` vs `tests/`. |
| Old OpenG `PTH0` flavour | `requires-stakeholder-input` | human | No sample on this machine; fail as "not readable", follow-up issue. |
| No `[distribution]` table | `inferable` | inference | No library means no members: every VI follows the non-member rule (any declared group). Predictability check: "all strict" was the alternative; it is ruled out because an app project's tests would then need their tools in `[dependencies]`. |
| Minimum LabVIEW | `inferable` | inference | `Project::labview`, as the slice states. No `labview` key → the save-version rule is skipped and the output says so. Alternative "fail" ruled out: S1 made the key optional. |
| Output and exit code | `inferable` | inference | List every finding, then exit 1, as `cargo check` reports every error. Alternative "stop at first" ruled out by the demo, which wants names. |
| `--json` | `inferable` | inference | Follows the existing `render/json.rs` convention if the CLI has a global JSON flag; no new flag. |
| Which files are walked | `inferable` | inference | `.vi .vit .ctl .ctt` under DIR, skip the venv and dot-folders; the venv is someone else's code. |
| Create / update / delete of findings | `inferable` | inference | Read-only command: it writes nothing (completeness sweep). |
| Auth, audit | `inferable` | inference | Not applicable: local, read-only, no network. |

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion
- [x] Architecture constrains without over-engineering
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged — inferable with rationale or resolved by human
