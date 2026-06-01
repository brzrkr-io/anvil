# In-Terminal Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Incremental, highlight-all search over the active tab's grid + scrollback, with a low-profile bottom search bar.

**Architecture:** A pure `Search` (in `src/terminal/`) scans a `Terminal`'s content rows into a match list. `Terminal` gains read accessors over the combined scrollback+grid line space. `main.zig` owns the search bar state, routes input, and tints matched cells; `searchbar.zig` draws the bottom bar.

**Tech Stack:** Zig 0.16, the M1 terminal/render layers, the M2 config + multi-tab work.

**Spec:** `docs/superpowers/specs/2026-05-21-in-terminal-search-design.md` — read it first.

**Branch:** Continue on `feat/m2-config-theme` (sub-projects stack; no new branch).

**Repo facts to build on:** `Cell.cp` is a `u21` (defaults to `' '`). `Grid.rowConst(y) []const Cell`, `grid.width`/`grid.height`. `Terminal` has `viewportRow`, `viewportOffset`, `scrollViewport`, `history` (a `Scrollback` with `len()`/`get(i)`), `active()`/`activeConst()`. The multi-tab work added `topBarRows`-style offsetting in `main.zig` (the tab bar steals the top row) — search adds the bottom counterpart.

---

## Task 1: `Terminal` content-line accessors

**Files:**
- Modify: `src/terminal/terminal.zig`

Search and the renderer need one index space over scrollback + grid. Add four accessors.

- [ ] **Step 1: Add the accessors to `Terminal`** (after `viewportRow`, before the title/cwd accessors)

```zig
    // --- content-line access (for search) ---------------------------------

    /// Total content rows: scrollback length + active grid height.
    pub fn lineCount(self: *const Terminal) usize {
        return self.history.len() + self.activeConst().height;
    }

    /// Borrow content row `i`: 0..history.len() are scrollback rows (possibly
    /// shorter than grid width); the rest are active-grid rows (full width).
    /// Out-of-range returns an empty slice.
    pub fn line(self: *const Terminal, i: usize) []const Cell {
        const hist = self.history.len();
        if (i < hist) return self.history.get(i);
        const g = self.activeConst();
        const gy = i - hist;
        if (gy >= g.height) return &.{};
        return g.rowConst(gy);
    }

    /// The content row currently shown at viewport position `y`.
    pub fn contentRowOfViewport(self: *const Terminal, y: usize) usize {
        if (self.viewport_offset > y)
            return self.history.len() - self.viewport_offset + y;
        return self.history.len() + (y - self.viewport_offset);
    }

    /// Scroll the viewport so content row `target` is visible. Grid rows are
    /// always visible at offset 0; a scrollback row is brought near the top.
    pub fn scrollToLine(self: *Terminal, target: usize) void {
        const hist = self.history.len();
        if (target >= hist) {
            self.viewport_offset = 0;
        } else {
            self.viewport_offset = @min(hist - target, hist);
        }
    }
```

