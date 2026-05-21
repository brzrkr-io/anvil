//! The active terminal screen: a fixed `cols x rows` cell matrix plus the
//! cursor, a scroll region, an SGR "pen" template, and mode flags.
//!
//! The grid knows nothing about parsing or scrollback — it exposes primitive
//! editing operations (print, line feed, erase, insert/delete, scroll) and
//! the `Terminal` composes them. `lineFeed` returns the row that scrolled off
//! the top of the scroll region so the caller can archive it.

const std = @import("std");
const cell = @import("cell.zig");
const Cell = cell.Cell;

/// Tab stops sit every 8 columns, the conventional terminal default.
const tab_width: usize = 8;

/// The mode flags a grid tracks. Higher-level DEC private modes live on the
/// `Terminal`; these are the ones that change cell-writing behavior.
pub const Modes = struct {
    autowrap: bool = true,
    origin: bool = false,
    cursor_visible: bool = true,
    insert: bool = false,
};

/// An inclusive vertical scroll region. Defaults to the whole screen.
pub const ScrollRegion = struct {
    top: usize = 0,
    bottom: usize = 0,
};

pub const Grid = struct {
    alloc: std.mem.Allocator,
    width: usize,
    height: usize,
    /// Row-major cell matrix, `width * height` cells.
    cells: []Cell,
    /// A stable copy of the most recently scrolled-off top row. `scrollUp`
    /// fills this *before* mutating `cells`, so the slice it returns stays
    /// valid for the caller to archive into scrollback.
    scrolled_off: []Cell,

    cur_x: usize = 0,
    cur_y: usize = 0,
    /// True once the cursor "should have wrapped" but autowrap is deferred —
    /// the next printable character wraps first. This is the standard
    /// pending-wrap (a.k.a. last-column) latch.
    wrap_pending: bool = false,

    saved_x: usize = 0,
    saved_y: usize = 0,
    saved_pen: Cell = .{},

    region: ScrollRegion = .{},
    modes: Modes = .{},

    /// The SGR pen: a template cell whose color/attrs are stamped onto every
    /// printed character. `cp` is ignored.
    pen: Cell = .{},

    /// Allocate a `cols x rows` grid of blank cells.
    pub fn init(alloc: std.mem.Allocator, cols: usize, rows: usize) !Grid {
        const w = @max(cols, 1);
        const h = @max(rows, 1);
        const cells = try alloc.alloc(Cell, w * h);
        @memset(cells, Cell{});
        const scrolled_off = try alloc.alloc(Cell, w);
        @memset(scrolled_off, Cell{});
        return .{
            .alloc = alloc,
            .width = w,
            .height = h,
            .cells = cells,
            .scrolled_off = scrolled_off,
            .region = .{ .top = 0, .bottom = h - 1 },
        };
    }

    pub fn deinit(self: *Grid) void {
        self.alloc.free(self.cells);
        self.alloc.free(self.scrolled_off);
        self.* = undefined;
    }

    /// Borrow row `y` as a mutable slice of exactly `width` cells.
    pub fn row(self: *Grid, y: usize) []Cell {
        const start = y * self.width;
        return self.cells[start .. start + self.width];
    }

    /// Borrow row `y` immutably.
    pub fn rowConst(self: *const Grid, y: usize) []const Cell {
        const start = y * self.width;
        return self.cells[start .. start + self.width];
    }

    fn cellPtr(self: *Grid, x: usize, y: usize) *Cell {
        return &self.cells[y * self.width + x];
    }

    // --- printing ----------------------------------------------------------

    /// Write a printable scalar at the cursor, honoring autowrap and insert
    /// mode, then advance the cursor.
    pub fn print(self: *Grid, cp: u21) void {
        if (self.wrap_pending and self.modes.autowrap) {
            self.wrap_pending = false;
            self.carriageReturn();
            _ = self.lineFeedInternal();
        }

        if (self.modes.insert) self.shiftRowRight(self.cur_y, self.cur_x, 1);

        var written = self.pen;
        written.cp = cp;
        self.cellPtr(self.cur_x, self.cur_y).* = written;

        if (self.cur_x + 1 >= self.width) {
            // At the last column: latch a pending wrap rather than moving.
            self.wrap_pending = true;
        } else {
            self.cur_x += 1;
        }
    }

    // --- cursor motion -----------------------------------------------------

    pub fn carriageReturn(self: *Grid) void {
        self.cur_x = 0;
        self.wrap_pending = false;
    }

    /// Move down one line, scrolling the region when at its bottom. Returns
    /// the row that scrolled off the top, or null when no scroll occurred.
    pub fn lineFeed(self: *Grid) ?[]const Cell {
        return self.lineFeedInternal();
    }

    fn lineFeedInternal(self: *Grid) ?[]const Cell {
        self.wrap_pending = false;
        if (self.cur_y == self.region.bottom) {
            return self.scrollUp(1);
        }
        if (self.cur_y + 1 < self.height) self.cur_y += 1;
        return null;
    }

    pub fn backspace(self: *Grid) void {
        self.wrap_pending = false;
        if (self.cur_x > 0) self.cur_x -= 1;
    }

    /// Advance to the next 8-column tab stop, clamped to the last column.
    pub fn tab(self: *Grid) void {
        self.wrap_pending = false;
        const next = ((self.cur_x / tab_width) + 1) * tab_width;
        self.cur_x = @min(next, self.width - 1);
    }

    pub fn cursorUp(self: *Grid, n: usize) void {
        self.wrap_pending = false;
        const limit = self.cursorTopLimit();
        const step = @max(n, 1);
        self.cur_y = if (self.cur_y >= limit + step) self.cur_y - step else limit;
    }

    pub fn cursorDown(self: *Grid, n: usize) void {
        self.wrap_pending = false;
        const limit = self.cursorBottomLimit();
        const step = @max(n, 1);
        self.cur_y = @min(self.cur_y + step, limit);
    }

    pub fn cursorForward(self: *Grid, n: usize) void {
        self.wrap_pending = false;
        const step = @max(n, 1);
        self.cur_x = @min(self.cur_x + step, self.width - 1);
    }

    pub fn cursorBack(self: *Grid, n: usize) void {
        self.wrap_pending = false;
        const step = @max(n, 1);
        self.cur_x = if (self.cur_x >= step) self.cur_x - step else 0;
    }

    /// Absolute cursor move. With origin mode on, `y` is relative to the
    /// scroll region top. Coordinates are clamped to the screen.
    pub fn cursorTo(self: *Grid, x: usize, y: usize) void {
        self.wrap_pending = false;
        self.cur_x = @min(x, self.width - 1);
        if (self.modes.origin) {
            const absolute = self.region.top + y;
            self.cur_y = @min(absolute, self.region.bottom);
        } else {
            self.cur_y = @min(y, self.height - 1);
        }
    }

    pub fn cursorToColumn(self: *Grid, x: usize) void {
        self.wrap_pending = false;
        self.cur_x = @min(x, self.width - 1);
    }

    pub fn cursorToRow(self: *Grid, y: usize) void {
        self.wrap_pending = false;
        if (self.modes.origin) {
            self.cur_y = @min(self.region.top + y, self.region.bottom);
        } else {
            self.cur_y = @min(y, self.height - 1);
        }
    }

    fn cursorTopLimit(self: *const Grid) usize {
        return if (self.modes.origin) self.region.top else 0;
    }

    fn cursorBottomLimit(self: *const Grid) usize {
        return if (self.modes.origin) self.region.bottom else self.height - 1;
    }

    pub fn saveCursor(self: *Grid) void {
        self.saved_x = self.cur_x;
        self.saved_y = self.cur_y;
        self.saved_pen = self.pen;
    }

    pub fn restoreCursor(self: *Grid) void {
        self.cur_x = @min(self.saved_x, self.width - 1);
        self.cur_y = @min(self.saved_y, self.height - 1);
        self.pen = self.saved_pen;
        self.wrap_pending = false;
    }

    // --- erasing -----------------------------------------------------------

    /// Erase Display (ED). `mode` 0 = cursor to end, 1 = start to cursor,
    /// 2/3 = whole screen.
    pub fn eraseDisplay(self: *Grid, mode: u16) void {
        switch (mode) {
            0 => {
                self.eraseLine(0);
                var y = self.cur_y + 1;
                while (y < self.height) : (y += 1) self.blankRow(y);
            },
            1 => {
                var y: usize = 0;
                while (y < self.cur_y) : (y += 1) self.blankRow(y);
                self.eraseLine(1);
            },
            else => {
                var y: usize = 0;
                while (y < self.height) : (y += 1) self.blankRow(y);
            },
        }
    }

    /// Erase in Line (EL). `mode` 0 = cursor to end, 1 = start to cursor,
    /// 2 = whole line.
    pub fn eraseLine(self: *Grid, mode: u16) void {
        const r = self.row(self.cur_y);
        switch (mode) {
            0 => self.blankCells(r[self.cur_x..]),
            1 => self.blankCells(r[0 .. @min(self.cur_x + 1, self.width)]),
            else => self.blankCells(r),
        }
    }

    /// Erase Character (ECH): blank `n` cells from the cursor without moving.
    pub fn eraseChars(self: *Grid, n: usize) void {
        const count = @max(n, 1);
        const r = self.row(self.cur_y);
        const end = @min(self.cur_x + count, self.width);
        self.blankCells(r[self.cur_x..end]);
    }

    fn blankRow(self: *Grid, y: usize) void {
        self.blankCells(self.row(y));
    }

    /// Reset every cell in `slice` to a blank carrying the current pen's
    /// background — so an erase after `CSI 44m` paints a blue field.
    fn blankCells(self: *const Grid, slice: []Cell) void {
        var blank = Cell{};
        blank.bg = self.pen.bg;
        @memset(slice, blank);
    }

    // --- insert / delete ---------------------------------------------------

    /// Insert Character (ICH): shift the cursor row right by `n`, blanking
    /// the gap. Cells pushed past the right edge are lost.
    pub fn insertChars(self: *Grid, n: usize) void {
        self.shiftRowRight(self.cur_y, self.cur_x, @max(n, 1));
    }

    /// Delete Character (DCH): shift the cursor row left by `n`, blanking the
    /// vacated tail.
    pub fn deleteChars(self: *Grid, n: usize) void {
        const count = @max(n, 1);
        const r = self.row(self.cur_y);
        const src_start = @min(self.cur_x + count, self.width);
        var i = self.cur_x;
        while (src_start + (i - self.cur_x) < self.width) : (i += 1) {
            r[i] = r[src_start + (i - self.cur_x)];
        }
        self.blankCells(r[i..]);
    }

    /// Insert `n` blank lines at the cursor row, pushing lower lines down
    /// within the scroll region. No effect outside the region.
    pub fn insertLines(self: *Grid, n: usize) void {
        if (self.cur_y < self.region.top or self.cur_y > self.region.bottom) return;
        const count = @min(@max(n, 1), self.region.bottom - self.cur_y + 1);
        var y = self.region.bottom;
        while (y >= self.cur_y + count) : (y -= 1) {
            @memcpy(self.row(y), self.rowConst(y - count));
            if (y == self.cur_y + count) break;
        }
        var blank = self.cur_y;
        while (blank < self.cur_y + count) : (blank += 1) self.blankRow(blank);
    }

    /// Delete `n` lines at the cursor row, pulling lower lines up within the
    /// scroll region. No effect outside the region.
    pub fn deleteLines(self: *Grid, n: usize) void {
        if (self.cur_y < self.region.top or self.cur_y > self.region.bottom) return;
        const count = @min(@max(n, 1), self.region.bottom - self.cur_y + 1);
        var y = self.cur_y;
        while (y + count <= self.region.bottom) : (y += 1) {
            @memcpy(self.row(y), self.rowConst(y + count));
        }
        while (y <= self.region.bottom) : (y += 1) self.blankRow(y);
    }

    // --- region scrolling --------------------------------------------------

    /// Scroll the region up by `n` lines (SU). Returns a stable copy of the
    /// first line that scrolled off the top (only meaningful for n>=1; the
    /// earlier lines are discarded). The returned slice is the grid-owned
    /// `scrolled_off` buffer — valid until the next `scrollUp`. Used by both
    /// SU and line feed.
    pub fn scrollUp(self: *Grid, n: usize) ?[]const Cell {
        const span = self.region.bottom - self.region.top + 1;
        const count = @min(@max(n, 1), span);
        // Snapshot the top row before the copy loop overwrites it.
        @memcpy(self.scrolled_off, self.rowConst(self.region.top));

        var y = self.region.top;
        while (y + count <= self.region.bottom) : (y += 1) {
            @memcpy(self.row(y), self.rowConst(y + count));
        }
        while (y <= self.region.bottom) : (y += 1) self.blankRow(y);
        return self.scrolled_off;
    }

    /// Scroll the region down by `n` lines (SD).
    pub fn scrollDown(self: *Grid, n: usize) void {
        const span = self.region.bottom - self.region.top + 1;
        const count = @min(@max(n, 1), span);
        var y = self.region.bottom;
        while (y >= self.region.top + count) : (y -= 1) {
            @memcpy(self.row(y), self.rowConst(y - count));
            if (y == self.region.top + count) break;
        }
        var blank = self.region.top;
        while (blank < self.region.top + count) : (blank += 1) self.blankRow(blank);
    }

    /// Set the DECSTBM scroll region (1-based, inclusive). An invalid or
    /// empty range resets to the whole screen. The cursor homes afterward.
    pub fn setScrollRegion(self: *Grid, top_1based: usize, bottom_1based: usize) void {
        const top = if (top_1based == 0) 0 else top_1based - 1;
        const bottom = if (bottom_1based == 0) self.height - 1 else bottom_1based - 1;
        if (top >= bottom or bottom >= self.height) {
            self.region = .{ .top = 0, .bottom = self.height - 1 };
        } else {
            self.region = .{ .top = top, .bottom = bottom };
        }
        self.cursorTo(0, 0);
    }

    // --- helpers -----------------------------------------------------------

    /// Shift row `y` right by `n` starting at column `from`, blanking the gap.
    fn shiftRowRight(self: *Grid, y: usize, from: usize, n: usize) void {
        if (from >= self.width) return;
        const count = @min(n, self.width - from);
        const r = self.row(y);
        var i = self.width;
        while (i > from + count) {
            i -= 1;
            r[i] = r[i - count];
        }
        self.blankCells(r[from .. from + count]);
    }

    // --- resize ------------------------------------------------------------

    /// Resize to `cols x rows`, preserving overlapping content from the top
    /// left. The cursor and scroll region are clamped to the new bounds.
    pub fn resize(self: *Grid, cols: usize, rows: usize) void {
        const w = @max(cols, 1);
        const h = @max(rows, 1);
        if (w == self.width and h == self.height) return;

        const fresh = self.alloc.alloc(Cell, w * h) catch return;
        @memset(fresh, Cell{});
        const fresh_scratch = self.alloc.alloc(Cell, w) catch {
            self.alloc.free(fresh);
            return;
        };
        @memset(fresh_scratch, Cell{});

        const copy_h = @min(h, self.height);
        const copy_w = @min(w, self.width);
        var y: usize = 0;
        while (y < copy_h) : (y += 1) {
            const src = self.cells[y * self.width ..][0..copy_w];
            @memcpy(fresh[y * w ..][0..copy_w], src);
        }

        self.alloc.free(self.cells);
        self.alloc.free(self.scrolled_off);
        self.cells = fresh;
        self.scrolled_off = fresh_scratch;
        self.width = w;
        self.height = h;
        self.region = .{ .top = 0, .bottom = h - 1 };
        self.cur_x = @min(self.cur_x, w - 1);
        self.cur_y = @min(self.cur_y, h - 1);
        self.wrap_pending = false;
    }
};

