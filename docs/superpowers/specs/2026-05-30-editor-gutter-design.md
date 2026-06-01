# Editor line-number gutter

Date: 2026-05-30
Status: approved (design)

## Context

`src/editor.zig` is Anvil's native editable buffer. It renders into a Terminal
grid that the existing GPU path draws, so a gutter needs no renderer or shim
changes — it is simply the leftmost columns of that grid.

This is sub-project #1 of a larger "full native IDE" roadmap (gutter → buffer
hardening → LSP client core → diagnostics → navigation → completion → edits).
This spec covers the gutter only. Later slices get their own specs.

## Goal

Show line numbers in a left gutter, with the current line's number
highlighted. Content shifts right by the gutter width. Click and cursor mapping
account for the offset. No horizontal jitter.

## Design

### Width

`gw = digits(lines.len) + 1` — the digit count of the last line number plus one
trailing space. Example: 3231 lines → `"3231 "` → 5 columns. Recomputed each
`render` from `lines.len`, so it is stable across scrolling (depends on the
buffer's max line number, never the visible window). Clamp `gw = min(gw, cols-1)`
so content always keeps at least one column on a very narrow window. Empty
buffer → one line → `gw = 2` (`"1 "`).

### Render

In `Editor.render`, for each visible screen row `sr` (line index `li = top + sr`):

- Write the right-aligned decimal `li + 1` into grid columns `[0, gw)`.
- Write the line's content into columns `[gw, cols)`. Content width is
  `cols - gw` (truncates as before).

Gutter color: dim, `Color{ .indexed = 8 }` (same gray as comments). The current
line (`li == cur_row`) renders its number in `.default` (brighter) — the
current-line cue. No relative numbering (deferred).

### Cursor

`term.setCursor(scr_row + 1, gw + scr_col + 1)`, with the column clamped to
`cols - 1`. The existing off-screen-cursor clamp on `scr_row` is unchanged.

### Click

`Editor.click` subtracts `gw` from `screen_col` before mapping to a buffer
column: `buf_col = screen_col -| gw` (saturating; a click inside the gutter
lands at column 0 of that line). `session.editorClick` passes the raw grid
column unchanged — the subtraction lives in `click`.

### Shared helper

A private `fn gutterWidth(self: *const Editor) usize` returns `digits(lines.len)
+ 1`, depending only on `lines.len` (no `cols`, so `click` — which takes `rows`
but not `cols` — can call it). The narrow-window clamp (`min(gw, cols-1)`) is a
render-time content concern, applied locally in `render` where `cols` is known.
On a window narrower than the gutter, real click coordinates never exceed `cols`
anyway, so the saturating subtraction in `click` stays correct.

## Edge cases

- `gw >= cols` (window narrower than the gutter): clamp to `cols - 1`; content
  gets the final column.
- Empty buffer: `lines.len == 1` → `gw = 2`.
- Scrolled so the cursor is off-screen: gutter still renders for the visible
  window; cursor clamp behavior is unchanged from today.

## Testing

New tests (in `editor.zig`):

- Gutter digits present and right-aligned for a multi-line buffer (e.g. a
  10-line buffer → `gw = 3`, row 0 shows `" 1"` then a space at col 2).
- Click past the gutter maps to the correct buffer column
  (`click(row, gw + k)` → `cur_col == k`).
- Cursor column is offset by `gw` after render.
- Current-line number uses `.default`, other lines `.indexed = 8`.

Ripple (must update): existing assertions on `grid.at(0, 0)` now read a gutter
digit instead of content. Update them to assert at `at(0, gw)`:

- `editor.zig` — "render writes cells and places the cursor".
- `session.zig` — "editor session: load renders grid…" and "reloadEditor swaps
  the buffer in place…".

Verify: `.zig/zig build test` green, `./zig-out/bin/anvil --dump /tmp/x.png`
rc 0, live check that numbers render and clicks land correctly.

## Out of scope

Relative line numbers, configurable gutter visibility, diagnostic markers in the
gutter (arrives with the diagnostics slice), git-change markers, code folding.
