---
status: active
type: concept
created: 2026-05-21
updated: 2026-05-29 (regex + match-count)
sources:
  - ../../src/search.zig
  - ../../src/vt/terminal.zig
  - ../../src/render/renderer.zig
  - ../../src/app.zig
confidence: high
---

# Search System

> Note: the lower sections describe the former Rust design. The current Zig
> implementation is in `src/search.zig`, `src/regex.zig`, and `src/app.zig`
> (`emitSearch`). Key differences: fixed `[max_matches]Match` array (cap 512),
> smart-case substring mode, optional regex mode (Tab toggle), match count
> displayed as `cur/total` right-aligned in the search bar, `[R]`/`[R?]`
> indicator for regex mode.

Anvil provides incremental, highlight-all search over the active
tab's grid and scrollback. The search bar appears as a one-row overlay at the
bottom of the window; the grid shrinks by one row while it is open.

## Search Struct

`Search` (in `src/search.zig`) is a pure-logic struct owned by `App`
in `src/app.zig`. It holds no references to the terminal — it borrows the
terminal on each scan call and stores only the match list.

Fields:
- `query_buf: [u8; 256]` / `query_len: usize` — the current query bytes. Queries
  longer than 256 bytes are silently truncated.
- `matches: Vec<Match>` — every match found in the last scan.
  `Match` carries `row` (content-row index), `col` (start column), and `len`
  (codepoint count equal to the query codepoint count).
- `current: usize` — index of the highlighted match. `next`/`prev` cycle it
  with wrapping; `set_query`/`rescan` reset it to 0.

### Scan

`rescan(term)` clears the match list and re-scans every content row from index
0 to `term.line_count() - 1`. For each row, a sliding window of width
`query_len` (in codepoints) checks `row_matches_at` cell-by-cell.

Match list cap: `MAX_MATCHES = 2048`. When the cap is reached, `rescan`
returns early; subsequent rows are not scanned. This prevents a
single-character query over 100 k-line scrollback from allocating unbounded
memory.

### Smart-case

If any codepoint in the query is an uppercase ASCII letter (`A`–`Z`),
case-sensitive mode activates and the scan compares exact codepoints. Otherwise
the scan uses `lower_cp` (ASCII fold only) for case-insensitive matching.
Non-ASCII codepoints are compared exactly in both modes.

### UTF-8 Validation

`rescan` validates the query as UTF-8 before scanning. If the query
buffer contains invalid UTF-8 (possible because `query_buf` is written
byte-by-byte during query editing), the scan treats the query as empty and
returns with no matches.

### Cell Classification

`classify(row, col) MatchKind` reports how a cell should be tinted:

- `Current` — covered by the current match.
- `Other` — covered by any other match.
- `None` — not covered.

Two-pass implementation: pass 1 checks `matches[self.current]` directly
so `Current` always wins when a cell is covered by both the current match and
another match (e.g. overlapping matches from a repeated-character query such as
`"aa"` in `"aaa"`). Pass 2 loops the remaining matches for `Other`.

## Content-Row Index Space

`Terminal` exposes a unified index space over scrollback and the active grid:

- `line_count() -> usize` — total rows: `history.len() + grid.height`.
- `line(i) -> &[Cell]` — rows `0..history.len()-1` are scrollback (variable
  length); rows `history.len()..line_count()-1` are active-grid rows (always
  `grid.width` wide). Out-of-range returns an empty slice.
- `content_row_of_viewport(y) -> usize` — maps a viewport row (0-based) to its
  content-row index. At `viewport_offset == 0`, viewport row 0 maps to the
  first active-grid row (`history.len()`).
- `scroll_to_line(target)` — adjusts `viewport_offset` so content row
  `target` is visible. Grid rows (index >= `history.len()`) always resolve to
  offset 0. Scrollback rows scroll so the target row appears near the top.

These accessors are the only interface between `Search` and `Terminal`. The
renderer uses `content_row_of_viewport` in `draw_cell` to map a viewport cell's `y`
coordinate to a content-row index for `classify`.

## Bottom Search Bar

`draw_search_bar` (in `src/render/renderer.zig`) paints the bottom chrome row:

- Full-width background in `theme.ansi[8]` (bright-black / ash).
- Left-aligned label `"find: <query>"` in `theme.foreground`. The label is
  truncated to leave room for the counter.
- Right-aligned counter `"<current>/<total>"` in `theme.foreground`.

The bar is drawn only when `app.search_open` is true. `draw_search_bar` is called
at the end of `render_frame`, after the terminal grid is rendered.

## Top/Bottom Bar-Row Offset Split

The window has two reserved bar rows:

- **Top** (`top_bar_rows()`): 0 or 1 row for the tab bar (shown when 2+ tabs are
  open). This was introduced with the multi-tab sub-project.
- **Bottom** (`bottom_bar_rows()`): 0 or 1 row for the search bar (shown when
  `search_open` is true).

`resize_all_tabs` computes the grid row count as:

```
max(total_rows - top_bar_rows() - bottom_bar_rows(), 1)
```

Opening or closing the search bar calls `resize_all_tabs`, which sends SIGWINCH
to the shell so it reflits its output to the new grid height.

## Input Routing

While the search bar is open, `on_key_down` intercepts all keystrokes before they
reach the shell:

- Printable text: appended to the query (UTF-8, capped at 252 bytes in the
  editor to keep room for a 4-byte codepoint within the 256-byte buffer);
  `set_query` rescans immediately.
- `Backspace`: drops the last UTF-8 codepoint (byte-walks backwards over
  continuation bytes `0x80`–`0xBF`).
- `Enter`: advances to the next match and scrolls.
- `Esc`: closes the search bar.

The ⌘F / ⌘G / ⌘⇧G chords are parsed in `handle_tab_key` via the same
`chord_matches` + `Chord` mechanism used by tab keybindings. Defaults are
`cmd+f` / `cmd+g` / `cmd+shift+g`; all three are user-configurable in the TOML
config file under `[keybindings]`.

**⌘⇧F** opens the search bar in Block scope. It is hardcoded (not in the
config file) and always resolves to `cmd+shift+f`.

## Block-Scoped Search

`Search` carries a `scope: SearchScope` field (`All` | `Block`). When `Block`,
the scan is restricted to the OSC-133 block that contains the cursor row at the
time the bar was opened.

- `set_query_in_block(term, query, anchor_content_row)` — sets scope to Block,
  computes `block_row_range` from `PromptStart` marks, and scans only that range.
- `block_row_range(term, anchor)` — walks `prompt_marks()` to find the largest
  `PromptStart` with `abs_line ≤ anchor_abs` (block start) and the smallest
  `PromptStart` with `abs_line > anchor_abs` (block end). Falls back to
  `(0, line_count())` when there are no marks.
- `draw_search_bar` shows the prefix `"block find: "` instead of `"find: "` when
  `search.scope() == Block`.
- `open_search()` (⌘F) and `close_search()` both reset scope to `All`.

Switching tabs while searching calls `close_search()` — search state is not
preserved across tab switches.