// --- tests -----------------------------------------------------------------

const testing = std.testing;

fn rowText(g: *const Grid, y: usize, buf: []u8) []const u8 {
    var n: usize = 0;
    for (g.rowConst(y)) |c| {
        n += std.unicode.utf8Encode(c.cp, buf[n..]) catch 0;
    }
    return buf[0..n];
}

test "print advances the cursor and stamps the pen" {
    var g = try Grid.init(testing.allocator, 10, 3);
    defer g.deinit();
    g.pen.fg = .{ .palette = 2 };
    g.print('h');
    g.print('i');
    try testing.expectEqual(@as(usize, 2), g.cur_x);
    try testing.expectEqual(@as(u21, 'h'), g.rowConst(0)[0].cp);
    try testing.expectEqual(cell.Color{ .palette = 2 }, g.rowConst(0)[0].fg);
}

test "autowrap latches at the last column and wraps on next print" {
    var g = try Grid.init(testing.allocator, 4, 3);
    defer g.deinit();
    for ("abcd") |ch| g.print(ch);
    // After 4 prints in a 4-wide grid the wrap is pending, not yet applied.
    try testing.expect(g.wrap_pending);
    try testing.expectEqual(@as(usize, 0), g.cur_y);
    g.print('e');
    try testing.expectEqual(@as(usize, 1), g.cur_y);
    try testing.expectEqual(@as(usize, 1), g.cur_x);
    try testing.expectEqual(@as(u21, 'e'), g.rowConst(1)[0].cp);
}

