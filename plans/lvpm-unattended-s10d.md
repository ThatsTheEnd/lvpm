# Plan: --yes, --labview-bitness, and no `lvpm update` (s10d)

**Created**: 2026-11-02
**Branch**: claude/dev-team-ship-72c0fb
**Status**: implemented
**Gherkin persistence**: plan-file-only (step 1.2 writes `tests/features/build-slices/s10d-unattended.feature` itself, as S10a did; only the scenarios tagged `cucumber` below go there)
**Spec**: [docs/specs/lvpm-unattended-s10d.md](../docs/specs/lvpm-unattended-s10d.md) · Issue ThatsTheEnd/lvpm#74

## Goal

lvpm runs unattended (`--yes`, `LVPM_YES`) and picks the LabVIEW the user means
when two bitnesses of one version are installed (`--labview-bitness`). There is
no `lvpm update`; the README says why. Resolution, `lock` and `sync` do not change.

**Decision-defaults stances**: scope — only the three items the issue names;
replace-vs-merge — `venv remove`'s own `yes` is *replaced* by the global flag
(same spelling, so no user-visible break); integration — PR with auto-merge on
green checks (the default).

## Acceptance Criteria

- [ ] AC1 `lvpm --yes venv remove`, `lvpm venv remove -y` and `LVPM_YES=1 lvpm venv remove` delete without asking; `LVPM_YES` set to `0`, `false`, `no`, `off` or empty still asks (clap's falsey parser: `n`, `no`, `f`, `false`, `off`, `0`, empty, case-insensitive); any other value is on.
- [ ] AC2 Under `--json`, `venv remove` without `--yes`/`LVPM_YES` still exits 1 and its last stderr line contains both `--yes` and `LVPM_YES` (reworded from today's message); with either it deletes.
- [ ] AC3 `lvpm sync --yes` parses (a no-op for a command that never asks; parse-only, `sync` has no prompt to differ on).
- [ ] AC4 Faked list with 2026 32-bit and 64-bit: bitness 32 picks the 32-bit for `2026`, 64 the 64-bit (unit test in `target.rs`).
- [ ] AC5 No `--labview-bitness`: the list is not narrowed, so the first listed match wins as today (Windows `detect` lists 64-bit before 32-bit).
- [ ] AC6 `--labview-bitness 32` with only 64-bit installed fails with `no 32-bit LabVIEW installation detected (detected: LabVIEW 2026 (64-bit) …)` and a `hint:` naming `lvpm targets`. Nothing installed at all keeps today's `no LabVIEW installations detected` (through `OpEnv::roots`, whose `is_empty` check runs before `select`).
- [ ] AC7 `--labview-bitness` other than 32/64 is a clap `InvalidValue` error with `exit_code() == 2` whose text contains `[possible values: 32, 64]`; with `--prefix` it is a usage error too (plan addition, noted in the PR).
- [ ] AC8 In a venv bound to 64-bit, `--labview-bitness 32` fails naming the binding and the request, with a `hint:`; matching or absent flag works as today.
- [ ] AC9 No `lvpm update` (parse is a usage error); README's `## Upgrading` section contains "lvpm has no `lvpm update` command" and `lvpm lock --upgrade` before `lvpm sync`.

### Pinned wording

- AC6: `no 32-bit LabVIEW installation detected (detected: LabVIEW 2026 (64-bit)  v26.3)` + `\nhint: \`lvpm targets\` lists detected LabVIEW installations` (the `label()` of each installation, comma-separated, as `select` lists years today).
- AC8: `this venv is bound to LabVIEW 2026 (64-bit)  v26.3, not the 32-bit LabVIEW --labview-bitness asks for` + `\nhint: drop --labview-bitness, or run \`lvpm venv remove\` without it, then \`lvpm venv create --labview-bitness 32\``. Tests assert the full string.
- AC2: `` `venv remove` asks before it deletes; pass --yes (or set LVPM_YES=1) to delete under --json ``. The slice-3 `--json` scenario gains `And the error names "LVPM_YES"`.
- Help: `--yes`: "Answer yes to every prompt." (clap appends `[env: LVPM_YES=]`); `--labview-bitness`: "Pick the 32- or 64-bit LabVIEW when both of a version are installed."

### Interactions, decided

- Order: a venv is found first and checked for a clash (`find_venv`); only a pick outside a venv narrows the list.
- Narrowing happens before `select` / `select_for_project`, so the manifest's minimum still lets a newer year stand in: manifest 2025, installed 2025-64 + 2026-32, bitness 32 → 2026 32-bit.
- `--labview-version <path>` with a bitness that path does not have fails as `select` does today ("no LabVIEW target matching …"). Accepted; noted under Risks.
- `--labview-bitness` conflicts with `--prefix` (clap), as `--project` does.

## Slices

### Slice 1: unattended runs and bitness selection

**Depends-on:** none
**Files:** `Cargo.toml`, `src/main.rs`, `crates/lvpm-core/src/target.rs`, `crates/lvpm-core/src/ops/env.rs`, `crates/lvpm-core/src/ops/venv_cmd.rs`, `crates/lvpm-core/src/ops/init.rs`, `crates/lvpm-core/src/ops/query.rs`, `tests/features/build-slices/s10d-unattended.feature`, `tests/acceptance.rs`, `tests/steps/json.rs`, `tests/steps/venv.rs`, `tests/steps/docs.rs`, `tests/support/sandbox.rs`, `README.md`, `docs/usage.md`

**Behavior:** (each scenario is tagged with where it is tested)

```gherkin
Feature: s10d — run unattended, and pick the LabVIEW you mean

  # --yes ------------------------------------------------------------------

  @labview @cucumber
  Scenario Outline: a yes answers the venv remove prompt
    Given the project has a venv on the detected LabVIEW
    And LVPM_YES is "<env>"
    When "<command>" runs in the project
    Then it did not ask "[y/N]"
    And the venv no longer exists

    Examples:
      | command                  | env   |
      | lvpm --yes venv remove   |       |
      | lvpm venv remove -y      |       |
      | lvpm venv remove         | 1     |
      | lvpm --json venv remove  | 1     |

  @labview @cucumber
  Scenario Outline: a falsey LVPM_YES still asks
    Given the project has a venv on the detected LabVIEW
    And LVPM_YES is "<env>"
    When "lvpm venv remove" runs in the project and "n" is typed
    Then it asked "[y/N]"
    And the venv still exists

    Examples:
      | env   |
      | 0     |
      | false |
      | no    |
      | off   |
      |       |

  @labview @cucumber
  Scenario: typing y without --yes still deletes
    Given the project has a venv on the detected LabVIEW
    When "lvpm venv remove" runs in the project and "y" is typed
    Then the venv no longer exists

  # The --json refusal without yes is the existing scenario
  # "a command that would prompt fails instead under --json" (lvpm-tui slice 3).

  @unit:main.rs
  Scenario Outline: --yes and -y are global
    When "<args>" is parsed
    Then yes is on

    Examples:
      | args                    |
      | lvpm --yes venv remove  |
      | lvpm venv remove -y     |
      | lvpm sync --yes         |

  # --labview-bitness --------------------------------------------------------

  @unit:target.rs
  Scenario Outline: bitness picks between two installations of one version
    Given LabVIEW 2026 is installed as 32-bit and as 64-bit
    When LabVIEW 2026 is selected with bitness <bitness>
    Then the <bitness>-bit installation is chosen

    Examples:
      | bitness |
      | 32      |
      | 64      |

  @unit:target.rs
  Scenario: no bitness leaves the list as it is
    Given LabVIEW 2026 is installed as 64-bit, then as 32-bit
    When LabVIEW 2026 is selected with no bitness
    Then the 64-bit installation, listed first, is chosen

  @unit:target.rs
  Scenario: a bitness nothing has is refused, naming what was found
    Given only LabVIEW 2026 64-bit is installed
    When the installations are narrowed to bitness 32
    Then it fails with "no 32-bit LabVIEW installation detected"
    And the message names "LabVIEW 2026 (64-bit)" and "lvpm targets"

  @unit:target.rs
  Scenario: a bitness with nothing installed is not narrowed
    Given no LabVIEW is installed
    When the installations are narrowed to bitness 32
    Then the result is empty and no error is raised

  @unit:env.rs
  Scenario: a manifest's minimum still lets a newer year stand in
    Given LabVIEW 2025 64-bit and 2026 32-bit are installed, and the manifest wants 2025
    When a headless install picks with bitness 32
    Then LabVIEW 2026 32-bit is chosen

  @unit:query.rs
  Scenario: targets lists only the requested bitness
    Given LabVIEW 2026 is installed as 32-bit and as 64-bit
    When the targets are listed with bitness 64
    Then only LabVIEW 2026 (64-bit) is listed

  @unit:init.rs
  Scenario: init offers only the requested bitness
    Given LabVIEW 2026 is installed as 32-bit and as 64-bit
    When a project is initialised with bitness 32
    Then the venv is bound to LabVIEW 2026 32-bit

  @unit:main.rs
  Scenario Outline: a bitness that is not 32 or 64 is a usage error
    When "lvpm --labview-bitness <value> targets" is parsed
    Then it is an InvalidValue usage error with exit code 2 listing "[possible values: 32, 64]"

    Examples:
      | value |
      | 16    |
      | 40    |
      | x64   |
      | 0     |

  @unit:main.rs
  Scenario: --labview-bitness with --prefix is a usage error
    When "lvpm --prefix dest --labview-bitness 32 list" is parsed
    Then it is a usage error

  @unit:env.rs
  Scenario Outline: a bitness against the venv's binding
    Given a project venv bound to LabVIEW 2026 64-bit, with 32-bit also installed
    When the destination is settled with bitness "<flag>"
    Then <outcome>

    Examples:
      | flag | outcome                                                     |
      | 32   | it fails with the full pinned AC8 message                   |
      | 64   | the venv is the destination                                 |
      |      | the venv is the destination                                 |

  @unit:main.rs
  Scenario: lvpm tui refuses --labview-bitness but not --yes
    When each global option is given to "lvpm tui"
    Then every one but --yes is refused

  # update -------------------------------------------------------------------

  @unit:main.rs
  Scenario: there is no lvpm update
    When "lvpm update" is parsed
    Then it is a usage error

  @cucumber
  Scenario: the README says what to run instead of an update
    Then README.md has a "## Upgrading" section containing "has no `lvpm update` command"
    And in it "lvpm lock --upgrade" comes before "lvpm sync"
```

**Steps:**

#### Step 1.1: global `--yes` / `-y` with `LVPM_YES`

**Complexity**: standard
**IMPLEMENT**: Enable clap's `env` feature. Add `yes: bool` to `Cli` (`global = true`, `short = 'y'`, `env = "LVPM_YES"`, help as pinned). Remove `VenvCmd::Remove { yes }`; pass `cli.yes` into `cmd_venv`. Reword the `--json` refusal as pinned. Update `lvpm_tui_refuses_every_global_option` (main.rs:1254) to exempt `yes`.
**TEST**: parse tests — the three `--yes`/`-y` rows; assert only `yes == true` cases, so an exported `LVPM_YES` cannot flake them. Full suite green.
**REFACTOR**: keep `VenvCmd::Remove` a unit variant; no leftover `yes` binding.
**Files**: `Cargo.toml`, `src/main.rs`
**Commit**: `feat(s10d): global --yes and LVPM_YES answer every prompt`

#### Step 1.2: acceptance scenarios for `--yes` and `LVPM_YES`

**Complexity**: standard
**IMPLEMENT**: `tests/features/build-slices/s10d-unattended.feature` with the `@cucumber` scenarios (the `venv remove` ones also `@labview`), added to `FEATURE_FILES`. Add `And the error names "LVPM_YES"` to the slice-3 `--json` refusal scenario (tests/features/lvpm-tui/slice-3-json-and-the-exit-code-table.feature). Steps: `Given LVPM_YES is "<v>"` sets an extra child variable on the sandbox (like `set_headless`; empty string still exported), `Then it did not ask "[y/N]"` (stderr lacks it), `Then the venv no longer exists`. The sandbox's `env_clear` keeps an ambient `LVPM_YES` out of every other scenario.
**TEST**: `cargo test --test acceptance` green locally (the `@labview` ones run in CI's LabVIEW container / with `LVPM_TEST_LABVIEW=1`).
**REFACTOR**: the extra-variable setter mirrors `set_headless`; no second runner.
**Files**: `tests/features/build-slices/s10d-unattended.feature`, `tests/features/lvpm-tui/slice-3-json-and-the-exit-code-table.feature`, `tests/acceptance.rs`, `tests/steps/json.rs`, `tests/steps/venv.rs`, `tests/support/sandbox.rs`
**Commit**: `test(s10d): acceptance scenarios for --yes and LVPM_YES`

#### Step 1.3: `target::of_bitness` narrows a list

**Complexity**: standard
**IMPLEMENT**: `pub fn of_bitness(targets: &[LvTarget], bitness: Option<u8>) -> Result<Vec<LvTarget>>` in `target.rs`: `None` or an empty list returns the list as is; `Some(b)` keeps that bitness and fails with the pinned AC6 message when nothing matches.
**TEST**: the `@unit:target.rs` scenarios, composing `select(&of_bitness(..)?, "2026")`, asserting the full pinned AC6 message, the first-listed pick for AC5, and `select(&of_bitness(&[], Some(32))?, "2026")` giving today's `no LabVIEW installations detected`.
**REFACTOR**: give the test helper `lv(year)` a bitness.
**Files**: `crates/lvpm-core/src/target.rs`
**Commit**: `feat(s10d): narrow LabVIEW installations to one bitness`

#### Step 1.4: `--labview-bitness` reaches every pick

**Complexity**: standard
**IMPLEMENT**: global `--labview-bitness` (`PossibleValuesParser::new(["32", "64"])` mapped to `u8`, `conflicts_with = "prefix"`, help as pinned) → `DestinationFlags::labview_bitness`. `OpEnv::pickable()` returns `of_bitness(&self.targets, flags.labview_bitness)`; doc on `targets`: "detected list; choose from `pickable()`". `roots`, `headless_roots`, `venv create` (venv_cmd.rs:36), `init`'s `choose_labview` (init.rs:164, called before anything is written) and `ops::query::targets` (now `Result<Vec<_>>`; main.rs adapts) use it. `init.rs:389` passes a chosen path and keeps the full list. `tui_gets_options` refuses the flag; the tui test feeds it `64`.
**TEST**: the `@unit:main.rs` bitness parse scenarios (`ErrorKind::InvalidValue`, `exit_code() == 2`, `[possible values: 32, 64]`), `@unit:env.rs` manifest-fallback, `@unit:query.rs`, `@unit:init.rs`, `--labview-version 2026` + bitness 32 through `roots()`, and `--labview-version <path>` of a 64-bit install + bitness 32 failing with "no LabVIEW target matching".
**REFACTOR**: no pick site reads `env.targets` directly (grep).
**Files**: `src/main.rs`, `crates/lvpm-core/src/ops/env.rs`, `crates/lvpm-core/src/ops/venv_cmd.rs`, `crates/lvpm-core/src/ops/init.rs`, `crates/lvpm-core/src/ops/query.rs`
**Commit**: `feat(s10d): --labview-bitness picks between installations of one version`

#### Step 1.5: a bitness that contradicts the venv is refused

**Complexity**: standard
**IMPLEMENT**: in `OpEnv::find_venv`, after `venv::find` returns a venv, fail with the pinned AC8 message when `flags.labview_bitness` is set and differs from `v.target.bitness`.
**TEST**: the `@unit:env.rs` venv Outline (32 fails with the full pinned AC8 message; 64 and none select the venv), with a faked venv like `a_labview_version_picks_among_the_injected_installations`.
**REFACTOR**: the message builds from `label()`, not a second format.
**Files**: `crates/lvpm-core/src/ops/env.rs`
**Commit**: `feat(s10d): refuse a --labview-bitness the venv is not bound to`

#### Step 1.6: docs — the two flags, and no `lvpm update`

**Complexity**: trivial
**IMPLEMENT**: `docs/usage.md` global-flag table gains `--yes` / `-y` (`LVPM_YES`; `0`, `false`, `no`, `off` or empty is off) and `--labview-bitness`; README's `--json` note mentions `LVPM_YES`; README gains `## Upgrading`: no `lvpm update` — run `lvpm lock --upgrade`, then `lvpm sync` (uv does the same: `uv lock --upgrade`, `uv sync`). Add the `lvpm update` parse test and a README variant of the `docs.rs` section step, plus an "in it A comes before B" step.
**TEST**: the two `update` scenarios; full suite green.
**REFACTOR**: check the README anchor blocks still close.
**Files**: `README.md`, `docs/usage.md`, `src/main.rs`, `tests/steps/docs.rs`, `tests/features/build-slices/s10d-unattended.feature`
**Commit**: `docs(s10d): document --yes and --labview-bitness; why there is no lvpm update`

## AC → test trace

| AC | Tested by |
|---|---|
| AC1 | 1.1 parse tests; 1.2 cucumber (`@labview`) |
| AC2 | existing lvpm-tui slice-3 `--json` scenario; 1.2 `--json` + `LVPM_YES=1` row |
| AC3 | 1.1 parse test |
| AC4, AC5, AC6 | 1.3 `target.rs` unit tests |
| AC7 | 1.4 parse tests |
| AC8 | 1.5 `env.rs` unit tests |
| AC9 | 1.6 parse test + README cucumber step |
| pick sites (spec) | 1.4 `env.rs` / `query.rs` / `init.rs` unit tests |

## Parallelization

```mermaid
graph TD
  S1[Slice 1: unattended runs and bitness selection]
```

| Wave | Slices (parallel) |
|------|-------------------|
| 1 | 1 |

## Pre-PR Quality Gate

- [ ] `cargo test --workspace` and `cargo test --test acceptance` pass
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo fmt --check` passes
- [ ] `/code-review` passes
- [ ] README and docs/usage.md updated

## Risks & Open Questions

- **Deviation from the issue's Files line**: `ops/env.rs`, `ops/venv_cmd.rs`, `ops/init.rs`, `ops/query.rs` and test support change too. The PR labels it `deviation`.
- The `venv remove` cucumber scenarios need a real LabVIEW (`@labview`), so they run only in CI's LabVIEW container; selection logic is covered locally by unit tests.
- A shell with `LVPM_YES` exported now skips the `venv remove` prompt. Mitigated: `venv remove` already prints `removed <dir>`.
- `--labview-version <path>` with a mismatched bitness reports "no LabVIEW target matching", not a bitness message. Low priority; left as is.
- The bitness clash also fires for `venv status`/`venv remove` (shared `find_venv`), while a mismatched `--labview-version` in a venv is silently ignored. The spec chose the error; the hint says how to get out.
- `lvpm update` stays an unrecognized-subcommand error (human, 2026-11-02): no hidden stub.

## Build AC gate (2026-11-02)

spec-compliance-review flagged AC1, AC2, AC3, AC6, AC7, AC9 (missing assertions) and the AC8 hint's sequencing trap. Human chose "Tighten, then build": all folded in above; spec AC2/AC3 updated to match.

## Plan Review Summary

Plan tier: standard — reviewers: Acceptance, Design, UX (Strategic not in tier; Parallelization skipped — single-slice plan).

- **Acceptance**: needs-revision → approve after one revision. The three blockers (tui test break, unimplementable "nothing read from stdin", unpinned AC5) are fixed. Remaining: AC3 is parse-only; the AC2 message is reworded (note it in the PR).
- **Design**: approve. Narrowing at pick sites via `OpEnv::pickable()` + `target::of_bitness` is right; filtering in `op_env` would break the venv-clash message and fail non-picking commands. Folded in: keep `LVPM_YES` out of tests, and `conflicts_with = "prefix"`.
- **UX**: needs-revision (warnings only), all folded in: pinned wording with `hint:` lines, `PossibleValuesParser` so clap prints `[possible values: 32, 64]`, help strings, `LVPM_YES` named in the `--json` refusal, decided interaction order. Not taken: a hidden `lvpm update` stub, left to the human gate.

## Build Progress

### Slices (grouped by wave)

#### Wave 1
- [x] Slice 1: unattended runs and bitness selection
  - [x] Step 1.1: global `--yes` / `-y` with `LVPM_YES`
  - [x] Step 1.2: acceptance scenarios for `--yes` and `LVPM_YES`
  - [x] Step 1.3: `target::of_bitness` narrows a list
  - [x] Step 1.4: `--labview-bitness` reaches every pick
  - [x] Step 1.5: a bitness that contradicts the venv is refused
  - [x] Step 1.6: docs — the two flags, and no `lvpm update`
