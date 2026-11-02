<!-- spec-version: 1 -->
# Spec: `lvpm build` runs the check (S5b)

Issue: [ThatsTheEnd/lvpm#64](https://github.com/ThatsTheEnd/lvpm/issues/64).
Deviation from [build-slices.md](../vision/build-slices.md#slice-s5b-lvpm-build-runs-the-check):
a `[dependencies]` entry that no member links to warns instead of failing, and
`lvpm build --no-check` is added.

## Intent Description

`lvpm build` packages whatever the library lists, so today it ships a package
with a broken subVI link or a link into a package the manifest never declared.
The consumer finds out when LabVIEW loads the VI. After this change, `build`
runs `lvpm check` over the library's members first and writes nothing when a
member breaks a rule, naming the VI and the link, as `lvpm check` does.

The check also tells which packages the members link into. A link into a venv
package missing from `[dependencies]` fails the build. A `[dependencies]` entry
that no member links to is reported as a warning: the package may be loaded by
path at run time, which the linker table cannot show.

`lvpm build --no-check` skips the gate and builds as `build` does today.

## Architecture Specification

- `ops::check` gains, per checked VI, the packages its resolved links belong to
  (`Search::owner`, already there). It is returned on `Checked` as the set of
  packages used by library members; `lvpm check`'s output does not change.
- `ops::build::build(env, check: bool)`: when `check` is true, it runs
  `ops::check::check(env, Some(library folder))` before reading any member,
  keeps only findings whose VI is a library member, and returns a typed
  refusal carrying those findings when any remain. Nothing is written to
  `dist/` on refusal. `Built` gains the unused `[dependencies]` names.
- The gate needs the venv binding, as `check` does. With no venv, build fails
  through `check`'s own error, with a hint to run `lvpm sync`.
- `src/main.rs`: `Cmd::Build { no_check: bool }` (`--no-check`). A refusal
  prints the findings with `render::check`'s finding lines and exits 1. An
  unused dependency goes in the report's warning.
- Unchanged: `lvpm check`, the `.lvpkg` format, `publish`, atomic write.

## Acceptance Criteria

1. `lvpm build` on a project where a library member has a broken link exits 1,
   prints `<VI>: link not found: <link>`, and `dist/` holds no `.lvpkg`.
2. A previous `.lvpkg` in `dist/` is unchanged after a refused build.
3. A member linking into a venv package that is not in `[dependencies]` (absent
   or only in another table) refuses the build, naming the VI, link and package.
4. A non-member VI (a test next to the library) with a broken link does not
   stop the build.
5. A `[dependencies]` entry no member links to: build succeeds and stderr warns
   naming the package.
6. With no venv, `lvpm build` exits 1, writes nothing, and the message names
   `lvpm sync`.
7. `lvpm build --no-check` builds a project with a broken member link and with
   no venv, as S1's build does.
8. Every other check rule (too new, not separated, unreadable, not a resource)
   on a member refuses the build too.
9. Pilot A builds with the gate on (`@labview`-free; existing S0 scenario).
10. `docs/usage.md` documents the gate and `--no-check`.

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| member | A file the top-level library lists (and its classes list), the set `build` packages | `verified` | S1 spec, user answer |
| gate | The check `build` runs before writing | `verified` | slice S5b |
| unused dependency | A `[dependencies]` name no member's resolved link belongs to | `verified` | user answer |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| Build with no venv | `requires-stakeholder-input` | human | Require a venv; fail naming `lvpm sync`. |
| Which VIs the gate holds | `requires-stakeholder-input` | human | Library members only. |
| `[dependencies]` entry not linked | `requires-stakeholder-input` | human | Warn, still build (deviation). |
| Escape hatch | `requires-stakeholder-input` | human | Add `--no-check`. |
| `--no-check` also skips the venv requirement | `inferable` | inference | The venv is needed only by the check; skipping it restores S1 behaviour. Alternative "still need venv" has no use. |
| Links not checked (absolute, no installation) | `inferable` | inference | `lvpm check` exits 0 on them; the gate matches `check`. |
| A dev-dependency used by a member | `inferable` | inference | `check` already reports it as `Undeclared` with `only_in`; members are held to `[dependencies]`. |
| Unused-dependency warning with `--no-check` | `inferable` | inference | No links read, so nothing to report. |
| JSON output of a refusal | `inferable` | inference | `--json` follows `check`'s shape for findings plus `"command": "build"`, as other failed reports do. |
| Completeness sweep (CRUD, auth, audit) | `inferable` | inference | Read-only gate over local files; no entities created, no auth surface. Error handling covered by AC 1–3, 6. |

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion
- [x] Architecture constrains without over-engineering
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged — inferable with rationale or resolved by human