test "autowrap disabled overwrites the last column" {
    var g = try Grid.init(testing.allocator, 4, 3);
    defer g.deinit();
    g.modes.autowrap = false;
    for ("abcd") |ch| g.print(ch);
    g.print('X');
    try testing.expectEqual(@as(usize, 0), g.cur_y);
    try testing.expectEqual(@as(u21, 'X'), g.rowConst(0)[3].cp);
}

test "carriage return and line feed" {
    var g = try Grid.init(testing.allocator, 8, 4);
    defer g.deinit();
    g.print('a');
    g.carriageReturn();
    try testing.expectEqual(@as(usize, 0), g.cur_x);
    _ = g.lineFeed();
    try testing.expectEqual(@as(usize, 1), g.cur_y);
}

test "line feed at region bottom scrolls and returns the lost row" {
    var g = try Grid.init(testing.allocator, 8, 3);
    defer g.deinit();
    for ("top") |ch| g.print(ch); // row 0
    g.cursorTo(0, 1);
    for ("mid") |ch| g.print(ch); // row 1
    g.cursorTo(0, 2); // bottom row
    const scrolled = g.lineFeed();
    // The line feed scrolled the whole screen up by one; "top" fell off.
    try testing.expect(scrolled != null);
    var buf: [16]u8 = undefined;
    var sbuf: [16]u8 = undefined;
    var n: usize = 0;
    for (scrolled.?) |c| n += std.unicode.utf8Encode(c.cp, sbuf[n..]) catch 0;
    try testing.expectEqualStrings("top     ", sbuf[0..n]);
    // Row 0 now holds what was row 1.
    try testing.expectEqualStrings("mid     ", rowText(&g, 0, &buf));
    try testing.expectEqual(@as(usize, 2), g.cur_y);
}

