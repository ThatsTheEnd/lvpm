<!-- spec-version: 14.0.1 -->
# Spec: Install hooks off by default (slice S12)

**Format:** dev-team specs 14.0.1 · **Status:** approved by the user on 2026-11-03; the three stakeholder answers were given the same day · **Date:** 2026-11-03 ·
**Slice:** [#76](https://github.com/ThatsTheEnd/lvpm/issues/76), [S12 in build-slices.md](../vision/build-slices.md) ·
**Rules:** [ADR 3](../adr/0003-install-hooks-off-by-default.md); [ADR 4](../adr/0004-sources-first-wins-and-pinning.md), decision 5

## Intent Description

A `.vip` can declare hook VIs that run inside LabVIEW, with the user's
rights, when the package is installed or uninstalled. Today a global install
on a developer machine runs them without asking. That is npm's default, and
the route of several supply-chain attacks.

After this slice, installing a package runs none of its code unless that
package is allowed. In a project, `hooks = "run"` on the package's line in
`lvpm.toml` allows it. For a global install without a project, the user
config key `hooks.run` lists the allowed packages. `--hooks` allows every
hook for one command. A skipped hook is never silent: lvpm prints a warning
that names the package and the hook, and gives the command that runs it now
and the line that allows it on every install. The install still succeeds.

A package whose install hook did not run does not run its uninstall hooks
either. Venvs still never run hooks. Headless machines still skip every hook
unless `--hooks` is given, allowed or not.

## Architecture Specification

**Who decides.** One rule, `HookPolicy` in `ops/install.rs`, now per package:

| Where | A package's hooks run when |
|---|---|
| venv | never (unchanged) |
| headless (`LV_RTE_HEADLESS`) | `--hooks` only (unchanged) |
| LabVIEW installation, from a project (`sync --global`, `add`/`remove` with `--global`) | `--hooks`, or the root `lvpm.toml` has `hooks = "run"` on a line for that name |
| LabVIEW installation, no project (`lvpm install`, `install --global`) | `--hooks`, or the name is in the user config `hooks.run` |

Names match as `source_pin_of` matches them (any case). The allowlist covers
the package's PreInstall and PostInstall hooks; uninstall hooks follow the
record (below). A dependency's own `lvpm.toml` is never read for hooks: only
the root project's `Project` is passed in.

**The manifest.** `hooks = "run"` joins `version`, `github` and `index` on a
table-form line, in every dependency table. Any other value is refused when
the manifest is read, naming the line. `Project` gains
`hooks_allowed: BTreeSet<String>` (names as written), filled like
`source_pins`. `hooks` is not a lock input: it does not change what resolves.
`manifest_edit::set` already keeps keys other than `version` and `index`, so
`lvpm add pkg@<newer>` keeps `hooks = "run"`; a test proves it.

**The user config.** `config.rs` gains `[hooks] run = [..]`, read through
`LVPM_CONFIG_HOOKS_RUN` (comma- or path-list separated like `sources.local`)
or the file, and written by `lvpm config set hooks.run <name>...`, which
replaces the list, as `sources.local` does. `config get hooks.run` lists it.

**The warning.** A new event, `HooksSkipped { package, hooks, reason, run_now, dry_run, allow }`,
emitted once per package after the install or sync's summary (and before an
error, so a later failure never hides it), in place of the current
`HooksNotRun` line and the headless hooks note on a LabVIEW installation (the
venv keeps `HooksNotRun`). `reason` is `NotAllowed`, `Headless` or
`AlreadyInstalled`. `allow` is data the renderer turns into text: add
`hooks = "run"` to the package's existing line (any `github`/`index` key
stays), a new `name = { version = "*", hooks = "run" }` line for a transitive
dependency, `lvpm config set hooks.run <existing names> <names>` globally, or
`--hooks` headless. Rendered:

```
warning: pilot_a: skipped install hook PostInstall (hooks are off by default; see docs/labview-integration.md#hooks)
  run it now:     lvpm run-hooks pilot_a
  always run it:  add hooks = "run" to the pilot_a line in [dependencies] of lvpm.toml
```

