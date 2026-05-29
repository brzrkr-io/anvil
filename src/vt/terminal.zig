const std = @import("std");
const cell = @import("cell.zig");
const Cell = cell.Cell;
const Color = cell.Color;
const Grid = @import("grid.zig").Grid;
const Scrollback = @import("scrollback.zig").Scrollback;

const scrollback_cap = 5000;

pub const Terminal = struct {
    grid: Grid,
    cx: u16 = 0,
    cy: u16 = 0,
    saved_cx: u16 = 0,
    saved_cy: u16 = 0,
    stash: ?Grid = null, // primary grid, parked while the alt screen is active
    alt_cx: u16 = 0,
    alt_cy: u16 = 0,
    scrollback: Scrollback,
    view_offset: usize = 0, // lines scrolled up into history; 0 = live bottom
    pen: Cell = .{},

    pub fn init(alloc: std.mem.Allocator, rows: u16, cols: u16) !Terminal {
        return .{
            .grid = try Grid.init(alloc, rows, cols),
            .scrollback = try Scrollback.init(alloc, scrollback_cap),
        };
    }

    pub fn deinit(self: *Terminal) void {
        self.grid.deinit();
        if (self.stash) |*g| g.deinit();
        self.scrollback.deinit();
    }

    /// Cells for visible row `r`, drawn from scrollback when scrolled up.
    pub fn viewRow(self: *Terminal, r: u16) []Cell {
        const sb = self.scrollback.len();
        const logical = (sb - self.view_offset) + r;
        if (logical < sb) return self.scrollback.at(logical);
        return self.grid.row(@intCast(logical - sb));
    }

    /// Scroll the viewport: positive = back into history, negative = toward live.
    pub fn scrollView(self: *Terminal, delta: i32) void {
        const sb: i64 = @intCast(self.scrollback.len());
        var off: i64 = @as(i64, @intCast(self.view_offset)) + delta;
        off = std.math.clamp(off, 0, sb);
        self.view_offset = @intCast(off);
    }

    pub fn print(self: *Terminal, cp: u21) void {
        if (self.cx >= self.grid.cols) {
            self.cx = 0;
            self.lineFeed();
        }
        const c = self.grid.at(self.cy, self.cx);
        c.* = .{ .cp = cp, .fg = self.pen.fg, .bg = self.pen.bg, .attrs = self.pen.attrs };
        self.cx += 1;
    }

    pub fn lineFeed(self: *Terminal) void {
        if (self.cy + 1 >= self.grid.rows) {
            if (self.stash == null) {
                self.scrollback.push(self.grid.row(0));
                if (self.view_offset > 0 and self.view_offset < self.scrollback.len())
                    self.view_offset += 1; // stay anchored to history while scrolled
            }
            self.grid.scrollUp(1);
        } else {
            self.cy += 1;
        }
    }

    pub fn carriageReturn(self: *Terminal) void {
        self.cx = 0;
    }

    pub fn backspace(self: *Terminal) void {
        if (self.cx > 0) self.cx -= 1;
    }

    pub fn tab(self: *Terminal) void {
        const next = (self.cx / 8 + 1) * 8;
        self.cx = @min(next, self.grid.cols - 1);
    }

    pub fn cursorUp(self: *Terminal, n: u16) void {
        self.cy -= @min(n, self.cy);
    }

    pub fn cursorDown(self: *Terminal, n: u16) void {
        self.cy = @min(self.cy + n, self.grid.rows - 1);
    }

    pub fn cursorForward(self: *Terminal, n: u16) void {
        self.cx = @min(self.cx + n, self.grid.cols - 1);
    }

    pub fn cursorBack(self: *Terminal, n: u16) void {
        self.cx -= @min(n, self.cx);
    }

    pub fn setCursor(self: *Terminal, row1: u16, col1: u16) void {
        const r = if (row1 > 0) row1 - 1 else 0;
        const c = if (col1 > 0) col1 - 1 else 0;
        self.cy = @min(r, self.grid.rows - 1);
        self.cx = @min(c, self.grid.cols - 1);
    }

    pub fn cursorCol(self: *Terminal, col1: u16) void {
        const c = if (col1 > 0) col1 - 1 else 0;
        self.cx = @min(c, self.grid.cols - 1);
    }

    pub fn cursorRow(self: *Terminal, row1: u16) void {
        const r = if (row1 > 0) row1 - 1 else 0;
        self.cy = @min(r, self.grid.rows - 1);
    }

    pub fn saveCursor(self: *Terminal) void {
        self.saved_cx = self.cx;
        self.saved_cy = self.cy;
    }

    pub fn restoreCursor(self: *Terminal) void {
        self.cx = @min(self.saved_cx, self.grid.cols - 1);
        self.cy = @min(self.saved_cy, self.grid.rows - 1);
    }

    pub fn deleteChars(self: *Terminal, n: u16) void {
        const r = self.grid.row(self.cy);
        const cnt = @min(n, self.grid.cols - self.cx);
        std.mem.copyForwards(Cell, r[self.cx .. self.grid.cols - cnt], r[self.cx + cnt ..]);
        @memset(r[self.grid.cols - cnt ..], Cell.blank);
    }

    pub fn insertChars(self: *Terminal, n: u16) void {
        const r = self.grid.row(self.cy);
        const cnt = @min(n, self.grid.cols - self.cx);
        std.mem.copyBackwards(Cell, r[self.cx + cnt ..], r[self.cx .. self.grid.cols - cnt]);
        @memset(r[self.cx .. self.cx + cnt], Cell.blank);
    }

    pub fn eraseChars(self: *Terminal, n: u16) void {
        const r = self.grid.row(self.cy);
        const end = @min(self.cx + n, self.grid.cols);
        @memset(r[self.cx..end], Cell.blank);
    }

    pub fn setMode(self: *Terminal, mode: u16, enable: bool) void {
        switch (mode) {
            47, 1047, 1049 => if (enable) self.enterAlt() else self.exitAlt(),
            else => {},
        }
    }

    fn enterAlt(self: *Terminal) void {
        if (self.stash != null) return;
        const g = Grid.init(self.grid.alloc, self.grid.rows, self.grid.cols) catch return;
        self.alt_cx = self.cx;
        self.alt_cy = self.cy;
        self.stash = self.grid;
        self.grid = g;
        self.cx = 0;
        self.cy = 0;
        self.view_offset = 0;
    }

    fn exitAlt(self: *Terminal) void {
        if (self.stash) |primary| {
            self.grid.deinit();
            self.grid = primary;
            self.stash = null;
            self.cx = @min(self.alt_cx, self.grid.cols - 1);
            self.cy = @min(self.alt_cy, self.grid.rows - 1);
        }
    }

    pub fn eraseInLine(self: *Terminal, mode: u16) void {
        const r = self.grid.row(self.cy);
        switch (mode) {
            0 => @memset(r[self.cx..], Cell.blank),
            1 => @memset(r[0 .. self.cx + 1], Cell.blank),
            2 => @memset(r, Cell.blank),
            else => {},
        }
    }

    pub fn eraseInDisplay(self: *Terminal, mode: u16) void {
        switch (mode) {
            0 => {
                self.eraseInLine(0);
                var r = self.cy + 1;
                while (r < self.grid.rows) : (r += 1) self.grid.clearRow(r);
            },
            1 => {
                var r: u16 = 0;
                while (r < self.cy) : (r += 1) self.grid.clearRow(r);
                self.eraseInLine(1);
            },
            2 => self.grid.clear(),
            else => {},
        }
    }

    pub fn sgr(self: *Terminal, params: []const u16) void {
        if (params.len == 0) {
            self.pen = .{};
            return;
        }
        var i: usize = 0;
        while (i < params.len) : (i += 1) {
            switch (params[i]) {
                0 => self.pen = .{},
                1 => self.pen.attrs.bold = true,
                4 => self.pen.attrs.underline = true,
                7 => self.pen.attrs.reverse = true,
                22 => self.pen.attrs.bold = false,
                24 => self.pen.attrs.underline = false,
                27 => self.pen.attrs.reverse = false,
                30...37 => self.pen.fg = .{ .indexed = @intCast(params[i] - 30) },
                39 => self.pen.fg = .default,
                40...47 => self.pen.bg = .{ .indexed = @intCast(params[i] - 40) },
                49 => self.pen.bg = .default,
                90...97 => self.pen.fg = .{ .indexed = @intCast(params[i] - 90 + 8) },
                100...107 => self.pen.bg = .{ .indexed = @intCast(params[i] - 100 + 8) },
                38 => i += self.extendedColor(params[i..], &self.pen.fg),
                48 => i += self.extendedColor(params[i..], &self.pen.bg),
                else => {},
            }
        }
    }

    fn extendedColor(_: *Terminal, p: []const u16, out: *Color) usize {
        if (p.len >= 3 and p[1] == 5) {
            out.* = .{ .indexed = @intCast(p[2] & 0xff) };
            return 2;
        }
        if (p.len >= 5 and p[1] == 2) {
            out.* = .{ .rgb = .{ .r = @intCast(p[2] & 0xff), .g = @intCast(p[3] & 0xff), .b = @intCast(p[4] & 0xff) } };
            return 4;
        }
        return 0;
    }
};