test "cursor moves clamp at screen bounds" {
    var g = try Grid.init(testing.allocator, 10, 5);
    defer g.deinit();
    g.cursorUp(99);
    try testing.expectEqual(@as(usize, 0), g.cur_y);
    g.cursorDown(99);
    try testing.expectEqual(@as(usize, 4), g.cur_y);
    g.cursorForward(99);
    try testing.expectEqual(@as(usize, 9), g.cur_x);
    g.cursorBack(99);
    try testing.expectEqual(@as(usize, 0), g.cur_x);
}

test "cursorTo positions absolutely and clamps" {
    var g = try Grid.init(testing.allocator, 10, 5);
    defer g.deinit();
    g.cursorTo(3, 2);
    try testing.expectEqual(@as(usize, 3), g.cur_x);
    try testing.expectEqual(@as(usize, 2), g.cur_y);
    g.cursorTo(99, 99);
    try testing.expectEqual(@as(usize, 9), g.cur_x);
    try testing.expectEqual(@as(usize, 4), g.cur_y);
}

test "tab advances to 8-column stops" {
    var g = try Grid.init(testing.allocator, 30, 2);
    defer g.deinit();
    g.tab();
    try testing.expectEqual(@as(usize, 8), g.cur_x);
    g.cursorToColumn(10);
    g.tab();
    try testing.expectEqual(@as(usize, 16), g.cur_x);
}

