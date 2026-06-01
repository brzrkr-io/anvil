# Editor line-number gutter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render a left line-number gutter in Anvil's native editor, with the current line highlighted, content shifted right, and click/cursor mapping that accounts for the offset.

**Architecture:** All changes live in `src/editor.zig`. The gutter is the leftmost columns of the same `Terminal` grid the editor already paints into — no renderer or shim changes. One new width helper (`gutterWidth`), one new draw helper (`drawGutter`), and edits to `render`/`click`. Three existing tests in `session.zig` and one in `editor.zig` assert content at `grid.at(0,0)`, which now holds a gutter digit — they get updated.

**Tech Stack:** Zig 0.16, `.zig/zig` toolchain. Build: `.zig/zig build`, test: `.zig/zig build test`, format: `.zig/zig fmt src build.zig`, render check: `./zig-out/bin/anvil --dump /tmp/x.png`.

**Commits:** This repo's owner gates commits — do NOT run `git commit` unless the owner says so. Where a task ends in "Commit", stage the files and propose the commit; run it only on the owner's go-ahead.

---

### Task 1: `gutterWidth` helper

Width depends only on line count (digits of the last line number + one pad space), so the content column never jitters while scrolling.

**Files:**
- Modify: `src/editor.zig` (add private method near the other internals, after `lineLen` ~line 147; add test in the test block)

- [ ] **Step 1: Write the failing test**

Add to the test section of `src/editor.zig` (after the "tab expands to spaces" test):

