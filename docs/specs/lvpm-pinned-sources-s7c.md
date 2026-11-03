<!-- spec-version: 14.0.1 -->
# Spec: Pinned sources (slice S7c)

**Format:** dev-team specs 14.0.1 · **Status:** gate passed; the one stakeholder answer was given by the user on 2026-11-03 · **Date:** 2026-11-03 ·
**Slice:** [#67](https://github.com/ThatsTheEnd/lvpm/issues/67), [S7c in build-slices.md](../vision/build-slices.md) ·
**Rules:** [ADR 4](../adr/0004-sources-first-wins-and-pinning.md), decision 2 and its consequences

## Intent Description

Since S13, a package name comes from the first source that lists it. That
order protects an internal package only while the internal source stays
first, and only if every developer's manifest lists it first. A public index
placed above it, or a local folder that holds a same-named copy, still
decides where the package comes from.

After this slice, a dependency line can name the source it must come from:
`hexagon_lib_motion = { version = ">=2.0", index = "internal" }`. A pinned
package comes from that source or not at all. If the source does not list
it, or cannot be reached, lvpm fails and names the package and the source.
It never falls back to another source, not even to a cache import or a
`sources.local` folder. Every name without a pin still follows first-wins.

`lvpm add <pkg> --index <name>` writes the pin. Adding, changing or removing
a pin makes the lock stale.

## Architecture Specification

**The manifest.** `index = "<name>"` joins `version` and `github` on a
table-form line in `[dependencies]`, `[dev-dependencies]` and every
`[dependency-groups.<group>]`. `<name>` is a key of `[sources]` whose value is
an index URL. `defaults`, `min-age-days` and any name `[sources]` lacks are
refused when the manifest is read, naming the line and the source.
`Project` gains `index: BTreeMap<String, String>` (name as written → source
name), filled like `github`: a name pinned in two tables must name the same
source, or the read fails. `github` and `index` on one line stay an error.

**Resolution.** The pin changes who owns a name in `first_source_wins`. For a
pinned name, the owner is the pinned source's URL, not the first source that
lists it. Every other source's entries for that name are dropped, including
`cache` imports and `sources.local` folders. Unpinned names keep S13's rule.
`load_each` takes the pins as a map from `name_key` to the source URL, so
every command that loads an index with a project agrees: `lock`, `add`,
`info`, `search` and the TUI. Callers without a project (global `install`)
pass no pins.

A pin applies to its own line only. The pinned package's dependencies
resolve by first-wins, unless they have a pinned line of their own (ADR 4:
list a transitive dependency as a direct one with `version = "*"`).

**Failures.** A pinned source that does not list the name, or lists it with
no version that fits, makes `lock` fail naming the package and the source. An
unreachable pinned source already fails the load (`load_index`, S13).

**The lock.** A new optional `[inputs.index]` table maps a name, as its line
is written, to the source name. `sources_changed` compares it with the
manifest, as it does `github`, so a pin added, changed or removed gives the
existing "sources changed" reason. No `FORMAT_VERSION` bump: the table is
absent when no line has a pin, so every existing lock keeps its bytes, and no
existing manifest can hold a pin (today `index` is refused). Each locked
package already records its source URL, so `sync` is unchanged.

**`lvpm add --index <name>`.** A new flag on `add`, valid with `--dev` and
`--group`. It writes the table form `{ version = "...", index = "<name>" }`,
or sets `index` on a line that is already a table. Without `--index`, `add`
on a table line changes only `version` and keeps `index` (ADR 4
consequences). The constraint `add` writes comes from the pinned source's
newest version. A name that does not exist in `[sources]` fails, and
`lvpm.toml` and `lvpm.lock` are restored, as for any failed `add`.

| Part | Where |
|---|---|
| `index` key, `Project::index`, the unknown-source check | `crates/lvpm-core/src/project.rs` (`dependency_table`, `parse_dependencies`) |
| Pins in the first-wins filter | `crates/lvpm-core/src/index.rs` (`load_each`, `first_source_wins`) |
| Package and source in the failure message | `crates/lvpm-core/src/solve.rs` or `ops/lock.rs`, where S13 builds the source text |
| `[inputs.index]`, staleness | `crates/lvpm-core/src/lockfile.rs` (`Inputs`, `sources_changed`) |
| Writing and keeping `index` | `crates/lvpm-core/src/manifest_edit.rs` (`set`) |
| `--index` | `crates/lvpm-core/src/ops/add_remove.rs`, `src/main.rs` (`Add`) |
| Scenarios | `tests/features/sources/s7c-pinned-sources.feature` (new), listed in `tests/acceptance.rs` `FEATURE_FILES`; stub feeds on `127.0.0.1`, as S13's scenarios use |
| Docs | docs/usage.md (`[sources]` section), docs/manifest.md (the dependency table) |

**Unchanged.** `[sources]` syntax. First-wins for unpinned names. The
`github` key. `lvpm remove` (it removes the whole line, pin included).
Global `lvpm install`. `lvpm install <name>` in a project, which is `add`
without `--index`.

**Out of scope.** `scopes` and scoped names (`@org/name`); a root table of
pins for transitive dependencies; pinning a public default (NI, JKI), which
has no name in `[sources]`; a flag that removes a pin (edit the line by
hand).

## Acceptance Criteria

Each criterion cites the issue (#67) or ADR 4. Criteria with no citation are
inferences and are in the Ambiguity Log.

1. **Pin beats a higher version.** A name pinned to the second `[sources]`
   entry resolves from it, although the first entry lists a higher version.
   *(#67 AC1; ADR 4 §2)*
2. **Pin beats local inputs.** A pinned name resolves from its source although
   a `sources.local` folder and a `cache add` import list it. *(ADR 4 §2;
   decided by the user)*
3. **Not listed.** A pinned name its source does not list makes `lvpm lock`
   fail with exit ≠ 0, naming the package and the source, although another
   source lists it. *(#67 AC2)* The same holds when the source lists the name
   with no version that fits. *(inference)*
4. **Unreachable.** A pinned source that cannot be reached makes `lvpm lock`
   fail naming the source. *(#67 AC3)*
5. **`lvpm add <pkg> --index <name>`** writes
   `<pkg> = { version = ">=<resolved>", index = "<name>" }` into the chosen
   table, with the version taken from the pinned source. The short form
   `<pkg> = ">=1.0"` still parses. *(#67 AC4)* `add` without `--index` on a
   pinned line keeps the pin. *(ADR 4 consequences)*
6. **Unknown source.** A pin naming a source that `[sources]` does not define
   fails when the manifest is read, naming the line and the source, for every
   command that reads it. `index = "defaults"` and `index = "min-age-days"`
   fail the same way. `lvpm add <pkg> --index <unknown>` fails and leaves
   `lvpm.toml` and `lvpm.lock` unchanged. *(#67 AC5; the reserved keys and
   `add` are inferences)*
7. **Pins are lock inputs.** Adding, changing or removing a pin makes
   `lvpm lock --check` fail with the "sources changed" reason. *(#67 AC6)*
8. **Transitive pin.** A package that is only a transitive dependency,
   listed as `name = { version = "*", index = "<name>" }`, resolves from that
   source. *(#67 AC7)*
9. **One pin per name.** The same name pinned to two sources in two
   dependency tables fails when the manifest is read. *(inference, as S4's
   `github`)*
10. **Every command agrees.** With the setup of AC1, `lvpm info <name>` in
    the project shows only the pinned source's versions. *(inference, as
    S13 AC8)*
11. **Nothing else changes.** Every existing scenario passes. A lock written
    before this slice is not stale because of it, and its bytes do not
    change on `lvpm lock`. *(inference)*
12. **Documented.** docs/usage.md and docs/manifest.md show the `index` key,
    the no-fallback rule, that a pin beats local inputs, and `--index`. The GitHub Pages site
    and cli reference reflect this new behaviour.
    *(inference: build-slices rules require docs for behaviour users see)*

Demo: pilot A pinned to a hosted index of its own, with a same-named,
higher-versioned decoy on a second source. *(#67)*

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| pin | `index = "<name>"` on a dependency line: that package comes only from the `[sources]` entry of that name | `verified` | ADR 4 §2 |
| pinned source | The `[sources]` entry a pin names | `verified` | ADR 4 §2 |
| source | One place lvpm reads packages from: the cache imports, one `sources.local` folder, one `[sources]` entry, or one public default | `verified` | S13 spec |
| local inputs | `cache add` imports and `sources.local` folders, which S13 puts first | `verified` | S13 spec |
| first source wins | An unpinned name is taken only from the earliest source that lists it | `verified` | ADR 4 §1 |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| Does a local input (cache import, `sources.local` folder) beat a pin | `requires-stakeholder-input` | user, 2026-11-03 | No: the pin wins and local copies are ignored. Follows ADR 4 §2 ("only from that source") and matches uv's `[tool.uv.sources]` index pins. Known cost: trying a local build of a pinned package means dropping the pin for that run. Alternative declined: local inputs first, pin picks among hosted sources only. |
| How pins enter the lock's inputs (the slice asks /specs to decide) | `inferable` | inference | A new optional `[inputs.index]` table, name → source name, compared in `sources_changed`. The same shape S4 chose for `[inputs.github]`, and ADR 4 says "S7c adds each dependency's pin". |
| Whether the lock format version goes up (the slice asks /specs to decide) | `inferable` | inference | No. The table is absent without pins, so existing locks keep their bytes; no existing manifest can hold a pin, because `index` is refused today. S4 made the same call for `[inputs.github]`. An older lvpm ignores the table, as it ignores `[inputs.github]` (no `deny_unknown_fields`). |
| Record the source name or its URL in `[inputs.index]` | `inferable` | inference | The name as written, like `github` holds the line's value. A changed URL already gives "sources changed" through `inputs.sources`; a renamed key then also does, which is harmless. |
| Stale reason for a pin change | `inferable` | inference | The existing "sources changed": a pin says where a package comes from, as `github` and the source order do. No new reason. |
| Where pins apply in the filter | `inferable` | inference | In `first_source_wins`, once, so `lock`, `add`, `info`, `search` and the TUI agree, as S13 chose. |
| A version miss on the pinned source | `inferable` | inference | Fails naming package and source, as S13 AC4 does for the first source. ADR 4 §2 forbids fallback without exception. |
| Pinned package's own dependencies | `inferable` | inference | Not pinned. ADR 4 puts a pin on one line and says to list a transitive dependency as a direct one to pin it. uv pins work the same way. |
| `index = "defaults"` / `"min-age-days"` | `inferable` | inference | Refused as unknown sources: they are `[sources]` keys but not index entries, and a pin needs an index URL. |
| Pinning a public default (NI, JKI) | `inferable` | inference | Out of scope. #67 AC5 says a pin names a source `[sources]` defines; the defaults have no name there. A user who needs it lists the URL in `[sources]`. |
| Same name pinned differently in two tables | `inferable` | inference | Error at read, as S4 does for two `github` values: one name has one source. |
| `add --index` on an existing line | `inferable` | inference | Sets or replaces `index` and keeps the other keys and the comment. ADR 4: `add` on a table line keeps the other keys. |
| `add` without `--index` on a pinned line | `inferable` | inference | Keeps the pin. ADR 4 consequences: `add` on a table line changes only `version`. |
| Removing a pin from the command line | `inferable` | inference | Out of scope; the slice asks only for writing one. Edit the line by hand. |
| `--index` on `install` in a project | `inferable` | inference | Out of scope; the slice names only `lvpm add`. |
| `--index` with `github` already on the line | `inferable` | inference | Fails as the manifest read does today ("github and index cannot be used together"), and the files are restored. |
| Scoped names in the AC wording | `inferable` | user, 2026-11-03 | The AC was reworded to `lvpm add <pkg> --index <name>`. Scoped names do not work in any built slice; no slice owns them yet. |
| CRUD sweep: pin | `inferable` | inference | Create and update: `add --index` or a hand edit. Read: every command through the manifest. Delete: hand edit or `lvpm remove` of the whole line. |
| Authentication / authorization | `inferable` | inference | Unchanged. Feeds are anonymous HTTP today; a private internal source is not in scope. |
| Audit / logging | `inferable` | inference | The lock records each package's source URL and now each pin, which is the record of where a package came from. |
| Error handling | `inferable` | inference | Every failure names the package and the source (AC3, AC4, AC6); a failed `add` restores both files, as `add` already does. |

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion (pin wins → AC1, AC2, AC8; no fallback → AC3, AC4; `add --index` → AC5; unknown source → AC6; lock input → AC7; one pin per name → AC9; every command → AC10; old locks → AC11; docs → AC12)
- [x] Architecture constrains without over-engineering (one key, one map into the existing filter, one optional lock table, one flag)
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged. The one `requires-stakeholder-input` item was answered by the user.
