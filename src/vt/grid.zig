const std = @import("std");
const Cell = @import("cell.zig").Cell;

pub const Grid = struct {
    cells: []Cell,
    rows: u16,
    cols: u16,
    alloc: std.mem.Allocator,

    pub fn init(alloc: std.mem.Allocator, rows: u16, cols: u16) !Grid {
        const cells = try alloc.alloc(Cell, @as(usize, rows) * cols);
        @memset(cells, Cell.blank);
        return .{ .cells = cells, .rows = rows, .cols = cols, .alloc = alloc };
    }

    pub fn deinit(self: *Grid) void {
        self.alloc.free(self.cells);
    }

    pub fn at(self: *Grid, r: u16, col: u16) *Cell {
        return &self.cells[@as(usize, r) * self.cols + col];
    }

    pub fn row(self: *Grid, r: u16) []Cell {
        const start = @as(usize, r) * self.cols;
        return self.cells[start .. start + self.cols];
    }

    pub fn clear(self: *Grid) void {
        @memset(self.cells, Cell.blank);
    }

    pub fn clearRow(self: *Grid, r: u16) void {
        @memset(self.row(r), Cell.blank);
    }

    pub fn scrollUp(self: *Grid, n: u16) void {
        const lines = @min(n, self.rows);
        const moved = (self.rows - lines);
        if (moved > 0) {
            const dst = self.cells[0 .. @as(usize, moved) * self.cols];
            const src = self.cells[@as(usize, lines) * self.cols ..][0 .. @as(usize, moved) * self.cols];
            std.mem.copyForwards(Cell, dst, src);
        }
        @memset(self.cells[@as(usize, moved) * self.cols ..], Cell.blank);
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
    g.scrollUp(1);
    try std.testing.expectEqual(@as(u21, 'b'), g.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, 'c'), g.at(1, 0).cp);
    try std.testing.expectEqual(@as(u21, ' '), g.at(2, 0).cp);
}
