# In-Terminal Search — Design Spec

> Status: draft for review. Sub-project 3 of 4 in milestone **M2**.
> Created: 2026-05-21. Owner-approved direction via brainstorm answers.

## Goal

Search the active tab's terminal content — visible grid plus scrollback — with
an incremental, highlight-all search bar: type and every match lights up live,
the current match emphasized, ⌘G/⌘⇧G to step through.

## Context

After M2 sub-projects 1-2 the app has a config system, themes, and multi-tab.
Each tab owns a `Terminal` (a primary/alternate grid + a `Scrollback` ring).
Search runs over the active tab's `Terminal`. A tab bar already steals the top
text row when 2+ tabs are open (`tabbar.zig`); the search bar is the bottom
counterpart.

## Decisions (settled with the owner)

1. **Search UX** — incremental, highlight-all. Typing rebuilds matches live;
   all matches highlight, the current one emphasized.
2. **Match** — plain substring, **smart-case** (case-insensitive unless the
   query contains an uppercase letter). No regex.
3. **Placement** — a one-text-row bar at the **bottom** of the window.
4. **Keybindings** — ⌘F open, ⌘G next, ⌘⇧G previous, Esc close — added to the
   `config.zon` `Keybindings` struct, live-reloadable, consistent with the
   multi-tab shortcuts.
5. **Scope** — search targets the active tab; switching tabs closes the bar.
6. **Scan strategy** — scan-on-change: every query edit re-scans the whole
   scrollback + grid and rebuilds the match list (cheap codepoint comparison;
   a few ms for a full 100k-row ring — fine for search-as-you-type).

## Architecture

```
src/terminal/search.zig   Search — query, match scan, match list, current index
src/render/searchbar.zig  drawSearchBar — the bottom one-row bar
src/config/config.zig     + search keybindings on Keybindings
src/main.zig              search state, input routing, match highlighting,
                          bottom-row grid offset
```

### Content addressing

Search and the renderer need a single index space over all of a tab's lines.
Define a tab's **content rows** as the concatenation: scrollback rows
`0 .. history.len()` followed by grid rows `0 .. grid.height`. A
`content_row` is an index into that sequence. `Terminal` gains two read
accessors so `search.zig` and the renderer do not reach into internals:

```zig
/// Total content rows: scrollback length + grid height.
pub fn lineCount(self: *const Terminal) usize;
/// Borrow content row `i` (0 = oldest scrollback ... lineCount()-1 = last grid
/// row). Scrollback rows may be shorter than grid width; callers handle that.
pub fn line(self: *const Terminal, i: usize) []const Cell;
```

(`line` reuses the existing `viewportRow` composition logic where it pads short
scrollback rows.)

### `Search`

```zig
pub const Match = struct {
    row: usize, // content_row
    col: usize, // start column
    len: usize, // length in cells (== query codepoint count)
};

pub const Search = struct {
    alloc: std.mem.Allocator,
    query: [256]u8 = undefined,   // UTF-8 query text
    query_len: usize = 0,
    matches: std.ArrayList(Match),
    current: usize = 0,           // index into `matches`; 0 when empty

    pub fn init(alloc) Search;
    pub fn deinit(self: *Search) void;

    /// Replace the query and re-scan `term`. Resets `current` to the first
    /// match at or after the current viewport, else 0.
    pub fn setQuery(self: *Search, term: *const Terminal, text: []const u8) !void;

    /// Re-run the scan against `term` keeping the query (after new output).
    pub fn rescan(self: *Search, term: *const Terminal) !void;

    pub fn next(self: *Search) void; // wraps; no-op when empty
    pub fn prev(self: *Search) void;

    /// Highlight kind for a cell, for the renderer.
    pub fn classify(self: *const Search, row: usize, col: usize) MatchKind;
    // MatchKind = enum { none, other, current };

    pub fn count(self: *const Search) usize;       // matches.len
    pub fn currentMatch(self: *const Search) ?Match;
};
```

The scan: for each content row, decode the cells' codepoints, slide the query
across, record every start position. Smart-case: if the query has any uppercase
ASCII letter, compare exactly; otherwise lowercase both sides. The query is
decoded to codepoints once per scan.

## Data flow

- **⌘F** — open the search bar. The bar takes the bottom text row, so every
  tab's grid + PTY resize by −1 row (the search bar applies to the active tab
  but the row is reserved uniformly, mirroring the tab-bar resize logic).
