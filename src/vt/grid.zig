const std = @import("std");
const Cell = @import("cell.zig").Cell;

pub const Grid = struct {
    cells: []Cell,
    wrapped: []bool, // per-row: true if the line soft-wrapped into the next
    rows: u16,
    cols: u16,
    alloc: std.mem.Allocator,

    pub fn init(alloc: std.mem.Allocator, rows: u16, cols: u16) !Grid {
        const cells = try alloc.alloc(Cell, @as(usize, rows) * cols);
        @memset(cells, Cell.blank);
        const wrapped = try alloc.alloc(bool, rows);
        @memset(wrapped, false);
        return .{ .cells = cells, .wrapped = wrapped, .rows = rows, .cols = cols, .alloc = alloc };
    }

    pub fn deinit(self: *Grid) void {
        self.alloc.free(self.cells);
        self.alloc.free(self.wrapped);
    }

    pub fn at(self: *Grid, r: u16, col: u16) *Cell {
        return &self.cells[@as(usize, r) * self.cols + col];
    }

    pub fn row(self: *Grid, r: u16) []Cell {
        const start = @as(usize, r) * self.cols;
        return self.cells[start .. start + self.cols];
    }

    pub fn clear(self: *Grid, blank: Cell) void {
        @memset(self.cells, blank);
        @memset(self.wrapped, false);
    }

    pub fn clearRow(self: *Grid, r: u16, blank: Cell) void {
        @memset(self.row(r), blank);
        self.wrapped[r] = false;
    }

    pub fn scrollUp(self: *Grid, n: u16, blank: Cell) void {
        const lines = @min(n, self.rows);
        const moved = (self.rows - lines);
        if (moved > 0) {
            const dst = self.cells[0 .. @as(usize, moved) * self.cols];
            const src = self.cells[@as(usize, lines) * self.cols ..][0 .. @as(usize, moved) * self.cols];
            std.mem.copyForwards(Cell, dst, src);
            std.mem.copyForwards(bool, self.wrapped[0..moved], self.wrapped[lines..][0..moved]);
        }
        @memset(self.cells[@as(usize, moved) * self.cols ..], blank);
        @memset(self.wrapped[moved..], false);
    }

    /// Scroll rows in the inclusive region [top, bot] up by n; bottom n rows
    /// of the region are blanked. top/bot are 0-based row indices.
    pub fn scrollRegionUp(self: *Grid, top: u16, bot: u16, n: u16, blank: Cell) void {
        if (bot < top or bot >= self.rows) return;
        const height = bot - top + 1;
        const lines = @min(n, height);
        const moved = height - lines;
        var i: u16 = 0;
        while (i < moved) : (i += 1) {
            const d = top + i;
            const s = top + i + lines;
            std.mem.copyForwards(Cell, self.row(d), self.row(s));
            self.wrapped[d] = self.wrapped[s];
        }
        i = 0;
        while (i < lines) : (i += 1) self.clearRow(bot - i, blank);
    }

    /// Scroll rows in the inclusive region [top, bot] down by n; top n rows
    /// of the region are blanked.
    pub fn scrollRegionDown(self: *Grid, top: u16, bot: u16, n: u16, blank: Cell) void {
        if (bot < top or bot >= self.rows) return;
        const height = bot - top + 1;
        const lines = @min(n, height);
        const moved = height - lines;
        var i: u16 = 0;
        while (i < moved) : (i += 1) {
            const d = bot - i;
            const s = bot - i - lines;
            std.mem.copyForwards(Cell, self.row(d), self.row(s));
            self.wrapped[d] = self.wrapped[s];
        }
        i = 0;
        while (i < lines) : (i += 1) self.clearRow(top + i, blank);
    }
};

test "init blanks the grid" {
    var g = try Grid.init(std.testing.allocator, 3, 4);
    defer g.deinit();
    try std.testing.expectEqual(@as(usize, 12), g.cells.len);
    try std.testing.expectEqual(@as(u21, ' '), g.at(2, 3).cp);
}

test "scrollUp shifts rows and blanks the bottom" {
    var g = try Grid.init(std.testing.allocator, 3, 2);
    defer g.deinit();
    g.at(0, 0).cp = 'a';
    g.at(1, 0).cp = 'b';
    g.at(2, 0).cp = 'c';
    g.scrollUp(1, Cell.blank);
    try std.testing.expectEqual(@as(u21, 'b'), g.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, 'c'), g.at(1, 0).cp);
    try std.testing.expectEqual(@as(u21, ' '), g.at(2, 0).cp);
}
