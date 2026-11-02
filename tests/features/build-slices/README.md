# Build-slice scenarios: the pilots

`s0-pilot-loop.feature` runs the loop every build slice is judged by
(build → publish → install into project B's venv → mass compile) on the two
pilot libraries named in [docs/vision/build-slices.md](../../../docs/vision/build-slices.md#pilot-libraries).

Project B is the consumer project: it depends on both pilots and installs
them from a folder feed of the archives.

## The pilot archives

The `.vip` files are committed in [tests/fixtures/pilots](../../fixtures/pilots),
and [SHA256SUMS](../../fixtures/pilots/SHA256SUMS) holds each one's SHA-256. Every
scenario that reads them checks them against it first, so a changed archive fails the
run instead of testing something else. They are committed rather than
downloaded because the harness keeps `lvpm` off the network, and 2.4 MB is
small. Each archive carries its own `license` file. The LabVIEW Open Source
Project packages are 0BSD. HSE Logger's spec says "BSD License", but its
`license` file is a modified BSD that forbids redistribution "in source
form". A `.vip` holds source VIs, so whether committing this archive is
allowed is an open question for its vendor (see the S1 decisions log). The license
file was read on 2026-10-26.

| Archive | Why it is here |
|---|---|
| `hse_lib_hse_logger-2.2.2.114.vip` | pilot A |
| `labview_open_source_lib_serializer-1.2.1.16.vip` | pilot B |
| `labview_open_source_lib_data_manipulation-1.6.0.16.vip` | pilot B's one dependency |
| `labview_open_source_project_lib_labview_open_source_palette-1.0.2.7.vip` | Data Manipulation's dependency |

Obtained 2026-10-26 from the public NI index (`download.ni.com`, the source
vipm.io lists) with `lvpm install <name> --prefix <scratch>`, which leaves
each archive in lvpm's cache under its SHA-256. To move a pilot to a newer
version, do the same, copy the archive here, regenerate `SHA256SUMS` with
`sha256sum *.vip`, and update the version in the feature file.

## The pilot sources

S1, S5 and S9 build and check the pilots' sources. They are pinned by commit
and not committed here. Pilot A's LICENSE forbids redistributing it in source
form, so the harness never fetches or copies it anywhere but a test sandbox.
The `@pilot-source` scenarios read it from a checkout that you make:

```console
git clone https://code.hampel-soft.com/open-source/hse-logger
git -C hse-logger checkout f7cc16a391faa1e982b54ae22d2f43c9df1ef581
LVPM_TEST_PILOT_A=<path to hse-logger> cargo test --test acceptance
```

Pilot B's source is public, and the `@pilot-b-source` scenarios (S5) read it
from a clone in the same way, through `LVPM_TEST_PILOT_B`:

```console
git clone https://github.com/LabVIEW-Open-Source/Serializer
git -C Serializer checkout 723d7a672b5f0eafa9f98ea63629ba5855d50227
LVPM_TEST_PILOT_A=<path to hse-logger> LVPM_TEST_PILOT_B=<path to Serializer> cargo test --test acceptance
```

Each scenario checks that its HEAD is the pinned commit and that the folder
the build copies (`Source` or `source`) has no modified, untracked or ignored
files; changes elsewhere in the checkout are accepted. Without a variable, the harness does not run its scenarios
and says how many it left out.

| Pilot | Repository | Tag | Commit |
|---|---|---|---|
| A: HSE Logger 2.2.2.114 | https://code.hampel-soft.com/open-source/hse-logger | `v2.2.2` | `f7cc16a391faa1e982b54ae22d2f43c9df1ef581` |
| B: OpenSerializer 1.2.1.16 | https://github.com/LabVIEW-Open-Source/Serializer | `1.2.1` | `723d7a672b5f0eafa9f98ea63629ba5855d50227` |

## Running them

```console
LVPM_TEST_LABVIEW=1 cargo test --test acceptance
```

The `@labview` scenarios are also `@interactive-labview`, so headless CI skips them. They need:

- LabVIEW 2026, with none of the pilots installed in it (`vi.lib`, `user.lib`, `instr.lib`). The real
  installation wins over a venv, so a pilot there would be tested in place of
  the venv's copy. The scenario fails naming the library it found.
- LabVIEWCLI (`LabVIEWCLI.exe` on `PATH`, or in
  `C:\Program Files (x86)\National Instruments\Shared\LabVIEW CLI`).

It installs without relinking, starts a LabVIEW on the venv, waits for its VI
Server port, and only then runs `LabVIEWCLI -OperationName MassCompile` on
the venv's packages, so LabVIEWCLI never starts a LabVIEW of its own. The
venv is a scratch copy, so the compile works in place. A failed compile names
the VIs the log calls bad.

Scenarios that use the installation's own LabVIEW first claim it with a file
in `%TEMP%`, so two test runs never share one. If another run holds the
claim, or something already answers VI Server on the installation's port
(your own LabVIEW, say), they are not run and the harness says why. The
LabVIEW the run started is ended when the run ends, even after a panic.

## What `lvpm check` says about the pilots

`s5-lvpm-check.feature` copies the pilots' source folders into a sandbox, binds
a venv to an installation folder that is absent, and runs `lvpm check` on the
copy. The VI counts and lines below are what the pinned commits give.

| Pilot | Project | VIs checked |
|---|---|---|
| A | `Source` as `[distribution]` library `Source/HSE-Logger/hse-logger.lvlib`, `labview = ">=2016"` | 90 |
| B | `source` as `[distribution]` library `source/OpenSerializer.lvlib`, `labview = ">=2013"` | 30 |

**The demo.** `Source/HSE-Logger/LogRecord/Generate Callchain.vi` is called by
exactly one VI of pilot A, `Source/HSE-Logger/LogRecord/Init.vi`. Delete it
from the copy and `lvpm check` prints:

```text
Source/HSE-Logger/LogRecord/Init.vi: link not found: Source/HSE-Logger/LogRecord/Generate Callchain.vi
```

The demo project has no `[distribution]` table: with one, the library file
`LogRecord.lvclass` lists the deleted VI and `lvpm check` stops with
`... lists ... Generate Callchain.vi, which is not on disk` before it reads any VI.

**Pilot B and its dependency.** With `labview_open_source_lib_data_manipulation`
installed in the venv from the committed archive and `[dependencies]` empty, the
check exits 1 with one line per VI and link, among them:

```text
source/Serializer.INI/INI.Attributes to Variant.vi: <vilib>/LabVIEW Open Source Project/Data Manipulation/KVP Parser.vi is in labview_open_source_lib_data_manipulation, which is not in [dependencies]
```

**Both pilots pass.** With `labview_open_source_lib_data_manipulation` in
`[dependencies]` (pilot B) and the bound installation absent, `lvpm check` prints:

| Pilot | stdout | stderr |
|---|---|---|
| A | `VIs checked: 90, problems: 0, links not checked: 26` | `warning: links not checked: 19 into vi.lib (the bound installation <path> is not on this machine)` and `warning: links not checked: 7 absolute paths not on this machine (saved elsewhere; LabVIEW searches for them when it loads)` |
| B | `VIs checked: 30, problems: 0, links not checked: 57` | `warning: links not checked: 57 into vi.lib (the bound installation <path> is not on this machine)` |

Two rules make that so. A link inside a file counts as found: pilot A's 56
links to a class's private data control (`Source/HSE-Logger/Logger/Logger.lvclass/Logger.ctl`)
and pilot B's 3 links to a path below a VI
(`source/Serializer.INI/ExtractSectionsFromString.vi/LabVIEW`) lead into a file
that is there. A link to an absolute path that is not on this machine is not
checked: pilot A's 7 were saved where the author worked
(`C:/Users/admin/_Git/hse-logger/...`, `C:/_work/hampel-soft/open-source/hse-logger/...`)
and LabVIEW looks for the file by name when it loads the VI.