Globally, the per-package warnings have the first two lines, and one line
after the last of them reads `always run them:  lvpm config set hooks.run
<existing names> <every skipped name>`, so pasting it once drops nobody.
Headless, the parenthesis reads `(LabVIEW runs headless; hooks = "run" and
hooks.run do not apply)`, there is no run-it-now line, and the last line is
`always run it:  add --hooks to this command`. A package that declared only a
PreInstall hook gets no `run it now:` line (a PreInstall cannot run once the
files are in place) but a line saying to uninstall, allow it, and install
again. A dry run says `would skip` and has no run-it-now line. A package
already installed at the same version with a skipped PostInstall, and now
allowed, is not re-run: the warning says its hook was skipped when it was
installed and gives `run it now:`. The allowlist applies from the next install
or upgrade. Warnings go to stderr, as other warnings do; `--json` carries the
event's fields. Exit code stays 0.

**The record.** The install record already lists `skipped_hooks`. On a
LabVIEW installation a skipped package now gets every declared hook in it,
as a venv install does. `lvpm run-hooks <pkg>` that succeeds removes every
install and uninstall entry, PreInstall included, so the package then counts
as hooked. A PreInstall-only package has nothing for `run-hooks` to run and
stays unhooked until it is reinstalled allowed. `lvpm list` already shows the
list.

**Uninstall.** `uninstall_one` runs PreUninstall and PostUninstall only when
the record's `skipped_hooks` holds neither PreInstall nor PostInstall. A
record from before this slice has none listed for a global install, so its
hooks run as before. Headless records, and records whose PostInstall failed,
already list their install hooks, so their uninstall hooks stop running (ADR 3 §4). A skipped uninstall hook is reported with a new
`UnrunReason::InstallHookSkipped`.

| Part | Where |
|---|---|
| `hooks` key, `Project::hooks_allowed` | `crates/lvpm-core/src/project.rs` (`dependency_table`, `LineKeys`) |
| `hooks.run` key | `crates/lvpm-core/src/config.rs` |
| Per-package policy, `HooksSkipped`, the `allow` text | `crates/lvpm-core/src/ops/install.rs`, `ops/sync.rs`, `events.rs` |
| `run-hooks` clears the skip | `crates/lvpm-core/src/ops/hooks.rs`, `install.rs` |
| Uninstall follows the record | `crates/lvpm-core/src/ops/uninstall.rs` |
| Rendering | `src/render/install.rs`, `src/render/uninstall.rs`, `src/render/hooks.rs`, `src/render/mod.rs` |
| Wiring the allowlist into the env | `src/main.rs` |
| `add` keeps `hooks` | `crates/lvpm-core/src/manifest_edit.rs` (test only) |
| Scenarios | `tests/features/hooks/s12-install-hooks.feature` (new), listed in `tests/acceptance.rs` `FEATURE_FILES`; a fake installation with a hook-declaring package, as `ops::testing` builds |
| Docs | README.md (hooks line), docs/usage.md, docs/manifest.md, docs/labview-integration.md, docs/cli.md |

**Unchanged.** Venvs. Headless without `--hooks`. `--hooks`. `run-hooks`
arguments. The lock. Packages lvpm builds (ADR 2: no hooks).

**Out of scope.** Running hooks in a venv; pinning each allowed hook VI's
hash in `lvpm.lock` (ADR 3); a CLI flag that writes `hooks = "run"` (edit the
line); a root table that allows a transitive name without listing it (ADR 4);
an allowlist that applies headless.

## Acceptance Criteria

