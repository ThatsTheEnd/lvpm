<!-- spec-version: 1 -->
# Spec: --yes, --labview-bitness, and no `lvpm update` (s10d)

Issue: [ThatsTheEnd/lvpm#74](https://github.com/ThatsTheEnd/lvpm/issues/74)

## Intent Description

lvpm must run unattended and pick the LabVIEW the user means. A CI job runs
`lvpm sync --yes` (or sets `LVPM_YES=1`) and never blocks on a prompt. A machine
with LabVIEW 2026 32-bit and 64-bit installed side by side lets the user choose
with `--labview-bitness 32|64`. A bitness that contradicts the venv's binding is
an error, not a silent switch.

`lvpm update` is not added. Like uv (`uv lock --upgrade`, then `uv sync`),
lvpm keeps upgrading in `lock`. The README says so and gives the two commands.

## Architecture Specification

- **`--yes` / `-y`**: a global flag on `Cli` in `src/main.rs`, also set by
  `LVPM_YES` through clap's `env` feature. clap reads `0`, `false`, `no`, `off`
  and the empty string as false. It replaces `VenvCmd::Remove`'s own `yes`, so
  `lvpm venv remove -y` keeps parsing. `venv remove` is the only prompt
  (`confirm_removal`). The `--json` refusal stays: without `--yes`, `venv
  remove` fails under `--json`.
- **`--labview-bitness <32|64>`**: a global flag. It goes into
  `DestinationFlags::labview_bitness: Option<u8>`. A new function in
  `crates/lvpm-core/src/target.rs` narrows a target list to one bitness. It
  fails when installations exist but none has that bitness. Every place that
  *picks* an installation uses the narrowed list: `OpEnv::roots`,
  `OpEnv::headless_roots`, `venv create`, `init` and `lvpm targets`.
  Venv lookup still reads the full list. If the venv found is bound to another
  bitness, `OpEnv::find_venv` fails and names both.
- `lvpm tui` refuses `--labview-bitness` like the other destination flags. It
  ignores `--yes`, which answers prompts the TUI does not have.
- No change to resolution, `lock` or `sync`.
- Docs: `docs/usage.md` global-flag table, and the README's `--json` note and a
  short "no `lvpm update`" note.

## Acceptance Criteria

1. `lvpm --yes venv remove`, `lvpm venv remove -y` and `LVPM_YES=1 lvpm venv
   remove` delete the venv without reading stdin. With `LVPM_YES=0` it asks.
2. Under `--json`, `venv remove` without `--yes` or `LVPM_YES` still fails.
   Its message now names both `--yes` and `LVPM_YES`. With either one, it deletes.
3. `lvpm sync --yes` parses. `sync` has no prompt, so the flag changes nothing.
4. With a faked list holding LabVIEW 2026 32-bit and 64-bit, bitness 32 picks
   the 32-bit one for `2026`, and bitness 64 picks the 64-bit one. A unit test
   in `target.rs` covers this.
5. With no `--labview-bitness`, selection is unchanged.
6. `--labview-bitness 32` with only 64-bit installations fails. The message
   says no 32-bit LabVIEW was found and lists what was found.
7. `--labview-bitness` with a value other than 32 or 64 is a usage error
   (exit 2).
8. Inside a venv bound to a 64-bit LabVIEW, `--labview-bitness 32` fails and
   names the venv's binding. A matching bitness, or no flag, works as today.
9. `lvpm update` does not exist. The README says why and gives `lvpm lock
   --upgrade` followed by `lvpm sync`.

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| bitness | 32 or 64, the `LvTarget::bitness` of a detected installation | `verified` | `target.rs`; user answered the venv-clash question in those terms |
| prompt | a question lvpm reads an answer to from stdin | `verified` | only `confirm_removal` in `src/main.rs` today |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| `lvpm update`: alias of `lock --upgrade` + `sync`, or left out | `requires-stakeholder-input` | human | Left out, with a README note. Matches uv, which has no `update` command. |
| Env var for `--yes` | `requires-stakeholder-input` | human | `LVPM_YES`, through clap `env`. Matches uv's `UV_*` env vars. |
| `--labview-bitness` against a venv bound to another bitness | `requires-stakeholder-input` | human | Error. Stricter than uv, which recreates the venv. |
| Which prompts `--yes` answers | `inferable` | inference | `venv remove` is the only stdin read. The `uninstall --all` prompt in docs/cli.md §9 is proposed and not built. Out of scope. |
| `--yes` on `lvpm tui` | `inferable` | inference | Not refused: it has no destination meaning, and `LVPM_YES` in a shell must not break `lvpm tui`. |
| Default with no `--labview-bitness` | `inferable` | inference | Unchanged (AC 5). Out of scope per the issue: "any change to resolution". |
| Does `lvpm targets` honour the flag | `inferable` | inference | Yes. It lists what lvpm can pick from, and the same narrowing applies. |
| Short `-y` as a global flag | `inferable` | inference | No other `-y` exists, and global flags reach subcommands, so `venv remove -y` keeps working. |

Completeness sweep: entities are the flags and an env var, so there is no
create/read/update/delete. Error handling is covered by AC 2, 6, 7 and 8.
There is no auth or audit surface.

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion
- [x] Architecture constrains without over-engineering
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged — inferable with rationale or resolved by human
