<!-- spec-version: 14.0.1 -->
# Spec: venv install resaves packages before they land in the add-on folder

**Format:** dev-team specs 14.0.1 · **Status:** gate passed; the four stakeholder answers were given by the user on 2026-10-27 · **Date:** 2026-10-27 ·
**Evidence:** [PR #55 comment](https://github.com/ThatsTheEnd/lvpm/pull/55#issuecomment-6440371424)

## Intent Description

A package built from old LabVIEW sources can be broken in a venv. The
measured case is `TypeDescriptor_SetName.vi` in
`labview_open_source_lib_data_manipulation` 1.6.0.16, last saved in LabVIEW
2013. LabVIEW 2026 reports it broken with type errors when it loads from an
add-on folder. The same bytes compile in a plain folder. After LabVIEW 2026
has saved the VI once, it compiles in the add-on folder too. Every caller is
broken with it, including the OpenSerializer VIs in pilot B's venv.

`lvpm install` into a venv therefore has the venv's LabVIEW load and save
each package in a staging folder first. Only then does it move the package
into the add-on folder. A venv then works with such packages without a
manual mass compile. If the resave fails, the package is still installed and
lvpm says which package failed, so the user knows where to look.

## Architecture Specification

**Placement ([ADR 5](../adr/0005-core-and-cli-split.md)).** All of it is in
`lvpm-core`. The CLI changes only in the wording of the new events
(`src/render`).

| Part | Where |
|---|---|
| Staged unpack, resave and move for a venv destination | `crates/lvpm-core/src/ops/install.rs` (`apply`), with the venv paths in `venv.rs` |
| The resave pass over one staged package | reuses `relink.rs` and `Relink Package.vi` (load, then save what LabVIEW marks modified) |
| Report of VIs still broken after the save | an extension of the relink VI's `report log out` JSON, read in `relink.rs` |
| New events (resave started, done, failed for a package) | `crates/lvpm-core/src/events.rs`, worded in `src/render` |

**Flow for one venv install run.**

1. Packages are handled in dependency order, dependencies first.
2. For each package, lvpm unpacks its files into
   `<venv>/.lvpm/staging/<package>/1/`. That layout mirrors
   `<venv>/<package>/1/`, so the relative links inside the package stay
   valid after the move.
3. lvpm then runs the resave pass over the staged folders. It uses the
   venv's LabVIEW over VI Server and starts that LabVIEW the way the relink
   pass does today.
4. lvpm moves the staged tree to `<venv>/<package>/1/`. It is a rename on
   the same volume.
5. The install manifest records the final paths, never the staging paths.
6. The existing relink pass, the PostInstall hooks and the palette refresh
   run afterwards, unchanged.

**Constraints**
- **Venv only.** `--global`, `--labview-version` and `--prefix` installs do
  not change.
- **Every package is staged and loaded.** lvpm does not read VI versions.
  LabVIEW decides what to save, because it marks an old-version VI modified
  when it loads it.
- **`--no-relink` and `--headless` do not skip the resave.** They skip only
  the relink pass, as today.
- **A failed resave never stops the install.** A failure is a LabVIEW error,
  a timeout, or VIs that are still not executable after the save. lvpm moves
  the package into the add-on folder anyway, as it was unpacked or as far as
  the save got. It emits a warning that names the package and, when they are
  known, the broken VIs. The install exits 0 unless something else fails.
- The staging folder must not be an add-on root that LabVIEW scans: `.lvpm/`
  has no `lvaddoninfo.json`. If staging under `.lvpm/` still loads as add-on
  context, the plan moves staging to another folder on the same volume. The
  plan measures this before it builds anything else.
- An interrupted install can leave a staging folder behind. The next install
  removes `<venv>/.lvpm/staging/` before it starts.
- Dry run: says that a resave is planned, per package, and writes nothing.
- No new dependency.

## Acceptance Criteria

1. **Resave works (`@labview`).** Install pilot B into a fresh venv on LabVIEW
   2026, with `--no-relink` as the pilot test does today. A CLI mass compile of
   `<venv>` then reports no bad VI. Today it reports `TypeDescriptor_SetName.vi`
   and its callers.
2. **Files land in place.** After a venv install, every installed file is under
   `<venv>/<package>/1/`, the manifest names only those paths, and
   `<venv>/.lvpm/staging/` is empty or gone.
3. **Venv only.** A `--prefix` install writes no staging folder and emits no
   resave event (headless test). Global installs keep their current output.
4. **Flags.** `--no-relink` skips the relink pass but not the resave. An
   install under `--headless` resaves too.
5. **Failure is a warning.** When the resave pass errors or times out, the
   package is still installed in `<venv>/<package>/1/`. lvpm prints a warning
   that names the package. The exit code is 0. This is a headless test with a
   fake VI Server failure.
6. **Broken VIs are named.** When the save leaves VIs that are not executable,
   the warning lists them, relative to the package folder.
7. **Leftover staging.** A staging folder from an interrupted run is removed
   at the start of the next venv install.
8. **Dry run.** `--dry-run` into a venv lists the resave as planned per package
   and creates no staging folder.

## Measurements

Slice 1 of `plans/lvpm-venv-resave.md`, 2026-10-27, LabVIEW 2026 (64-bit)
v26.3, `data_manipulation` 1.6.0.16 and OpenSerializer 1.2.1.16, scratch
venvs installed with `lvpm install --no-relink`. Each mass compile is
`LabVIEWCLI -OperationName MassCompile` against a venv LabVIEW started with
`lvpm launch` on the venv port (3490), and that PID is stopped afterwards.
The logs are in the session scratchpad, not in the repository.

| Row | What was done | Result | Decision |
|---|---|---|---|
| Baseline | Mass compile `<venv>` after a plain install | 21 bad-VI lines: `TypeDescriptor_SetName.vi` ×18, `Variant_InsertElementIntoCluster.vi` ×2, `Tree to KVP.vi` ×1 | The failure reproduces |
| M0 | `lvpm relink` on `<venv>/<pkg>/1/` in place, then a mass compile | The relink VI saved 6 files (the `.lvlib`/`.lvclass` files), not `TypeDescriptor_SetName.vi`. The mass compile is unchanged: 21 lines | An in-place resave does not fix it. Staging is needed |
| M1a | Mass compile the tree at `<venv>/.lvpm/staging/<pkg>/1/`, no `lvaddoninfo.json`, no add-on copy | Exit 3, the same 3 VIs broken | Not usable: it is still add-on context |
| M1b | The same at the sibling `<venv>/.staging-<pkg>/1/` | Exit 3, the same 3 VIs broken | Not usable: everything under `<venv>` loads as add-on context |
| M1c | The same at `<project>/.lv-stage-out/<pkg>/1/`, outside `<venv>` | Exit 0, no bad VI. The mass compile rewrote all 98 files | Staging must be outside `<venv>` |
| M2 | `lvpm relink` (embedded `Relink Package.vi`) on the M1c folder | Saved the same 6 files. `TypeDescriptor_SetName.vi` is not in the report | **Stop condition.** The relink VI saves only what LabVIEW marks modified, and a version-only VI is not marked |
| M3 (partial) | Move the M1c tree (mass compiled outside) to `<venv>/<pkg>/1/`, restore `lvaddoninfo.json`, mass compile `<venv>` | Exit 0, no bad VI, OpenSerializer included | A full save outside the venv, then the move, fixes the venv |
| M2b (option 1) | Fresh venv; move the tree to the sibling `<project>/.lv-venv.staging/<pkg>/1/`; over VI Server (lvpm's own client), open each `.vi`/`.ctl` and Save Instrument it in place; move it back; mass compile `<venv>` | 92 of 92 saved, 0 failed, in 84 s; the 6 library files untouched. The venv mass compile: exit 0, no bad VI | A forced save of every VI over VI Server fixes it, with no LabVIEWCLI. Chosen by the human on 2026-10-27 |
| Startup | `lvpm launch` until the port listens | 14 s cold, 6–8 s warm | — |

Other observations:
- The LabVIEW that `lvpm relink` starts can still run about 15 s after
  relink returns, with no port open. lvpm does not wait for it to exit.
- The forced save took 84 s for 92 files, mostly cold loads. The relink VI
  takes about 3 s per folder because it saves only 6 files.
- Not yet measured: the one-session vs. fresh-LabVIEW-per-package rows of
  M3, the OpenSerializer resave, and the upgrade row.

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| Old-version VI | A VI whose file was last saved by a LabVIEW older than the venv's LabVIEW | `verified` | User request ("old-version VIs") |
| Add-on folder | `<venv>/<package>/1/`, which LabVIEW loads through `LVAddons.AdditionalLocations` | `unverified` | `venv.rs`, the venv's `labview.ini` |
| Staging folder | `<venv>/.lvpm/staging/<package>/1/`: the same layout, outside any add-on root | `unverified` | This spec |
| Resave | Load a staged package in the venv's LabVIEW and save what LabVIEW marks modified | `unverified` | `relink.rs`, `Relink Package.vi` |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| Which destinations resave | `requires-stakeholder-input` | human | Venv only. The breakage was measured only in add-on folders. |
| Which packages are staged | `requires-stakeholder-input` | human | Every package. LabVIEW saves only what it upgrades on load, and lvpm reads no VI versions. |
| `--no-relink` / `--headless` | `requires-stakeholder-input` | human | The resave still runs. It is about correctness, so only the relink pass is skipped. |
| Resave failure | `requires-stakeholder-input` | human | Install anyway and warn, naming the package. |
| Mechanism | `inferable` | inference | Reuse `Relink Package.vi`. It already loads and saves in the target LabVIEW over VI Server, and `docs/ogpm-model.md` lists "mass-compile-by-load-and-save" as done through it. Predictability check: the alternative is LabVIEWCLI MassCompile. It is ruled out because lvpm drives LabVIEW only over VI Server. |
| Does the relink VI save a VI that is modified only by a version upgrade | `inferable` | inference | `docs/ci-docker.md` says unrelinked VIs "show as modified and want saving", and the VI saves modified VIs. The plan verifies this first in LabVIEW (AC1). |
| Order across packages | `inferable` | inference | Dependencies come first, so a dependent loads already-resaved dependencies from their add-on folders. Loading against a not-yet-resaved dependency would break the dependent during its own save. |
| Staging location | `inferable` | inference | Under `<venv>/.lvpm/` on the same volume, so the move is a rename. The layout is mirrored so in-package relative links hold. It must not be scanned as an add-on, which the plan measures first. |
| Links to dependencies saved from staging | `inferable` | inference | They can be stored relative to the staging path. The relink pass after the move repairs them. With `--no-relink` they point into stable venv paths. The plan checks this in AC1. |
| Detecting broken VIs | `inferable` | inference | Follows from the user's failure answer, which covers "a VI stays broken". The relink VI's JSON report gains the not-executable list. |
| Existing venvs that are already broken | `inferable` | inference | Out of scope. Reinstalling, for example by deleting `.lv-venv` and running `lvpm sync`, runs the new path. The user docs say so. |
| Uninstall | `inferable` | inference | Unchanged. The manifest holds final paths only (AC2). |
| Global vi.lib has the same breakage | `inferable` | inference | Unmeasured, and out of scope by the user's answer on destinations. |

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion (resave → AC1; placement → AC2; venv only → AC3; flags → AC4; failure → AC5, AC6; robustness → AC7; dry run → AC8)
- [x] Architecture constrains without over-engineering (reuses the relink VI and its pass; no version parsing; no new dependency)
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged — inferable with rationale or resolved by human
