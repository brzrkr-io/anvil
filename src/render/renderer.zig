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
    pad_y: f32, // top padding (clears the title bar)
    pad_bottom: f32 = 0,
    atlas: Atlas = .{},

    /// Rows/cols that fit a viewport of `px_w` x `px_h` device pixels.
    pub fn gridSize(self: Renderer, px_w: f32, px_h: f32) GridSize {
        const usable_w = px_w - 2 * self.pad_x;
        const usable_h = px_h - self.pad_y - self.pad_bottom;
        const cols = if (usable_w > self.cell_w) usable_w / self.cell_w else 1;
        const rows = if (usable_h > self.cell_h) usable_h / self.cell_h else 1;
        return .{
            .cols = @intFromFloat(@max(1, @floor(cols))),
            .rows = @intFromFloat(@max(1, @floor(rows))),
        };
    }

    /// Rows/cols for a pane of `w` x `h` device pixels, using uniform inner
    /// padding (panes carry no title bar).
    pub fn paneGrid(self: Renderer, w: f32, h: f32) GridSize {
        const usable_w = w - 2 * self.pad_x;
        const usable_h = h - 2 * self.pad_x;
        const cols = if (usable_w > self.cell_w) usable_w / self.cell_w else 1;
        const rows = if (usable_h > self.cell_h) usable_h / self.cell_h else 1;
        return .{
            .cols = @intFromFloat(@max(1, @floor(cols))),
            .rows = @intFromFloat(@max(1, @floor(rows))),
        };
    }

    /// Emit one instance per cell into `out` starting at `out[0]`, positioning
    /// the grid at device-pixel origin (`ox`, `oy`). Returns the count written.
    /// `out` must hold at least rows*cols entries.
    pub fn buildInstances(self: Renderer, term: *Terminal, ox: f32, oy: f32, out: []CellInstance) usize {
        var n: usize = 0;
        var r: u16 = 0;
        while (r < term.grid.rows) : (r += 1) {
            const cells = term.viewRow(r);
            const y = oy + @as(f32, @floatFromInt(r)) * self.cell_h;
            var c: u16 = 0;
            while (c < term.grid.cols) : (c += 1) {
                const cell = if (c < cells.len) cells[c] else @import("../vt/cell.zig").Cell.blank;
                var fg = palette.resolve(cell.fg, true);
                var bg = palette.resolve(cell.bg, false);
                if (cell.attrs.reverse) {
                    const t = fg;
                    fg = bg;
                    bg = t;
                }
                if (term.isSelected(r, c)) {
                    fg = palette.selectionFg();
                    bg = palette.selectionBg();
                }
                out[n] = .{
                    .x = ox + @as(f32, @floatFromInt(c)) * self.cell_w,
                    .y = y,
                    .fg = fg.f32x4(),
                    .bg = bg.f32x4(),
                    .uv = self.atlas.uvOrigin(cell.cp),
                };
                n += 1;
            }
        }
        return n;
    }

    /// A block cursor at the terminal's cursor cell: the cell's own glyph with
    /// fg/bg swapped. Append after the cell instances so it draws on top.
    pub fn cursorInstance(self: Renderer, term: *Terminal, ox: f32, oy: f32) CellInstance {
        const cx = @min(term.cx, term.grid.cols - 1);
        const cy = @min(term.cy, term.grid.rows - 1);
        const cell = term.grid.at(cy, cx);
        const fg = palette.resolve(cell.fg, true);
        const bg = palette.resolve(cell.bg, false);
        return .{
            .x = ox + @as(f32, @floatFromInt(cx)) * self.cell_w,
            .y = oy + @as(f32, @floatFromInt(cy)) * self.cell_h,
            .fg = bg.f32x4(),
            .bg = fg.f32x4(),
            .uv = self.atlas.uvOrigin(cell.cp),
        };
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
    const n = rd.buildInstances(&t, 0, 0, &buf);
    try std.testing.expectEqual(@as(usize, 6), n);
    try std.testing.expectEqual(@as(f32, 0), buf[0].x);
    try std.testing.expectEqual(@as(f32, 0), buf[0].y);
    // last cell is row 1, col 2 → x=2*cell_w, y=1*cell_h
    try std.testing.expectEqual(@as(f32, 20), buf[5].x);
    try std.testing.expectEqual(@as(f32, 20), buf[5].y);
}

test "buildInstances offsets every cell by the pane origin" {
    var t = try Terminal.init(std.testing.allocator, 1, 1);
    defer t.deinit();
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [1]CellInstance = undefined;
    _ = rd.buildInstances(&t, 100, 50, &buf);
    try std.testing.expectEqual(@as(f32, 100), buf[0].x);
    try std.testing.expectEqual(@as(f32, 50), buf[0].y);
}

test "buildInstances carries glyph uv and resolves color" {
    var t = try Terminal.init(std.testing.allocator, 1, 1);
    defer t.deinit();
    t.pen.fg = .{ .indexed = 1 };
    t.print('A');
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [1]CellInstance = undefined;
    _ = rd.buildInstances(&t, 0, 0, &buf);
    try std.testing.expectEqual(rd.atlas.uvOrigin('A'), buf[0].uv);
    const red = palette.indexed(1).f32x4();
    try std.testing.expectEqual(red, buf[0].fg);
}

test "cursorInstance swaps colors at the cursor cell" {
    var t = try Terminal.init(std.testing.allocator, 2, 3);
    defer t.deinit();
    t.print('a'); // cursor advances to col 1, row 0
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    const ci = rd.cursorInstance(&t, 0, 0);
    try std.testing.expectEqual(@as(f32, 10), ci.x); // col 1 * cell_w
    try std.testing.expectEqual(@as(f32, 0), ci.y);
    // blank cell under cursor: fg=default_fg, bg=default_bg, swapped on the cursor
    try std.testing.expectEqual(palette.defaultFg().f32x4(), ci.bg);
    try std.testing.expectEqual(palette.defaultBg().f32x4(), ci.fg);
}

test "cursorInstance clamps cursor past last column" {
    var t = try Terminal.init(std.testing.allocator, 1, 2);
    defer t.deinit();
    t.cx = 5; // deferred-wrap can leave cx == cols
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    const ci = rd.cursorInstance(&t, 0, 0);
    try std.testing.expectEqual(@as(f32, 10), ci.x); // clamped to col 1 * cell_w
}

test "buildInstances swaps fg/bg on reverse" {
    var t = try Terminal.init(std.testing.allocator, 1, 1);
    defer t.deinit();
    t.pen.attrs.reverse = true;
    t.print('x');
    const rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [1]CellInstance = undefined;
    _ = rd.buildInstances(&t, 0, 0, &buf);
    try std.testing.expectEqual(palette.defaultBg().f32x4(), buf[0].fg);
    try std.testing.expectEqual(palette.defaultFg().f32x4(), buf[0].bg);
}