test "print wraps and advances cursor" {
    var t = try Terminal.init(std.testing.allocator, 2, 3);
    defer t.deinit();
    for ("abcd") |ch| t.print(ch);
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, 'c'), t.grid.at(0, 2).cp);
    try std.testing.expectEqual(@as(u21, 'd'), t.grid.at(1, 0).cp);
    try std.testing.expectEqual(@as(u16, 1), t.cx);
    try std.testing.expectEqual(@as(u16, 1), t.cy);
}

test "lineFeed scrolls at bottom" {
    var t = try Terminal.init(std.testing.allocator, 2, 2);
    defer t.deinit();
    t.print('x');
    t.lineFeed();
    t.lineFeed();
    try std.testing.expectEqual(@as(u16, 1), t.cy);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 0).cp);
}

test "setCursor and eraseInLine" {
    var t = try Terminal.init(std.testing.allocator, 3, 5);
    defer t.deinit();
    for ("hello") |ch| t.print(ch);
    t.setCursor(1, 3);
    try std.testing.expectEqual(@as(u16, 0), t.cy);
    try std.testing.expectEqual(@as(u16, 2), t.cx);
    t.eraseInLine(0);
    try std.testing.expectEqual(@as(u21, 'e'), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 2).cp);
}

test "deleteChars shifts row left and blanks tail" {
    var t = try Terminal.init(std.testing.allocator, 1, 6);
    defer t.deinit();
    for ("abcdef") |ch| t.print(ch);
    t.cursorCol(2); // cx = 1 (on 'b')
    t.deleteChars(2); // remove 'b','c'
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, 'd'), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u21, 'f'), t.grid.at(0, 3).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 4).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 5).cp);
}