test "eraseLine variants" {
    var g = try Grid.init(testing.allocator, 6, 2);
    defer g.deinit();
    for ("abcdef") |ch| g.print(ch);
    g.cursorToColumn(3);
    g.eraseLine(0); // cursor to end
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("abc   ", rowText(&g, 0, &buf));

    for ("ABCDEF", 0..) |ch, i| g.row(0)[i].cp = ch;
    g.cursorToColumn(2);
    g.eraseLine(1); // start to cursor inclusive
    try testing.expectEqualStrings("   DEF", rowText(&g, 0, &buf));
}

test "eraseDisplay clears below the cursor" {
    var g = try Grid.init(testing.allocator, 4, 3);
    defer g.deinit();
    for (0..3) |y| for (0..4) |x| {
        g.row(y)[x].cp = 'x';
    };
    g.cursorTo(2, 1);
    g.eraseDisplay(0);
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("xxxx", rowText(&g, 0, &buf));
    try testing.expectEqualStrings("xx  ", rowText(&g, 1, &buf));
    try testing.expectEqualStrings("    ", rowText(&g, 2, &buf));
}

test "insert and delete characters" {
    var g = try Grid.init(testing.allocator, 6, 2);
    defer g.deinit();
    for ("abcdef") |ch| g.print(ch);
    g.cursorToColumn(2);
    g.insertChars(2);
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("ab  cd", rowText(&g, 0, &buf));

    for ("abcdef", 0..) |ch, i| g.row(0)[i].cp = ch;
    g.cursorToColumn(1);
    g.deleteChars(2);
    try testing.expectEqualStrings("adef  ", rowText(&g, 0, &buf));
}

