# `lvpm check` fixtures

Saved on 2026-11-02 with LabVIEW 2026 (64-bit), our own work. Each VI was
composed with pylabview and then opened and saved by LabVIEW 2026 itself, so
every file below is LabVIEW-written. All are saved with "Separate compiled
code from source file" on, except `NotSeparated.vi`.

| File | What it is | Links (block: kind, components) |
|---|---|---|
| `project/Lib.lvlib` | Library holding `Good.vi`, `Helper.vi`, `UsesDep.vi`, `UsesDevOnly.vi` | — |
| `project/Helper.vi` | numeric in, +1, numeric out | LIvi: rel `Lib.lvlib` |
| `project/Good.vi` | calls `Helper.vi` | LIvi: rel `Lib.lvlib`, rel `Helper.vi` · LIbd: rel `Helper.vi`, empty |
| `project/NotSeparated.vi` | `Good.vi` saved as a copy with compiled code kept in the file; not a library member | LIvi: rel `Helper.vi` · LIbd: rel `Helper.vi`, rel `Lib.lvlib` |
| `project/Broken.vi` | called `Sub/Gone.vi`, deleted after saving | LIvi: rel `Sub/Gone.vi` · LIbd: the same, empty |
| `project/Archive.llb` | holds `InLlb.vi`; `Removed.vi` was deleted from it in place with NI's `Delete From VI Library.vi` | — |
| `project/UsesLlb.vi` | calls `Archive.llb/InLlb.vi` and `Archive.llb/Removed.vi` | LIvi: both, rel · LIbd: each followed by an empty |
| `project/UsesDep.vi` | calls `Dep.vi`, saved while it sat in `vi.lib/Check Fixture/` | LIvi: `<vilib>/Check Fixture/Dep.vi`, rel `Lib.lvlib` |
| `project/UsesDevOnly.vi` | calls `DevDep.vi` the same way | LIvi: `<vilib>/Check Fixture Dev/DevDep.vi`, rel `Lib.lvlib` |
| `project/Tests/TestDevOnly.vi` | not a member; calls `DevDep.vi` | LIvi: `<vilib>/Check Fixture Dev/DevDep.vi` |
| `project/UsesNi.vi` | calls NI's `vi.lib/Utility/Get Current LV Bitness.vi` | LIvi: `<vilib>/Utility/Get Current LV Bitness.vi` |
| `packages/Check Fixture/Dep.vi`, `packages/Check Fixture Dev/DevDep.vi` | the package files; the scenarios install them into a sandbox venv | none |

"rel" paths are relative to the VI's folder. "empty" is a zero-length
`PTH0` record, which LabVIEW writes after most links in `LIbd`. `LIfp` is
empty in every fixture.

To remake: compose the VIs, save them in this order (callees first), put
`Dep.vi` and `DevDep.vi` into the installation's `vi.lib` only while saving
their callers, then delete `Sub/Gone.vi` and remove `Removed.vi` from the
`.llb`. Never resave a caller after its target is gone.

Made at run time by the test support, not committed: a VI whose `LIvi`
(or `LIfp`, `LIbd`) section size runs past the end of the file, and `x.vi`
holding the text `not a VI`.

## Pilot spike

Read from the VIs in the committed pilot `.vip`s (scratch scripts and the
reader itself, 2026-11-02): all pilot links use one `PTH0` flavour, and no
pilot holds an `.llb`. `LVSR` bit `0x400` is set on every pilot library file;
the one exception is `PostInstall.vi`, a VIPM build hook. Pilot A was saved in
2016 and pilot B in 2013. The reader ran over all 216 pilot VIs: 0 unreadable,
1034 links, and `LIfp` holds links in the pilots (it is empty only in the
fixtures above).
