# The project manifest (`lvpm.toml`)

`lvpm.toml` declares what a LabVIEW project depends on: the packages it
needs, the minimum LabVIEW it targets, and any hosted package indexes beyond
the public ones. It is the file you commit; the venv (`.lv-venv/` by default)
and the shared package cache are derived from it and are not.

This document is the reference for the format as implemented in
[`crates/lvpm-core/src/project.rs`](../crates/lvpm-core/src/project.rs) (and, for `vipm.toml`,
[`crates/lvpm-core/src/vipm_interop.rs`](../crates/lvpm-core/src/vipm_interop.rs)), how commands find and use
it, and how it relates to VIPM's `vipm.toml`. Planned extensions are listed
at the end and in
[roadmap.md](roadmap.md#the-manifest-and-the-lockfile).

> **This fork (2026-10-30).** This file came in with the upstream merge
> (Flydroid/lvpm). Named dependency groups are built here as
> `[dependency-groups.<name>]` tables, as in uv, not as upstream's
> `[dependencies.<name>]` ([why](vision/build-slices.md#slice-s11-dependency-groups)).
> `[dev-dependencies]` is the group `dev`: synced by default, skipped with
> `--no-dev` (see [usage.md](usage.md)). A `vipm.toml`'s `[dev-dependencies]`
> is read into it.

## A complete example

```toml
[project]
name = "YourProject"
version = "0.1.0"
labview = ">=2025"                # the oldest LabVIEW the project is meant for

[sources]                         # repositories beyond the public indexes
mirror = "http://host:8090/files" # a hosted index.vipr folder
defaults = true                   # false: only the sources listed here
min-age-days = 14                 # skip versions released fewer than 14 days ago

[dependencies]
delacor_lib_qmh = "7.1.2.1547"    # exactly this version
oglib_error = ">=6.0.1"           # the newest version satisfying the floor
labview_open_source_lib_serializer = { github = "LabVIEW-Open-Source/Serializer", version = ">=1.2" }  # from that repo's releases

[dependency-groups.test]           # installed only with --group test
jki_lib_caraya = "*"              # the newest version

[nipm.dependencies]               # NI Package Manager packages: reported, not installed
ni-daqmx-labview-support = "26.0"
```

Every table is optional. A file containing only `[project]` is a valid
manifest with no dependencies. `lvpm init` writes the first one:
`name` from the directory, `version` `0.1.0`, `labview` and an empty
`[dependencies]`; with `--from` the `name` and `version` come from the source
instead ([usage.md](usage.md#starting-a-project)). `lvpm venv create`
writes the same skeleton, without `version`, only when it finds no manifest.

## Finding the manifest

A project is wherever its manifest is, the way cargo finds `Cargo.toml`. The
manifest a command uses is chosen in this order, first match wins:

1. `venv create --manifest <FILE>`: that file, whatever its name.
   (`install --manifest` is refused in this fork: in a project `install` is
   `sync` from `lvpm.lock`, and the lock is made from `lvpm.toml` only.)
2. The repository of the venv in effect: `<repo>/lvpm.toml`, if it exists.
3. `--project <DIR>`: `<DIR>/lvpm.toml`, if it exists.
4. The nearest `lvpm.toml` at or above the working directory.

No manifest is fine: `lvpm install <name>` works without a project. When a
manifest is found, its `[sources]` apply to every command that reads indexes
(`search`, single-package `install`, full `install`), not only to a
manifest-driven install. `min-age-days` is the exception: a single-package
install that is not an `lvpm add` never applies it (see
[A minimum release age](#a-minimum-release-age)).

Only the file name `lvpm.toml` is discovered. A `vipm.toml` is read only
when named with `venv create --manifest` or `init --from`, and is converted first (see
[Compatibility with `vipm.toml`](#compatibility-with-vipmtoml)).

`venv create --manifest` puts the venv beside the manifest and binds it to
the manifest's `labview`.

## `[project]`

| Key       | Type   | Used for                                                                                       |
| --------- | ------ | ---------------------------------------------------------------------------------------------- |
| `name`    | string | Shown in the install banner. Optional.                                                         |
| `version` | string | Recorded, not acted on. Optional.                                                              |
| `labview` | string | The **minimum** LabVIEW year the project supports, written `">=YYYY"`, for example `">=2025"`. Optional but recommended. |

`labview` is a floor, not a pin:

- `lvpm venv create` binds the venv to `--labview-version` when given, else to
  `labview`. A version newer than `labview` is accepted; an older one is
  refused. With neither, `venv create` asks for one.
- On a headless machine (`LV_RTE_HEADLESS` set) with no venv beside the
  manifest, `lvpm install` targets the LabVIEW `labview` names, or a newer
  installed one, instead of refusing (see the README's
  [CI in a container](ci-docker.md)).
- A global install still takes its target from `--labview-version`.

## `[sources]`

Hosted package indexes added to the search, in file order, ahead of the
public defaults, and OCI registries that a dependency pins with `index`
([below](#an-oci-registry-as-a-source)).

| Key        | Type    | Meaning                                                                           |
| ---------- | ------- | --------------------------------------------------------------------------------- |
| `<name>`   | string  | An `http://` or `https://` folder holding an `index.vipr`, or an `oci://host[:port]/prefix` registry. The name is a label, and what `index` names. |
| `defaults` | boolean | `true` (default): also use the public indexes. `false`: only the sources listed. |
| `min-age-days` | integer | Skip versions released fewer than this many days ago. `0` (default) is off. A negative value, a non-integer, or a value larger than 4294967295 is an error. See [A minimum release age](#a-minimum-release-age). |

A source can't be labelled `min-age-days`. `defaults` is the switch only when
it is `true` or `false`; a string under `defaults` is read as a source.

The public defaults are NI's `download.ni.com/.../index.vipr` and JKI's
`jkisoft.ogpd`. They count as one source, so a name both list has the versions
of both ([ADR 9](adr/0009-the-public-defaults-are-one-source.md)). A path on disk is refused here rather than silently resolving
nothing: local `.vip` directories are a per-user setting
(`lvpm config set sources.local`) and imported packages live in the shared
cache (`lvpm cache add`), because a filesystem path rarely means the same
thing on a teammate's machine or a CI runner.

### An OCI registry as a source

A source whose value starts with `oci://` is a container registry: ghcr.io,
GitLab, Azure Container Registry or any other that speaks the OCI distribution
API. `lvpm publish --to oci://host/prefix` puts a package there
([usage](usage.md#oci-registries)).

```toml
[sources]
team = "oci://ghcr.io/acme/lv"

[dependencies]
acme_lib = { version = ">=1.0", index = "team" }
```

The package `acme_lib` lives in the repository `ghcr.io/acme/lv/acme_lib` (the
name in lower case), one tag per version.

**An OCI source is explicit.** It is asked only for the packages that pin it
with `index = "team"`, and a package without that pin never comes from it. It
takes no part in the first-source-wins order of
[ADR 4](adr/0004-sources-first-wins-and-pinning.md): a registry has no list of
its packages to read, so asking it for every name would send one request per
public package, and ghcr.io answers `403` for a repository that does not exist
(observed, not documented by ghcr.io).
[ADR 7](adr/0007-oci-sources-are-explicit.md) has the reasoning. A package's own
dependencies are not pinned by that: list each one that lives in the registry
as a direct dependency with `version = "*"` and its own `index`.

- A pinned package the registry cannot serve never falls through to another
  source. `lvpm lock` fails when the registry does not list it, refuses the
  lookup (`401` or `403`) or cannot be reached, and says which package, source
  and registry. It reports every failing package together, one line each.
- An OCI source with no package pinned to it prints a warning, once:
  `OCI source "team" has no package pinned to it and is never asked; pin a package with index = "team"`.
  When no source lists a name, the error ends with
  `note: OCI source "team" is explicit; pin with index = "team"`.
- The URL is `oci://host[:port]/prefix`. lvpm talks `https` to it, and plain
  `http` only to `127.0.0.1` and `localhost`.
- A URL with `user:password@` is refused when sources are resolved, naming `[sources]`:
  `[sources] team: credentials do not belong in lvpm.toml; remove the user:password@ part and run docker login ghcr.io`.
  Credentials come from the Docker credential store only: run `docker login`
  (see [OCI registries](usage.md#oci-registries)).
- The lock records the package's `source` (the `oci://` URL) and the `digest`
  of its OCI manifest. Changing an `oci://` source makes the lock out of date.

### A minimum release age

A version that was just published can be malicious: a maintainer account is
taken over, or a release is tampered with, and it is usually found out within
days. `min-age-days = 14` tells lvpm not to choose a version whose release
date is less than 14 days ago; it takes the newest version that is old enough.
This applies to direct and transitive dependencies. A version with no release
date is allowed: entries a feed lists without one, local `.vip` directories,
`lvpm cache add` imports and `.lvpkg` files all have no release date, so the
limit does not protect against them. A release date lvpm cannot read is
treated the same way, as undated and allowed. It reads RFC 2822 dates
(`Thu, 19 May 2016 15:46:24 GMT`) ending in `GMT` or a `±HHMM` offset; an
offset without a sign, a feed quirk beyond RFC 2822, is read as positive.

- The limit is whole days of 86400 seconds, counted from the release date
  the feed lists. Dates are shown in UTC; the human `eligible` day is the
  first whole UTC day on which the version is eligible. A release date in the future counts
  as too new.
- Pinning matters: an exact pin is exempt from min-age-days, so `alpha = "1.1"` locks 1.1 even
  when it is three days old. So is a version named on `lvpm add alpha@1.1`
  or `lvpm lock --upgrade-package alpha@1.1`. When the manifest's constraint
  already admits that version, the upgrade flag pins only for that run. When
  it excludes it, lvpm rewrites the entry and keeps its kind: an exact entry
  becomes `"1.1"`, so the pin stays in `lvpm.toml`; a floor becomes `">=1.1"`,
  which is not a pin, so the next re-lock applies the limit again. To keep a young version across later re-locks, pin it
  exactly in `lvpm.toml`.
- It protects only as far as you trust the release date the feed lists; lvpm
  cannot check that date.
- A held-back version never falls through to a later source: the package's
  source pin decides its versions if it has one, otherwise the first source that
  lists it.
- When the limit keeps out a newer version, lvpm says so on stderr, for
  example `held back alpha 1.1 by min-age-days = 14 (released 2026-10-28,
  eligible 2026-11-12); using 1.0`, and `--json` writes a `held_back` event
  ([JSON output](reference/json-output.md#the-held_back-event)). When no version is old enough,
  the lock fails, lists the held-back versions and the date the first becomes
  eligible.
- `lvpm.lock` records the limit, so changing it makes the lock out of date.
  Time passing does not: the limit takes effect at the next `lvpm lock`,
  `lvpm lock --upgrade`, `lvpm add`, `lvpm remove` or `lvpm sync` that
  resolves again, and an up-to-date `lvpm.lock` is not resolved again.
  `lvpm sync --locked` and `--frozen` never resolve, so they never apply it.
- A plain install never applies the limit: `lvpm install --global <name>`, or
  `lvpm install <name>` with `--prefix` or `--labview-version`, or outside a
  project. Inside a project it uses the project's `[sources]` but not its
  `min-age-days`; outside one it reads the indexes alone. In a project,
  `lvpm install <name>` with none of those flags is an `lvpm add`, which does
  apply it. An install from `lvpm tui` into a project's venv is an
  `lvpm add` too and applies it. Outside a project the TUI makes a plain
  install into the newest LabVIEW installation, without the limit.
  `--labview-version` in a project with a venv writes into that venv and does
  not apply it.

## `[dependencies]`

One line per package: the package name as the index knows it (the `.vip`
basename without version, e.g. `oglib_error`), and a quoted constraint.

| Constraint      | Meaning                                                |
| --------------- | ------------------------------------------------------ |
| `"7.1.2.1547"`  | Exactly this version; no other is accepted.            |
| `">=6.0.1"`     | The newest available version not below the floor.     |
| `"*"` or `""`   | The newest available version.                          |

Rules:

- The version must be quoted. `oglib_array = 6.0` is a TOML float and is
  rejected, as is any other value that is not a string or a table (arrays
  included). A table is the form [below](#a-dependency-from-github), with
  `index` [after it](#a-dependency-from-one-source).
- Versions compare component by component the way VIPM numbers them
  (`major.minor.patch.build`); odd but real forms such as `1.0.1-1` are
  accepted as written.
- Other operators (`<`, `=`, `~`, `^`, ranges) are rejected with an error
  naming the dependency, rather than misread.
- The install plan lists dependencies before the packages that need them.

### A dependency from GitHub

A line can be a table instead of a string. `[dev-dependencies]` takes the
same form.

```toml
[dependencies]
labview_open_source_lib_serializer = { github = "LabVIEW-Open-Source/Serializer", version = ">=1.2" }
```

| Key       | Meaning                                                                                              |
| --------- | ---------------------------------------------------------------------------------------------------- |
| `version` | Required. The same constraint as the string form; `{ version = "*" }` is the same as `"*"`.         |
| `github`  | `owner/repo`. The package comes from that repo's releases and from nowhere else.                    |

A GitHub dependency is never looked up in `[sources]`, even when a source
lists the same name in a higher version. If the repo has no fitting version,
the command fails and does not fall back. Drafts and pre-releases are skipped.

**The release file.** In each release, the package is the asset named
`<name>-<version>.lvpkg`, or `<name>-<version>.vip` when the release has no
`.lvpkg`. `<name>` is the key on the dependency line (case does not matter),
and `lvpm build` writes this name. Other assets are ignored.

- The version comes from the file name, not the tag. A tag `1.2.1` that holds
  `labview_open_source_lib_serializer-1.2.1.16.vip` is version `1.2.1.16`. The
  tag only groups the files.
- Two releases with the same version fail and name both tags; delete one file.
- The name and version inside the package must equal the key and the file
  name's version. If they differ, `lvpm lock` fails and installs nothing.
- The package's own requirements resolve through `[sources]` as usual. To
  take one of them from GitHub too, list it as a direct dependency.
- `lvpm lock` pins the file's SHA-256. A file re-uploaded with other bytes
  makes `lvpm sync` fail naming both hashes.

**What is refused.** These fail when the manifest is read, naming the package:

- `github` together with `index`: a GitHub dependency comes only from its repo.
  This holds across tables too: a name with `github` in one table and `index`
  in another is refused, naming both tables.
- Any key besides `version`, `github`, `index` and `hooks`: `unknown key "foo"; expected version, github, index or hooks`.
- A table without `version`.
- A `github` value that is not `owner/repo`, such as `"https://github.com/o/r"`
  (drop the `https://github.com/`).

Tokens, rate limits and the commands that call GitHub are in
[usage.md](usage.md#github-dependencies).

### A dependency from one source

The `index` key is a source pin: it names the one `[sources]` entry a package
comes from. `[dev-dependencies]` and every `[dependency-groups.<group>]` take
it too.

```toml
[sources]
internal = "http://host:8090/files"

[dependencies]
hexagon_lib_motion = { version = ">=2.0", index = "internal" }
```

| Key       | Meaning                                                                                                |
| --------- | ------------------------------------------------------------------------------------------------------ |
| `version` | Required. The same constraint as the string form.                                                       |
| `index`   | The name of a `[sources]` entry: an `index.vipr` folder or an `oci://` registry. The package comes from that source only.   |

Here is what the index key does:

- The package is pinned to one source and never falls back. If the source
  does not list it, lists no version that fits, or cannot be reached, the
  command fails and names the package and the source.
- It beats every other source: a `sources.local` folder and a `lvpm cache add`
  import that list the same name are ignored, and so is a source listed
  earlier or one that has a higher version. The public indexes are ignored
  too; they have no name in `[sources]`, so a package cannot be pinned to one
  unless you list its URL there.
- A name has one source pin. The same name pinned to two different sources in
  two tables is refused. A line for the same name without `index` in another
  table follows the pin.
- It does not reach the package's dependencies. They resolve through
  `[sources]` as usual. To pin one of them too, list it as a direct
  dependency with `version = "*"` and its own `index`.
- Changing, adding or removing it makes `lvpm.lock` out of date.
  A lock written before source pins existed is not.

**What is refused.** These fail when the manifest is read, naming the line:

- `index` together with `github`, on one line or on two lines of one name.
- An `index` that is not a key of `[sources]`, with the keys it has. `defaults`
  and `min-age-days` are settings, not sources, so they are refused too.
- A name pinned to two sources.

`lvpm add <name> --index <NAME>` writes the line
([usage.md](usage.md#adding-and-removing-dependencies)).

### Allowing install hooks

Install hooks are off by default. To let a package's hooks run, give its line
the `hooks` key. It takes the one value `"run"`, in `[dependencies]`,
`[dev-dependencies]` and the groups, next to `version`, `github` and `index`:

```toml
[dependencies]
pilot_a = { version = "*", hooks = "run" }
```

- Only the root `lvpm.toml` counts. A `hooks = "run"` in a dependency's own
  `lvpm.toml` allows nothing. To allow a package that only another package
  needs, list it yourself with `version = "*"`.
- Any other value fails when the manifest is read:
  `[dependencies] pilot_a: hooks must be "run" (delete the key to keep hooks off), got "yes"`.
  `"Run"` is refused too.
- `hooks` is not part of the lock: changing it leaves `lvpm.lock` current.
  `lvpm add pilot_a@<newer>` keeps it.
- It applies to installs into a LabVIEW installation, from `lvpm sync` and
  `lvpm add`. A venv runs no hooks, and a headless LabVIEW runs them only
  with `--hooks`.

What a hook is, and all the rules, are in
[labview-integration.md](labview-integration.md#hooks).

### How the manifest resolves

`lvpm install` with no package name installs the selected tables
(`[dependencies]` and `dev`, plus any `--group`, see
[named dependency groups](#named-dependency-groups)) as one plan with one relink
pass:

- Each root asks for what its constraint says. A dependency declared by a
  package's own spec is always treated as a floor.
- Resolution is depth-first from all roots at once, with no backtracking: the
  first resolution of a name wins. Pin a version exactly in the manifest when
  a transitive floor would otherwise pick something you do not want.
- A transitive dependency that is already installed at a version satisfying
  its floor is left alone. A root always resolves to what the index says, and
  the installer then decides whether that is a no-op, an upgrade, or a refused
  downgrade (`--allow-downgrade` permits it).
- Every root that cannot be found is collected and reported together, so one
  typo does not hide the others. A missing transitive dependency is reported
  and skipped.
- An empty selection is an error for a manifest-driven install.

In this fork, `lvpm install <name>[@<version>]` inside a project is `lvpm add`:
it writes the package into `lvpm.toml`, locks and syncs. `--global`,
`--prefix` or `--labview-version` install it without touching the manifest.
A bare `lvpm install` in a project is `lvpm sync`: the venv is made to match
`lvpm.lock`, and what the lock does not list is uninstalled. Outside a
project, `install` stays additive. See [usage.md](usage.md#the-lock-file).

Uninstall can also remove unshared dependencies after confirmation; use
`--with-deps` or `--keep-deps` for non-interactive use. It warns about
dependents and keeps shared dependencies. A removal that needs the dependency
list refuses installation records written before lvpm kept one; reinstall the
package, or pass `--keep-deps`. In a venv, packages declared in any dependency
group and their installed transitive dependencies are kept during automatic
dependency removal. Explicitly uninstalling a declared package warns and leaves
the manifest unchanged. `uninstall --all` still removes every package.

## Named dependency groups

Named sets of packages the project needs only sometimes: test frameworks,
CI tooling, build tools. They belong to the project and never become package
metadata (as in [PEP 735](https://peps.python.org/pep-0735/) and uv).
`[dev-dependencies]` is the group `dev`; every other group is a table
`[dependency-groups.<name>]` with the shape of `[dependencies]`:

```toml
[dependencies]
delacor_lib_qmh = "7.1.2.1547"

[dev-dependencies]                # the group "dev"
jki_lib_caraya = "*"

[dependency-groups.ci]
ni_vi_analyzer_tests = "1.2.0.3"
oglib_error = ">=6.0"

[dependency-groups.build]
vipm_package_builder = "*"
```

- **Same syntax as `[dependencies]`.** A group line takes the same
  constraints, including the GitHub table form, so a line can move between
  tables unchanged.
- **Names** are letters, digits, `-` and `_`. `dev` is always `[dev-dependencies]`,
  so `[dependency-groups.dev]` (in any case) is an error naming it. An empty
  table defines a group with no packages.
- **One lock.** A package named in several tables is resolved once. `lvpm.lock`
  records, per package, the groups that alone need it
  ([usage.md](usage.md#the-lock-file)).

`lvpm sync` (and a bare `lvpm install`, which is `sync`) installs
`[dependencies]` and `dev`. The flags choose more or less:

| Command                              | Installs                          |
| ------------------------------------ | --------------------------------- |
| `lvpm sync`                          | `[dependencies]` + dev            |
| `lvpm sync --group ci`               | `[dependencies]` + dev + ci       |
| `lvpm sync --no-dev`                 | `[dependencies]` only             |
| `lvpm sync --no-dev --group ci`      | `[dependencies]` + ci             |
| `lvpm sync --group ci --group build` | `[dependencies]` + dev + ci + build |

`--group` is repeatable. `--group dev --no-dev` is refused, and a `--group`
the manifest does not define fails before anything changes, listing the
defined groups (`dev` is always one). A package that `[dependencies]` needs is
installed whatever the flags; one that only unselected groups need is
uninstalled, and the plan names those groups.

`lvpm add <pkg> --group ci` writes the package under `[dependency-groups.ci]`,
creating the table when it is missing and saying so; `lvpm remove <pkg> --group ci`
takes it out. `--dev` is `--group dev`; `--dev` with `--group` is refused.
If the package is in other tables than the one asked for, the error names every
table that holds it, with the flag for each. `add` and `remove` install
`[dependencies]`, dev and the edited group, and never uninstall a package the new
lock still lists, so tooling installed by `lvpm sync --group ci` survives a later
`lvpm add x`. Plain `lvpm sync` is the exact one.

## `[nipm.dependencies]`

NI Package Manager packages the project needs (drivers, toolkits). lvpm
reads only the names, reports them after resolving (`note: the manifest also
lists N [nipm.dependencies]`), and does not install them; install them with
NIPM. Values may be a version string or a table; neither is interpreted yet.

## `[policy]`

What a GitHub release's attestation must say before lvpm locks it. Both keys
are optional, and with neither lvpm trusts a release on first use. A package
from an OCI registry is checked the same way, with one difference
([usage](usage.md#signatures-on-oci-packages)).

```toml
[policy]
require-signature = true                               # default false
trusted-publishers = ["https://github.com/hexagon/*"] # default []
```

`require-signature` refuses a release with no attestation. Each
`trusted-publishers` pattern starts with `https://github.com/`; in it `*`
matches any run of characters except `/`, and case does not matter. A
non-empty list refuses a release attested by any other repository, and one
with no attestation. Any other key in `[policy]` is refused. What is checked,
and when, is in [usage](usage.md#policy).

## `[application]`

Names the LabVIEW project and build specification that `lvpm build --app`
builds. Both keys are required. The table is for an application; a library
uses `[distribution]` ([usage](usage.md#building-a-package)).

```toml
[project]
name    = "my_app"   # the SBOM names the application after these two
version = "1.0.0"

[application]
project    = "app/My App.lvproj"  # a .lvproj inside the folder with lvpm.toml
build-spec = "My Application"     # the build specification's name in that project
```

- `project` is a path relative to `lvpm.toml`. It must end in `.lvproj`, name
  a file, and stay inside the project folder. A link that leads out of the
  folder is refused.
- `build-spec` is the name of a build specification under `My Computer` in
  that project, spelled as LabVIEW shows it. Other targets are not built.
- `[project] name` and `version` must be set. They name the application in
  the SBOM.
- `[application]` is not a lock input: changing it does not make `lvpm.lock`
  out of date.

An unknown key is refused when any command reads `lvpm.toml`:

```text
[application] has unknown key "target"; expected project or build-spec
```

The other checks run when `lvpm build` starts, before it starts LabVIEW. This
way `lvpm lock` still works while you set up the `.lvproj`. Each message names
the problem:

| Problem | Message |
| --- | --- |
| `[application]` is not a table | `application must be a table` |
| `project` is missing or empty | `[application] needs project (the LabVIEW project, a .lvproj path relative to lvpm.toml)` |
| `build-spec` is missing or empty | `[application] needs build-spec (the build specification's name in the LabVIEW project)` |
| A key is not a string | `[application] project must be a string` (or `build-spec`) |
| `project` does not end in `.lvproj` | `[application] project "<path>" must end in .lvproj` |
| `project` names no file | `[application] project "<path>" is not a file` |
| `project` leaves the folder | `[application] project "<path>" lies outside the folder with lvpm.toml` |
| `name` or `version` is missing | `an application build needs project.name and project.version in lvpm.toml, for the SBOM` |

What `lvpm build --app` needs besides the manifest, and what it writes, is in
[usage](usage.md#building-an-application).

## Unknown keys and tables

Keys and tables lvpm does not know are ignored, not rejected. This keeps a
manifest readable by older lvpm builds as the format grows, and lets one file
carry another tool's sections (see below). The flip side is that a misspelt
key such as `labveiw` is silently ignored; check the install banner, which
echoes the manifest path, project name, groups and `labview` minimum it read.
Inside a dependency table, in `[policy]` and in `[application]` the rule is the opposite: an unknown key is refused
([above](#a-dependency-from-github)), because ignoring it would drop what you
asked for. (Upstream ignores `[dev-dependencies]` in an `lvpm.toml` and reads
`[dependencies.dev]`; here `[dev-dependencies]` is the group `dev`.)

## Compatibility with `vipm.toml`

VIPM 2026 introduced its own TOML project file, `vipm.toml`, with the same
cargo-like shape. The notes below are against VIPM's published documentation
(docs.vipm.io, 2026 Q3); VIPM is closed source, so anything not documented
there is marked as unknown.

### Reading a `vipm.toml` with lvpm

lvpm never discovers `vipm.toml`. Named with `--manifest` (or `init --from`), it is converted
into lvpm's model first (`crates/lvpm-core/src/vipm_interop.rs`) and then used exactly like
an `lvpm.toml`. Every command that reads one says so:

```console
manifest: C:\proj\vipm.toml (VIPM format, converted)
  not used: project.labview-bitness, build.lib
  labview-version "2024" read as minimum labview = ">=2024"
  [dev-dependencies] read as lvpm's [dev-dependencies]
```

The `not used` line appears only when something was dropped; the last two
only when the file sets `labview-version` or `[dev-dependencies]`.

```console
$ lvpm venv create --manifest vipm.toml        # venv beside it, bound to labview-version or newer
```

In this fork a `vipm.toml`'s packages cannot be installed yet: `install
--manifest` is refused, and `lock` and `sync` read only `lvpm.toml`. Upstream
installs them with `lvpm install --manifest vipm.toml`. `lvpm init --from
vipm.toml` copies them into an `lvpm.toml`, after which `sync` installs them
(below).

`venv create --manifest` writes no `lvpm.toml` skeleton: a skeleton is
written only when `venv create` has no manifest to read. New venvs remember
the chosen manifest for subsequent installs and dependency-aware uninstalls.

To move a `vipm.toml` project over instead, `lvpm init --from vipm.toml`
writes an `lvpm.toml` from it ([usage.md](usage.md#from-a-vipmtoml)) and leaves
the `vipm.toml` as it is: `name`, `version`, `[dependencies]` and
`[dev-dependencies]` are kept (exact versions stay exact), `[nipm.dependencies]`
is copied as written, `labview-version` is reported and not used for `labview`,
and the rest is named as not used, as above.

| `vipm.toml` content                                  | lvpm                                                                                             |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `[project] name`, `version`                          | Read.                                                                                            |
| `[project] labview-version = "2024"`                 | `venv create --manifest` reads it as `labview`: VIPM's pinned year becomes lvpm's minimum. `init --from` only reports it. |
| `[project] labview-bitness`                          | Not used. lvpm takes bitness from the selected LabVIEW installation.                            |
| `[project] description`, `authors`, `license`, ...   | Not used.                                                                                        |
| `[dependencies] name = "1.2.3.4"`                    | Read as an exact pin. Empty versions and operators (`>=`, `*`, ...) are rejected: VIPM documents exact versions only. |
| `[dev-dependencies]`                                 | Read as lvpm's `[dev-dependencies]`, the group `dev`, exact pins.                                |
| `[nipm.dependencies]` (string or `{ version, feed }`) | Names reported, not installed.                                                                   |
| `[nipm.dev-dependencies]`, `[nipm.feeds]`            | Not used.                                                                                        |
| `[build.*]`                                          | Not used. Building is out of lvpm's scope.                                                      |

### Reading an `lvpm.toml` with VIPM

| `lvpm.toml` content                    | VIPM (as documented)                                                                        |
| -------------------------------------- | ------------------------------------------------------------------------------------------- |
| `[dependencies]` exact versions        | Compatible today. Once lvpm reads a bare version as a caret, the same line means exact to VIPM and "compatible" to lvpm. |
| `"=x"`, `"^x"`, `"~x"`                 | Planned in lvpm; not documented for VIPM, which expects a plain version. Likely rejected. |
| `">=x.y"`, `"*"`, `""`                 | Not compatible: VIPM requires an exact version and rejects empty strings when loading.     |
| `[project] labview`                    | Not read: VIPM's key is `labview-version`, and it is a pin rather than a minimum.         |
| `[dependency-groups.<name>]` tables    | Not documented for `vipm.toml`; likely rejected.                                            |
| `[sources]`                            | Not documented for `vipm.toml`; VIPM configures repositories in the application. Unknown. |
| `[nipm.dependencies]` strings          | Compatible.                                                                                 |

### Semantic differences

- **LabVIEW version.** VIPM's `labview-version` is the team's pinned year,
  overridden per developer by `.vipm/config.toml` or `--labview-version`.
  lvpm's `labview` is a minimum; the per-developer choice is the venv binding
  (`lvpm venv create --labview-version`) or `--labview-version`, and nothing
  per-developer is written next to the manifest besides the gitignored venv.
- **Constraints.** VIPM records exact versions in `vipm.toml` and resolves
  "latest" at `vipm add` time. lvpm lets the manifest carry floors and `*`
  and resolves them at install time; reproducibility is the job of
  `lvpm.lock`, as `vipm.lock` is for VIPM. With the planned Cargo
  grammar a bare version would be a caret in lvpm but exact in VIPM; the
  compatibility layer already turns VIPM's bare versions into exact pins, so
  it keeps VIPM's meaning either way.
- **Dev dependencies.** VIPM installs `[dev-dependencies]` only with
  `vipm install --dev`. lvpm syncs them by default, as uv does, and skips them
  with `--no-dev`.
- **Installation target.** VIPM installs into the LabVIEW installation. lvpm
  installs into the project's venv by default, and refuses to install into
  the machine from inside a project without a venv (except headless).
- **NIPM.** VIPM installs `[nipm.*]` packages through NI Package Manager.
  lvpm only reports them.

### One file for both tools

Keep the file as `vipm.toml` and point lvpm at it with `--manifest`: the
conversion reads `labview-version`, `[dependencies]` and
`[dev-dependencies]` with VIPM's meaning, so nothing needs duplicating.
lvpm-only settings cannot go in it: the conversion rejects floors and `*`,
reports `[sources]` as not used and requires every `[dependencies]` entry to
be an exact-version string. lvpm's `[dependency-groups]` tables are therefore not usable
in a `vipm.toml`; VIPM's handling of them is undocumented.

The reverse, an `lvpm.toml` that VIPM also reads, works only within the
intersection, with both LabVIEW keys:

```toml
[project]
name = "NovaXC"
version = "0.1.0"
labview = ">=2025"        # lvpm: minimum
labview-version = "2025"  # VIPM: pinned year

[dependencies]
delacor_lib_qmh = "7.1.2.1547"
oglib_error = "6.0.1.20"

[nipm.dependencies]
ni-daqmx-labview-support = "26.0"
```

VIPM would have to be pointed at it as a path (`vipm install
./path/to/lvpm.toml`); VIPM's CLI accepts a manifest path, but its handling
of the `lvpm.toml` name is undocumented. Both tools read
`[dev-dependencies]`.

This stops working once bare versions become carets in lvpm: the same line
would pin in VIPM and float in lvpm. The `vipm.toml` route above is
unaffected.

## To do

Tracked in [roadmap.md](roadmap.md#the-manifest-and-the-lockfile):

- `lvpm.lock`, `lvpm sync` and `lvpm add` / `lvpm remove` are built in this
  fork (see [usage.md](usage.md#the-lock-file)); upstream still lists them here.
- **Future constraint grammar (Cargo)**, one for every dependency table:

  | Written           | Meaning                                    | Example range                       |
  | ----------------- | ------------------------------------------ | ----------------------------------- |
  | `"7.1.2.1547"`    | caret (bare), same as `^`                  | `>=7.1.2.1547, <8`                  |
  | `"^7.1.2.1547"`   | caret: leftmost non-zero part stays fixed  | `^0.3.1` is `>=0.3.1, <0.4`         |
  | `"~7.1.2"`        | tilde: only the last given part may change | `>=7.1.2, <7.2`                     |
  | `"=7.1.2.1547"`   | exact                                      |                                     |
  | `">=6.0.1"`       | floor                                      |                                     |
  | `"*"`             | newest                                     |                                     |

  - Versions have up to four parts (`major.minor.patch.build`); caret and
    tilde apply to whichever parts are written. Year-numbered packages
    (`2018.0.7.45`) get "same year" from a caret, so pin them with `=` when
    that is too loose.
  - **Breaking change**: today a bare version is exact. Existing manifests
    keep their meaning only if their pins are rewritten as `=x`; lvpm should
    say so once, when it reads a bare version that a caret would widen.
  - Python's `==` is not accepted: `=` is the one spelling for exact, and
    `==` fails with a hint to use it.
  - `name@version` stays a CLI shorthand for exact (`lvpm install`,
    `lvpm add`), as in `cargo add`; it is not written in the manifest.
    `lvpm add` writes `=x` when given a version and a caret on the resolved
    version otherwise, as `cargo add` does.
- **More group controls**: `include-groups`, `--only-group`, `--all-groups`, a
  configurable default set (`[project] default-groups`), and groups on a global
  `lvpm install`. Groups are built ([above](#named-dependency-groups)).
- **`[venv]` overrides**: `dir`, `vi-server-port`, `bitness`.
- **`vipm.toml` conversion, remaining fields**: `labview-bitness` is checked
  against the selected target once `[venv] bitness` exists. Writing an
  `lvpm.toml` from the converted result (a one-off migration command) is a
  possible follow-up.
- **Open decision: should lvpm discover `vipm.toml`?** Today it does not; a
  `vipm.toml` is converted only when named with `venv create --manifest`. Options:

  | Option                                              | For                                                                                   | Against                                                                                                           |
  | --------------------------------------------------- | ------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
  | Explicit only (today)                               | Never surprising; a VIPM project is used by lvpm only when someone asks for it.       | In this fork a vipm.toml's packages cannot be installed at all; only `venv create --manifest` reads it.            |
  | Fall back to `vipm.toml` when no `lvpm.toml` exists | VIPM projects work with lvpm unchanged.                                               | `venv create` and `install` would act on a file VIPM owns; a team using both tools gets two readings of one file. |
  | Fall back, but say so on every command              | Same as above, and the conversion note makes the source visible.                      | Noise for projects that use lvpm through VIPM's file on purpose.                                                  |

  Related doubts, to settle with it:
  - When both files exist, `lvpm.toml` should win; should lvpm warn that
    `vipm.toml` is being ignored?
  - VIPM's `labview-version` is a pin; lvpm reads it as a minimum. Should a
    converted project refuse a newer LabVIEW instead?
  - (Upstream only, where `install --manifest` is accepted.) Should it look
    for the venv from the manifest's folder instead of the working directory,
    so *what* and *where* always agree?