Each criterion cites the issue (#76) or an ADR. Criteria marked
*(inference)* are in the Ambiguity Log.

1. **Off by default.** A global install, on a non-headless machine, of a
   package with a `PostInstall` hook does not run it, prints the warning, and
   exits 0. *(#76 AC1; ADR 3 §1, §2)* The same holds for PreInstall.
   *(inference)*
2. **The warning.** It names the package and the hook and shows
   `lvpm run-hooks <pkg>` and the allowlist line, in the wording of the
   architecture section. *(#76 AC2)* Following either runs the hook
   *(manual: needs LabVIEW)*. *(#76 AC2)*
3. **Project allowlist.** With `hooks = "run"` on the package's line, the
   next `lvpm sync --global` runs its hooks and prints no warning for it.
   *(#76 AC3; ADR 4 §5)*
4. **Global allowlist.** With the name in `hooks.run`, the next global
   install runs its hooks and prints no warning. *(#76 AC3; ADR 3 §3)*
   `lvpm config set hooks.run a b` / `config get hooks.run` write and read
   the list. *(inference)*
5. **`--hooks`** runs every hook of that one command, allowlisted or not.
   *(#76 AC4)*
6. **Uninstall follows install.** A package whose install hook was skipped
   runs no uninstall hook, and the uninstall says so. *(#76 AC5; ADR 3 §4)*
   After a successful `lvpm run-hooks <pkg>`, its uninstall hooks run.
   *(inference)*
7. **`add` keeps the allow.** `lvpm add pkg@<newer>` on a line with
   `hooks = "run"` keeps `hooks = "run"`. *(#76 AC7)*
8. **Only the root counts.** `hooks = "run"` in a dependency's own
   `lvpm.toml` allows nothing. *(#76 AC8; ADR 3 §3)*
9. **Transitive allow.** A transitive dependency listed as
   `name = { version = "*", hooks = "run" }` runs its hook. *(#76 AC9;
   ADR 4 consequences)*
10. **Bad value.** `hooks = "<anything but run>"` fails when the manifest is
    read, naming the line. *(inference)*
11. **Headless unchanged.** Headless, an allowlisted package's hooks are
    skipped unless `--hooks`, and the warning says `(LabVIEW runs headless)`.
    *(decided by the user)*
12. **Nothing else changes.** Venv installs run no hook and report as today.
    Every existing scenario passes, apart from those that expected a global
    install to run hooks by default, which now expect the warning.
    *(inference)*
13. **Documented.** The README hooks line states the new default. *(#76 AC6)*
    docs/usage.md, docs/manifest.md, docs/labview-integration.md and the CLI
    reference show `hooks = "run"`, `hooks.run` and the warning.
    *(inference: build-slices rules require docs for behaviour users see)*

Demo: install pilot A globally; lvpm skips its `PostInstall` and says how to
allow it. *(#76)*

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| hook | A VI a `.vip` declares under `[Script VIs]`: PreInstall, PostInstall, PreUninstall, PostUninstall | `verified` | ADR 3 |
| install hook | PreInstall or PostInstall | `verified` | ADR 3 §4 |
| allowlist | The names whose hooks run: `hooks = "run"` lines in the root `lvpm.toml`, or the user config `hooks.run` without a project | `verified` | ADR 3 §3, ADR 4 §5, user 2026-11-03 |
| global install | An install into the LabVIEW installation, not a venv or `--prefix` | `verified` | ADR 3 |
| skipped hook | A declared hook lvpm did not run; listed in the install record's `skipped_hooks` | `verified` | install record |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| The user-config key (the slice asks /specs to decide) | `requires-stakeholder-input` | user, 2026-11-03 | `hooks.run`, a list: `[hooks] run = [..]`, `lvpm config set hooks.run ..`, `LVPM_CONFIG_HOOKS_RUN`. Mirrors `hooks = "run"`. Declined: `hooks.allow`. uv has no install hooks; pnpm's `onlyBuiltDependencies` is the nearest precedent. |
| The warning's wording (the slice asks /specs to decide) | `requires-stakeholder-input` | user, 2026-11-03 | Grouped: `warning: <pkg>: skipped install hook <Hook> (hooks are off by default)`, then `run it now:` and `always run it:` lines. Declined: one line with an inline hint. |
| Does an allowlisted hook run headless | `requires-stakeholder-input` | user, 2026-11-03 | No. Headless skips every hook unless `--hooks`, as today; the allowlist acts on developer machines only. Declined: one rule everywhere. Cost accepted: CI and dev machines can differ. |
| Headless warning text | `inferable` | inference | Same shape; the reason reads `(LabVIEW runs headless)` and the fix is `pass --hooks`, since the allowlist does not help there (user's answer above). |
| Which allowlist a project command uses | `inferable` | inference | Commands that act on a project (`sync`, `add`, `remove`) use the root `lvpm.toml`; `lvpm install` without a project, or `install --global`, uses `hooks.run`. ADR 3 §3 assigns each to one; usage.md says `install --global` is a global install. |
| Does the allow cover PreInstall too | `inferable` | inference | Yes. ADR 3 allows hooks "per package"; a PostInstall that runs without its PreInstall breaks DQMH's marker contract. |
| `run it now:` for a PreInstall-only package | `inferable` | inference | Left out. `run-hooks` runs PostInstall only, and a PreInstall cannot run after the files are installed. The allowlist line still shows. |
| Allowlist line for a transitive dependency | `inferable` | inference | `name = { version = "*", hooks = "run" }`, the form ADR 4 names for a transitive pin or allow. |
| Global allowlist command when the list has names | `inferable` | inference | `config set` replaces the list, so the hint lists the existing names plus the new one; pasting it never drops an allowed package. |
| Values of `hooks` other than `run` | `inferable` | inference | Refused at read: the line format refuses unknown keys today, and a typo must not silently mean "skip". |
| Is `hooks` a lock input | `inferable` | inference | No. It does not change what resolves or where from; `lock --check` compares resolution inputs only. |
| What counts as "the install hook ran" for uninstall | `inferable` | inference | Neither PreInstall nor PostInstall is in `skipped_hooks`. A failed PostInstall is already recorded there, so its uninstall hooks do not run either; `run-hooks` success clears every install and uninstall entry, PreInstall included (amended at plan review: clearing only PostInstall left two-hook packages blocked). |
| Records written before S12 | `inferable` | inference | A global record lists no install hook as skipped, because those installs ran them; its uninstall hooks run as before. |
| Venv reporting | `inferable` | inference | Unchanged `HooksNotRun` line: nothing can allow a hook there (ADR 3 §1), so the remedies would mislead. |
| Dry run | `inferable` | inference | Prints the same warning for hooks it would skip: the reader learns before installing. |
| Where the warning goes | `inferable` | inference | After the install summary, once per package, to stderr like other warnings; `--json` gets `HooksSkipped`. |
| A CLI flag to write `hooks = "run"` | `inferable` | inference | Out of scope; the issue asks only that `add` keeps it. Edit the line. |
| CRUD sweep: allowlist entry | `inferable` | inference | Create/update: edit `lvpm.toml`, or `config set hooks.run`. Read: every install/sync; `config get hooks.run`. Delete: edit the line, or `config set` without the name. |
| CRUD sweep: skipped-hook record | `inferable` | inference | Created by an install that skips; read by `list` and `uninstall`; cleared by a successful `run-hooks`; deleted with the record on uninstall. |
| Authentication / authorization | `inferable` | inference | The allowlist is the authorization. Only the root manifest or the user's own config grants it (ADR 3 §3); a dependency cannot. |
| Audit / logging | `inferable` | inference | Every skip is printed and recorded in `skipped_hooks`, shown by `lvpm list`. |
| Error handling | `inferable` | inference | A skip is a warning, exit 0 (ADR 3 §2). A bad `hooks` value fails the manifest read. A hook that fails stays reported as today. |

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion (off by default → AC1; warning → AC2; project and global allowlists → AC3, AC4, AC8, AC9; `--hooks` → AC5; uninstall → AC6; `add` keeps → AC7; bad value → AC10; headless → AC11; venv and old behaviour → AC12; docs → AC13)
- [x] Architecture constrains without over-engineering (one manifest key, one config key, one event, one policy rule)
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged. The three `requires-stakeholder-input` items were answered by the user.