- [ ] **Step 2: Write the failing tests** (in `terminal.zig`'s test block, using the existing `makeTerminal` helper)

```zig
test "lineCount and line span scrollback then grid" {
    var t = try makeTerminal(10, 3);
    defer t.deinit();
    // Fresh terminal: no scrollback, grid height 3.
    try testing.expectEqual(@as(usize, 3), t.lineCount());
    // Feed enough newlines to push rows into scrollback.
    t.feed("a\r\nb\r\nc\r\nd\r\ne\r\n");
    try testing.expect(t.lineCount() > 3);
    // The oldest content row is a scrollback row; the last is a grid row.
    const last = t.line(t.lineCount() - 1);
    try testing.expectEqual(@as(usize, 10), last.len); // grid row is full width
}

test "contentRowOfViewport matches viewport composition" {
    var t = try makeTerminal(10, 3);
    defer t.deinit();
    t.feed("1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n");
    // At offset 0, viewport row 0 is the first grid row.
    try testing.expectEqual(t.history.len(), t.contentRowOfViewport(0));
}
```

- [ ] **Step 3: Run the tests**

Run: `zig build test --summary all`
Expected: PASS, zero failures. If `history`/`activeConst`/`viewport_offset` field or method names differ, adjust to the real `terminal.zig`.

- [ ] **Step 4: Commit**

```bash
git add src/terminal/terminal.zig
git commit -m "feat(terminal): content-line accessors for search"
```

(End every commit message in this plan with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.)

---

## Task 2: `Search` — struct and match scan

**Files:**
- Create: `src/terminal/search.zig`
- Modify: `src/main.zig` (add `_ = @import("terminal/search.zig");` to the `test {}` block)

The `Search` struct, the query, and the scan that fills the match list.

- [ ] **Step 1: Write `src/terminal/search.zig`**

```zig
//! Incremental substring search over a Terminal's content rows (scrollback +
//! grid). Pure logic — the renderer and input layer drive it.

const std = @import("std");
const Terminal = @import("terminal.zig").Terminal;
const Cell = @import("cell.zig").Cell;

/// A run of matched cells on one content row.
pub const Match = struct {
    row: usize, // content_row index
    col: usize, // start column
    len: usize, // length in cells (query codepoint count)
};

pub const MatchKind = enum { none, other, current };

/// Upper bound on stored matches — a 1-character query over deep scrollback
/// could otherwise match millions of times.
pub const max_matches = 2048;

pub const Search = struct {
    alloc: std.mem.Allocator,
    query_buf: [256]u8 = undefined,
    query_len: usize = 0,
    matches: std.ArrayList(Match),
    current: usize = 0, // index into matches; 0 when empty

    pub fn init(alloc: std.mem.Allocator) Search {
        return .{ .alloc = alloc, .matches = std.ArrayList(Match).empty };
    }

    pub fn deinit(self: *Search) void {
        self.matches.deinit(self.alloc);
    }

    pub fn query(self: *const Search) []const u8 {
        return self.query_buf[0..self.query_len];
    }

    pub fn count(self: *const Search) usize {
        return self.matches.items.len;
    }

    pub fn currentMatch(self: *const Search) ?Match {
        if (self.matches.items.len == 0) return null;
        return self.matches.items[self.current];
    }

    /// Replace the query text and re-scan `term`. Truncates text past 256
    /// bytes. `current` resets to 0.
    pub fn setQuery(self: *Search, term: *const Terminal, text: []const u8) void {
        const n = @min(text.len, self.query_buf.len);
        @memcpy(self.query_buf[0..n], text[0..n]);
        self.query_len = n;
        self.rescan(term);
    }

    /// Re-run the scan with the existing query (e.g. after new shell output).
    pub fn rescan(self: *Search, term: *const Terminal) void {
        self.matches.clearRetainingCapacity();
        self.current = 0;

        // Decode the query to codepoints; detect smart-case.
        var q: [256]u21 = undefined;
        var qn: usize = 0;
        var case_sensitive = false;
        var it = std.unicode.Utf8View.initUnchecked(self.query()).iterator();
        while (it.nextCodepoint()) |cp| {
            if (qn >= q.len) break;
            if (cp >= 'A' and cp <= 'Z') case_sensitive = true;
            q[qn] = cp;
            qn += 1;
        }
        if (qn == 0) return; // empty query -> no matches

        var r: usize = 0;
        const total = term.lineCount();
        while (r < total) : (r += 1) {
            const row = term.line(r);
            if (row.len < qn) continue;
            var c: usize = 0;
            while (c + qn <= row.len) : (c += 1) {
                if (rowMatchesAt(row, c, q[0..qn], case_sensitive)) {
                    self.matches.append(self.alloc, .{ .row = r, .col = c, .len = qn }) catch return;
                    if (self.matches.items.len >= max_matches) return;
                }
            }
        }
    }
};

/// True when `row[col..col+q.len]` equals `q` (codepoint-wise, case-folded
/// for ASCII letters unless `case_sensitive`).
fn rowMatchesAt(row: []const Cell, col: usize, q: []const u21, case_sensitive: bool) bool {
    for (q, 0..) |qc, i| {
        const cc = row[col + i].cp;
        if (case_sensitive) {
            if (cc != qc) return false;
        } else {
            if (lowerCp(cc) != lowerCp(qc)) return false;
        }
    }
    return true;
}

/// Lowercase an ASCII letter codepoint; everything else unchanged.
fn lowerCp(cp: u21) u21 {
    return if (cp >= 'A' and cp <= 'Z') cp + 32 else cp;
}
```

- [ ] **Step 2: Add the module to the test aggregator**

In `src/main.zig`'s `test { }` block add:

```zig
    _ = @import("terminal/search.zig");
```

- [ ] **Step 3: Write the failing tests** (end of `src/terminal/search.zig`)

The tests build a `Terminal` directly (its `makeTerminal` test helper is not exported):

```zig
const testing = std.testing;

test "finds a substring in the grid" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("hello world");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "world");
    try testing.expectEqual(@as(usize, 1), s.count());
    const m = s.currentMatch().?;
    try testing.expectEqual(@as(usize, 6), m.col);
    try testing.expectEqual(@as(usize, 5), m.len);
}

test "smart-case: lowercase query is case-insensitive" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("The Cat SAT");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "cat");
    try testing.expectEqual(@as(usize, 1), s.count()); // matches "Cat"
}

test "smart-case: an uppercase letter forces case-sensitive" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("the cat Cat");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "Cat");
    try testing.expectEqual(@as(usize, 1), s.count()); // only the capitalized one
}

test "empty query yields no matches" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("anything");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "");
    try testing.expectEqual(@as(usize, 0), s.count());
    try testing.expect(s.currentMatch() == null);
}

test "finds multiple matches left-to-right" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("aa aa aa");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "aa");
    try testing.expectEqual(@as(usize, 3), s.count());
}
```

- [ ] **Step 4: Run the tests**

Run: `zig build test --summary all`
Expected: PASS, zero failures. If `std.ArrayList(Match).empty` or the `Utf8View` API differs in the installed Zig 0.16, adjust (the multi-tab work confirmed `std.ArrayList(T).empty` + `append(alloc, x)` + `deinit(alloc)`).

- [ ] **Step 5: Commit**

```bash
git add src/terminal/search.zig src/main.zig
git commit -m "feat(search): Search struct and substring scan"
```

---

## Task 3: `Search` — navigation and classification

**Files:**
- Modify: `src/terminal/search.zig`

`next`/`prev` cycle the current match; `classify` tells the renderer how to tint a cell.

- [ ] **Step 1: Add the methods to `Search`** (inside the struct, after `rescan`)

```zig
    /// Advance the current match (wraps). No-op when there are no matches.
    pub fn next(self: *Search) void {
        if (self.matches.items.len == 0) return;
        self.current = (self.current + 1) % self.matches.items.len;
    }

    /// Step the current match back (wraps). No-op when there are no matches.
    pub fn prev(self: *Search) void {
        if (self.matches.items.len == 0) return;
        self.current = (self.current + self.matches.items.len - 1) % self.matches.items.len;
    }

    /// How the cell at content (`row`,`col`) should be tinted.
    pub fn classify(self: *const Search, row: usize, col: usize) MatchKind {
        for (self.matches.items, 0..) |m, i| {
            if (m.row != row) continue;
            if (col >= m.col and col < m.col + m.len) {
                return if (i == self.current) .current else .other;
            }
        }
        return .none;
    }
```

- [ ] **Step 2: Write the failing tests** (add to `search.zig`'s test block)

```zig
test "next and prev wrap around" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("x x x");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "x");
    try testing.expectEqual(@as(usize, 3), s.count());
    try testing.expectEqual(@as(usize, 0), s.current);
    s.next();
    try testing.expectEqual(@as(usize, 1), s.current);
    s.next();
    s.next();
    try testing.expectEqual(@as(usize, 0), s.current); // wrapped
    s.prev();
    try testing.expectEqual(@as(usize, 2), s.current); // wrapped back
}

test "next/prev are no-ops with no matches" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("abc");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "zzz");
    s.next();
    s.prev();
    try testing.expectEqual(@as(usize, 0), s.current);
}

test "classify tags current, other, and none" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("ab ab");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "ab");
    // current match is index 0 at (grid row 0, col 0..2)
    const r0 = t.contentRowOfViewport(0);
    try testing.expectEqual(MatchKind.current, s.classify(r0, 0));
    try testing.expectEqual(MatchKind.current, s.classify(r0, 1));
    try testing.expectEqual(MatchKind.other, s.classify(r0, 3));
    try testing.expectEqual(MatchKind.none, s.classify(r0, 2)); // the space
}
```

- [ ] **Step 3: Run the tests**

Run: `zig build test --summary all`
Expected: PASS, zero failures.

- [ ] **Step 4: Commit**

```bash
git add src/terminal/search.zig
git commit -m "feat(search): match navigation and cell classification"
```

---

## Task 4: Search keybindings in config

**Files:**
- Modify: `src/config/config.zig`

Add three search actions to the `Keybindings` struct (the multi-tab work created it).

- [ ] **Step 1: Add fields to `Keybindings`** in `src/config/config.zig`

Add these three fields to the existing `Keybindings` struct (alongside `new_tab` etc.):

```zig
    search_open: []const u8 = "cmd+f",
    search_next: []const u8 = "cmd+g",
    search_prev: []const u8 = "cmd+shift+g",
```

- [ ] **Step 2: Write the failing test** (in `config.zig`'s test block)

```zig
test "config parses a search keybinding override" {
    var loaded = try parseSlice(testing.allocator,
        ".{ .keybindings = .{ .search_open = \"ctrl+s\" } }");
    defer loaded.deinit();
    try testing.expectEqualStrings("ctrl+s", loaded.config.keybindings.search_open);
    try testing.expectEqualStrings("cmd+g", loaded.config.keybindings.search_next); // default
}
```

- [ ] **Step 3: Run the tests**

Run: `zig build test --summary all`
Expected: PASS, zero failures.

- [ ] **Step 4: Commit**

```bash
git add src/config/config.zig
git commit -m "feat(config): search keybindings"
```

---

## Task 5: `searchbar.zig` — the bottom search bar

**Files:**
- Create: `src/render/searchbar.zig`
- Modify: `src/main.zig` (add `_ = @import("render/searchbar.zig");` to the `test {}` block)

`drawSearchBar` paints the bottom raster row: the query text and a `current/total` counter.

- [ ] **Step 1: Write `src/render/searchbar.zig`**

```zig
//! The in-terminal search bar — one text row at the bottom of the window.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;
const Search = @import("../terminal/search.zig").Search;

/// Draw the search bar across the bottom raster row. `bottom_row` is the cell
/// row index of that last row. Shows a "find:" prefix, the query, and a
/// `current/total` match counter.
pub fn drawSearchBar(
    raster: *Raster,
    font: Font,
    theme: Theme,
    search: *const Search,
    bottom_row: usize,
) void {
    const cell_w = font.metrics.cell_w;
    const total_cols: usize = @intFromFloat(@as(f64, @floatFromInt(raster.width)) / cell_w);
    if (total_cols == 0) return;

    // Bar background across the whole bottom row.
    var c: usize = 0;
    while (c < total_cols) : (c += 1) raster.cellBg(font, c, bottom_row, theme.ansi[8]);

    // Compose the bar text: "find: <query>" left-aligned, "<cur>/<total>" right.
    var line_buf: [512]u8 = undefined;
    const cur = if (search.count() == 0) 0 else search.current + 1;
    const text = std.fmt.bufPrint(&line_buf, "find: {s}", .{search.query()}) catch "find:";
    var i: usize = 0;
    while (i < text.len and i < total_cols) : (i += 1) {
        raster.cellGlyph(font, i, bottom_row, font.glyph(text[i]), theme.foreground);
    }

    var count_buf: [32]u8 = undefined;
    const counter = std.fmt.bufPrint(&count_buf, "{d}/{d}", .{ cur, search.count() }) catch "";
    if (counter.len < total_cols) {
        const start = total_cols - counter.len;
        for (counter, 0..) |ch, j| {
            raster.cellGlyph(font, start + j, bottom_row, font.glyph(ch), theme.foreground);
        }
    }
}
```

- [ ] **Step 2: Add the module to the test aggregator**

In `src/main.zig`'s `test { }` block add:

```zig
    _ = @import("render/searchbar.zig");
```

- [ ] **Step 3: Write the failing test** (end of `src/render/searchbar.zig`)

```zig
const testing = std.testing;

test "drawSearchBar fills the bottom row background" {
    const f = try Font.init("Menlo", 26.0);
    defer f.deinit();
    var r = try Raster.init(testing.allocator, 400, 200);
    defer r.deinit();
    r.clear(.{ 0, 0, 0 });

    var s = Search.init(testing.allocator);
    defer s.deinit();

    const theme = @import("../config/theme.zig").mineral_dark;
    const cell_h: usize = @intFromFloat(f.metrics.cell_h);
    const bottom_row: usize = (200 / cell_h) - 1;
    drawSearchBar(&r, f, theme, &s, bottom_row);

    // A pixel inside the bottom row now carries the bar background (ansi[8]).
    const px_y: usize = bottom_row * cell_h + cell_h / 2;
    const px = (px_y * r.width + 4) * 4;
    try testing.expectEqual(theme.ansi[8][2], r.pixels[px + 0]); // B channel == ansi8 blue
}
```

- [ ] **Step 4: Run the tests**

Run: `zig build test --summary all`
Expected: PASS, zero failures.

- [ ] **Step 5: Commit**

```bash
git add src/render/searchbar.zig src/main.zig
git commit -m "feat(render): bottom search bar"
```

---

## Task 6: Search render integration in `main.zig`

**Files:**
- Modify: `src/main.zig`

Add the search state, the top/bottom bar-row split, draw the search bar, resize on open/close, and tint matched cells. **Locate edits by content** — line numbers drift.

- [ ] **Step 1: Add state and imports**

- Import: `const Search = @import("terminal/search.zig").Search;`, `const searchbar = @import("render/searchbar.zig");`, and `const SearchMod = @import("terminal/search.zig");` (for `MatchKind`).
- Add to `App`: `search: Search,` and `search_open: bool = false,`.
- In `main`, after `g` is initialized: `g.search = Search.init(alloc);` — or include `.search = Search.init(alloc)` in the `g` initializer (initializer is cleaner; `Search.init` only needs the allocator).

- [ ] **Step 2: Generalize the bar-row offset**

The multi-tab work added a `barRows()` returning the *top* offset. Rename/replace it with two helpers and update every caller:

```zig
/// Rows taken by the tab bar at the top (0 or 1).
fn topBarRows() usize {
    return if (g.tabs.barVisible()) 1 else 0;
}
/// Rows taken by the search bar at the bottom (0 or 1).
fn bottomBarRows() usize {
    return if (g.search_open) 1 else 0;
}
```

Find every current use of `barRows()` and replace with `topBarRows()`. In `resizeAllTabs`, the grid row count becomes `@max(total_rows -| topBarRows() -| bottomBarRows(), 1)`.

- [ ] **Step 3: Draw the search bar in `renderFrame`**

After the tab bar is drawn and the active tab's terminal is rendered, add:

```zig
    if (g.search_open) {
        const ch: usize = @intFromFloat(g.font.metrics.cell_h);
        const total_rows = @max(g.raster.height / ch, 1);
        searchbar.drawSearchBar(&g.raster, g.font, g.theme, &g.search, total_rows - 1);
    }
```

- [ ] **Step 4: Tint matched cells in `drawCell`**

`drawCell` already computes `ry = y + topBarRows()` (formerly `barRows()`) and resolves `bg`. Override `bg` for matched cells when search is open — but **only on non-cursor cells**, so the cursor still wins on its own cell. Place this block right *after* the `if (is_cursor) { ... }` handling, guarded with `!is_cursor`:

```zig
    if (g.search_open and !is_cursor) {
        const crow = g.tabs.current().terminal.contentRowOfViewport(y);
        switch (g.search.classify(crow, x)) {
            .current => bg = g.theme.accent,
            .other => bg = g.theme.ansi[8],
            .none => {},
        }
    }
```

The existing background-draw condition (`is_cursor or bg != background`) already fires for a match-tinted `bg`, so the highlight is painted. The glyph keeps the cell's own `fg`.

- [ ] **Step 5: Add open/close helpers**

```zig
/// Open the search bar (re-scanning the active tab) and reflow for the row.
fn openSearch() void {
    if (g.search_open) return;
    g.search_open = true;
    g.search.setQuery(&g.tabs.current().terminal, g.search.query());
    resizeAllTabs();
    g.dirty = true;
}
/// Close the search bar and reflow.
fn closeSearch() void {
    if (!g.search_open) return;
    g.search_open = false;
    resizeAllTabs();
    g.dirty = true;
}
```

- [ ] **Step 6: Rescan on new output, and close on tab switch**

- In `onTick`, after a tab is drained and fed, if `g.search_open` and that tab is the active tab, call `g.search.rescan(&g.tabs.current().terminal)` and set `g.dirty = true`.
- In `handleTabKey`, every branch that changes `g.tabs.active` (next/prev/jump) must call `closeSearch()` before returning (the spec: switching tabs closes search).

- [ ] **Step 7: Build, test, run**

Run: `zig build test --summary all` — all tests pass, zero failures.
Run: `zig build` — exit 0, no warnings.
Run: `( zig build run & sleep 5; kill %1 2>/dev/null )` — confirm the app still launches and renders (search bar not yet reachable without Task 7 input).

- [ ] **Step 8: Commit**

```bash
git add src/main.zig
git commit -m "feat(search): render integration — bar, offset, match tint"
```

---

## Task 7: Search input routing in `main.zig`

**Files:**
- Modify: `src/main.zig`

⌘F/⌘G/⌘⇧G open and navigate; while the bar is open, typing edits the query.

- [ ] **Step 1: Parse the search keybindings**

Add to `App`: `keys_search_open: ?cfg_mod.Chord = null`, `keys_search_next: ?cfg_mod.Chord = null`, `keys_search_prev: ?cfg_mod.Chord = null`. In `loadKeybindings` (added by multi-tab Task 7), parse the three new fields:

```zig
    g.keys_search_open = cfg_mod.parseChord(kb.search_open);
    g.keys_search_next = cfg_mod.parseChord(kb.search_next);
    g.keys_search_prev = cfg_mod.parseChord(kb.search_prev);
```

- [ ] **Step 2: Handle search chords in `handleTabKey`** (or a sibling `handleSearchKey`)

Add, near the other chord checks (these are ⌘ combos, matched the same way):

```zig
    if (g.keys_search_open) |chd| if (chordMatches(chd, mods, cp)) {
        openSearch();
        return true;
    };
    if (g.keys_search_next) |chd| if (chordMatches(chd, mods, cp)) {
        if (!g.search_open) openSearch();
        g.search.next();
        scrollToCurrentMatch();
        g.dirty = true;
        return true;
    };
    if (g.keys_search_prev) |chd| if (chordMatches(chd, mods, cp)) {
        if (!g.search_open) openSearch();
        g.search.prev();
        scrollToCurrentMatch();
        g.dirty = true;
        return true;
    };
```

Add the helper:

```zig
/// Scroll the active tab so the current search match is visible.
fn scrollToCurrentMatch() void {
    if (g.search.currentMatch()) |m| {
        g.tabs.current().terminal.scrollToLine(m.row);
    }
}
```

- [ ] **Step 3: Route query editing while the bar is open**

In `onKeyDown`, before the normal (non-⌘) key path sends bytes to the PTY, intercept when `g.search_open`. The query editor handles printable text, Backspace, Enter, Esc:

```zig
    // (after the ⌘-combo block, before extractKey for normal keys)
    if (g.search_open) {
        const p = extractKey(event) orelse return;
        switch (p.key) {
            .escape => closeSearch(),
            .enter => {
                g.search.next();
                scrollToCurrentMatch();
                g.dirty = true;
            },
            .backspace => {
                // Drop the last UTF-8 codepoint from the query.
                var qlen = g.search.query_len;
                while (qlen > 0 and (g.search.query_buf[qlen - 1] & 0xC0) == 0x80) qlen -= 1;
                if (qlen > 0) qlen -= 1;
                const q = g.search.query_buf[0..qlen];
                g.search.setQuery(&g.tabs.current().terminal, q);
                scrollToCurrentMatch();
                g.dirty = true;
            },
            .text => |cp| {
                // Append the codepoint's UTF-8 to the query and re-scan.
                var tmp: [256]u8 = undefined;
                const base = g.search.query();
                if (base.len + 4 <= tmp.len) {
                    @memcpy(tmp[0..base.len], base);
                    const n = std.unicode.utf8Encode(cp, tmp[base.len..]) catch 0;
                    g.search.setQuery(&g.tabs.current().terminal, tmp[0 .. base.len + n]);
                    scrollToCurrentMatch();
                    g.dirty = true;
                }
            },
            else => {}, // arrows etc. ignored while searching
        }
        return; // search swallows the key — never reaches the shell
    }
```

Confirm the `keys.Key` union tag names (`.escape`, `.enter`, `.backspace`, `.text`) against `src/app/keys.zig` and adjust if they differ. `setQuery` truncates past 256 bytes, so the `query_buf` is never overrun even though the editor guards at 252.

- [ ] **Step 4: Build, test, run**

Run: `zig build test --summary all` — all tests pass, zero failures.
Run: `zig build` — exit 0, no warnings; `( zig build run & sleep 6; kill %1 2>/dev/null )` — no crash.

- [ ] **Step 5: Commit**

```bash
git add src/main.zig
git commit -m "feat(search): keyboard routing — open, navigate, query editing"
```

---

## Task 8: End-to-end verification and closeout

**Files:**
- Modify: `todo.txt`, `wiki/`

- [ ] **Step 1: Full test run**

Run: `zig build test --summary all` — every test passes, zero failures.

- [ ] **Step 2: Interactive verification**

Run `zig build run`. Verify by hand:
- ⌘F opens the bottom search bar; the grid shrinks by one row.
- Typing a query highlights all matches live; the current match is in the accent color, others muted.
- ⌘G / Enter steps forward, ⌘⇧G steps back; the viewport scrolls to keep the current match visible (including matches up in scrollback).
- The `current/total` counter updates.
- Esc closes the bar; the grid grows back; highlights clear.
- Switching tabs (⌘⇧]) while searching closes the bar.
- New shell output while searching keeps matches correct.

Capture a screenshot with the bar open and matches highlighted (`screencapture -x`). Do not leave a config file in `~/.config/anvil/`.

- [ ] **Step 3: Close out docs**

- `todo.txt`: check off the M2 in-terminal-search item.
- `wiki/`: add `wiki/concepts/search-system.md` (frontmatter per `wiki/index.md`) covering `Search`, the content-row index space, smart-case, the match cap, and the bottom-bar offset; link it from `wiki/index.md`. Append a `wiki/log.md` entry.

- [ ] **Step 4: Commit**

```bash
git add todo.txt wiki/
git commit -m "docs: record M2 in-terminal-search sub-project"
```

---

## Done criteria

- `zig build test` passes; sub-project 1-2 tests plus the new terminal/search/config/searchbar tests are all green.
- `zig build run`: ⌘F search works — incremental highlight-all, ⌘G/⌘⇧G navigation with scroll-to-match, Esc closes; the bar auto-reflows the grid.
- This completes M2 sub-project 3 of 4. Last: shell integration.
