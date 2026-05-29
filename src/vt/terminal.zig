const std = @import("std");
const cell = @import("cell.zig");
const Cell = cell.Cell;
const Color = cell.Color;
const Grid = @import("grid.zig").Grid;

pub const Terminal = struct {
    grid: Grid,
    cx: u16 = 0,
    cy: u16 = 0,
    pen: Cell = .{},

    pub fn init(alloc: std.mem.Allocator, rows: u16, cols: u16) !Terminal {
        return .{ .grid = try Grid.init(alloc, rows, cols) };
    }

    pub fn deinit(self: *Terminal) void {
        self.grid.deinit();
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