test "insert and delete lines within the scroll region" {
    var g = try Grid.init(testing.allocator, 4, 4);
    defer g.deinit();
    for (0..4) |y| {
        const ch: u21 = @intCast('1' + y);
        for (0..4) |x| g.row(y)[x].cp = ch;
    }
    g.cursorTo(0, 1);
    g.insertLines(1);
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("1111", rowText(&g, 0, &buf));
    try testing.expectEqualStrings("    ", rowText(&g, 1, &buf));
    try testing.expectEqualStrings("2222", rowText(&g, 2, &buf));

    for (0..4) |y| {
        const ch: u21 = @intCast('1' + y);
        for (0..4) |x| g.row(y)[x].cp = ch;
    }
    g.cursorTo(0, 1);
    g.deleteLines(1);
    try testing.expectEqualStrings("1111", rowText(&g, 0, &buf));
    try testing.expectEqualStrings("3333", rowText(&g, 1, &buf));
}

test "scroll region restricts line feed" {
    var g = try Grid.init(testing.allocator, 4, 5);
    defer g.deinit();
    for (0..5) |y| {
        const ch: u21 = @intCast('1' + y);
        for (0..4) |x| g.row(y)[x].cp = ch;
    }
    g.setScrollRegion(2, 4); // rows index 1..3
    g.cursorTo(0, 3); // bottom of region
    _ = g.lineFeed();
    var buf: [8]u8 = undefined;
    // Row 0 untouched, region scrolled, row 4 untouched.
    try testing.expectEqualStrings("1111", rowText(&g, 0, &buf));
    try testing.expectEqualStrings("3333", rowText(&g, 1, &buf));
    try testing.expectEqualStrings("4444", rowText(&g, 2, &buf));
    try testing.expectEqualStrings("    ", rowText(&g, 3, &buf));
    try testing.expectEqualStrings("5555", rowText(&g, 4, &buf));
}

test "resize preserves top-left content and clamps cursor" {
    var g = try Grid.init(testing.allocator, 6, 3);
    defer g.deinit();
    for ("hello") |ch| g.print(ch);
    g.cursorTo(5, 2);
    g.resize(3, 2);
    try testing.expectEqual(@as(usize, 3), g.width);
    try testing.expectEqual(@as(usize, 2), g.height);
    try testing.expectEqual(@as(usize, 2), g.cur_x);
    try testing.expectEqual(@as(usize, 1), g.cur_y);
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("hel", rowText(&g, 0, &buf));
}

test "save and restore cursor round-trips position and pen" {
    var g = try Grid.init(testing.allocator, 10, 4);
    defer g.deinit();
    g.cursorTo(4, 2);
    g.pen.attrs.bold = true;
    g.saveCursor();
    g.cursorTo(0, 0);
    g.pen.attrs.bold = false;
    g.restoreCursor();
    try testing.expectEqual(@as(usize, 4), g.cur_x);
    try testing.expectEqual(@as(usize, 2), g.cur_y);
    try testing.expect(g.pen.attrs.bold);
}

test "erase paints the pen background" {
    var g = try Grid.init(testing.allocator, 4, 2);
    defer g.deinit();
    g.pen.bg = .{ .palette = 4 };
    g.eraseLine(2);
    try testing.expectEqual(cell.Color{ .palette = 4 }, g.rowConst(0)[0].bg);
}
