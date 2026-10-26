# Build slices: from vision to working code

Spec: [README.md](README.md) (the vision) and
[distribution-design.md](distribution-design.md) (the design).

This file cuts the vision into vertical slices. Each slice ends in a loop
that works end to end and can be shown with one demo. Each slice is written
so the dev-team pipeline (`/specs` → `/plan` → `/build` → `/pr`) can take it
from spec to pull request without this conversation.

**Re-cut 2026-10-26.** The lockfile plan ([../../plans/lvpm-lockfile.md](../../plans/lvpm-lockfile.md))
shipped most of the old S3 and S10: `lvpm.lock` with SHA-256, `lock`,
`lock --check`, `lock --upgrade`, exact `sync`, `--locked`, `add`, `remove`
and `[dev-dependencies]`. The domain code now lives in `crates/lvpm-core`.
S3 is gone, S10 keeps only what is left, and S1, S5 and S7 are split.

## How to run a slice

1. Create the issues once: `/issues-from-plan docs/vision/build-slices.md`.
   It makes a parent issue from the spec and one child issue per slice,
   linked by the `Depends-on` lines below.
2. Ship one slice at a time: `/ship <issue URL>`. `/specs` turns the slice
   into intent, architecture notes and acceptance criteria and asks the
   questions listed under "Decide in `/specs`".
3. After each merge, run the slice's demo by hand and adjust the next
   slices here if something you learned changes them.

## Rules for every slice

- One slice, one PR. If `/specs` finds a slice too big, split it here first.
- Behaviour is tested by `cargo test`: unit tests in the crates, scenarios in
  `tests/features/` run by the cucumber harness (`tests/acceptance.rs`).
  Scenarios that need a real LabVIEW are tagged `@labview` and run only with
  `LVPM_TEST_LABVIEW=1`. Scenarios for behaviour a later slice brings are
  tagged `@pending-<slice>`, and the harness reports them as not run.
- Domain code goes into `crates/lvpm-core/src/` (operations under `ops/`),
  and it does no terminal I/O. `src/main.rs` is only the CLI front end.
- Format knowledge comes only from published specifications and observed
  files ([PROVENANCE.md](../../PROVENANCE.md)). No VIPM or JKI code.
- [../roadmap.md](../roadmap.md) belongs to the upstream author: don't edit
  it. Update this folder and the README command table instead.