```zig
test "gutterWidth counts digits of the last line plus a pad space" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    // One line -> "1 " -> width 2.
    try std.testing.expectEqual(@as(usize, 2), ed.gutterWidth());
    // Ten lines -> "10 " -> width 3.
    try ed.input("a\nb\nc\nd\ne\nf\ng\nh\ni\nj");
    try std.testing.expectEqual(@as(usize, 3), ed.gutterWidth());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `.zig/zig build test`
Expected: FAIL — `error: no member named 'gutterWidth' in 'Editor'`.

- [ ] **Step 3: Write minimal implementation**

Add this method to the `Editor` struct (place it just above `fn lineLen`):

```zig
/// Width of the line-number gutter: digit count of the last line number plus
/// one trailing pad space. Depends only on `lines.len`, so the content column
/// stays fixed while scrolling. Callers clamp to the grid width.
fn gutterWidth(self: *const Editor) usize {
    var n = self.lines.items.len; // always >= 1
    var digits: usize = 1;
    while (n >= 10) : (n /= 10) digits += 1;
    return digits + 1;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `.zig/zig build test`
Expected: PASS.

- [ ] **Step 5: Commit** (await owner go-ahead)

```bash
git add src/editor.zig
git commit -m "feat(editor): add gutterWidth helper"
```

---

### Task 2: Render the gutter and offset content + cursor

**Files:**
- Modify: `src/editor.zig` — `render` (~line 302), add `drawGutter` helper, update the "render writes cells" test (~line 461), add a new gutter test.

- [ ] **Step 1: Update the existing render test and add a gutter test**

Replace the existing test `"render writes cells and places the cursor"` with the version below (content now starts at column `gw`, and the cursor column is offset by `gw`):

```zig
test "render writes cells and places the cursor" {
    const alloc = std.testing.allocator;
    var term = try Terminal.init(alloc, 4, 10);
    defer term.deinit();
    var ed = try Editor.initEmpty(alloc);
    defer ed.deinit();
    try ed.input("ab"); // one line -> gw = 2
    ed.render(&term, 4, 10);
    // Gutter digit '1' for the (current) first line at col gw-1 = 1.
    try std.testing.expectEqual(@as(u21, '1'), term.grid.at(0, 1).cp);
    // Content shifted right by gw = 2.
    try std.testing.expectEqual(@as(u21, 'a'), term.grid.at(0, 2).cp);
    try std.testing.expectEqual(@as(u21, 'b'), term.grid.at(0, 3).cp);
    try std.testing.expectEqual(@as(u16, 0), term.cy);
    try std.testing.expectEqual(@as(u16, 4), term.cx); // cur_col 2 + gw 2
}
```

Add a new test directly after it:

```zig
test "gutter shows right-aligned line numbers, current line brighter" {
    const alloc = std.testing.allocator;
    var term = try Terminal.init(alloc, 4, 20);
    defer term.deinit();
    var ed = try Editor.initEmpty(alloc);
    defer ed.deinit();
    // 12 lines -> gw = digits(12)+1 = 3. Numbers right-aligned in [0,gw-1),
    // col gw-1 (=2) is the pad space.
    try ed.input("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12");
    ed.cur_row = 0;
    ed.cur_col = 0;
    ed.top = 0;
    ed.render(&term, 4, 20);
    // Row 0 = line 1 -> " 1": col0 blank, col1 '1'.
    try std.testing.expectEqual(@as(u21, ' '), term.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, '1'), term.grid.at(0, 1).cp);
    // Row 3 = line 4 -> " 4".
    try std.testing.expectEqual(@as(u21, '4'), term.grid.at(3, 1).cp);
    // Content starts at col gw = 3.
    try std.testing.expectEqual(@as(u21, 'l'), term.grid.at(0, 3).cp);
    // Current line number is bright (.default); others dim (indexed 8).
    try std.testing.expectEqual(Color.default, term.grid.at(0, 1).fg);
    try std.testing.expectEqual(Color{ .indexed = 8 }, term.grid.at(3, 1).fg);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `.zig/zig build test`
Expected: FAIL — the render tests mismatch (content still at col 0, no gutter digits).

- [ ] **Step 3: Add `drawGutter` and update `render`**

Add this helper to the `Editor` struct (place it just above `fn render`):

```zig
/// Paint a right-aligned 1-based line number into columns [0, gw) of screen
/// row `sr`. The current line's number is bright; others are dim. `term.reset`
/// has already blanked the row, so unwritten leading columns stay spaces,
/// giving right-alignment for free.
fn drawGutter(self: *Editor, term: *Terminal, sr: u16, gw: u16, li: usize) void {
    if (gw < 2) return; // no room for a digit plus the pad space
    const fg: Color = if (li == self.cur_row) .default else .{ .indexed = 8 };
    var num = li + 1;
    var c: u16 = gw - 2; // rightmost digit column (gw-1 is the pad space)
    while (true) {
        term.grid.at(sr, c).* = .{ .cp = @intCast('0' + (num % 10)), .fg = fg };
        num /= 10;
        if (num == 0 or c == 0) break;
        c -= 1;
    }
}
```

Replace the body of `render` with the gutter-aware version. The current `render` is:

```zig
    pub fn render(self: *Editor, term: *Terminal, rows: u16, cols: u16) void {
        term.reset();
        if (self.follow) self.ensureVisible(rows);

        var tok_buf: [512]syntax.Token = undefined;
        var sr: u16 = 0;
        while (sr < rows) : (sr += 1) {
            const li = self.top + sr;
            if (li >= self.lines.items.len) break;
            const line = self.lines.items[li].items;
            const n_toks = syntax.tokenizeLine(self.lang, line, &tok_buf);

            var col: u16 = 0;
            var ti: usize = 0;
            while (ti < n_toks and col < cols) : (ti += 1) {
                const tok = tok_buf[ti];
                const fg = roleColor(tok.role);
                var bi: usize = tok.start;
                while (bi < tok.start + tok.len and col < cols) : (bi += 1) {
                    const byte = line[bi];
                    const cp: u21 = if (byte >= 0x20 and byte < 0x7f) byte else ' ';
                    term.grid.at(sr, col).* = .{ .cp = cp, .fg = fg };
                    col += 1;
                }
            }
        }

        // After a manual scroll the cursor may sit outside the viewport; clamp
        // its on-screen row to the visible edge so it never underflows or draws
        // off-grid.
        const max_row: usize = if (rows > 0) rows - 1 else 0;
        const scr_row: usize = if (self.cur_row < self.top)
            0
        else
            @min(self.cur_row - self.top, max_row);
        const max_col: usize = if (cols > 0) cols - 1 else 0;
        const scr_col = @min(self.cur_col, max_col);
        term.setCursor(@intCast(scr_row + 1), @intCast(scr_col + 1));
    }
```

Replace it with:

```zig
    pub fn render(self: *Editor, term: *Terminal, rows: u16, cols: u16) void {
        term.reset();
        if (self.follow) self.ensureVisible(rows);

        // Gutter occupies the leftmost columns; clamp so content keeps >= 1 col.
        const gw_cap: usize = if (cols > 0) cols - 1 else 0;
        const gw: u16 = @intCast(@min(self.gutterWidth(), gw_cap));

        var tok_buf: [512]syntax.Token = undefined;
        var sr: u16 = 0;
        while (sr < rows) : (sr += 1) {
            const li = self.top + sr;
            if (li >= self.lines.items.len) break;

            self.drawGutter(term, sr, gw, li);

            const line = self.lines.items[li].items;
            const n_toks = syntax.tokenizeLine(self.lang, line, &tok_buf);

            var col: u16 = gw;
            var ti: usize = 0;
            while (ti < n_toks and col < cols) : (ti += 1) {
                const tok = tok_buf[ti];
                const fg = roleColor(tok.role);
                var bi: usize = tok.start;
                while (bi < tok.start + tok.len and col < cols) : (bi += 1) {
                    const byte = line[bi];
                    const cp: u21 = if (byte >= 0x20 and byte < 0x7f) byte else ' ';
                    term.grid.at(sr, col).* = .{ .cp = cp, .fg = fg };
                    col += 1;
                }
            }
        }

        // After a manual scroll the cursor may sit outside the viewport; clamp
        // its on-screen row to the visible edge so it never underflows or draws
        // off-grid.
        const max_row: usize = if (rows > 0) rows - 1 else 0;
        const scr_row: usize = if (self.cur_row < self.top)
            0
        else
            @min(self.cur_row - self.top, max_row);
        const max_col: usize = if (cols > 0) cols - 1 else 0;
        const scr_col = @min(self.cur_col + gw, max_col);
        term.setCursor(@intCast(scr_row + 1), @intCast(scr_col + 1));
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `.zig/zig build test`
Expected: PASS for both render tests (the `session.zig` ripple tests still fail — fixed in Task 4).

- [ ] **Step 5: Commit** (await owner go-ahead)

```bash
git add src/editor.zig
git commit -m "feat(editor): render line-number gutter, offset content and cursor"
```

---

### Task 3: Click maps past the gutter

**Files:**
- Modify: `src/editor.zig` — `click` (~line 286), add a test.

- [ ] **Step 1: Write the failing test**

Add after the gutter render test:

```zig
test "click maps past the gutter to the buffer column" {
    const alloc = std.testing.allocator;
    var ed = try Editor.initEmpty(alloc);
    defer ed.deinit();
    try ed.input("hello world"); // one line -> gw = 2
    ed.click(0, 2 + 3, 4); // screen col gw+3 -> buffer col 3
    try std.testing.expectEqual(@as(usize, 0), ed.cur_row);
    try std.testing.expectEqual(@as(usize, 3), ed.cur_col);
    // A click inside the gutter saturates to buffer column 0.
    ed.click(0, 1, 4);
    try std.testing.expectEqual(@as(usize, 0), ed.cur_col);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `.zig/zig build test`
Expected: FAIL — `cur_col` is 5 (gutter not subtracted) instead of 3.

- [ ] **Step 3: Update `click`**

Current `click`:

```zig
    pub fn click(self: *Editor, screen_row: usize, screen_col: usize, rows: u16) void {
        _ = rows;
        const target = self.top + screen_row;
        self.cur_row = if (target >= self.lines.items.len)
            (if (self.lines.items.len > 0) self.lines.items.len - 1 else 0)
        else
            target;
        self.cur_col = @min(screen_col, self.lineLen(self.cur_row));
        self.goal_col = self.cur_col;
    }
```

Replace the `cur_col` line so it subtracts the gutter width (saturating):

```zig
    pub fn click(self: *Editor, screen_row: usize, screen_col: usize, rows: u16) void {
        _ = rows;
        const target = self.top + screen_row;
        self.cur_row = if (target >= self.lines.items.len)
            (if (self.lines.items.len > 0) self.lines.items.len - 1 else 0)
        else
            target;
        const buf_col = screen_col -| self.gutterWidth();
        self.cur_col = @min(buf_col, self.lineLen(self.cur_row));
        self.goal_col = self.cur_col;
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `.zig/zig build test`
Expected: PASS for the click test.

- [ ] **Step 5: Commit** (await owner go-ahead)

```bash
git add src/editor.zig
git commit -m "feat(editor): account for gutter width in click mapping"
```

---

### Task 4: Fix `session.zig` ripple tests + full verify

The gutter shifts content right by `gw = 2` for the single-line buffers these tests use, and the editor cursor column gains `+gw`.

**Files:**
- Modify: `src/session.zig` — tests at ~line 240-242, ~line 265, ~line 270.

- [ ] **Step 1: Update the assertions**

In test `"editor session: load renders grid, type + save round-trips through disk"`, the buffer is `"const x = 1;\n"` (one line, `gw = 2`). Change:

```zig
    try std.testing.expectEqual(@as(u21, 'c'), s.term.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u16, 0), s.term.cy);
    try std.testing.expectEqual(@as(u16, 0), s.term.cx);
```

to:

```zig
    try std.testing.expectEqual(@as(u21, 'c'), s.term.grid.at(0, 2).cp); // content past gw=2
    try std.testing.expectEqual(@as(u16, 0), s.term.cy);
    try std.testing.expectEqual(@as(u16, 2), s.term.cx); // cur_col 0 + gw 2
```

In test `"reloadEditor swaps the buffer in place, repainting the grid"` (buffers `"alpha\n"` and `"bravo\n"`, one line each, `gw = 2`). Change:

```zig
    try std.testing.expectEqual(@as(u21, 'a'), s.term.grid.at(0, 0).cp);
```

to:

```zig
    try std.testing.expectEqual(@as(u21, 'a'), s.term.grid.at(0, 2).cp);
```

and:

```zig
    try std.testing.expectEqual(@as(u21, 'b'), s.term.grid.at(0, 0).cp);
```

to:

```zig
    try std.testing.expectEqual(@as(u21, 'b'), s.term.grid.at(0, 2).cp);
```

- [ ] **Step 2: Run the full test suite**

Run: `.zig/zig build test`
Expected: PASS (all editor + session tests green).

- [ ] **Step 3: Format + render check**

Run: `.zig/zig fmt src build.zig && .zig/zig build && ./zig-out/bin/anvil --dump /tmp/x.png`
Expected: fmt clean, build rc 0, dump rc 0.

- [ ] **Step 4: Live check (owner-driven)**

Build + redeploy as the owner prefers, open a multi-line file in the editor, confirm: right-aligned numbers render, the current line's number is brighter, clicking a character places the cursor on that character (not shifted), and scrolling keeps numbers aligned with content.

- [ ] **Step 5: Commit** (await owner go-ahead)

```bash
git add src/session.zig
git commit -m "test(session): account for editor gutter offset in grid assertions"
```

---

## Notes

- `Color` is already imported in `editor.zig` (`const Color = @import("vt/cell.zig").Color;`), so `Color.default` / `Color{ .indexed = 8 }` need no new import.
- `grid.at(r, col)` takes `u16` args and returns `*Cell` (`cp: u21`, `fg: Color`). `gw` is built as `u16` to match.
- No `app.zig` or shim change: `app.zig`'s `editorClick` passes the raw grid column; the gutter subtraction lives entirely in `Editor.click`.