- **Typing** — each edit calls `Search.setQuery`, which re-scans and rebuilds
  `matches`. The viewport scrolls to show `current`.
- **⌘G / Enter** — `Search.next`; **⌘⇧G** — `Search.prev`; both scroll the
  viewport so `currentMatch()` is visible.
- **Esc** — close the bar; grid + PTY resize +1; highlights cleared.
- **New shell output while open** — `onTick` calls `Search.rescan` for the
  active tab when it has a live query, so matches stay correct.
- **Tab switch while open** — close the search bar (decision 5).

## Rendering

- `drawSearchBar(raster, font, theme, search)` paints the bottom raster row:
  the query text, a `current/total` counter (e.g. `3/17`), themed — query text
  in `theme.foreground`, bar background in `theme.ansi[8]`.
- Cell highlighting: when the renderer draws a viewport cell, it maps the
  viewport row to a `content_row` and calls `search.classify(row, col)`:
  `current` → accent background, `other` → a muted highlight
  (`theme.ansi[8]` or a blended shade), `none` → normal. Highlight is a
  background fill behind the glyph; the glyph stays the cell's own color.

## Bar layout — top and bottom

The tab bar (top) and search bar (bottom) are independent. Generalize the M1
single offset into two:

```zig
fn topBarRows() usize;    // 1 when the tab bar is visible, else 0
fn bottomBarRows() usize; // 1 when the search bar is open, else 0
```

The terminal grid height is `total_rows - topBarRows() - bottomBarRows()`
(floored at 1). Viewport cells render at raster row `y + topBarRows()`. The
search bar draws at raster row `total_raster_rows - 1`. Opening/closing the
search bar resizes every tab, exactly like the tab-bar show/hide path.

## Input routing

`main.zig` gains a search-active flag. When the search bar is open, `keyDown`
routes to the query editor instead of the active tab's PTY:

- printable keys → append a codepoint to the query (calls `setQuery`)
- Backspace → delete the last codepoint
- Enter → `next`; Esc → close
- ⌘G / ⌘⇧G work whether or not the bar has focus (they also open it if closed)
- all other keys are swallowed while the bar is open (do not reach the shell)

⌘F / ⌘G / ⌘⇧G are matched via the existing `Chord`/`chordMatches` machinery,
parsed from new `Keybindings` fields (`search_open`, `search_next`,
`search_prev`).

## Error handling

| Situation | Behavior |
|---|---|
| Empty query | No matches, no highlight, bar shows `0/0`. |
| No match for a non-empty query | `0/0`; `next`/`prev` are no-ops. |
| Query longer than the 256-byte buffer | Extra input ignored (bar is for short queries). |
| `matches` allocation fails | Treated as no matches; logged; search stays usable. |
| Tab switched while searching | Bar closes, grid resizes back. |
| Invalid search keybinding chord in config | Falls back to that action's default. |

## Testing

`zig build test` is the gate; sub-project 1-2 tests stay green. `Search` is
pure logic over a `Terminal` and gets the coverage:

- A query matching text in the grid → correct `Match` row/col/len.
- A match in scrollback (content row < `history.len()`).
- Smart-case: lowercase query matches mixed-case text; a query with an
  uppercase letter matches case-sensitively.
- Multiple matches on one row, and across rows — ordering is
  top-to-bottom, left-to-right.
- `next`/`prev` wrap around; both no-op on an empty match list.
- Empty query and no-match query → `count() == 0`.
- `classify` returns `current` for the current match's cells, `other` for the
  rest, `none` elsewhere.
- `Terminal.lineCount`/`line` accessors over scrollback + grid.

The search bar rendering, input routing, and viewport-scroll-to-match are
integration-verified (`zig build run`): ⌘F, type, watch highlights, ⌘G/⌘⇧G,
Esc.

## Out of scope (deliberate)

- Regex, whole-word, and fuzzy matching.
- Search-and-replace.
- Cross-tab / all-tabs search.
- Persisting the last query.
- Match-position scrollbar ticks.

## File summary

| File | Change |
|---|---|
| `src/terminal/search.zig` | Create — `Search`, `Match`, the scan. |
| `src/render/searchbar.zig` | Create — `drawSearchBar`. |
| `src/terminal/terminal.zig` | Modify — add `lineCount` + `line` accessors. |
| `src/config/config.zig` | Modify — `search_open`/`search_next`/`search_prev` on `Keybindings`. |
| `src/main.zig` | Modify — search state, input routing, match highlighting, `topBarRows`/`bottomBarRows`. |
