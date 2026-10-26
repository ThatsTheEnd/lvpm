# 1. `lvpm.toml`: one `[project]`, a `[distribution]` table, LabVIEW builds stay in the `.lvproj`

Date: 2026-10-26

## Status

Accepted

## Context

S1 adds `lvpm build`, so `lvpm.toml` needs keys that describe a package. The
vision proposed a new `[package]` table with `name`, `version`, `labview` and
`library`. `lvpm.toml` already has a `[project]` table with the same first
three keys, so every repo would have had two names and two versions.

"Build" also means two different things here:

- a `.lvproj` build spec makes LabVIEW compile something (PPL, exe, source
  distribution);
- `lvpm build` packs files into a `.vip` or `.lvpkg`.

`vipm.toml` mixes the two. Its `[build.<name>]` table takes
`type = "ppl" | "exe" | "lvproj" | "vipb"`, which restates build specs in TOML
and puts VIPM packaging under the same word. A `[build]` table in `lvpm.toml`
would look like VIPM's and do something else.

uv separates them: `pyproject.toml` has one `[project]` identity,
`[build-system]` names the backend that compiles, and `uv build` is the
frontend that calls the backend and ships the distribution.

lvpm has no users yet, so no names need a migration.

## Decision

1. **The `.lvproj` owns how to compile; `lvpm.toml` owns what to ship.** lvpm
   never redefines a build spec. A `ppl` package names an existing build spec,
   the way `[build-system]` names a backend, and `lvpm build` runs it.
2. **One identity: `[project]`.** `name` and `version` live there and nowhere
   else. A package is a project that has a `[distribution]` table. There is no
   `[package]` table.
3. **`labview` is a constraint**, written like uv's `requires-python`:
   `labview = ">=2021"`. It uses the `[dependencies]` constraint syntax. Venvs
   bind to the newest installed LabVIEW that satisfies it, and source packages
   are saved at its lower bound.
4. **Packaging settings live in `[distribution]`**, with VIPM-only fields in
   `[distribution.vip]`. lvpm never uses a `[build]` table.
5. **Use VIPM's key name when the meaning is the same, and a different name when
   it isn't.** So `name`, `version`, `description`, `authors`, `license`,
   `repository` and `top-level-library` match `vipm.toml`. `labview` stays
   different from VIPM's `labview-version`, because it is a minimum and not one
   version.
6. **Dependencies stay in `[dependencies]` and `[dev-dependencies]`.**
   `[distribution]` does not get a second list.
7. **The command stays `lvpm build`.** For a `ppl` package it runs the LabVIEW
   build spec and then packs the result, as `uv build` calls its backend and then
   ships the distribution.

```toml
[project]
name    = "@hexagon/motion-lib"
version = "2.1.0"
labview = ">=2021"

[distribution]
top-level-library = "src/Motion.lvlib"   # membership defines the payload
kinds   = ["source"]
formats = ["vip"]

[distribution.vip]                       # only what VIPM needs and lvpm cannot derive
name         = "hexagon_lib_motion"
vendor       = "Hexagon"
display-name = "Motion Library"
```

The key that names a PPL's build spec is fixed by the slice that adds PPLs.

## Consequences

- S1 parses `[distribution]` and changes the `labview` parser from a plain
  version to a constraint. S1 accepts only `>=`, and `project.rs` hands the
  lock and venv code the lower bound, so they don't change. A plain `labview = "2025"` (the current README form)
  becomes an error that suggests `">=2025"`.
- `package.toml` inside a `.lvpkg` (S7a) reuses these key names.
- A `vipm.toml` and an `lvpm.toml` side by side share `[project]`,
  `[dependencies]` and `[dev-dependencies]`. Only the LabVIEW key and the
  packaging table differ, and both differ on purpose.
