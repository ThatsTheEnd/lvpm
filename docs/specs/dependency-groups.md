<!-- spec-version: 1 -->
# Spec: Dependency groups (slice S11, #75)

## Intent Description

CI needs tooling a developer machine does not, and the other way round, and
both should come from one `lvpm.lock`. A project names such sets as
dependency groups: `[dependency-groups.<name>]` tables beside
`[dev-dependencies]`, which stays and is the group `dev` (ADR 1, point 7; the
same model as uv's PEP 735 groups).

`lvpm sync` installs `[dependencies]` and `dev`, as today. `--group <name>`
adds a group; `--no-dev` leaves `dev` out. What the selected groups do not
need is not installed, and is taken out when it is. `lvpm add/remove --group
<name>` edit a group's table; `--dev` stays and means `--group dev`.

## Architecture Specification

- **Manifest (`project.rs`).** `Project` gains `groups: Vec<(String,
  Vec<(String, Constraint)>)>` in file order, parsed with the same
  `parse_dependencies` (error prefix `[dependency-groups.<name>]`) and the same
  one-name-one-repo `github` check across every table. `dev_dependencies`
  stays the field for `[dev-dependencies]`. A group whose name is `dev`
  (any case) fails: `[dependency-groups.dev] is not allowed: the group dev is
  [dev-dependencies]`. A `[dependency-groups]` value that is not a table fails
  naming the key.
- **Lockfile (`lockfile.rs`).** `FORMAT_VERSION` = 2. `LockedPackage.dev`
  becomes `groups: Vec<String>`, sorted, skipped when empty: the groups
  (`dev` included) through which alone the package is needed. Empty means
  `[dependencies]` needs it. `Inputs` gains `dependency-groups`, a map of
  group name to its `name → constraint` map. Staleness checks run per table
  (main, `dev`, each group) with the existing three rules; `StaleReason`'s
  `dev: bool` becomes the table it names. A v1 lock reads as
  `FormatOutdated`.
- **Lock (`ops/lock.rs`).** One resolution over every table, as now. A
  package's `groups` is the set of non-main tables from whose entries it is
  reachable, empty when `[dependencies]` reaches it. `constraint_updates`
  covers group tables.
- **Sync (`ops/sync.rs`).** `SyncOptions.no_dev: bool` stays; add `groups:
  Vec<String>`. A package is wanted when its `groups` is empty or meets the
  selected set (`dev` unless `no_dev`, plus `groups`). An unselected
  installed package is removed with a reason naming its groups; a dev-only one
  keeps today's `DevOnly` wording. A `--group` that the manifest does not
  define fails before anything is done, naming it.
- **Selection.** One `Selection { no_dev, groups, keep_listed }` decides
  what a sync wants. `lvpm sync` uses `keep_listed = false` (exact). `lvpm
  add/remove` sync with main + dev + the edited group and `keep_listed =
  true`: a package the new lock still lists is never removed, so ci
  tooling installed by `sync --group ci` survives a later `lvpm add x`.
- **Group names.** `[A-Za-z0-9_-]+`, checked in one place and used by the
  manifest reader and the CLI. `dev` is always a known group, with or
  without a `[dev-dependencies]` table. An unknown `--group` on sync or
  remove fails before the lock is touched and lists the defined groups.
- **JSON.** Events that carry `dev: bool` keep it and gain `group:
  Option<String>`, skipped when none: `--json` output is additive.
- **Edit (`manifest_edit.rs`, `ops/add_remove.rs`, `ops/uninstall.rs`).**
  `DependencyTable` gains `Group(String)`, header `[dependency-groups.<name>]`,
  created when missing. The "package is in another table" hint and
  `UninstalledStillListed` name the group's flag (`--group ci`).
- **CLI (`src/main.rs`, `src/render/`).** `--group <name>` (repeatable) on
  `sync` and wherever `--no-dev` is accepted; `--group <name>` on `add` and
  `remove`, conflicting with `--dev`. `--group dev` on add/remove equals
  `--dev`. `--group dev` with `--no-dev` on sync is a usage error.
- **Docs.** ADR 1 stands. Update build-slices.md S11 and the manifest.md
  note and "Named dependency groups" section to this syntax; usage.md gets
  the flags.
- **Out of scope:** `--only-group`, `--all-groups`, include-group,
  a configurable default group set, groups on a global `lvpm install`.

## Acceptance Criteria

1. `lvpm sync --group ci` installs the `ci` group's packages; `lvpm sync`
   alone does not install them and removes them when installed. (#75 AC1)
2. A package that `[dependencies]` and `ci` both need is installed by every
   `sync`, with and without `--group ci`, and with `--no-dev`. (#75 AC2)
3. Adding, removing, or changing to an excluding constraint an entry in a
   group makes `lvpm lock --check` report the lock stale, naming the entry
   and `[dependency-groups.<name>]`. (#75 AC3)
4. A manifest with `[dependency-groups.dev]` fails to read with a message
   naming `[dev-dependencies]`. (#75 AC4)
5. Every existing dev scenario and recorded CLI output still passes
   unchanged, except the lock text `dev = true` now written
   `groups = ["dev"]` and `version = 2`. (#75 AC5)
6. `lvpm add pkg --group ci` writes `pkg` under `[dependency-groups.ci]`,
   creating the table; `lvpm remove pkg --group ci` removes it; `--dev` and
   `--group dev` produce the same file. (#75 in-scope 3)
7. `lvpm lock` writes `groups = ["ci", "dev"]` for a package that only `ci`
   and `dev` reach, and no `groups` for one `[dependencies]` reaches.
8. `lvpm sync --group nope` fails naming `nope` and changes nothing.
9. A v1 `lvpm.lock` is reported stale (format outdated), the message names
   `lvpm lock`, and `lvpm lock` rewrites it as v2.
10. `lvpm add/remove` install main + dev + the edited group and remove no
    package the new lock still lists.
11. `--json` events keep `dev` and add `group` when a group is edited;
    `reason` on a removal stays a string (`groups_only`, with a separate
    `groups` list).

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| group | A named table of dependencies a project needs only sometimes; `[dev-dependencies]` is the group `dev`, the rest are `[dependency-groups.<name>]` | `verified` | ADR 1 pt 7; user, 2026-11-01 |
| main | `[dependencies]`; always installed; not a group | `verified` | ADR 1 |
| selected groups | `dev` unless `--no-dev`, plus each `--group` | `verified` | user, 2026-11-01 |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| Syntax: ADR 1 `[dependency-groups.<name>]` vs upstream `[dependencies.<name>]` (build-slices.md) | `requires-stakeholder-input` | human | ADR 1 / uv model; docs follow |
| Does sync install `dev` by default | `requires-stakeholder-input` | human | Yes, like uv and today |
| Lock field name/shape | `requires-stakeholder-input` | human | `groups = [..]`, format v2 |
| Which groups add/remove sync | `requires-stakeholder-input` | human | Default + edited + already-installed; read as "never remove a package the new lock lists" (plan review, 2026-11-01) |
| Unknown `--group` on sync | `inferable` | inference | uv errors; silently ignoring a typo would skip tooling in CI |
| Group name characters | `inferable` | inference | A dot or quote in a header makes a different TOML table; bare-key set is the safe subset |
| `--json` event shape | `inferable` | inference | Recorded golden JSON pins `dev`; additive `group` keeps it stable |
| `--group dev` with `--no-dev` | `inferable` | inference | Contradictory; usage error |
| Reserved `dev` check case-insensitive | `inferable` | inference | Package names compare case-insensitively here; one spelling for `dev` is ADR 1's point |
| Other group names compared as written | `inferable` | inference | TOML keys are case-sensitive; no evidence of a need to fold |
| Package reached by both dev and ci, not main | `inferable` | inference | Generalises today's `dev`: wanted when any of its groups is selected |
| Empty group table | `inferable` | inference | Valid, defines the group; `--group` on it is no error |
| Same package in a group and main with different constraints | `inferable` | inference | Same as dev+main today: both go to the solver |
| `lock --upgrade` rewrites group constraints | `inferable` | inference | `constraint_updates` already covers every table it knows |
| Groups on global `install` / venv-less install | `inferable` | inference | Issue scopes groups to sync/add/remove; out of scope |
| v1 lock compatibility | `inferable` | inference | Existing `FormatOutdated` path handles it |
| Completeness sweep, group entity: create (add/manifest), read (sync/lock), update (lock --upgrade), delete (remove, hand edit → stale) | `inferable` | inference | All covered by AC 1, 3, 6; auth/audit n/a (local CLI); errors AC 4, 8 |

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion
- [x] Architecture constrains without over-engineering
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged — inferable with rationale or resolved by human
