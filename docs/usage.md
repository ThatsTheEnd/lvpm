# Using lvpm

## Commands

| Command                                              | What it does                                                                                                                                 |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `lvpm init`                                          | Start a project here: write an `lvpm.toml` and a `.gitignore`, make a git repository and a venv ([Starting a project](#starting-a-project)); an existing `lvpm.toml` is never replaced |
| `lvpm init --from <file.vipc>`                       | The same, with the packages VIPM chose in a `.vipc` as dependencies ([From a .vipc](#from-a-vipc))                                           |
| `lvpm init --from vipm.toml`                         | The same, with the project and packages of a VIPM 2026 `vipm.toml` ([From a vipm.toml](#from-a-vipmtoml))                                    |
| `lvpm install <name>[@<version>]`                    | Outside a project, or with `--global`, `--prefix` or `--labview-version`: resolve the package and its dependencies, download, verify MD5, unpack, relink, run the hooks of packages you allowed ([Install hooks](#install-hooks)). In a project it is `lvpm add` |
| `lvpm install`                                       | Inside a project: the same as `lvpm sync`                                                                                                    |
| `lvpm sync`                                          | Make the destination hold exactly what `lvpm.lock` lists, one plan with one relink pass ([The lock file](#the-lock-file))                    |
| `lvpm add <name>[@<constraint>]` / `--dev` / `--group <name>` / `--index <NAME>` | Add a dependency to `lvpm.toml`, lock and sync                                                                                      |
| `lvpm remove <name>` / `--dev` / `--group <name>`    | Remove a dependency from `lvpm.toml`, lock and sync                                                                                          |
| `lvpm lock` / `--check` / `--upgrade`                | Write `lvpm.lock`, check it, or move packages to newer versions                                                                              |
| `lvpm uninstall <name>` / `--all`                    | Remove exactly the files that were installed and prune empty folders                                                                         |
| `lvpm list`                                          | What lvpm has installed into the selected target, and which hooks it did not run                                                             |
| `lvpm search <query>` / `--all [query]`              | Search the package indexes; `--all` lists every version of every package and its source                                                      |
| `lvpm info <package>[@<version>]`                    | Show a package's description, requirements, files and versions, from its archive (downloaded, then cached)                                   |
| `lvpm targets`                                       | Detected LabVIEW installations                                                                                                               |
| `lvpm tui`                                           | Browse the catalog and install packages in a terminal UI; takes no options                                                                   |
| `lvpm relink <name>` / `--all`                       | Rerun the relink pass from the install manifests, without reinstalling                                                                       |
| `lvpm run-hooks <name>` / `--all`                    | Run a package's PostInstall hook now, for one that failed or was skipped during install ([Install hooks](#install-hooks))                    |
| `lvpm refresh`                                       | Rebuild palettes and the File/Tools/Help menus from disk (`install` does this itself)                                                        |
| `lvpm start`                                         | Start the selected LabVIEW and wait until its VI Server answers                                                                              |
| `lvpm venv create` / `status` / `remove`             | Manage the project's venv (see [Per-project dependencies](#per-project-dependencies-venvs))                                                  |
| `lvpm launch [<lvproj>]`                             | Start a LabVIEW on the project's venv                                                                                                        |
| `lvpm build`                                         | Check the library's members as `lvpm check` does, then package the library as `dist/<name>-<version>.lvpkg`, from `[distribution]`; `--no-check` skips the check ([Building a package](#building-a-package)) |
| `lvpm publish --to <folder>`                         | Copy the built package into a folder feed; a published file is never replaced ([Publishing a package](#publishing-a-package))                |
| `lvpm check [DIR]`                                   | Read every VI under `DIR` (the project by default) without LabVIEW and name each one saved in a newer LabVIEW than `labview`, with compiled code not separated, or with a link to a file that is not there or into a package the manifest does not let that VI use, or that lvpm cannot read as a LabVIEW resource file; exits 1 when it finds any ([Checking a project](#checking-a-project)) |
| `lvpm vi-probe` / `lvpm vi-run` / `lvpm vi-save` / `lvpm app-probe` | VI Server primitives, for diagnosis (see [VI Server](labview-integration.md#vi-server))                                                                            |

Flags that apply to every command:

| Flag                       | Meaning                                                                                                   |
| -------------------------- | --------------------------------------------------------------------------------------------------------- |
| `--labview-version <YYYY>` | Install into this LabVIEW version (see `lvpm targets`)                                                    |
| `--project <DIR>`          | Use that project's venv instead of looking upwards from the current directory; for `init`, the folder to write the project into |
| `--global`                 | Use the LabVIEW installation itself, even from inside a project that has a venv                           |
| `--prefix <DIR>`           | Install into a scratch directory instead of LabVIEW (file copy only; nothing to relink or run hooks with) |
| `--refresh`                | Ask each index whether it changed (a conditional request) instead of trusting the cache in `%LOCALAPPDATA%\lvpm\cache` |
| `--yes` / `-y`             | Answer yes to every prompt (today only `venv remove` asks). `LVPM_YES` does the same; `0`, `false`, `no`, `off` or empty leaves it off |
| `--labview-bitness <32\|64>` | Use only 32- or 64-bit LabVIEW installations, to pick one when both of a version are installed; a venv bound to the other bitness is an error (`--global` skips the venv) |
| `--json`                   | Write JSON Lines to stdout for a script or front end ([JSON output and exit codes](https://github.com/ThatsTheEnd/lvpm#json-output-and-exit-codes)). Not taken by `app-probe`, `vi-probe`, `vi-save` or `vi-run` (exit 2) |

`install` flags: `--dry-run` shows every file, relink folder and hook and
changes nothing; `--no-deps` skips dependencies and `--allow-downgrade`
permits replacing an installed package with an older version (both only for
an install that is not a sync or an `add`: a lock always holds the full set);
`--no-relink` skips the
relink pass and `--relink` forces it where it is off by default (a headless
LabVIEW, see [CI in a container](ci-docker.md)); `--hooks` runs the install
hooks of every package for this command, where they are off by default
(see [Install hooks](#install-hooks); a headless LabVIEW runs none without
it). `--relink-timeout <SECS>`
(default 900) bounds one folder's relink; `--keep-open` leaves the LabVIEW
started for a venv relink running. `sync`, `add` and `remove` take `--dry-run`
and the relink and hook flags as well.

## Install hooks

Install hooks are off by default. A hook is package code that runs inside
LabVIEW with your rights, so lvpm runs a package's hooks only if you allowed
that package. The table of rules is in
[Hooks](labview-integration.md#hooks). To allow one:

- In a project, put `hooks = "run"` on the package's line in `lvpm.toml`:
  `pilot_a = { version = "*", hooks = "run" }` ([manifest](manifest.md#dependencies)).
- Outside a project, name it in the user config:
  `lvpm config set hooks.run pilot_a` ([Configuration](#configuration)).
- For one command, add `--hooks`. It allows every package.

An install that skips a hook still exits 0. It warns on stderr, once per
package, with the lines to run it now and to allow it for good:

```
warning: pilot_a: skipped install hook PostInstall (hooks are off by default; see docs/labview-integration.md#hooks)
  run it now:     lvpm run-hooks pilot_a
  always run it:  add hooks = "run" to the pilot_a line in [dependencies] of lvpm.toml
```

The allowlist applies from the next install or upgrade. For a package that
is already installed, run `lvpm run-hooks <pkg>`. A venv and a scratch
`--prefix` tree run no hooks, and a headless LabVIEW runs them only with
`--hooks`.

## Starting a project

`lvpm init` starts a project in the current folder (`--project <DIR>` names
another one, which must exist). It reads no package feed and starts no
LabVIEW.

```console
$ mkdir Serializer
$ cd Serializer
$ lvpm init
wrote C:\Git\Serializer\lvpm.toml
  labview = ">=2026"
wrote C:\Git\Serializer\.gitignore for LabVIEW
initialized a git repository
  add a remote:  git remote add origin <url>
  first push:    git add -A && git commit -m "lvpm init" && git push -u origin HEAD
created C:\Git\Serializer\.lv-venv
  bound to       LabVIEW 2026 (64-bit)  v26.3
next: lvpm sync
```

- `lvpm.toml` is named after the folder, at version `0.1.0`, with an empty
  `[dependencies]`. `labview` is `>=` the year of `--labview-version`, else of
  the newest installed LabVIEW; with none installed it is left out and init
  warns. An existing `lvpm.toml` is never replaced, and a `--labview-version`
  that is not a year writes nothing. A `lvpm.toml` in a parent folder does not
  count.
- Git: `git init`, unless the folder is already inside a repository, which is
  left as it is. A missing `git` is a warning.
- `.gitignore`: [GitHub's LabVIEW template](https://github.com/github/gitignore/blob/main/LabVIEW.gitignore)
  (built libraries, `.exe` and `.dll`, `.lvlps` and other metadata) plus the
  venv folder when it is inside the project. An existing `.gitignore` is left
  as it is.
- Venv: bound to the chosen LabVIEW, as `lvpm venv create` would. When it
  cannot be made (no such LabVIEW, or one before 2024 Q1) init says `no venv:`
  and why, and still succeeds; run `lvpm venv create` once you can.
- With several LabVIEWs installed, init lists them. To use another one, delete
  the venv and bind a new one (edit `labview` in `lvpm.toml` too, if it is
  higher than the version you pick):

  ```console
  $ lvpm venv remove
  $ lvpm venv create --labview-version 2025
  ```

  For the other bitness of the same year, add `--labview-bitness 32` (or `64`),
  or give its installation path, which init prints:
  `lvpm venv create --labview-version "<path>"`.

### From a .vipc

`lvpm init --from <file.vipc>` also turns a VIPM configuration into
dependencies. The path is read from the working directory, not from
`--project`. Each package in the `.vipc` is handled by its VIPM flags:

| In the `.vipc`                      | In `lvpm.toml`                                                       |
| ----------------------------------- | -------------------------------------------------------------------- |
| Dependency (of another package)     | Skipped, and counted: the package that needs it brings it            |
| Pinned                              | An exact version: `demo_lib = "1.5.0.15"`                            |
| Anything else                       | A floor: `demo_lib = ">=1.5.0.15"`                                   |
| Included (the archive is inside)    | Imported into the cache, like `lvpm cache add`, unless it is a Dependency; still added or skipped as above |

A package init adds and marks Included whose archive is not in the `.vipc` is a warning,
and one whose archive is not a package, or is another package or version than
the `.vipc` lists, fails. A name listed twice follows its last listing. A
`config.xml` over 1 MiB, an archive over 256 MiB or archives over 1 GiB in
all are refused. The `.vipc`'s own LabVIEW is only
reported (`the .vipc targets LabVIEW 2013; lvpm.toml says >=2026`); `labview`
comes from `--labview-version` or the installed LabVIEW, as above. Run
`lvpm sync` next. A `.vipc` that cannot be read fails and writes nothing. A
`.dragon` file is refused: it is not supported and not planned.

### From a vipm.toml

`lvpm init --from vipm.toml` starts from VIPM 2026's project file (any file named
`vipm.toml`, read from the working directory like a `.vipc`). It is read, never
changed.

```console
$ lvpm init --from vipm.toml
wrote C:\Git\daq-tool\lvpm.toml
  labview = ">=2026"
  the vipm.toml targets LabVIEW 2021; lvpm.toml says >=2026
added 1 dependency from vipm.toml
  labview_open_source_lib_serializer 1.2.1.16
manifest: vipm.toml (VIPM format, converted)
  not used: tool
  [dev-dependencies] read as lvpm's [dev-dependencies]
initialized a git repository
  ...
```

- `[project]` `name` and `version` are kept (the folder's name and `0.1.0` when
  the file has none). `labview-version` is only reported, as a `.vipc`'s LabVIEW
  is: `labview` comes from `--labview-version` or the installed LabVIEW.
- `[dependencies]` and `[dev-dependencies]` keep their exact versions.
  `[nipm.dependencies]` is copied as written; `lvpm sync` reports those packages
  and does not install them.
- What lvpm has no use for is listed on the `not used:` line, as for
  `venv create --manifest` ([the table](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/manifest.md#compatibility-with-vipmtoml)).
  A `vipm.toml` that cannot be converted (a floor such as `">=1.0"` where VIPM
  takes only exact versions, or text that is not TOML) fails and writes nothing.

## The project manifest

`lvpm.toml` at the repo root says what a project depends on and which hosted
indexes to use; `lvpm init` writes the first one ([Starting a project](#starting-a-project)).
Local package folders are configured globally instead. Any
`lvpm` command run at or below it picks it up; hosted `[sources]` apply to
`search` and to a single-package `install` as much as to a full one.
`min-age-days` is the exception: a single-package `install` that is not an
`lvpm add` never applies it (see
[A minimum release age](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/manifest.md#a-minimum-release-age)).

```toml
[project]
name = "NovaXC"
version = "0.1.0"
labview = ">=2025"                # the oldest LabVIEW the project is meant for

[sources]                         # repositories beyond the public indexes
mirror = "http://host:8090/files" # a hosted index.vipr folder
defaults = true                   # false: only the sources listed here

[dependencies]
delacor_lib_qmh = "7.1.2.1547"    # exactly this version
oglib_error = ">=6.0.1"           # the newest version satisfying the floor
jki_lib_state_machine = "*"       # the newest version
hexagon_lib_motion = { version = ">=2.0", index = "mirror" } # only from the mirror source

[dev-dependencies]                # the group dev: test tooling; same syntax; skipped with --no-dev
jki_lib_caraya = "*"

[dependency-groups.ci]            # installed only with --group ci
ni_vi_analyzer_tests = "1.2.0.3"

[nipm.dependencies]               # NI Package Manager packages: reported, not installed
ni-daqmx-labview-support = "26.0"
```

The `hexagon_lib_motion` line carries a source pin: that package comes from
`mirror` and from no other source ([Package sources](#package-sources)).

`labview` is a minimum, not a pin: a venv binds to that version or a newer
one. Dependencies a package carries are always floors, so what a project pins
exactly stays exact and the rest resolves to the newest version that fits.

Before 0.6, `labview` was a bare year. lvpm now refuses `labview = "2025"`
and asks for `labview = ">=2025"`; change the line and nothing else.

## The lock file

`lvpm.toml` is what you ask for, with loose constraints. `lvpm.lock`, beside
it, is what you got: every package, direct or transitive, with its exact
version, SHA-256 and source. lvpm writes it; do not edit it. Commit both.

```console
$ lvpm lock                 # resolve lvpm.toml, write lvpm.lock
$ lvpm sync                 # make the project's venv hold exactly what lvpm.lock lists
```

A teammate or a CI job that syncs the same lock gets the same packages,
byte for byte. A new release of a package never changes the lock; only an
upgrade does.

**Locking.** `lvpm lock` picks, for every package, the newest version that
satisfies all the constraints and runs on the manifest's `labview` (locking
refuses a manifest without one). If the constraints cannot all hold, it names
the ones in conflict and writes nothing. It reads `lvpm.toml` and the package
sources, never what is installed, and leaves a lock that still represents the
manifest as it is. A requirement that no source lists is recorded as
unresolved and reported as a warning; a direct dependency that cannot be found
fails. Constraints are exact (`7.1.2.1547`), `>=` or `*`.

**Out of date.** A lock is out of date when a dependency was added or removed
in any table, a constraint no longer admits the locked version, `labview` changed, the
hosted `[sources]` changed, including their order, a dependency's `github`
repo was added, removed or changed, a dependency's source pin (`index`) was
added, removed or changed, or `[policy]` changed. New releases upstream do not make it so, and
neither does the per-user `sources.local`. A lock written before source pins
existed is not out of date because of them. `lvpm lock --check` writes nothing:
it exits 0 if `lvpm.lock` exists and is current, and otherwise exits non-zero
with the reason on one line. It reads no source, so it is cheap.

**Lock format.** The lock says which groups alone need a package:
`groups = ["ci", "dev"]`; a package that `[dependencies]` needs has no `groups`.
Before groups, a dev-only package was marked
`dev = true`. Such a lock, format 1, is out of date: `lvpm lock --check`,
`lvpm sync --locked` (the default on a headless machine) and `lvpm sync --frozen`
refuse it and name `lvpm lock`. `lvpm lock`, a plain `lvpm sync`, `add` and
`remove` write it again as format 2; commit the result.

**Syncing.** `lvpm sync` installs what is missing, replaces what is at another
version, and uninstalls the lvpm-installed packages the lock does not list. It
prints each change and why before it acts, and does not ask. `--dry-run`
prints that plan and changes nothing, not even the lock. Each archive is taken
from the cache by SHA-256, else from any source with the same name and version
whose SHA-256 matches; one that does not match is never installed. If none
does, sync fails naming the package, version and SHA-256, and tells you to
`lvpm cache add <file>` the archive or point `sources.local` at a folder that
has it. The destination is the project's venv, or the LabVIEW installation
with `--global`. Packages from `[dev-dependencies]` are synced too; `--no-dev`
leaves them out and uninstalls them if they are there. `--group <name>` (repeat
it for more) also syncs a `[dependency-groups.<name>]`, and a package that only
unselected groups need is uninstalled; a name the manifest does not define fails
and lists the defined ones. `--group dev` with `--no-dev` is refused. A bare
`lvpm install` takes the same flags. Groups are described in
[manifest.md](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/manifest.md#named-dependency-groups).

```console
$ lvpm sync                         # [dependencies] and dev
$ lvpm sync --group ci              # and the ci group; CI would run this
$ lvpm sync --no-dev                # [dependencies] only
```

With no lock, or an out-of-date one, `lvpm sync` makes a new one first. To
refuse that instead:

| Flag          | When the lock is missing or out of date                                       |
| ------------- | ----------------------------------------------------------------------------- |
| (none)        | Lock again, then sync                                                         |
| `--locked`    | Fail and change nothing                                                       |
| `--frozen`    | Sync the lock as it is, even out of date; fail only if there is no lock       |
| `--no-locked` | Lock again, even where `--locked` is the default (see [CI](ci-docker.md)) |

### Adding and removing dependencies

```console
$ lvpm add oglib_error              # writes oglib_error = ">=<the version it locked>"
$ lvpm add "gamma@>=0.5"            # with a constraint
$ lvpm add --dev jki_lib_caraya     # into [dev-dependencies]
$ lvpm add --group ci ci_tools      # into [dependency-groups.ci], made if it is new
$ lvpm add gamma --index mirror     # a source pin: gamma comes only from the [sources] entry mirror
$ lvpm remove gamma                 # --dev or --group ci if it is in another table
```

`add` and `remove` edit `lvpm.toml` (comments and layout stay), then lock and
sync. If the lock or the sync fails, `lvpm.toml` and `lvpm.lock` are put back
as they were. A package that another one still requires stays in the lock
after `remove`, and the output says so. They install `[dependencies]`, dev and
the group they edit, and never uninstall a package the new lock still lists, so
what `lvpm sync --group ci` installed stays; plain `lvpm sync` is the exact one.
`--dev` is `--group dev`. A misspelt `--group` on `add` makes a new group, and
says `created [dependency-groups.<name>], a new group`. With `--json` the events
keep `dev` and add `group`, and `created_table`, when they apply. An `added` or
`updated` edit of a pinned line also carries `index`, the source's name.

**A source pin.** `--index <NAME>` writes a source pin, as the table form
`{ version = ">=<the newest version the source lists>", index = "<NAME>" }`, and
takes that version from `<NAME>` alone. The `>=<newest>` applies only when no
constraint is given: `add <name>@<constraint> --index <NAME>` writes the
constraint as given. It works with `--dev` and `--group`.
`<NAME>` must be a key of `[sources]`. If it is not, or the source does not
list the package, `add` fails and puts `lvpm.toml` and `lvpm.lock` back. On a
line that is already a table, `--index` sets or replaces `index` and keeps the
other keys; on a line with `github` it fails, since the two cannot be
combined. `add` without `--index` on a pinned line keeps the source pin and
changes only `version`. No flag removes a source pin: edit the line, or
`lvpm remove` the package. What a source pin does is described in
[Package sources](#package-sources).

**Quote a constraint.** PowerShell and cmd read `>` as a redirect, so
`lvpm add gamma@>=0.5` sends lvpm's output into a file named `=0.5` and hands
lvpm only `gamma@`. Put the argument in double quotes, as in
`lvpm add "gamma@>=0.5"`; that works in both.

In a project, `lvpm install <name>` with no destination flag is `lvpm add
<name>`, and says so in one line; a bare `lvpm install` is `lvpm sync`. With
`--global`, `--prefix` or `--labview-version`, or outside a project,
`lvpm install <name>` is the plain install described above and changes no
manifest or lock.

### Upgrading

Locked versions stay put until you ask:

```console
$ lvpm lock --upgrade                         # everything to the newest version the constraints allow
$ lvpm lock --upgrade-package gamma           # only gamma, to its newest
$ lvpm lock --upgrade-package gamma@1.2.0     # only gamma, to 1.2.0; repeat the flag for more packages
$ lvpm sync
```

With `--upgrade-package`, the other packages stay where the lock has them
unless they cannot. If the manifest's constraint excludes the version asked
for, lvpm rewrites it and keeps its kind: an exact pin becomes the new
version, `>=1.0` becomes `>=` the new version. A `*` is never rewritten.
There is no separate `update` command.

## GitHub dependencies

A project can take one package straight from a GitHub repo's releases:

```toml
[dependencies]
labview_open_source_lib_serializer = { github = "LabVIEW-Open-Source/Serializer", version = ">=1.2" }
```

That package comes from that repo and never from `[sources]`. Which release
file lvpm picks, how its version is read and what the manifest refuses are in
[manifest.md](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/manifest.md#a-dependency-from-github). Edit the line by hand;
`lvpm add` does not write `github`, and on a table line it changes only
`version`.

**Which commands call GitHub.** `lvpm lock`, `lvpm add`, `lvpm remove`,
`lvpm lock --upgrade` and `--upgrade-package` list the repo's releases and
download the file. `lvpm sync` does when the cache lacks the file or it has to
lock first. `lvpm lock --check`, `search`, `info` and `lvpm install <name>`
outside a project never call GitHub.

**The token.** lvpm uses the first of these, and calls the API without one if
none exists:

1. `GH_TOKEN`
2. `GITHUB_TOKEN`
3. the output of `gh auth token`

An empty variable counts as unset. A private repo needs a token that can see
it. lvpm sends the token only to GitHub's API, not to the storage host a
download redirects to, and never writes it to `lvpm.toml`, `lvpm.lock`, the
cache or any message. The API address is `https://api.github.com`, or
`GITHUB_API_URL` when that is set; GitHub Actions sets it.

In GitHub Actions the workflow's token is not in the environment by itself;
pass it with `env: { GITHUB_TOKEN: "${{ github.token }}" }`. It sees only the
workflow's own repository.

**Rate limits.** Without a token GitHub allows 60 requests an hour. When the
rate limit is used up, lvpm fails at once, names the repo, and gives the time
the limit resets when GitHub sends one. Without a token it also says to run
`gh auth login` or set `GH_TOKEN`. It never waits and
retries. A repo's list of releases is cached like a feed index; `--refresh`
and an upgrade of that package list it again.

**Publishing from a tag.** Copy
[`.github/workflow-templates/lvpm-release.yml`](https://github.com/ThatsTheEnd/lvpm/blob/main/.github/workflow-templates/lvpm-release.yml)
from lvpm's repository into `.github/workflows/` of the repository that holds
your package. The repository needs an `lvpm.toml` with `[project]` and
`[distribution]` ([Building a package](#building-a-package)). Pushing a tag
then runs `lvpm build --no-check` and attaches `dist/<name>-<version>.lvpkg` to the tag's
release, creating the release if there is none, and replacing the file if the
tag is pushed again. The template names the lvpm release it builds with in
`LVPM_VERSION`; raise it to use a newer one. The runner has no LabVIEW, so
there is no venv to check the library against: the template skips the check and
prints a `::warning::` that the build is unchecked. Run `lvpm build` on your own
machine to check. Projects depend on the package under its `[project]` `name`.

## [policy]

By default lvpm trusts a GitHub release on first use: the lock pins its
SHA-256 and says nothing about who built it. A `[policy]` table in `lvpm.toml`
asks for more. Copy this and change the repository:

```toml
[policy]
require-signature = true                               # refuse a release with no attestation
trusted-publishers = ["https://github.com/hexagon/*"] # refuse an attestation from any other repository
```

Any other key in `[policy]` is refused.

A signature here is a GitHub artifact attestation: a statement, signed by the
publisher's GitHub Actions workflow, that this workflow built this archive.
`trusted-publishers` lists patterns for the repository that ran the workflow.
In a pattern `*` matches any run of characters except `/`, so
`https://github.com/hexagon/*` is every repository of `hexagon`. Case does not
matter and every pattern starts with `https://github.com/`. A non-empty list
also refuses a release with no attestation, as `require-signature` does.

When `lvpm lock` fetches a GitHub package it asks GitHub whether the archive
has an attestation, and if it has, checks it with `gh attestation verify`. It
writes the workflow as `signer` and the repository as `subject` under the
package in `lvpm.lock`. Needed for that: the GitHub CLI 2.49 or newer
(`gh`), and a token for it, which `gh auth login` or `GH_TOKEN` gives; without
them lvpm cannot check. If anything is refused, `lvpm lock` names every
package and why, writes no lock and exits 1.

- An attestation that `gh` rejects is refused whatever the policy says.
- A release with no attestation, or one that lvpm could not check, is
  refused when the policy requires an attestation; the error says what to do.
- An attestation from a repository that no pattern matches is refused.

**What it covers.** Only packages from a GitHub repo. Packages from a feed,
vipm.io, NI or a local folder are not checked, `[policy]` or not. A package
the lock already holds with a signer is judged again from the lock under the
current policy, with no network; one it holds with no signer is asked again
only when the policy now requires an attestation. So the `signer` and
`subject` in `lvpm.lock` are what lvpm recorded, not proof: review a change to
them in a lock diff as you would a new version. An up-to-date lock is not
checked again, so `lvpm sync` from it checks nothing new; changing `[policy]`
makes the lock out of date, and the next `lvpm lock` checks it again.

**The fail-open limit.** With no `[policy]`, a release lvpm cannot check
(no `gh`, no token, GitHub unreachable or rate limited) is locked with no
signer, and after the summary a warning says which packages and why. To record
a signer later, fix the cause and set `require-signature = true`: the next
`lvpm lock` checks every GitHub package that has none, or delete `lvpm.lock`.

**Publishing.** A release carries an attestation when it is built with the
[release template](https://github.com/ThatsTheEnd/lvpm/blob/main/.github/workflow-templates/lvpm-release.yml)
(see [GitHub dependencies](#github-dependencies)), which attests
`dist/*.lvpkg` before it uploads it. A release made any other way has none and
fails a policy that requires one.

## Per-project dependencies (venvs)

A project can keep its packages to itself, the way a Python virtualenv or
`node_modules` does, instead of sharing one `vi.lib` with every other project
on the machine. [`lvpm init`](#starting-a-project) makes one when it can;
`lvpm venv create` is for a project that has none (a hand-written `lvpm.toml`,
or init said `no venv:`) or after `lvpm venv remove`.

```console
$ cd C:\Git\MyProject
$ lvpm venv create --labview-version 2026
created C:\Git\MyProject\.lv-venv
  bound to       LabVIEW 2026 (64-bit)  v26.3
  VI Server port 3735

$ lvpm install                    # sync: exactly what lvpm.lock lists, into .lv-venv
$ lvpm add oglib_error            # one more dependency: lvpm.toml, lvpm.lock and .lv-venv
$ lvpm launch MyProject.lvproj    # a LabVIEW that sees the venv
```

- `lvpm venv create` makes `.lv-venv/` beside `lvpm.toml` by default and binds it to a
  LabVIEW version: `--labview-version`, else the manifest's `labview`.
  Binding to a newer IDE than the manifest's minimum is fine; an older one is
  refused. The venv directory ignores itself in git; the manifest is what you
  commit. `lvpm venv status` shows the binding, `lvpm venv remove` deletes
  the venv and everything in it.
- No activation step. Like cargo or npm, any `lvpm` command run at or below a
  directory holding a venv uses that venv. `--project <DIR>` names one from
  outside; `--global` says the installation itself is meant. An `lvpm.toml`
  with no venv beside it is an error, never a silent global install, except
  on a headless machine (see [CI in a container](ci-docker.md)).
  Every venv-mode command prints which venv it resolved to.
- `lvpm launch [<lvproj>]` starts a LabVIEW on the venv: a copy of the
  target's `LabVIEW.ini` with the venv mounted and VI Server on a port of its
  own, handed over with `-pref`. Your primary LabVIEW, if open, is untouched.

How it works: the venv directory is an [LVAddons](https://labviewwiki.org/wiki/LVAddons)
location, one addon per package, each mirroring the LabVIEW install dir
under `<pkg>/1/`. A LabVIEW started with `LVAddons.AdditionalLocations`
pointing there overlays them onto its own tree, so a package in the venv
resolves as `<vilib>/...` exactly as a globally installed one would. The VIs
you write against it link portably, and `C:\Program Files` never changes.
Needs LabVIEW 2024 Q1 or later.

Two things follow from LabVIEW reading an addon's contents at launch:

- After `lvpm install`, a LabVIEW already open on the venv cannot see the new
  files. Restart it with `lvpm launch`. For the same reason the relink runs
  in a LabVIEW that lvpm starts fresh on the venv, and is refused while one
  is already open there.
- The real installation always wins over the overlay: a package installed
  both globally and in the venv loads from the global copy, whatever its
  version. Keep the target installation clean of what the venv provides.

### The resave

A venv install opens and saves each package's VIs in the venv's LabVIEW
before the package reaches `.lv-venv`. A VI saved in an older LabVIEW can
otherwise load broken from an add-on folder. The package is unpacked into
`.lv-venv.staging/` beside the venv, resaved there, then moved into place.

```console
starting the venv's LabVIEW for the resave (about a minute per package)
resave (1/2) oglib_error ... 12 saved (48s)
resave (2/2) oglib_string ... already saved in LabVIEW 2026, skipped
```

- It costs one LabVIEW start per run and about 40 to 85 seconds per package,
  mostly the cold load of its VIs. A package whose VIs are all saved in the
  venv's LabVIEW version already is skipped, and starts no LabVIEW.
- `--no-relink` and headless installs skip only the relink. The resave
  still runs, and the relink line says so. `--dry-run` lists the packages with
  VIs it may resave, including ones the version check will skip, and starts
  no LabVIEW.
- A failed resave is a warning, not an error: the package is still
  installed and the exit code stays 0. The warning names the package and its
  folder. Reinstall the package; if it fails again, run `lvpm launch` and
  mass compile that folder. A recap at the end lists every package that
  failed. If the venv's LabVIEW cannot start at all, one warning lists the
  packages installed without a resave.
- Scripts read the failures from `--json`: the install summary carries a
  `resave_failed` list, present only when a package failed.
- A venv installed before this release has packages that were never
  resaved. Delete `.lv-venv` and run `lvpm sync` to reinstall them.

## Building a package

`lvpm build` packages one LabVIEW library as a `.lvpkg`, lvpm's own package
format. It needs no LabVIEW. `lvpm install` reads a `.lvpkg` from every feed
it reads a `.vip` or `.ogp` from. VIPM does not read a `.lvpkg`; it is for
lvpm users ([ADR 6](adr/0006-lvpkg-only-vip-read-only.md)). Describe the
package in `lvpm.toml`:

```toml
[project]
name    = "hse_lib_hse_logger"   # the package name; a scoped name ("@org/x") is refused for now
version = "2.2.2.114"            # 3 or 4 numeric parts; written as it is here
labview = ">=2016"               # becomes the package's LabVIEW gate

[distribution]
top-level-library = "Source/HSE-Logger/hse-logger.lvlib"  # required; relative to lvpm.toml
kinds        = ["source"]                    # optional; only ["source"] for now
vendor       = "Hampel Software Engineering" # required
display-name = "HSE Logger"                  # required
install-dir  = "vi.lib/Hampel Software Engineering/HSE Logger"  # optional; this is the default
```

- The package holds the library's members as the `.lvlib` lists them: its
  VIs and controls, its classes and their members, and every file in an
  auto-populating folder. A file in the library's folder that the library
  does not list is left out. Every member must be on disk and inside the
  library's folder.
- Members install below `install-dir` with the layout they have below the
  library's folder. Without `install-dir` that is
  `vi.lib/<vendor>/<display-name>`.
- `[dependencies]` are written into the package as they are in `lvpm.toml`.
- The package is written to `dist/<name>-<version>.lvpkg`. The same sources
  give the same bytes. An existing file of that name is replaced; a `.vip`
  from an earlier build is left where it is. A failed build names the key
  or file at fault and leaves an earlier package in `dist/` as it was.
- Any other key under `[distribution]` is an error that names the key.
- Paths in `[distribution]` use forward slashes and stay inside their
  folder: `top-level-library` inside the project, `install-dir` below the
  LabVIEW folder. A listed member that is a link to a file outside the
  library's folder is refused; links inside an auto-populating folder are
  skipped.
- `install-dir` is refused if it starts with `/`, has a `\` or a `:`, has an
  empty segment, a segment that is only dots, or a segment that ends in a dot
  or a space. It is refused too if a segment is a Windows device name
  (`CON`, `PRN`, `AUX`, `NUL`, `COM0` to `COM9`, `LPT0` to `LPT9` and their `¹²³` forms, `CONIN$`
  or `CONOUT$`, in any case and with or without an extension). Windows
  would drop or reinterpret those, so the files would not land where the
  package says. Trailing `/` characters are dropped.
- The default `install-dir` goes through the same check. A `vendor` that ends
  in a dot, like `"Acme Inc."`, therefore needs an `install-dir` of its own:
  `install-dir = "vi.lib/Acme Inc/Lib"`.
- The package name takes letters, digits, `_`, `-` and `.`, but not `..`. A quote or line
  break in `vendor` or `display-name` is refused.

**The check.** `lvpm build` checks the library's members first, with the
rules of [`lvpm check`](#checking-a-project), and writes nothing when one
breaks a rule. The members are the files the `.lvlib` lists; a VI the library
does not list is not checked. The check needs a venv, so run
`lvpm venv create` and `lvpm sync` before the first build, or pass
`--no-check`. Without a venv and without the flag, the build exits 1 and says
which two commands to run.

When a member breaks a rule, the build exits 1, `dist/` is left as it was,
and stdout lists what `lvpm check` would list. `VIs checked` counts the
library's members, not every VI in the project:

```console
$ lvpm build
Broken.vi: link not found: Sub/Gone.vi
next: restore the file, or open the VI in LabVIEW and relink it
VIs checked: 3, problems: 1, links not checked: 0
```

The last line on stderr is `build refused: 1 problem(s) in library members,
nothing written; fix them or pass --no-check`. With `--json` the result is
`check`'s, with `"command": "build"`.

- A package in `[dependencies]` that no member links to is a warning on
  stderr, and the build goes on:
  `[dependencies] names spare, which no member links to (ignore this if it is loaded by path at run time)`.
  It is not printed when a link was not checked, such as an absolute path,
  since that link may lead into the package.
- A link from a member into a package that is not in `[dependencies]`, or only
  in `[dev-dependencies]`, is a finding and refuses the build.
- `lvpm build --no-check` skips the check. It needs no venv, and stderr says
  `check skipped (--no-check)`. Use it where there is no venv, such as a CI
  runner without LabVIEW; check the library with `lvpm check` or a plain
  `lvpm build` on a machine that has the venv.
- A VI member (`.vi .vit .vim .ctl .ctt`) is checked wherever the library lists it, under a dot-folder or
  behind a link too, since the build packages it there. Only `lvpm check`,
  which walks a folder, skips such names.

### Installing a .lvpkg

A `.lvpkg` lists the SHA-256 of every file in it (`RECORD`), and lvpm checks
the list before it writes anything. A file that is missing from the list, a
listed file the archive lacks, or a hash that disagrees makes the package
fail, and the message names the file. What happens next depends on where the
package came from:

- In a package folder (`sources.local`), the file is ignored with a warning
  that names the bad entry. If nothing else provides the package, the
  install fails because the package is not found.
- From a hosted feed, or with `lvpm cache add`, the file is refused and
  nothing is written.

A package folder, `lvpm cache add` and a hosted `index.vipr` all take a
`.lvpkg` beside a `.vip`. lvpm tells the format from what is inside the
archive, not from its extension; the extension only picks which files a
package folder and `lvpm cache add` look at.

### Upgrading from [distribution.vip]

`formats` and the `[distribution.vip]` table are gone: lvpm writes a
`.lvpkg` only, and a manifest that still has either fails and names the key.
`vendor`, `display-name` and `install-dir` moved up into `[distribution]`.
The package name is `[project] name`; `name`, `icon` and `dependency-names` in the
old table have no replacement, since a scoped name is refused for now.

Before:

```toml
[distribution]
top-level-library = "Source/HSE-Logger/hse-logger.lvlib"
formats = ["vip"]

[distribution.vip]
vendor       = "Hampel Software Engineering"
display-name = "HSE Logger"
install-dir  = "vi.lib/Hampel Software Engineering/HSE Logger"
```

After:

```toml
[distribution]
top-level-library = "Source/HSE-Logger/hse-logger.lvlib"
vendor       = "Hampel Software Engineering"
display-name = "HSE Logger"
install-dir  = "vi.lib/Hampel Software Engineering/HSE Logger"
```

Check `install-dir` against the rules above when you move it: an old one
that VIPM accepted may now be refused.

## Publishing a package

`lvpm publish --to <folder>` copies the package `lvpm build` wrote for the
current manifest, `dist/<name>-<version>.lvpkg`, into a folder feed. Another
project that lists that folder in `sources.local` then locks and installs
it.

```console
$ lvpm build
$ lvpm publish --to D:/feeds/packages
published D:\feeds\packages\demo_lib-1.2.3.lvpkg
```

- `--to` is required. The folder must exist; a relative path is read from
  where the command runs, not from the project.
- A published file is never replaced. If the folder already holds a file of
  that name, publish fails, names it, and leaves it as it was: give the
  project a new version and build again.
- Only the current version is published. Older packages in `dist/` stay
  where they are.
- Publish does not build. With no package in `dist/` for the current
  manifest it fails and says to run `lvpm build`.
- The copy is written beside its final name and linked into place, so a
  reader of the folder never sees half a package. That needs a file system
  with hard links (NTFS and ext4 have them); on one without them, publish
  fails and names the folder.

## Checking a project

`lvpm check` reads every VI under a folder as bytes and starts no LabVIEW, so
it runs on a machine that has none. It needs the project's venv
(`lvpm venv create`): the venv says which LabVIEW installation `vi.lib` links
are looked for in, and which package owns which file.

```console
lvpm check
lvpm check Source
lvpm check --json
```

`DIR` is read from where the command runs; with none, it is the whole project.
The walk takes `.vi`, `.vit`, `.vim`, `.ctl` and `.ctt` files, and skips the venv and its
staging folder, every name that starts with a dot, and links.

**The rules.** Each finding is one VI breaking one rule; a rule broken twice
by the same link is one finding.

- **Save version.** A VI may not be saved in a LabVIEW newer than the minimum
  in `[project] labview` (`>=2025` is 2025). Without `labview`, the version is
  not checked, and stderr says so.
- **Separated compiled code.** Each VI must have "Separate compiled code from
  source file" turned on.
- **Links.** Every file a VI links to must be there. A relative link is read
  from the VI file's folder; a `..` moves up. `<vilib>`, `<userlib>` and
  `<instrlib>` links are looked for in the LabVIEW installation the venv is
  bound to first, then in each venv package, as LabVIEW does. A link through an
  `.llb` needs the `.llb` to exist and list that member. The path has to
  exist exactly: a file somewhere below an existing folder is not enough. A
  link below a file that is there leads into it (`Logger.lvclass/Logger.ctl`,
  a class's control stored in its `.lvclass`) and is found; the folder form
  fails. An absolute link is looked for as written, and one whose file is not
  on this machine is not checked (see below).
- **Dependencies.** A link into a file that a venv package installed belongs
  to that package, and the VI may use it only if `lvpm.toml` lists the package
  where that VI may draw on it. The VIs of the `[distribution]`
  `top-level-library` (its members) may use `[dependencies]` only; any other VI,
  such as a test, may use every table (`[dev-dependencies]` and the
  `[dependency-groups]`). Without `[distribution]` no VI is a member. A package
  that is only a dependency of a dependency does not count: list it. A file the
  installation itself holds is LabVIEW's own and is always allowed. `lvpm.lock`
  is not read.
- **Readable.** A file that is not a LabVIEW resource file, or whose link table
  lvpm cannot read, is a finding, never a pass.

**The output.** One line per finding on stdout, sorted by VI path, then rule,
then link. Paths are relative to the project and use `/`. A link is shown as
stored for `<vilib>` and the other roots, and otherwise as the path it led to.

```text
Broken.vi: link not found: Sub/Gone.vi
Garbled.vi: linker table not readable (LIvi at byte 1234): format not supported yet
New.vi: saved in LabVIEW 2026, newer than the minimum 2025
NotSeparated.vi: compiled code is not separated from the source
UsesDep.vi: <vilib>/Check Fixture/Dep.vi is in check_fixture_dep, which is not in [dependencies]
UsesDevOnly.vi: <vilib>/Check Fixture Dev/DevDep.vi is in check_fixture_other, which is only in [dev-dependencies]
x.vi: not a LabVIEW resource file
```

A package in a dependency group reads `only in [dependency-groups] ci`. Then
one `next:` line per rule that occurred (the undeclared ones, one per package:
`next: add <pkg> to [dependencies] in lvpm.toml`), and always the summary:

```text
VIs checked: 12, problems: 3, links not checked: 57
```

The exit code is 1 when there is a finding and 0 when there is none. With
`--json` the result has the same shape either way:
`{"ok": bool, "vis": N, "not_checked": K, "findings": [{"vi", "kind", "link", "package", "table", "saved", "minimum", "block", "offset"}]}`;
a field that does not apply is `null`.

**Links lvpm cannot look for** are counted in `links not checked`, with a
warning on stderr that says why (`warning: links not checked: 57 into vi.lib (the
bound installation ... is not on this machine)`, `... into <resource> (lvpm
does not resolve this root)`, or `warning: links not checked: 7 absolute paths
not on this machine (saved elsewhere; LabVIEW searches for them when it
loads)`). In pilots A and B such a path is where the file was on the machine
that saved the VI, and LabVIEW 2026 finds the file by name when it loads the
VI, so it is not a broken link here. A network path, or a name that would leave
its folder (`..`, a separator), is counted the same way and never looked for.
The warnings are text output only; `--json` carries the count, not the cause. A venv package's own files are still checked when the
installation is absent.

**Limits.**

- A VI saved while one of its subVIs was missing may store that link as "not a
  path", which names nothing. `lvpm check` skips such a link; it cannot see it.
- The older `PTH0` layout of some OpenG packages is not read yet
  ([#108](https://github.com/ThatsTheEnd/lvpm/issues/108)).
- A link below a file is found when the file is there; `lvpm check` does not
  open the file to see that the item is in it (only an `.llb` is read).

## Package sources

Two public, anonymous, plain-HTTP package directories, which between them
cover roughly 98% of the packages listed on vipm.io:

| Source                                                    | Packages |
| --------------------------------------------------------- | -------- |
| `download.ni.com/evaluation/labview/lvtn/vipm/index.vipr` | ~2,530   |
| `www.jkisoft.com/packages/jkisoft.ogpd`                   | ~2,290   |

Both are OGPM's Package Directory format, `[Package <name>-<version>]`
sections with a `Package.URL`, unchanged since `openg.ogpd` in 2004; `.vipr`
adds an MD5 per entry. Anything they lack can come from an index folder of
your own, named in the manifest's `[sources]`. A `[sources]` entry is an
`http://` or `https://` folder holding an `index.vipr`. Local directories of
`.vip`, `.ogp` and `.lvpkg` files are configured globally with `lvpm config set sources.local`
instead; lvpm reads their specs and package files directly. These local
directories reuse cached spec metadata when file size and modification time
match; installation still reads their original archives unless imported.

**The first source wins.** A package name comes from exactly one source: the
first one, in this order, that lists it, unless the name has a source pin
([below](#package-sources)).

1. Packages imported with `lvpm cache add`.
2. Each `sources.local` folder, in config order.
3. Each `[sources]` entry, in file order.
4. The public NI index, then JKI's, unless `defaults = false`.

A later source never replaces a name and never adds versions to it, even a
higher one. If the first source has no version that fits, or cannot be
reached, the command fails and names that source. `lvpm search` and
`lvpm info` show only the versions that source lists. To take a package from
another source, list that source earlier.

**Pinning a package to one source.** Listing a source first protects a package
only while every manifest keeps that order. To make the source part of the
dependency, name it with `index`, a key of `[sources]`:

```toml
[sources]
internal = "http://host:8090/files"

[dependencies]
hexagon_lib_motion = { version = ">=2.0", index = "internal" }
```

`hexagon_lib_motion` is now pinned to one source: it comes from `internal` or
not at all. If `internal` does not list it, lists no version that fits, or
cannot be reached, lvpm fails and names the package and the source. It never
falls back to another source, even one that lists a higher version. A source
pin beats a sources.local folder and a cache import: their copies of the
name are ignored, so to try a local build of a pinned package, remove the
`index` key in `lvpm.toml`. Each such edit makes the lock out of date, so lock
again before the next `sync`.

- `lvpm add <name> --index <NAME>` writes the source pin
  ([Adding and removing dependencies](#adding-and-removing-dependencies)).
- A name has one source pin. The line pins the name wherever it is listed, so
  the same name pinned to two sources in two tables is refused, and an
  unpinned line for that name in another table follows the pin.
- The source pin does not reach the package's dependencies. What `hexagon_lib_motion` requires
  follows the first-source-wins rule above. To pin a dependency of a package
  too, list it as a direct dependency with `version = "*"` and its own `index`.
- `index` and `github` cannot be on one line. `<NAME>` must be a key of
  `[sources]`, not `defaults` or `min-age-days`, or the manifest is refused
  when it is read. The public indexes have no name in `[sources]`, so they
  cannot be pinned to; list their URL as a source to do that.
- `lvpm search` and `lvpm info` in the project show a pinned package's versions
  from its source only. Adding, changing or removing a source pin makes the lock
  out of date.

Use `lvpm cache add` to import a `.vip`, `.ogp` or `.lvpkg`, or all such files directly
inside a directory, into the shared content-addressed cache:

```powershell
lvpm cache add D:\LabVIEW\Packages\some_lib-1.2.3.vip
lvpm cache add D:\LabVIEW\Packages
lvpm --refresh cache add D:\LabVIEW\Packages
```

Imports record SHA-256, MD5, package metadata, and the original path. Repeated
imports skip unchanged files when their cached blob still exists; `--refresh`
reads and hashes them again. Imported packages are available to search and
installation even after the originals are deleted. Cache reads verify the
SHA-256 and installation also checks MD5. Local index files and cache
list/remove/prune commands are not implemented yet.

Remote package installs check the cache by name, version, and the feed's MD5
before downloading. A single distinct SHA-256 match is verified and reused,
including archives imported from disk. Missing, corrupt, or ambiguous matches
are downloaded and verified, then stored with SHA-256 and MD5 for future runs.
Packages without a feed MD5 are stored but downloaded again on later installs:
name/version alone does not identify their bytes. `--refresh` refreshes feeds,
not already verified package blobs. Dry runs may populate the cache without
writing package files into the target.

Remote feeds, downloads, and imported packages are recorded in `%LOCALAPPDATA%\lvpm\cache`, in a
SQLite index (`lvpm-cache.db`) over a content-addressed store of the bodies
themselves. For feed bodies, without `--refresh` the stored body is used and
no feed request goes out. With it, the `ETag` and `Last-Modified` the server last gave
are sent back as a conditional request: only a feed that actually changed is
downloaded again, and an unchanged one answers `304` and costs no bytes. See
[Configuration](#configuration) to move the cache.

## Configuration

Settings are named the way npm names `npm_config_<key>`: the environment
variable is `LVPM_CONFIG_<KEY>` (or lowercase `lvpm_config_<key>`), and an
empty value counts as unset. Precedence, highest first:

1. Environment variable `LVPM_CONFIG_<KEY>`.
2. User config file at `%LOCALAPPDATA%\lvpm\config.toml` (Linux:
  `$XDG_CONFIG_HOME/lvpm/config.toml`, else `~/.config/lvpm/config.toml`),
  written by `lvpm config set`.
3. Built-in default.

| Variable | Default | What it sets |
| --- | --- | --- |
| `LVPM_CONFIG_CACHE` | `%LOCALAPPDATA%\lvpm\cache` (Linux: `$XDG_CACHE_HOME/lvpm/cache`, else `~/.cache/lvpm/cache`) | Where the cache DB, feed bodies, and imported package archives live |
| `LVPM_CONFIG_SOURCES_LOCAL` | Empty | Local package directories (`.vip`, `.ogp`, `.lvpkg`); separate paths with `;` on Windows or `:` on Linux |
| `LVPM_CONFIG_VENV_DIR` | `.lv-venv` | Project-relative venv directory (or an absolute path) |
| `LVPM_CONFIG_HOOKS_RUN` | Empty | Packages whose install hooks may run in an install outside a project ([Install hooks](#install-hooks)); separate names with commas or spaces |

```powershell
lvpm config set sources.local D:\LabVIEW\Packages
lvpm config get sources.local
lvpm config set sources.local                 # clear configured local sources
lvpm config set venv.dir .venv
lvpm config get venv.dir
lvpm config set hooks.run pilot_a pilot_b     # replaces the list; no names clears it
lvpm config get hooks.run
# Or set LVPM_CONFIG_SOURCES_LOCAL for this shell; it overrides config.toml.
$env:LVPM_CONFIG_SOURCES_LOCAL = 'D:\LabVIEW\Packages;E:\MorePackages'
# Relative venv.dir values resolve from each project root; absolute paths are used as-is.
$env:LVPM_CONFIG_VENV_DIR = '.venv'
lvpm search caraya
```

`LVPM_CONFIG_HOOKS_RUN` wins over the file. While it is set, the
`lvpm config set hooks.run ...` line in an install hook warning changes the
file but not what lvpm uses: unset the variable or edit its value.

In CI, set it on the job to keep the cache on a persistent volume between
runs.
