<!-- spec-version: 14.0.1 -->
# Spec: Application builds and SBOM (slice S9)

**Format:** dev-team specs 14.0.1 · **Status:** approved by the user on 2026-11-04 · **Date:** 2026-11-04 ·
**Slice:** [#70](https://github.com/ThatsTheEnd/lvpm/issues/70), [S9 in build-slices.md](../vision/build-slices.md#slice-s9-application-builds-and-sbom) ·
**Rules:** [ADR 2](../adr/0002-vip-first-lvpkg-later.md) (the SBOM is a CycloneDX file); [ADR 6](../adr/0006-lvpkg-only-vip-read-only.md) (the SBOM inside a `.lvpkg` moved to S9)

## Intent Description

`lvpm build` packages a library as a `.lvpkg` today. It cannot build an
application: a developer opens LabVIEW and runs the `.lvproj` build spec by
hand, against whatever is installed globally, and nothing records which
packages went into the build.

After this slice, an application project names its `.lvproj` and build spec
in `lvpm.toml`. `lvpm build` starts the venv's own LabVIEW, runs that build
spec in it, and writes a CycloneDX SBOM next to the build. The build sees
exactly the packages `lvpm sync` put in the venv, and the SBOM lists the
locked packages the application ships with, each with its version and
SHA-256. The same SBOM goes inside every `.lvpkg` that `lvpm build` writes.

The pilot application is NI's "Continuous Measurement and Logging" template
from LabVIEW 2026, copied into this repository and changed to call both pilot
libraries. It needs no hardware, so the build can run on any machine with
LabVIEW 2026.

## Architecture Specification

**The manifest.** A new table, read into `Project` as `application: Option<Application>`:

```toml
[application]
project    = "app/Continuous Measurement.lvproj"  # relative to lvpm.toml
build-spec = "Continuous Measurement and Logging Application"
```

Both keys are required strings. `project` must name an existing `.lvproj`
inside the project folder. Any other key is refused when the manifest is
read, naming the key. The build spec runs on the `My Computer` target; other
targets are out of scope. `[application]` is not a lock input.

**The command.** `lvpm build` gains `--app` and `--package`:

| Flags | What is built |
|---|---|
| none | every kind the manifest declares: the `.lvpkg` (from `[distribution]`), then the application (from `[application]`) |
| `--package` | the `.lvpkg` only |
| `--app` | the application only |
| both | same as none |

A flag for a table the manifest lacks fails, naming the missing table. A
manifest with neither table fails as `build` fails today without
`[distribution]`, and the message names both tables. `--no-check` keeps
applying to the `.lvpkg` only. When both kinds are built and the `.lvpkg`
fails, the application is not built.

**Before LabVIEW starts.** `ops/build.rs` refuses the application build,
naming the fix, when:

1. there is no `lvpm.lock` → `run lvpm lock`;
2. `Lockfile::staleness` reports the lock stale → `run lvpm lock`;
3. there is no venv, or the venv's installed packages differ from the lock's
   `[dependencies]` closure → `run lvpm sync`.

**The build.** `ops/build.rs` starts the venv's LabVIEW as
`ops/resave.rs` does (`launch::ensure_instance`, then a VI Server
`Connection`), behind the same kind of test seam (a `Launcher` trait the
tests replace). Over the connection it opens NI's
`<vi.lib>\AppBuilder\BuildTargetBuildSpecification.vi`, sets the project
path, the target `My Computer` and the build spec name, runs it, and reads
its error out and its output paths. A LabVIEW error fails the build with
LabVIEW's code and message. The instance lvpm spawned is closed afterwards,
on success and on failure. Headless (`LV_RTE_HEADLESS`) starts LabVIEW
headless, as every other venv command does.

**The SBOM.** A new module `crates/lvpm-core/src/sbom.rs` turns a
`Lockfile` into CycloneDX 1.5 JSON:

- `bomFormat`, `specVersion: "1.5"`, `version: 1`;
- `metadata.component`: the project itself, `type` `application` for an
  application and `library` for a `.lvpkg`, with the project's name and
  version;
- one `components` entry per package in the `[dependencies]` closure, sorted
  by name: `type: "library"`, `name`, `version`, `bom-ref` `<name>@<version>`,
  `hashes: [{alg: "SHA-256", content: <lock sha256>}]`, and an
  `externalReferences` entry of type `distribution` holding the lock's
  `source`;
- `dependencies`: each component's `requires`, by `bom-ref`.

Packages only dev or another group needs are left out. The output is
deterministic: no `serialNumber`, no timestamp, keys and arrays in a fixed
order, so the same lock gives the same bytes. The file is named
`bom.cdx.json`.

**Where the SBOM goes.**

- Application: written into the folder of the build spec's first output
  path, after LabVIEW reports success. A failed build writes no SBOM.
- `.lvpkg`: added at the archive's root as `bom.cdx.json` and listed in
  `RECORD` like every other entry, and `package.toml` says `format = 2`. A
  library with no `[dependencies]` gets an SBOM with no components, whatever
  the lock. A library with `[dependencies]` and no lock, a stale lock or an
  unreadable lock is still built, without an SBOM and with a warning on
  stderr (A1). `lvpkg`'s reader accepts the entry only under format 2, so
  packages without one still install.

**Events.** A new `Event::AppBuilt { outputs, sbom, took }`, rendered as
`built <build-spec> in <s>s: <first output>` and `sbom: <path>` on stdout,
and as JSON under `--json`. A failed LabVIEW build is an error with exit
code 1 and the message `LabVIEW could not build "<build-spec>": <code> <message>`.

**The pilot application.** `tests/fixtures/pilot-app/` holds a copy of
`<LabVIEW 2026>\ProjectTemplates\Source\Core\Continuous Measurement`, with:

- the build spec's destination changed to `builds/` inside the fixture, so a
  build never writes outside it;
- an `lvpm.toml` with `[application]`, both pilots in `[dependencies]`, and
  `labview = "2026"`;
- one call into each pilot: HSE Logger logs when acquisition starts and
  stops; OpenSerializer writes the settings to JSON when they are saved;
- a `README.md` naming the template, the LabVIEW version it came from, NI as
  its author, and what lvpm changed.

`builds/` is git-ignored.

**Docs.** `docs/manifest.md` documents `[application]`. `docs/usage.md`
documents `lvpm build --app` / `--package` and the SBOM. The README M2
milestone "`lvpm build`: `source` kind, `.lvpkg` format, SBOM" is ticked.

**Out of scope.** PPL packages (the `ppl` kind), targets other than
`My Computer`, installers' prerequisites, listing `.lvlibp` or other
checked-in binaries in the SBOM, signing the SBOM, the TUI.

## Acceptance Criteria

Each criterion cites the issue (#70), an ADR or a user answer. Criteria
marked *(inference)* are in the Ambiguity Log.

1. **App builds headless from its venv.** With the pilot application synced
   into its venv, `lvpm build --app` headless builds `Main.exe`, which
   starts. *(#70 AC1; manual, `@labview`: needs LabVIEW 2026)*
2. **SBOM contents.** The SBOM lists every package in the lock's
   `[dependencies]` closure with its version and SHA-256, and no other
   package. *(#70 AC2; user: [dependencies] only)*
3. **SBOM is valid.** It parses as CycloneDX 1.5 JSON with the fields of the
   architecture section; two runs over the same lock give identical bytes.
   *(user: CycloneDX 1.5 JSON; inference: deterministic)*
4. **SBOM next to the build.** After a successful app build, `bom.cdx.json`
   is in the folder of the first output path. A failed build writes none.
   *(#70 "shipped next to the build"; inference: placement)*
5. **SBOM inside the `.lvpkg`.** A `.lvpkg` `lvpm build` writes from a
   current lock, or from a manifest with no `[dependencies]` whatever the
   lock, has `bom.cdx.json` at its root, listed in `RECORD`, and says
   `format = 2`. With no lock, a stale lock or an unreadable lock it has none
   and the build warns (A1). A `.lvpkg` without one still installs.
   *(#70 scope; ADR 6; amended by A1 and A2 in [the plan](../../plans/lvpm-app-build-s9.md#spec-amendments-shown-at-this-gate))*
6. **Flags.** With both tables, `lvpm build` builds both, `--package` only
   the `.lvpkg`, `--app` only the application. A flag for a missing table
   fails, naming it. *(user, 2026-11-04)*
7. **Refusals before LabVIEW.** No lock, a stale lock, or a venv that differs
   from the lock each fail before LabVIEW starts, naming `lvpm lock` or
   `lvpm sync`. *(inference)*
8. **LabVIEW failure.** A build spec LabVIEW cannot build fails with exit
   code 1 and LabVIEW's code and message, and the spawned LabVIEW is closed.
   *(inference)*
9. **Bad manifest.** An unknown key in `[application]` fails when any command
   reads `lvpm.toml`. A missing or empty key, a wrong type, a missing
   `.lvproj`, a path not ending in `.lvproj` or a path outside the folder
   fails `lvpm build` before LabVIEW starts. Each names the problem.
   *(inference; amended by A4 in [the plan](../../plans/lvpm-app-build-s9.md#spec-amendments-shown-at-this-gate))*
10. **Nothing else changes.** Every existing scenario passes. `lvpm build` on
    a library-only manifest behaves as today, plus the SBOM entry.
    *(inference)*
11. **Pilot app in the repo.** `tests/fixtures/pilot-app/` holds the copied
    template, its README names NI's template and LabVIEW 2026, and it calls
    both pilots. *(user, 2026-11-04)*
12. **Documented.** docs/manifest.md and docs/usage.md describe
    `[application]`, the flags and the SBOM; the README M2 SBOM milestone is
    ticked. *(#70 scope)*

Demo: `lvpm sync` then `lvpm build --app` in `tests/fixtures/pilot-app`;
`Main.exe` and `bom.cdx.json` appear in `builds/`. The slice's original demo
("in the container") is replaced: see the Ambiguity Log.

## Glossary

| Term | Definition | Status | Source |
|------|------------|--------|--------|
| application | A project whose `lvpm.toml` has `[application]`; built by a `.lvproj` build spec, not packaged | `verified` | user 2026-11-04 |
| build spec | A build specification inside a `.lvproj`, named as LabVIEW names it | `verified` | LabVIEW |
| SBOM | A CycloneDX 1.5 JSON file, `bom.cdx.json`, listing the locked packages a build ships | `verified` | user 2026-11-04 |
| `[dependencies]` closure | The packages `[dependencies]` names, plus everything they require, as locked | `verified` | user 2026-11-04 (SBOM scope) |
| pilot application | The copied Continuous Measurement and Logging template in `tests/fixtures/pilot-app/` | `verified` | user 2026-11-04 |

## Ambiguity Log

| Decision | Classification | Resolved By | Rationale / Answer |
|----------|---------------|-------------|-------------------|
| SBOM format (the slice asks /specs to decide) | `requires-stakeholder-input` | user, 2026-11-04 | CycloneDX 1.5 JSON. Matches `uv export --format cyclonedx1.5`. Declined: 1.6 JSON, XML. |
| Where the pilot application comes from (the slice asks /specs to decide) | `requires-stakeholder-input` | user, 2026-11-04 | NI's Continuous Measurement and Logging template from LabVIEW 2026, copied into the repo and changed to use both pilots. First answer was `Zuehlke/ze_recorder`; dropped by the user because it needs LabVIEW 2025 32-bit, NI-DAQmx and two `.lvlibp` files. Declined: Actor Framework template, Feedback Evaporative Cooler. |
| Copying NI code into a public repo | `requires-stakeholder-input` | user, 2026-11-04 | Accepted, with a README naming the source. Declined: generating the project from the installed template at test time. |
| How the project names the build spec | `requires-stakeholder-input` | user, 2026-11-04 | `[application] project`, `build-spec` in `lvpm.toml`. Declined: CLI flags only; manifest plus CLI override. |
| How LabVIEW runs the build spec | `requires-stakeholder-input` | user, 2026-11-04 | lvpm's VI Server client in the venv's LabVIEW, running NI's `BuildTargetBuildSpecification.vi` (present in `vi.lib\AppBuilder` of LabVIEW 2026). Declined: LabVIEWCLI. |
| Which lock entries the SBOM lists | `requires-stakeholder-input` | user, 2026-11-04 | The `[dependencies]` closure, for the app and the `.lvpkg` alike. Declined: every lock entry (uv's default); adding checked-in `.lvlibp` files. |
| Both `[application]` and `[distribution]` | `requires-stakeholder-input` | user, 2026-11-04 | `--app` or `--package` picks one; no flag builds both. |
| The slice's "in the container" demo | `inferable` | inference | Replaced by a local demo on LabVIEW 2026. The user dropped the LabVIEW 2025 container route with ze_recorder as too complicated; no LabVIEW container exists yet. |
| Target inside the `.lvproj` | `inferable` | inference | `My Computer` only. The template has no other target, and the slice names no RT or FPGA builds. |
| SBOM file name | `inferable` | inference | `bom.cdx.json`: CycloneDX's recommended name pattern (`*.cdx.json`). |
| SBOM placement for the app | `inferable` | inference | The folder of the first output path LabVIEW returns: "next to the build" in #70, and that folder is the spec's destination. |
| SBOM is deterministic | `inferable` | inference | `.lvpkg` is deterministic (distribution-design §2), so its contents must be too. CycloneDX makes `serialNumber` and `timestamp` optional. |
| Lock and venv preconditions | `inferable` | inference | An SBOM from a stale lock, or a build against packages the lock does not name, would be false. `sync` and `check` already refuse in the same cases. |
| A library with no `[dependencies]` and no lock | `inferable` | inference | An empty `components` list. S1 libraries build without a lock today; requiring one would break them. |
| Old `.lvpkg` files without an SBOM | `inferable` | inference | Still install. The SBOM is information for people and scanners; nothing in lvpm reads it back. |
| Failure of the `.lvpkg` when both kinds are built | `inferable` | inference | Stops before the app build: one failed step fails the command, as `--no-check` refusals already do. |
| CRUD sweep: `[application]` table | `inferable` | inference | Create/update/delete: the user edits `lvpm.toml`; no command writes it (`lvpm init` writes no `[application]` in this slice). Read: `lvpm build`. |
| CRUD sweep: SBOM file | `inferable` | inference | Created by every successful build; replaced by the next build; never read by lvpm; deleted with the build output by the user. |
| CRUD sweep: build output | `inferable` | inference | Written by LabVIEW into the spec's destination; lvpm does not clean or delete it. |
| Authentication / authorization | `inferable` | inference | Not applicable: a local build. Only the root manifest names what is built. |
| Audit / logging | `inferable` | inference | The SBOM is the record of what went into a build; `AppBuilt` prints the outputs. |
| Error handling | `inferable` | inference | Preconditions refuse before LabVIEW starts. LabVIEW errors fail with their code. The spawned LabVIEW is always closed. No SBOM for a failed build. |

## Consistency Gate
- [x] Intent is unambiguous
- [x] Every behavior/goal maps to an acceptance criterion (app build → AC1, AC7, AC8; SBOM → AC2, AC3, AC4; SBOM in `.lvpkg` → AC5; manifest table → AC9; flags → AC6; pilot app → AC11; docs and milestone → AC12; no regressions → AC10)
- [x] Architecture constrains without over-engineering (one table, two flags, one module, one event, one NI VI)
- [x] Terminology consistent across artifacts
- [x] No contradictions between artifacts
- [x] Every gap/ambiguity finding is logged. The seven `requires-stakeholder-input` items were answered by the user.