test "insertChars shifts row right and blanks gap" {
    var t = try Terminal.init(std.testing.allocator, 1, 5);
    defer t.deinit();
    for ("abc") |ch| t.print(ch);
    t.cursorCol(1); // cx = 0
    t.insertChars(2);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 2).cp);
    try std.testing.expectEqual(@as(u21, 'c'), t.grid.at(0, 4).cp);
}

test "eraseChars blanks n cells without shifting" {
    var t = try Terminal.init(std.testing.allocator, 1, 5);
    defer t.deinit();
    for ("abcde") |ch| t.print(ch);
    t.cursorCol(2); // cx = 1
    t.eraseChars(2);
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 2).cp);
    try std.testing.expectEqual(@as(u21, 'd'), t.grid.at(0, 3).cp);
}

test "save and restore cursor" {
    var t = try Terminal.init(std.testing.allocator, 3, 5);
    defer t.deinit();
    t.setCursor(2, 3);
    t.saveCursor();
    t.setCursor(1, 1);
    t.restoreCursor();
    try std.testing.expectEqual(@as(u16, 1), t.cy);
    try std.testing.expectEqual(@as(u16, 2), t.cx);
}

test "alt screen swaps to a blank grid and restores primary" {
    var t = try Terminal.init(std.testing.allocator, 2, 4);
    defer t.deinit();
    for ("hi") |ch| t.print(ch); // primary: row 0 = "hi", cursor at col 2

    t.setMode(1049, true);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 0).cp); // alt is blank
    try std.testing.expectEqual(@as(u16, 0), t.cx);
    t.print('X');
    try std.testing.expectEqual(@as(u21, 'X'), t.grid.at(0, 0).cp);

    t.setMode(1049, false);
    try std.testing.expectEqual(@as(u21, 'h'), t.grid.at(0, 0).cp); // primary restored
    try std.testing.expectEqual(@as(u21, 'i'), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u16, 2), t.cx); // cursor restored
    try std.testing.expect(t.stash == null);
}

test "redundant alt enter is a no-op" {
    var t = try Terminal.init(std.testing.allocator, 2, 2);
    defer t.deinit();
    t.print('a');
    t.setMode(1049, true);
    t.setMode(47, true); // already in alt; must not lose the parked primary
    t.setMode(1049, false);
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 0).cp);
}

test "scrolled-off rows land in scrollback and viewRow reads them back" {
    var t = try Terminal.init(std.testing.allocator, 2, 2);
    defer t.deinit();
    t.print('a');
    t.carriageReturn();
    t.lineFeed();
    t.print('b');
    t.carriageReturn();
    t.lineFeed(); // 'a' row scrolls off into scrollback
    t.print('c');
    try std.testing.expectEqual(@as(usize, 1), t.scrollback.len());

    // live view: rows are 'b','c'
    try std.testing.expectEqual(@as(u21, 'b'), t.viewRow(0)[0].cp);
    try std.testing.expectEqual(@as(u21, 'c'), t.viewRow(1)[0].cp);

    // scroll back one: top row now the scrollback 'a'
    t.scrollView(1);
    try std.testing.expectEqual(@as(usize, 1), t.view_offset);
    try std.testing.expectEqual(@as(u21, 'a'), t.viewRow(0)[0].cp);
    try std.testing.expectEqual(@as(u21, 'b'), t.viewRow(1)[0].cp);

    // scrollView clamps at history depth and at live bottom
    t.scrollView(10);
    try std.testing.expectEqual(@as(usize, 1), t.view_offset);
    t.scrollView(-10);
    try std.testing.expectEqual(@as(usize, 0), t.view_offset);
}

test "sgr sets and resets pen" {
    var t = try Terminal.init(std.testing.allocator, 1, 4);
    defer t.deinit();
    t.sgr(&.{ 1, 31 });
    try std.testing.expect(t.pen.attrs.bold);
    try std.testing.expectEqual(Color{ .indexed = 1 }, t.pen.fg);
    t.sgr(&.{0});
    try std.testing.expect(!t.pen.attrs.bold);
    try std.testing.expectEqual(Color.default, t.pen.fg);
}
