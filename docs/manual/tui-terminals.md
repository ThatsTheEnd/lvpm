# lvpm-tui in real terminals: the manual checklist

The acceptance tests draw lvpm-tui on a test backend. They cannot show what a
real terminal does with raw mode, the alternate screen, a resize, or the
restore on exit. Run this checklist in each terminal below whenever a slice's
plan step says so, and copy the result table into the pull request.

## Before you start

Build the binary once:

```bash
cargo build -p lvpm-tui
```

Open each terminal in the repository root:

- **Windows Terminal**: `wt -d .`
- **conhost** (the classic console): `conhost.exe cmd.exe /k cd /d %CD%`

Before you start lvpm-tui, type `echo before` so that the screen has a line
you can recognize.

## Checks

| # | Do this | Expect this |
|---|---------|-------------|
| 1 | Make the window at least 80 columns by 24 rows. Run `target\debug\lvpm-tui.exe`. | The `echo before` line is gone. The top row shows ` lvpm ` in reverse video and the version. A bordered box fills the middle. The bottom row shows `q quit`. |
| 2 | Make the window narrower than 80 columns. | Only the message "The terminal is WxH; lvpm-tui needs at least 80x24." shows, with the current size. No border, no header. |
| 3 | Make the window wider again, back to at least 80x24. | The header, the box and the footer from check 1 come back. |
| 4 | Do checks 2 and 3 again, but make the window shorter than 24 rows. | The same as checks 2 and 3. |
| 5 | Press `q`. | lvpm-tui exits. The `echo before` line is back. The cursor shows. Type `abc`: the letters echo, and Enter runs the line. |
| 6 | Start lvpm-tui again and press Ctrl-C. | The same as check 5. |

## Result table

Copy this into the pull request, with one mark per cell: ✅ passed, ❌ failed
(add a note), or — not run.

| Check | Windows Terminal | conhost |
|-------|------------------|---------|
| 1 start | | |
| 2 too narrow | | |
| 3 back to 80x24 | | |
| 4 too short and back | | |
| 5 quit with q | | |
| 6 quit with Ctrl-C | | |
