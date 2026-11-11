# Using lvpm

## Commands

| Command                                              | What it does                                                                                                                                 |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `lvpm init`                                          | Start a project here: write an `lvpm.toml` and a `.gitignore`, make a git repository and a venv ([Starting a project](#starting-a-project)); an existing `lvpm.toml` is never replaced |
| `lvpm init --from <file.vipc>`                       | The same, with the packages VIPM chose in a `.vipc` as dependencies ([From a .vipc](#from-a-vipc))                                           |
| `lvpm init --from vipm.toml`                         | The same, with the project and packages of a VIPM 2026 `vipm.toml` ([From a vipm.toml](#from-a-vipmtoml))                                    |
| `lvpm install <name>[@<version>]`                    | Outside a project, or with `--global`, `--prefix` or `--labview-version`: resolve the package and its dependencies, download, verify MD5, unpack, run linkfix, run the hooks of packages you allowed ([Install hooks](#install-hooks)). In a project it is `lvpm add` |
| `lvpm install`                                       | Inside a project: the same as `lvpm sync`                                                                                                    |
| `lvpm sync`                                          | Make the destination hold exactly what `lvpm.lock` lists, one plan with one linkfix pass ([The lock file](#the-lock-file))                    |
| `lvpm add <name>[@<constraint>]` / `--dev` / `--group <name>` / `--index <NAME>` | Add a dependency to `lvpm.toml`, lock and sync                                                                                      |
| `lvpm remove <name>` / `--dev` / `--group <name>`    | Remove a dependency from `lvpm.toml`, lock and sync                                                                                          |
| `lvpm lock` / `--check` / `--upgrade`                | Write `lvpm.lock`, check it, or move packages to newer versions                                                                              |
| `lvpm uninstall <name>` / `--all` / `--with-deps` / `--keep-deps` | Remove exactly the files that were installed and prune empty folders. A named package can take the dependencies nothing else needs with it: it asks, and a script passes `--with-deps` or `--keep-deps` ([Uninstalling](#uninstalling)) |
| `lvpm list`                                          | What lvpm has installed into the selected target, and which hooks it did not run                                                             |
| `lvpm search <query>` / `--all [query]`              | Search the package indexes; `--all` lists every version of every package and its source                                                      |
| `lvpm info <package>[@<version>]`                    | Show a package's description, requirements, files and versions, from its archive (downloaded, then cached)                                   |
| `lvpm targets`                                       | Detected LabVIEW installations                                                                                                               |
| `lvpm tui`                                           | Browse the catalog and install packages in a terminal UI; in a project's venv an install is `lvpm add`: a pick from the list adds a floor (`>=version`) to `lvpm.toml`, a version picked in the details is pinned exactly; takes no options |
| `lvpm linkfix` / `--tree <DIR>` / `--dry-run` / `--verbose` / `--fix <methods>` / `--keep-compiled-code` / `--report <file>` | Repair the links and separate the compiled code of every LabVIEW file in the venv, or under `--tree`, without LabVIEW ([Linkfix](#linkfix)) |
| `lvpm relink <name>` / `--all`                       | Rerun the relink pass from the install manifests, without reinstalling                                                                       |
| `lvpm run-hooks <name>` / `--all`                    | Run a package's PostInstall hook now, for one that failed or was skipped during install ([Install hooks](#install-hooks))                    |
| `lvpm cache refresh`                                 | Fetch the package indexes again and re-read the local folders, so the next command sees new versions ([Refreshing the index cache](#refreshing-the-index-cache)) |
| `lvpm cache add <path>`                              | Import a `.vip`, `.ogp` or `.lvpkg`, or every one in a folder, into the cache ([Package sources](#package-sources)) |
| `lvpm refresh`                                       | Rebuild palettes and the File/Tools/Help menus from disk (`install` does this itself)                                                        |
| `lvpm start`                                         | Start the selected LabVIEW and wait until its VI Server answers                                                                              |
| `lvpm venv create` / `status` / `remove`             | Manage the project's venv (see [Per-project dependencies](#per-project-dependencies-venvs))                                                  |
| `lvpm launch [<lvproj>]`                             | Start a LabVIEW on the project's venv                                                                                                        |
| `lvpm build`                                         | Check the library's members as `lvpm check` does, then package the library as `dist/<name>-<version>.lvpkg`, from `[distribution]`; `--no-check` skips the check ([Building a package](#building-a-package)). Then build the application `[application]` names in the venv's LabVIEW and write its SBOM; `--package` or `--app` builds one kind only ([Building an application](#building-an-application)) |
| `lvpm publish --to <folder>`                         | Copy the built package into a folder feed; a published file is never replaced ([Publishing a package](#publishing-a-package))                |
| `lvpm publish --to oci://host/prefix`                 | Push the built package to an OCI registry as `<prefix>/<name>:<version>`; a published tag is never replaced ([OCI registries](#oci-registries)) |
| `lvpm check [DIR]`                                   | Read every VI under `DIR` (the project by default) without LabVIEW and name each one saved in a newer LabVIEW than `labview`, with compiled code not separated, or with a link to a file that is not there or into a package the manifest does not let that VI use, or that lvpm cannot read as a LabVIEW resource file; exits 1 when it finds any ([Checking a project](#checking-a-project)) |
| `lvpm index add <entries-dir> <package> --github <owner/repo> --url <url>` | List a package's file in `<entries-dir>/<name>.toml` of a community index checkout: a new file for a new name, or one more version in the file its owner has. Refuses, and writes nothing, when `--github` is not `owner/repo`, the name belongs to another repo, the version is listed, `--url` is not a file of the owner's GitHub releases, `<package>` is not a package, the entry file does not parse or would pass its size limit, or `<entries-dir>` is not a folder. Do not run it twice at once on one checkout. The `register` job of the release template runs it ([Publish to the community index](#publish-to-the-community-index)) |
| `lvpm index check --base <dir> --head <dir> [--author <login>]` | The community index bot's verdict on a PR: `<head>` is the PR's checkout, `<base>` the index it targets, `<login>` the PR's author. Prints `accepted <name> <version>` for each new version that passes every rule and a `refused ...` line with a `fix:` for each rule it breaks, and exits 1 if there is any. A new name that vipm.io lists is refused, and the check fails if the public indexes cannot be read or list nothing. A version whose owner the author is not (the owner's login, or a public member of the owning organisation) gets a `review ...` line instead, and so does an entry file of another owner that the PR rewrites in any way (a reordered key, the case of `name`), as does every version and rewritten file when there is no `--author`. Reads the checkouts as data and runs nothing of the PR; downloads each new version from its owner's releases with no token, and needs `gh` and a GitHub token to check its attestation. With `--json` the result carries the `verdict` (`merge`, `refuse` or `review`) and the comment to leave on a PR that is refused or left for review ([Publish to the community index](#publish-to-the-community-index)) |
| `lvpm vi-probe` / `lvpm vi-run` / `lvpm vi-save` / `lvpm app-probe` | VI Server primitives, for diagnosis (see [VI Server](labview-integration.md#vi-server))                                                                            |

Flags that apply to every command:

| Flag                       | Meaning                                                                                                   |
| -------------------------- | --------------------------------------------------------------------------------------------------------- |
| `--labview-version <YYYY>` | Install into this LabVIEW version (see `lvpm targets`)                                                    |
| `--project <DIR>`          | Use that project's venv instead of looking upwards from the current directory; for `init`, the folder to write the project into |
| `--global`                 | Use the LabVIEW installation itself, even from inside a project that has a venv                           |
| `--prefix <DIR>`           | Install into a scratch directory instead of LabVIEW (files and linkfix only; nothing to relink or run hooks with) |
| `--refresh`                | Ask each index whether it changed (a conditional request) instead of trusting the cache in `%LOCALAPPDATA%\lvpm\cache` |
| `--yes` / `-y`             | Answer yes to every prompt (`venv remove`, and `uninstall` about dependencies). `LVPM_YES` does the same; `0`, `false`, `no`, `off` or empty leaves it off |
| `--labview-bitness <32\|64>` | Use only 32- or 64-bit LabVIEW installations, to pick one when both of a version are installed; a venv bound to the other bitness is an error (`--global` skips the venv) |
| `--json`                   | Write JSON Lines to stdout for a script or front end ([JSON output and exit codes](https://github.com/ThatsTheEnd/lvpm#json-output-and-exit-codes)). Not taken by `app-probe`, `vi-probe`, `vi-save` or `vi-run` (exit 2) |

`install` flags: `--dry-run` shows every file, relink folder and hook and
changes nothing; `--no-deps` skips dependencies and `--allow-downgrade`
permits replacing an installed package with an older version (both only for
an install that is not a sync or an `add`: a lock always holds the full set);
linkfix runs over the LabVIEW files the install wrote and `--no-linkfix`
skips it ([Linkfix](#linkfix)); `--relink` runs LabVIEW's load-and-save relink
pass instead of linkfix (the hidden `--no-relink` is accepted and ignored);
`--hooks` runs the install
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
  `pilot_a = { version = "*", hooks = "run" }` ([manifest](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/manifest.md#allowing-install-hooks)).
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
To give the package a name that projects find without knowing the repo, see
[Publish to the community index](#publish-to-the-community-index).

## Publish to the community index

The community index is the repository
[`labview-package-manager/package-index`](https://github.com/labview-package-manager/package-index):
one file per package name, which says where each version's `.lvpkg` is. The
files themselves stay on your GitHub Releases. A project that lists the index
as a source ([A github: source](#a-github-source-is-a-community-index))
installs the package by its name alone.

To register a package, do these four steps once. Every later tag adds a version
with no further step:

1. Give the repository an `lvpm.toml` with `[project]` and `[distribution]`, as
   `lvpm build` needs ([Building a package](#building-a-package)):
   `[distribution]` must set `vendor` and `display-name`. The `[project]`
   `name` is the package's name in the index.
2. Copy
   [`lvpm-release.yml`](https://github.com/ThatsTheEnd/lvpm/blob/main/.github/workflow-templates/lvpm-release.yml)
   into `.github/workflows/` of the repository ([GitHub dependencies](#github-dependencies)).
3. Save a GitHub token as the repository's Actions secret `LVPM_INDEX_TOKEN`.
   Use a **classic** personal access token with the `public_repo` and
   `workflow` scopes. The `register` job forks the index and opens a pull
   request from your account. The `workflow` scope is needed because the
   branch starts at the index's tip: once the index has a workflow file that
   your fork lacks, GitHub refuses the push of a token without it. A classic
   token with these scopes is the tested choice. A fine-grained token may not
   work; it was not tested.
4. Push a tag.

The pull request must come from the owner's account: the account that owns the
package's repository, or a public member of the organisation that owns it (a
member makes the membership public in the organisation's People page). The
`register` job opens it from the account of `LVPM_INDEX_TOKEN`, so save the token
of such an account. A name that vipm.io already lists cannot be registered:
pick another `[project]` `name`.

The template's `register` job then runs `lvpm index add` on the package that
the `publish` job attached to the tag's release, pushes the entry to your fork
of the index, and opens a pull request. Its summary shows the pull request's
link. "Listed" below means present in the index. Without the secret, the job
does nothing and its summary says `not registered in the community index:
LVPM_INDEX_TOKEN is not set`, so a repository that copied the template before
this step existed sees no change. The `LVPM_VERSION` of the template must be
an lvpm release that has `lvpm index add`; raise it if the job fails there. To
register in another index, set the repository variable `LVPM_INDEX_REPO` to
its `owner/repo`. If your package's repository is itself a fork, GitHub turns
Actions off in it: enable them in its Actions tab, or the tag runs nothing.

**Protect the token.** A classic `public_repo` token can write to every
public repository its account can, and with `workflow` it can push workflow
files to every public repository you can push to. The bot merges only a pull
request from the owner's account, so the token's account decides who merges:

- A repository owned by your own account: use your own classic token
  (`public_repo` and `workflow`), and keep the secret in a GitHub Environment
  with a deployment protection rule, such as a required reviewer, and name
  that environment on the `register` job. Also limit the environment's
  deployment branches and tags to your release tags (for example `v*`), so a
  workflow on another branch cannot use the secret. Or accept that a maintainer merges
  the pull request by hand: a pull request from any other account, a machine
  account included, is left for a maintainer.
- A repository owned by an organisation: a machine account with no other
  repositories works, once it is a public member of the organisation.

The template does not set an environment; it is optional and yours to add:

```yaml
  register:
    environment: package-index
```

**What the bot decides.** A bot in the index repository checks each pull
request, reading it as data: the name, the file's SHA-256 and the attestation
that the template's `publish` job made ([`[policy]`](#policy)). It merges a pull request that passes every rule. It
comments the reasons on one it refuses, and leaves it open. A pull request
from an account that is not the owner (or a public member of the owning
organisation) is not refused and not merged: the bot comments who owns the
name, and a maintainer decides. So is a pull request that also rewrites an
entry file of another owner, whatever the change. A new name that vipm.io
lists is refused. The first repository to register a name owns it; only that repository's attested
releases add versions, and a listed version never changes. One pull request
adds at most 20 versions and 20 entry files, an entry file is at most 1 MiB,
and a name lvpm cannot install is refused.

**Who closes a refused PR.** The publisher closes it and replaces it by
pushing a new tag: the fix is a new version, which opens a new pull request on
its own branch, `lvpm/<owner>/<repo>/<tag>` (the repository that publishes,
and the tag, so two repositories of one account never replace each other's
pull request). Re-running the `register` job of a tag replaces that branch
and so updates its pull request. A maintainer of the index can close a stale
one.

When the job fails, its summary says why in a line that starts `register
failed:` and ends with what to do: save a classic token with `public_repo`,
add the `workflow` scope to the token, push a new tag for a version that is
already listed, publish from the repository that owns the name, or read the
job log.

A consumer adds two lines and nothing else:

```toml
[sources]
community = "github:labview-package-manager/package-index"

[dependencies]
labview_open_source_lib_serializer = ">=1.2"
```

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

**What it covers.** Only packages from a GitHub repo or an OCI registry (see
[Signatures on OCI packages](#signatures-on-oci-packages) for the difference).
Packages from a feed,
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
  `--ini KEY=VALUE` (repeatable) writes extra keys into the venv's
  `labview.ini` first, for diagnosis: `--ini NIER=True` for a crash minidump,
  `--ini DPrintfLogging=True` for a verbose log. They stay in that ini until
  you remove them; the installation's `LabVIEW.ini` is never touched.

How it works: the venv directory is an [LVAddons](https://labviewwiki.org/wiki/LVAddons)
location (its `packages/` folder), one addon per package, each mirroring the
LabVIEW install dir under `packages/<pkg>/1/`. The binding (`venv.json`), the
install records (`installed/`) and the venv LabVIEW's own state
(`labview.ini`, `objcache/`, `data/`, `temp/`) sit beside it. A venv made by
an older lvpm, with these under `.lvpm/`, is refused: close its LabVIEW, run
`lvpm venv remove` and `lvpm venv create`, then `lvpm install`. A LabVIEW started with `LVAddons.AdditionalLocations`
pointing there overlays them onto its own tree, so a package in the venv
resolves as `<vilib>/...` exactly as a globally installed one would. The VIs
you write against it link portably, and `C:\Program Files` never changes.
Needs LabVIEW 2024 Q1 or later.

Two things follow from LabVIEW reading an addon's contents at launch:

- After `lvpm install`, a LabVIEW already open on the venv cannot see the new
  files. Restart it with `lvpm launch`. For the same reason the resave (and the
  relink, with `--relink`) runs in a LabVIEW that lvpm starts fresh on the venv, and is refused while one
  is already open there.
- The real installation always wins over the overlay: a package installed
  both globally and in the venv loads from the global copy, whatever its
  version. Keep the target installation clean of what the venv provides.

### Linkfix

`install`, `sync` and `add` run linkfix after the files are copied and
before the PostInstall hooks. It edits only the LabVIEW files that install
wrote into the LabVIEW tree, and needs no LabVIEW:

- it repairs owner paths, relative paths, build-machine absolute paths and
  library qualifiers, so the VIs find each other where they were installed;
- it separates compiled code from source, so LabVIEW does not ask to save
  files it only loaded.

```console
linkfix: preparing installed files (repair links and separate compiled code; no LabVIEW needed)

120 LabVIEW files looked at: 87 changed, 33 already clean, 0 skipped
```

A file whose layout linkfix does not know is skipped and reported, not
guessed. A failed pass is a warning: the files stay as copied. `--no-linkfix`
skips the pass; `--relink` runs LabVIEW's relink pass instead.

`lvpm linkfix` runs the same edits on demand, over every package in the venv
or over a LabVIEW-shaped `--tree <DIR>`. `--dry-run` changes nothing,
`--verbose` names every changed and skipped file, `--fix owner,paths` picks
methods (`owner`, `paths`, `qualifiers`, `source-only`; all by default),
`--keep-compiled-code` leaves compiled code embedded, and `--report <file>`
writes a JSON report of each file. See [linkfix](linkfix.md) for what each
method changes.

### Uninstalling

`lvpm uninstall <name>` removes the package's files. When it has
dependencies that no other installed package and no entry of `lvpm.toml`
needs, it asks whether to remove them too:

```console
remove unshared dependencies (alpha) too? [y/N]
```

`--with-deps` removes them and `--keep-deps` keeps them, without asking.
Without a terminal to ask, one of the two is required. A dependency that
stays is named with the reason, and uninstall warns when the package is
declared in `lvpm.toml` (it does not edit the manifest) or other installed
packages still require it. A package installed by an older lvpm has no
dependency list; removing dependencies is then refused until it is
reinstalled, and `--keep-deps` still works.

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
- The relink pass is off unless `--relink` is given. The resave still
  runs, and the relink line says so. `--dry-run` lists the packages with
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
lvpm users ([ADR 6](adr/0006-lvpkg-only-vip-read-only.md)). To build an
application instead, see [Building an application](#building-an-application).
Describe the package in `lvpm.toml`:

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

**The SBOM.** The package carries `bom.cdx.json`, a CycloneDX 1.5 SBOM of
the `[dependencies]` in `lvpm.lock` ([the spec](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/specs/lvpm-app-build-s9.md)):
each locked package with its version, its SHA-256 and the source it came from,
and what it requires. A package that only `[dev-dependencies]` or a
`[dependency-groups]` table needs is left out. The same lock gives the same
bytes. `lvpm install` does not install the file. stdout says
`built <path> (N files, with SBOM)`.

- A project with no `[dependencies]` gets an SBOM that lists no packages,
  whatever the lock: missing, out of date or unreadable. The lock is not read.
- A project with `[dependencies]` and no `lvpm.lock`, a lock that cannot be
  read, or a lock that is out of date, still builds. The package has no SBOM,
  and stderr says why, after the check's own warnings and once the package is
  written:
  `no lvpm.lock, so the package has no SBOM; run lvpm lock to add one`,
  `lvpm.lock cannot be read (<reason>), so the package has no SBOM; run lvpm lock to add one` or
  `lvpm.lock is out of date (<reason>), so the package has no SBOM; run lvpm lock to add one`.
  Run `lvpm lock`, then build again.
- `--no-check` keeps the SBOM.
- A package with an SBOM says `format = 2` in its `package.toml`; one without
  stays `format = 1`. **These packages need an lvpm release that includes this change.** lvpm
  0.24 and older stop on them with
  `package.toml format = 2 was written by a newer lvpm; update lvpm`. Update
  lvpm where the package is installed. A package without an SBOM, and every
  package built before, installs with any version.

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

## Building an application

`lvpm build --app` builds a LabVIEW application. It runs one build
specification of your `.lvproj` in the venv's own LabVIEW, so the build sees
the packages `lvpm sync` put in the venv. It then writes `bom.cdx.json`, an
SBOM of those packages, next to the build. Name the project and the build
specification in `[application]` ([manifest](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/manifest.md#application)):

```toml
[application]
project    = "app/My App.lvproj"
build-spec = "My Application"
```

Application builds are tested on LabVIEW 2026 only. The build needs a venv
bound to LabVIEW 2026 or later.

### Choosing what to build

| Command | What is built |
| --- | --- |
| `lvpm build` | Every kind the manifest declares: the `.lvpkg` (from `[distribution]`), then the application (from `[application]`) |
| `lvpm build --package` | The `.lvpkg` only |
| `lvpm build --app` | The application only |
| `lvpm build --app --package` | The same as `lvpm build` with both tables |

A flag for a table the manifest lacks fails and names the table:
`--app needs [application] in lvpm.toml (see docs/manifest.md)`, or the same
for `--package` and `[distribution]`. A manifest with neither table fails with
`nothing to build: lvpm.toml has no [distribution] or [application]`.
`--no-check` applies to the `.lvpkg` only; with `--app` alone it fails with
`--no-check applies only to the package build, and this run builds none`.

When both kinds are built, lvpm first checks that the application can be
built. If the check refuses it, nothing is written. If the `.lvpkg` then fails,
the application is not built.

### Before you build

Run these once, and again whenever the dependencies change:

```console
$ lvpm lock                                  # lvpm.lock, current for lvpm.toml
$ lvpm venv create --labview-version 2026    # a venv bound to LabVIEW 2026
$ lvpm sync                                  # the venv holds what the lock lists
$ lvpm build --app
```

Unlike the package build, the application build does not go on without a
current lock: the SBOM from a stale lock would be wrong. lvpm checks the
following in order and, when one fails, starts no LabVIEW and writes nothing.
Each message names the command that fixes it:

| Problem | Message |
| --- | --- |
| No `lvpm.lock` | `no lvpm.lock; run lvpm lock` |
| The lock cannot be read | `lvpm.lock cannot be read (<reason>); run lvpm lock to regenerate it` |
| The lock is out of date | `lvpm.lock is out of date (<reason>); run lvpm lock` |
| No venv | `no venv in <folder>; run lvpm venv create and lvpm sync` |
| The venv differs from the lock | `the venv differs from lvpm.lock (<first change>); run lvpm sync` |
| The venv's LabVIEW is older than 2026 | `the application build needs LabVIEW 2026 or later; this venv uses LabVIEW <year>; run lvpm venv create --labview-version 2026` |

A package in the venv that only `[dev-dependencies]` or a group needs does not
make the venv differ. A package the lock does not list does. The errors for
`[application]` itself are in the [manifest reference](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/manifest.md#application).

### What happens during the build

1. stderr says `building application "<build-spec>" with LabVIEW <year> ...`.
2. lvpm starts the venv's LabVIEW, runs the build specification, and waits for
   it to finish. There is no overall time limit, but each VI Server call may
   wait up to 180 s; one that waits longer fails the build with its
   connection error.
3. lvpm closes the LabVIEW it started, whether the build worked or not.
4. lvpm writes `bom.cdx.json`.

With `LV_RTE_HEADLESS` set, LabVIEW starts headless. It shows no window and
looks idle while it builds. App Builder can take minutes; wait for the
`built` line.

A LabVIEW that is already open on the venv is refused with
`a LabVIEW is already running or starting on this venv (VI Server port <port>) — close it first.`
Close it, then build again. The build starts its own LabVIEW because LabVIEW
reads the venv's packages only when it starts.

On success stdout says where the application is:

```console
$ lvpm build --app
built application "My Application" in 50.3s: C:\proj\builds\My App\Main.exe
  (+2 more outputs)
sbom: C:\proj\builds\My App\bom.cdx.json
```

The first path is the first file LabVIEW generated, normally the executable.
The `(+N more outputs)` line appears only when LabVIEW made more files. With
both kinds, the `.lvpkg`'s `built <path> (N files, with SBOM)` line comes
first, as soon as the package is written.

### The SBOM

`bom.cdx.json` is a CycloneDX 1.5 file, the same kind a `.lvpkg` carries
(see [Building a package](#building-a-package)). It lists the packages in `lvpm.lock` that
`[dependencies]` needs, each with its version, SHA-256 and source, and names
the application as its subject. The same lock gives the same bytes.

- It goes in the folder of the build's first output, beside the executable.
- A failed build writes none, and leaves the one from an earlier build as it
  was. A successful build replaces it.
- If lvpm cannot write it, the error says where the build output is:
  `the build output is in <folder>, but bom.cdx.json could not be written: <cause>`.
- lvpm never deletes build output. Delete the folder yourself.

### When the build fails

If LabVIEW runs the build specification and it fails, lvpm exits 1 and writes
no SBOM:

```text
LabVIEW could not build "My Application": 1003 The VI is not executable
open the LabVIEW project and run the build spec there to see details
```

The code and message are LabVIEW's. If LabVIEW does not start, lvpm adds
`the venv uses LabVIEW <year>; lvpm venv create --labview-version <year> rebinds it`.
If LabVIEW reports success but no output file, the error is
`LabVIEW reported no output for "<build-spec>"`.

### JSON output

With `--json`, an application run adds an `application` key to the result.
`package` appears only when the run also built the `.lvpkg`. A kind that was
not built is left out. A run that builds only the `.lvpkg` keeps its old
result, `{"command": "build"}`.

```json
{"result": {"command": "build",
            "package": {"path": "dist/my_app-1.0.0.lvpkg", "files": 4, "sbom": true},
            "application": {"build_spec": "My Application",
                            "outputs": ["C:\\proj\\builds\\My App\\Main.exe"],
                            "sbom": "C:\\proj\\builds\\My App\\bom.cdx.json",
                            "took": 50.3}}}
```

`took` is in seconds. `package.sbom` is `false` for a `.lvpkg` written without
an SBOM. A failed build prints `{"error": "<message>"}` as its last line and exits 1.

### Limits

- The venv does not hide LabVIEW's own `vi.lib`. A package that is also
  installed globally can still shadow the venv's copy, as it can for any venv
  command (issue #78).
- Only the `My Computer` target is built.
- Ctrl-C stops lvpm but can leave the LabVIEW it started running. Close it
  by hand.
- lvpm lists no checked-in binaries, such as a `.lvlibp`, in the SBOM, and does
  not sign it.
- Keep the build folder's path short. A built `Main.exe` whose path is close to
  Windows' 260-character limit (measured at 251) crashes on start with
  0xC0000409; the same build copied to a short path runs. This is the
  application, not lvpm.

[The pilot application](https://github.com/ThatsTheEnd/lvpm/tree/main/tests/fixtures/pilot-app)
is a working example: NI's Continuous Measurement and Logging template, changed
to call two pilot libraries.

## Publishing a package

`lvpm publish --to <folder>` copies the package `lvpm build` wrote for the
current manifest (to push it to an OCI registry instead, see
[Publishing to a registry](#publishing-to-a-registry)), `dist/<name>-<version>.lvpkg`, into a folder feed. Another
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
`http://` or `https://` folder holding an `index.vipr`, an `oci://` registry
([OCI registries](#oci-registries)), or a `github:` community index
([below](#a-github-source-is-a-community-index)). Local directories of
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
4. The public NI and JKI indexes, unless `defaults = false`. These two count
   as one source: a name both list has the versions of both (ADR 9). NI's
   index stopped taking some packages JKI's still carries, such as DQMH 7.

Apart from the two public indexes, a later source never replaces a name and
never adds versions to it, even a higher one. If the first source has no version that fits, or cannot be
reached, the command fails and names that source. `lvpm search` and
`lvpm info` show only the versions that source lists, or both public indexes
list. To take a package from
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

### Refreshing the index cache

lvpm keeps every index it reads in `%LOCALAPPDATA%\lvpm\cache`
(`lvpm-cache.db`) and reads that copy from then on, without the network. A
package published after that is not seen until the cache is refreshed:

```powershell
lvpm cache refresh
```

It asks each feed whether it changed, as a conditional request, so an
unchanged feed costs no download; `github:` index files are downloaded
again in full. It also re-reads each `sources.local`
folder. It prints one line per source, `refreshed` or `failed` with the
reason, and the number of package versions listed. Run inside a project, it
reads that project's `[sources]` too. A `github:` community index has no
catalogue, so only the files of the project's dependencies, and of what they
require, are fetched again.

- `--refresh` on any other command does the same for that command only:
  `lvpm --refresh search dqmh`.
- In `lvpm tui`, `R` reloads the feeds.
- Deleting `%LOCALAPPDATA%\lvpm\cache` also works: the next command fetches
  everything again. It drops imported packages and downloaded archives too.

### A github: source is a community index

A `[sources]` entry `github:<owner>/<repo>` is a community index: a GitHub
repository with one `packages/<name>.toml` per package name, which lists where
each version's file is. lvpm reads only the files of the names it resolves, not
the whole index.

```toml
[sources]
community = "github:labview-package-manager/package-index"
```

Source order matters, because the first source that lists a name wins (see
above). List your private or company feeds before `community`: the index is
open to registrations, so a name that a feed of yours lists must be found in
that feed first, never in the index. A name listed by an earlier `[sources]`
entry comes from that entry, not from the index. A name the index lists comes from it before the public NI
and JKI indexes, which are read last. A name the index does not list is looked
up in the next source. A dependency pinned with `index = "community"` is taken
from the index or fails with `searched github:labview-package-manager/package-index`.

The package file comes from its owner's GitHub Releases. lvpm checks the file
against the SHA-256 the index lists, and the lock records the source as
`github-index:<owner>/<repo>` with that `sha256`. `[policy]` applies as it does
to a GitHub dependency: with `require-signature = true`, a package with no
attestation is refused. To get a package into the index, see
[Publish to the community index](#publish-to-the-community-index).

Remote feeds, downloads, and imported packages are recorded in `%LOCALAPPDATA%\lvpm\cache`, in a
SQLite index (`lvpm-cache.db`) over a content-addressed store of the bodies
themselves. For feed bodies, without `--refresh` the stored body is used and
no feed request goes out. With it, the `ETag` and `Last-Modified` the server last gave
are sent back as a conditional request: only a feed that actually changed is
downloaded again, and an unchanged one answers `304` and costs no bytes. See
[Configuration](#configuration) to move the cache.

## OCI registries

A team that already runs a container registry (ghcr.io, GitLab, Azure
Container Registry) can keep its packages there. One version of a package is
one OCI manifest tagged with the version; the `.lvpkg` is its layer, byte for
byte. Another project lists the registry as an `oci://` source and pins the
packages it takes from it ([manifest.md](https://github.com/ThatsTheEnd/lvpm/blob/main/docs/manifest.md#an-oci-registry-as-a-source)):

```toml
[sources]
team = "oci://ghcr.io/acme/lv"

[dependencies]
acme_lib = { version = ">=1.0", index = "team" }
```

An `oci://host[:port]/prefix` source is explicit: it is asked only for the
packages pinned to it, so a package needs a pin. A pinned package that the
registry cannot serve fails and never comes from another source
([ADR 7](adr/0007-oci-sources-are-explicit.md)). `lvpm catalog`, `lvpm info`
and the TUI show the plain listing text, without the publish hint that
`lvpm lock` and `lvpm install` add.

### Publishing to a registry

`lvpm publish --to oci://host/prefix` pushes the package `lvpm build` wrote to
`<prefix>/<name>:<version>`, with the name in lower case.

```console
$ lvpm build
$ lvpm publish --to oci://ghcr.io/acme/lv
uploading acme/lv/acme_lib:1.2.3 to ghcr.io
published acme_lib 1.2.3 to oci://ghcr.io/acme/lv/acme_lib:1.2.3
  OCI manifest digest sha256:5b2c…
  SBOM attached
```

- The version is the tag, so it must be a valid OCI tag. A version with `+`
  build metadata is refused before any request: `1.0.0+build.7 is not a valid
  OCI tag; publish a version without "+" build metadata`.
- The package's `bom.cdx.json` is pushed as an OCI referrer of the package, and
  the last line says so, or says `no SBOM in the package`. If that push fails
  after the package is live, lvpm prints `SBOM could not be attached: <cause>;
  the package is published and a rerun will be refused` and exits 1. The digest
  is printed either way.
- The target holds no credentials: `oci://user:secret@host/prefix` is refused
  with `--to: credentials do not belong in the target; remove the user:password@
  part and run docker login host`, and the user name and secret are not shown.

**Tags are immutable.** A published version is never replaced. Publish checks
that the tag does not exist, uploads the blobs, checks again, and only then
writes the OCI manifest. An existing tag fails and writes nothing:
`acme/lv/acme_lib:1.2.3 is already published at oci://ghcr.io/acme/lv; a
published version is never replaced; bump the version to publish again`. A check
that fails for any other reason than "no such tag" stops the publish too. A
publish that fails before the OCI manifest is written says `nothing was
published, it is safe to rerun`; the blobs it uploaded are reused.

The OCI API has no "create this tag only if it is new", so two publishers of
one version can still race in the moment between the second check and the
write. For tag immutability that the registry enforces itself, turn it on where
the registry has it: some registries can lock or protect tags (see your
registry's documentation).

**On ghcr.io, a `403` means the token lacks the package scope.** A refused
upload says `registry ghcr.io refused the upload to acme/lv/acme_lib; run docker
login ghcr.io with a token that can write packages; nothing was published, it
is safe to rerun`. Log in with a classic personal access token that has
`write:packages`.

### Credentials

Registry credentials come only from the Docker credential store, never from
`lvpm.toml`. lvpm reads `config.json` in `DOCKER_CONFIG`, else in `~/.docker`,
and takes the first of these for the registry's host:

1. the `credHelpers` entry for that host
2. `credsStore`
3. the `auths` entry for that host

A helper is the program `docker-credential-<name>`, run as `get` with the host
on its input. A helper that holds nothing for the host counts as no login. With
no login at all, lvpm asks anonymously, which is enough for a public package.
`docker login <registry>` is how to add one (`az acr login` does it for Azure
Container Registry). A helper that answers the user name `<token>` gives an
identity token, which lvpm sends to the registry's token service as a
refresh token.

A login the registry refuses is printed once per registry when lvpm lists
packages (`lock`, `install`, `search`, and a `sync` that has to lock first), and
once per package when `sync --locked` fetches the packages by digest.

A helper runs once per registry per run. If it does not answer within 30
seconds lvpm stops waiting: set `LVPM_OCI_HELPER_TIMEOUT_SECS` to a whole number
of seconds, 1 or more, to change that. What a helper prints is never shown, and
neither is a secret from the config. What lvpm says instead:

- `registry <host> refused the login from <credHelpers docker-credential-<name> | credsStore docker-credential-<name> | auths>; run docker login <host>`
- `credential helper docker-credential-<name> for <host> failed with exit code <code>; run it by hand to see why`
- `credential helper docker-credential-<name> for <host> was not found; check that it is installed and on PATH`
- `credential helper docker-credential-<name> for <host> did not answer within <secs>s`

On ghcr.io a repository that does not exist answers `403`, as one you may not
read does (observed behaviour, not one that ghcr.io documents). `lvpm lock` can only say `<package>: registry ghcr.io refused the
lookup for source "team"; the package is missing or your login cannot read it;
run docker login ghcr.io`, and does not fall through to another source.

### The lock and `sync --locked`

For an OCI package `lvpm.lock` records the `source` (the `oci://` URL) and the
`digest` of its OCI manifest, beside the `sha256` of the archive, which is the
layer's digest. `lvpm sync --locked` asks the registry only by that digest,
never by tag, and checks the layer's bytes against the lock's `sha256`. Someone
who moves the tag to other bytes changes nothing for a project that is locked,
and a package already in the cache needs no network at all. When the registry
fails the fetch, the message says what to do:

- `cannot fetch <package> <version> by digest from source "<source>": registry <host> refused it; run docker login <host>`
- `cannot fetch <package> <version> by digest from source "<source>": registry <host> no longer holds sha256:<hex>; run lvpm lock`
- `<package> <version> from source "<source>": registry <host> served bytes that do not match sha256:<hex>; nothing was installed`

A moved tag is never taken silently. When a plain `lvpm lock` (or any command
that locks again because the manifest changed) finds a locked version at a
different OCI manifest digest, it fails, names the package, the version, the
digest in `lvpm.lock` and the digest the registry has now, and writes nothing.
`lvpm lock --upgrade`, or `lvpm lock --upgrade-package <package>@<version>` for
one package without moving the others, accepts the new digest and records it
with the new archive's `sha256`. A version's age for `min-age-days` is the time
in the OCI manifest's `org.opencontainers.image.created` annotation.

### Signatures on OCI packages

The SBOM and the signature are OCI referrers of the package's OCI manifest.
lvpm finds them with the registry's referrers API. A registry without it keeps
them in the fallback tag `sha256-<hex>` that the OCI specification defines for
that, and lvpm reads that tag. A lookup that fails with anything but `404`, on
the API or on the fallback tag, is an error under every `[policy]`, never read
as "no signature": `<package> <version>: the signature lookup on registry <host>
failed: <cause>; the package was not locked`.

The signature is a Sigstore bundle that the publisher's CI attaches with
`push-to-registry`. The
[release template](https://github.com/ThatsTheEnd/lvpm/blob/main/.github/workflow-templates/lvpm-release.yml)
has an optional `publish-oci` job that does it: `lvpm publish --to oci://…`,
then `actions/attest-build-provenance` with `subject-name`, `subject-digest`
(the OCI manifest digest that `lvpm publish` printed) and
`push-to-registry: true`. That signs the OCI manifest, not the `.lvpkg`. The
job needs an lvpm release that has `lvpm publish --to oci://`, the first one
that ships it: the template's `LVPM_VERSION` pin (0.24.0 at the time of writing)
has none, so raise it first.

If the SBOM push fails, `lvpm publish` exits 1 after the package is live, the
job's attest step is skipped, and a rerun is refused because the tag exists.
`gh` cannot sign, so attest by hand: run `actions/attest-build-provenance` once
in a workflow of that repository with the same permissions as `publish-oci`,
giving the repository and digest from the publish log:

```yaml
- uses: actions/attest-build-provenance@v2
  with:
    subject-name: ghcr.io/acme/lv/acme_lib   # the published reference, no oci:// or tag
    subject-digest: sha256:5b2c…             # the OCI manifest digest publish printed
    push-to-registry: true
```

`lvpm lock` checks the bundle with `gh attestation verify` against the OCI
manifest it has already read, under the same [`[policy]`](#policy) as a GitHub
release, and records `signer` and `subject` in the lock. A bundle that `gh`
rejects, or whose certificate names no source repository, is refused whatever
the policy says. `lvpm sync --locked` trusts the locked signer and does not run
`gh`. Only the first bundle referrer of a package is used.

**The limit: an identity, not an endorsement.** For a GitHub release lvpm
passes `gh` the repository the dependency names. An OCI source declares no
repository, so lvpm takes it from the bundle's own certificate, and `gh` then
checks the bundle against that. `require-signature = true` without
`trusted-publishers` therefore accepts a bundle built by any GitHub repository:
the signer is an identity, not an endorsement, and that is weaker than for a
GitHub release. Only `trusted-publishers` gives assurance; list the
repositories you trust.

## Configuration

Settings are named the way npm names `npm_config_<key>`: the environment
variable is `LVPM_CONFIG_<KEY>` (or lowercase `lvpm_config_<key>`), and an
empty value counts as unset, except `LVPM_CONFIG_HOOKS_RUN`, where an empty
value is kept and turns the file's list off. Precedence, highest first:

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
file but not what lvpm uses: unset the variable or edit its value. An empty
value counts as set: it turns the file's list off.

In CI, set it on the job to keep the cache on a persistent volume between
runs.