- Tick the matching milestone box in [README.md](README.md#milestones) in
  the same PR.

## Decisions before S1

These are hard to undo, so settle them as ADRs (`/adr-tools`) before S1 starts:

1. **The `[package]` keys in `lvpm.toml`.** Users commit them. Check them
   against `vipm.toml` so nobody confuses the two files.
2. **`.vip` first, `.lvpkg` later.** Every consumer already reads `.vip`;
   `.lvpkg` is needed for OCI referrers and its `RECORD`/SBOM layout (S7a).
   Until then a `.vip` gets a detached attestation (S6).
3. **The upstream relationship.** Contribute back or keep a long-lived fork.
   It decides naming and how code is split into modules.

## Pilot libraries

Every slice's demo uses the same two libraries. Both are on vipm.io, so
lvpm installs them today, and both keep their source in public git.

| | Pilot A: HSE Logger | Pilot B: OpenSerializer |
|---|---|---|
| Stands for | inner source: a team's library in its own GitLab | open source on GitHub |
| Source | [code.hampel-soft.com/open-source/hse-logger](https://code.hampel-soft.com/open-source/hse-logger) | [github.com/LabVIEW-Open-Source/Serializer](https://github.com/LabVIEW-Open-Source/Serializer) |
| VIPM name | `hse_lib_hse_logger` | `labview_open_source_lib_serializer` |
| Library | `Source/HSE-Logger/hse-logger.lvlib` | `source/OpenSerializer.lvlib` |
| Saved in | LabVIEW 2016 | LabVIEW 2013 |
| Dependencies | none | `labview_open_source_lib_data_manipulation` (5 `.lvlib`s) |
| License | BSD (HSE) | 0BSD |

Why these two: A has test libraries (`Tests/Unit_Tests/**/*.lvlib`) next to
its source, so S1 can show that only library members ship. B has exactly one
dependency, which S1 must name in `Requires` and S5 must find as an external link.
A has no dependency, so the "at least one dependency" pilot rule holds only
for B. Checked 2026-10-26 from each repository's `.vipb`.

## Order

| Wave | Slices | Can run in parallel |
|---|---|---|
| 1 | S0 acceptance scenarios, S5 `lvpm check`, S10a–d project commands | yes |
| 2 | S1 `lvpm build` | — |
| 3 | S1v VIPM round trip, S2 publish to folder, S4 GitHub Releases, S5b build gate, S9 app builds + SBOM | yes (S2, S5b and S9 touch `src/main.rs` and `ops/build.rs`: merge one first, rebase the others) |
| 4 | S6 attestations and policy, S7a `.lvpkg` | yes |
| 5 | S7b OCI feeds, S8 community index | yes |
| 6 | S7c `@org` scopes | — |

Not sliced yet: the `ppl` kind (M2) and `lvpm convert` (M4). Add slices for
them after S7a. The M2 `.lvpkg` box is ticked by S7a, not by the M2 slices.

Spike the two biggest unknowns before you put dates on anything: whether
VIPM installs a `.vip` that lvpm wrote (S1v), and the linker-table reader
(S5). The Python prototype `deptree.py` that S5 was to port is not in this
repository, so S5 starts from the roadmap's description of it and from
observed VI files, not from its code.

---

### Slice S0: Acceptance scenarios for the pilot loop

**Depends-on:** none
**Files:** `tests/features/vision/`, `tests/steps/`, `tests/support/`, `tests/acceptance.rs`

Goal: one set of scenarios that every later slice is judged by.

In scope:
- `@labview` scenarios on the existing cucumber harness for the loop:
  build → publish → install into project B's venv → mass compile.
- Install and mass compile work today, for both pilots, from a folder source
  that holds their `.vip` files at pinned versions.
- Build and publish scenarios are written now and tagged `@pending-s1` and
  `@pending-s2`, so the run lists them as not available yet.
- Mass compile with `LabVIEWCLI -OperationName MassCompile`, as the roadmap's
  "CI for lvpm itself" step 4 describes; a failure names the VI from the log.

Out of scope: making the pending scenarios pass; running them in CI (needs
the container setup).

Acceptance criteria:
- [ ] `LVPM_TEST_LABVIEW=1 cargo test --test acceptance` installs both pilots from a folder source into a fresh project venv and mass compiles them.
- [ ] The run reports the build and publish scenarios as not run, naming S1 and S2.
- [ ] Without `LVPM_TEST_LABVIEW=1` the scenarios are reported as not run and `cargo test` stays green.
- [ ] How the pilot `.vip` files are obtained and pinned is written next to the feature file.

Demo: run the acceptance tests with LabVIEW; install and mass compile pass, build and publish show as not run.

Decide in `/specs`: where the pinned `.vip` files come from at test time
(downloaded from vipm.io and checked by SHA-256, or committed fixtures, if
the licenses allow it).

### Slice S1: `lvpm build` writes a `.vip`

**Depends-on:** none (the "Decisions before S1" ADRs)
**Files:** `src/main.rs`, `crates/lvpm-core/src/project.rs`, `crates/lvpm-core/src/spec.rs`, `crates/lvpm-core/src/lvlib.rs`, `crates/lvpm-core/src/ops/build.rs`

Goal: one command turns a library into a `.vip` that lvpm installs.

In scope:
- A `[package]` table in `lvpm.toml`: name, version, minimum LabVIEW,
  library path, dependencies, plus the `[package.vip]` fields
  [distribution-design.md](distribution-design.md#2-build-lvpm-build) lists.
- Collect files by `.lvlib` membership, not by folder glob.
- Write the `spec` file (the inverse of `spec::parse`) and a
  deterministic zip (sorted entries, fixed timestamps) into `dist/`.

Out of scope: VIPM acceptance (S1v), `.lvpkg`, link validation (S5), PPLs,
signing, SBOM.

Acceptance criteria:
- [ ] `lvpm build` writes `dist/<name>-<version>.vip`; building twice gives byte-identical files.
- [ ] `spec::parse` reads the written spec back to the same name, version, file groups and `Requires`.
- [ ] A VI on disk that is not a member of the library is not in the package (pilot A's test libraries).
- [ ] A missing or invalid `[package]` table fails with a message naming the key.
- [ ] The built `.vip` installs with `lvpm install` from a folder source; S0's `@pending-s1` build scenario passes.

Demo: `lvpm build` in pilot A, then `lvpm install` the result in project B.

Decide in `/specs`: the `[package]` keys are fixed by the ADR, not here.

### Slice S1v: VIPM installs an lvpm-built `.vip`

**Depends-on:** S0, S1
**Files:** `crates/lvpm-core/src/spec.rs`, `crates/lvpm-core/src/ops/build.rs`, `tests/features/vision/`

Goal: VIPM users can consume what lvpm builds.

In scope:
- A manual round trip: build both pilots, install each in VIPM, mass compile.
- Whatever spec or zip changes VIPM needs, with a test for each.

Out of scope: automating VIPM (it has no headless install we may script
against without its code).

Acceptance criteria:
- [ ] Both pilots built by lvpm install in VIPM and mass compile; the steps and the VIPM version are recorded next to S0's feature file.
- [ ] B's `.vip` makes VIPM install its dependency.
- [ ] README milestone "`.vip` output, verified by a round-trip install in VIPM" is ticked.

Demo: install pilot B's lvpm-built `.vip` in VIPM on a clean machine.

Decide in `/specs`: what lvpm does if VIPM rejects the file (fallback:
`.vip` for lvpm only and `.lvpkg` sooner).

### Slice S2: `lvpm publish --to <folder>`

**Depends-on:** S1
**Files:** `src/main.rs`, `crates/lvpm-core/src/ops/publish.rs`

Goal: a colleague installs your package from a shared folder.

In scope:
- Copy `dist/*.vip` into a folder or UNC share. Folder sources already index
  every `.vip` in a directory, so no index file is needed.
- Refuse to overwrite an existing version (published versions are immutable).

Out of scope: hosted feeds, signing.

Acceptance criteria:
- [ ] `lvpm publish --to <dir>` copies the package and prints where it went.
- [ ] Publishing a version that already exists in the folder fails and changes nothing.
- [ ] A second project with that folder in `sources.local` finds, locks and installs the package; S0's `@pending-s2` scenario passes.

Demo: publish pilot A to a share; project B installs it on another machine.

Decide in `/specs`: whether `--to` can default to a configured feed.

### Slice S4: GitHub Releases as a source

**Depends-on:** S1
**Files:** `crates/lvpm-core/src/project.rs`, `crates/lvpm-core/src/fetch.rs`, `crates/lvpm-core/src/index.rs`, `crates/lvpm-core/src/lockfile.rs`, `.github/workflow-templates/`

Goal: the open-source loop without any registry of your own.

In scope:
- `libX = { github = "owner/repo", version = "^1.2" }` in `lvpm.toml`.
- Versions come from release tags; the release file is `*.vip` (later `*.lvpkg` first).
- The lock records the GitHub source; its SHA-256 is pinned on first lock.
- A CI template that runs `lvpm build` on a tag and uploads with `gh release upload`.

Out of scope: attestations (S6), the community index (S8).

Acceptance criteria:
- [ ] A project installs pilot B from a GitHub release.
- [ ] A re-uploaded release file with a different hash fails `sync --locked`, and nothing is installed.
- [ ] Private repos work with the token from `gh auth token`; no token is ever written to `lvpm.toml` or the lock.
- [ ] The CI template publishes a release from a tag in a pilot B fork.

Demo: tag a fork of pilot B, CI publishes, project A installs it.

Decide in `/specs`: rate limits without a token; how the release file is named.

### Slice S5: `lvpm check`

**Depends-on:** none
**Files:** `crates/lvpm-core/src/linker.rs`, `crates/lvpm-core/src/ops/check.rs`, `src/main.rs`

Goal: find broken or undeclared links without LabVIEW.

In scope:
- A Rust reader for the VI linker table (RSRC blocks, `PTH0` records),
  closing the prototype's known gaps the roadmap lists.
- `lvpm check [DIR]`: every VI saved at or below the minimum LabVIEW, compiled code separated, no broken links, every external link in `vi.lib`, the package or a declared dependency.

Out of scope: running inside `build` (S5b); `lvpm tree` output beyond what the check needs.

Acceptance criteria:
- [ ] Fixture VIs with a broken link, an undeclared dependency and a too-new save version each fail with the VI and link named.
- [ ] Pilot A passes; pilot B passes only with its dependency declared.
- [ ] Runs without LabVIEW.

Demo: break a link in pilot A; `lvpm check` names the VI and the link.

Decide in `/specs`: where the fixture VIs come from, given PROVENANCE rules;
whether the minimum LabVIEW comes from `[package]` (needs S1) or a flag.

### Slice S5b: `lvpm build` runs the check

**Depends-on:** S1, S5
**Files:** `crates/lvpm-core/src/ops/build.rs`, `src/main.rs`

Goal: `build` refuses a package with broken or undeclared links.

Acceptance criteria:
- [ ] `lvpm build` on a library that fails `lvpm check` writes nothing to `dist/` and names the VI.
- [ ] `lvpm build` infers dependencies from the links and fails if they disagree with `[package]` dependencies.

Demo: break a link in pilot A; `lvpm build` refuses and says which VI.

### Slice S6: Attestations and consumer policy

**Depends-on:** S4
**Files:** `crates/lvpm-core/src/verify.rs`, `crates/lvpm-core/src/project.rs`, `crates/lvpm-core/src/lockfile.rs`, `.github/workflow-templates/`

Goal: installs prove who built a package, not just that the bytes match.

In scope:
- The CI template adds a GitHub artifact attestation.
- lvpm verifies it on lock and sync and records signer and provenance subject in the lock.
- `[policy]` with `require-signature` and `trusted-publishers`.

Out of scope: Notation, OCI referrers (S7b).

Acceptance criteria:
- [ ] A release built by the template installs; the lock shows its signer.
- [ ] With `require-signature = true`, an unattested release is refused.
- [ ] A release attested by a repo not in `trusted-publishers` is refused.

Demo: switch pilot A's policy on; an unsigned fork's release is refused.

Decide in `/specs`: verify with the `gh` CLI or a Rust Sigstore library.

### Slice S7a: `.lvpkg` format

**Depends-on:** S1v
**Files:** `crates/lvpm-core/src/ops/build.rs`, `crates/lvpm-core/src/lvpkg.rs`, `crates/lvpm-core/src/index.rs`

Goal: lvpm's own format, with per-file hashes and an SBOM inside.

In scope:
- `formats = ["lvpkg", "vip"]`: a deterministic `.lvpkg` with `package.toml`, `RECORD` and a CycloneDX SBOM.
- lvpm installs `.lvpkg` from every feed it reads `.vip` from, and checks `RECORD`.

Acceptance criteria:
- [ ] Both formats from one `lvpm build`; each is byte-identical across two builds.
- [ ] A `.lvpkg` with a file whose hash disagrees with `RECORD` is refused.
- [ ] README milestone "`lvpm build`: `source` kind, `.lvpkg` format, SBOM" is ticked.

Demo: build pilot B as `.lvpkg`, publish to a folder, install in project A.

Decide in `/specs`: `package.toml` keys (reuse the `[package]` ADR).

### Slice S7b: OCI feeds

**Depends-on:** S7a, S6
**Files:** `crates/lvpm-core/src/oci.rs`, `crates/lvpm-core/src/project.rs`, `crates/lvpm-core/src/ops/publish.rs`

Goal: teams publish to and pull from a standard registry.

In scope:
- `lvpm publish` to and install from ghcr, GitLab and ACR via OCI, pull by digest.
- Signatures and SBOM as OCI referrers.

Out of scope: running a registry; Azure Artifacts (no OCI support).

Acceptance criteria:
- [ ] A package published to ghcr installs in another project by digest.
- [ ] Credentials come from Docker credential helpers or `az acr login`, never from `lvpm.toml`.

Demo: pilot A in an org registry, consumed by project B.

Decide in `/specs`: OCI media types; Rust OCI client or `oras`.

### Slice S7c: `@org` scopes

**Depends-on:** S7b
**Files:** `crates/lvpm-core/src/project.rs`, `crates/lvpm-core/src/index.rs`, `crates/lvpm-core/src/solve.rs`

Goal: dependency confusion has nowhere to start.

In scope: `[[feeds]]` with `scopes`: `@org/*` resolves from exactly one feed.

Acceptance criteria:
- [ ] A public package with the same `@org/` name is never picked over the scoped feed.
- [ ] A scoped name with no matching feed fails with the scope named.

Demo: two pilot libraries in one org registry, consumed by an application, with a same-named decoy in a folder feed.

### Slice S8: Federated community index

**Depends-on:** S6
**Files:** `crates/lvpm-core/src/index.rs`, a separate index repository

Goal: open-source discovery and name ownership with no single owner.

In scope:
- An index repo: one TOML entry per package, files stay on GitHub Releases or OCI.
- Registration by pull request, opened by a release Action; a bot checks name, hash and attestation.
- lvpm reads the index as a feed.

Out of scope: hosting decisions (who owns the index organisation).

Acceptance criteria:
- [ ] A tag in pilot B's fork opens an index PR; the bot verifies and merges it.
- [ ] A project installs pilot B by name through the index.
- [ ] A changed hash for an existing version is rejected by the bot.

Demo: a third party's library joins the index through a PR.

Decide in `/specs`: the index organisation. Start this conversation in the community at S4, not here.

### Slice S9: Application builds and SBOM

**Depends-on:** S1 (the same `build` command)
**Files:** `crates/lvpm-core/src/ops/build.rs`, `crates/lvpm-core/src/sbom.rs`, `src/main.rs`

Goal: build an application from a `.lvproj` build spec in its venv, with an SBOM.

In scope:
- `lvpm build` runs a named `.lvproj` build spec through LabVIEW in the project's venv.
- A CycloneDX SBOM from `lvpm.lock`, shipped next to the build.

Out of scope: PPL packages (later, after S7a).

Acceptance criteria:
- [ ] A pilot application that uses both pilots builds headless from its venv (`@labview`).
- [ ] The SBOM lists every package from the lock with version and SHA-256.

Demo: build the pilot application in the container.

Decide in `/specs`: CycloneDX JSON or XML; where the pilot application
comes from.

### Slice S10a: `lvpm init` and `init --from`

**Depends-on:** none
**Files:** `src/main.rs`, `crates/lvpm-core/src/ops/init.rs`, `crates/lvpm-core/src/manifest_edit.rs`

Acceptance criteria:
- [ ] `lvpm init` writes a minimal `lvpm.toml` and refuses to overwrite one.
- [ ] `lvpm init --from project.vipc` turns each package in the `.vipc` into a dependency, then `lvpm sync` installs them.
- [ ] A line in the README command table.

Demo: `lvpm init --from` an existing `.vipc` project, then `lvpm sync`.

Decide in `/specs`: `.dragon` as well; pinned (`==`) or floor (`>=`) constraints from a `.vipc`.

### Slice S10b: `lvpm info`

**Depends-on:** none
**Files:** `src/main.rs`, `crates/lvpm-core/src/ops/query.rs`

Acceptance criteria:
- [ ] `lvpm info <pkg>` shows the versions every source offers, the dependencies and the files of one version.
- [ ] An unknown package fails and says which sources were searched.

### Slice S10c: `--json` and documented exit codes

**Depends-on:** none
**Files:** `src/main.rs`, `src/render/`, `crates/lvpm-core/src/errors.rs`

Acceptance criteria:
- [ ] `--json` gives machine-readable output for `list`, `search`, `info` (once it exists), `lock --check` and `sync`.
- [ ] Exit codes are documented in one table, and each code has a scenario.

Decide in `/specs`: whether to match VIPM's exit codes where they overlap.

### Slice S10d: `--yes`, `--labview-bitness`, `update`

**Depends-on:** none
**Files:** `src/main.rs`, `crates/lvpm-core/src/target.rs`

Acceptance criteria:
- [ ] A global `--yes` (and an env var) answers every prompt; `venv remove -y` keeps working.
- [ ] `--labview-bitness 32|64` picks between two installations of the same version.
- [ ] `lvpm update` is an alias of `lock --upgrade` plus `sync`, or the README says why not.

These four are good first issues for contributors.
