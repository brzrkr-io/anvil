const std = @import("std");
const Terminal = @import("../vt/terminal.zig").Terminal;
const palette = @import("palette.zig");
const CellInstance = @import("instance.zig").CellInstance;
const Atlas = @import("atlas.zig").Atlas;

pub const GridSize = struct { cols: u16, rows: u16 };

pub const Renderer = struct {
    cell_w: f32,
    cell_h: f32,
    pad_x: f32,
    pad_y: f32,
    atlas: Atlas = .{},

    /// Rows/cols that fit a viewport of `px_w` x `px_h` device pixels.
    pub fn gridSize(self: Renderer, px_w: f32, px_h: f32) GridSize {
        const usable_w = px_w - 2 * self.pad_x;
        const usable_h = px_h - 2 * self.pad_y;
        const cols = if (usable_w > self.cell_w) usable_w / self.cell_w else 1;
        const rows = if (usable_h > self.cell_h) usable_h / self.cell_h else 1;
        return .{
            .cols = @intFromFloat(@max(1, @floor(cols))),
            .rows = @intFromFloat(@max(1, @floor(rows))),
        };
    }

    /// Emit one instance per cell into `out`; returns the count written.
    /// `out` must hold at least rows*cols entries.
    pub fn buildInstances(self: Renderer, term: *Terminal, out: []CellInstance) usize {
        var n: usize = 0;
        var r: u16 = 0;
        while (r < term.grid.rows) : (r += 1) {
            var c: u16 = 0;
            while (c < term.grid.cols) : (c += 1) {
                const cell = term.grid.at(r, c);
                var fg = palette.resolve(cell.fg, true);
                var bg = palette.resolve(cell.bg, false);
                if (cell.attrs.reverse) {
                    const t = fg;
                    fg = bg;
                    bg = t;
                }
                out[n] = .{
                    .col = @floatFromInt(c),
                    .row = @floatFromInt(r),
                    .fg = fg.f32x4(),
                    .bg = bg.f32x4(),
                    .uv = self.atlas.uvOrigin(cell.cp),
                };
                n += 1;
            }
        }
        return n;
    }
};

test "gridSize floors to whole cells minus padding" {
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 5, .pad_y = 5 };
    // (800 - 10) / 10 = 79 cols ; (500 - 10) / 20 = 24.5 -> 24 rows
    const g = rd.gridSize(800, 500);
    try std.testing.expectEqual(@as(u16, 79), g.cols);
    try std.testing.expectEqual(@as(u16, 24), g.rows);
}

test "gridSize never returns zero" {
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    const g = rd.gridSize(1, 1);
    try std.testing.expectEqual(@as(u16, 1), g.cols);
    try std.testing.expectEqual(@as(u16, 1), g.rows);
}

test "buildInstances emits one per cell with positions" {
    var t = try Terminal.init(std.testing.allocator, 2, 3);
    defer t.deinit();
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [6]CellInstance = undefined;
    const n = rd.buildInstances(&t, &buf);
    try std.testing.expectEqual(@as(usize, 6), n);
    try std.testing.expectEqual(@as(f32, 0), buf[0].col);
    try std.testing.expectEqual(@as(f32, 0), buf[0].row);
    // last cell is row 1, col 2
    try std.testing.expectEqual(@as(f32, 2), buf[5].col);
    try std.testing.expectEqual(@as(f32, 1), buf[5].row);
}

test "buildInstances carries glyph uv and resolves color" {
    var t = try Terminal.init(std.testing.allocator, 1, 1);
    defer t.deinit();
    t.pen.fg = .{ .indexed = 1 };
    t.print('A');
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [1]CellInstance = undefined;
    _ = rd.buildInstances(&t, &buf);
    try std.testing.expectEqual(rd.atlas.uvOrigin('A'), buf[0].uv);
    const red = palette.indexed(1).f32x4();
    try std.testing.expectEqual(red, buf[0].fg);
}

test "buildInstances swaps fg/bg on reverse" {
    var t = try Terminal.init(std.testing.allocator, 1, 1);
    defer t.deinit();
    t.pen.attrs.reverse = true;
    t.print('x');
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [1]CellInstance = undefined;
    _ = rd.buildInstances(&t, &buf);
    try std.testing.expectEqual(palette.default_bg.f32x4(), buf[0].fg);
    try std.testing.expectEqual(palette.default_fg.f32x4(), buf[0].bg);
}
