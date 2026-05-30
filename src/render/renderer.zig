const std = @import("std");
const Terminal = @import("../vt/terminal.zig").Terminal;
const palette = @import("palette.zig");
const inst = @import("instance.zig");
const CellInstance = inst.CellInstance;
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
    pub fn gridSize(self: *const Renderer, px_w: f32, px_h: f32) GridSize {
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
    pub fn paneGrid(self: *const Renderer, w: f32, h: f32) GridSize {
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
    /// `y_shift` slides the grid up by a sub-row amount (device px, negative =
    /// up), and `base` is the integer view_offset used to read rows; pass
    /// term.view_offset and 0 for the normal (non-animated) case.
    /// When `y_shift` is non-zero an extra row (r == rows) is drawn to fill the
    /// revealed strip at the bottom; that row is blank when at totalLines.
    /// `out` must hold at least (rows+1)*cols entries when y_shift != 0.
    pub fn buildInstances(self: *Renderer, term: *Terminal, ox: f32, oy: f32, y_shift: f32, base: usize, out: []CellInstance) usize {
        const width = @import("../vt/width.zig").charWidth;
        const frac_threshold: f32 = 1e-3;
        const draw_extra = @abs(y_shift) > frac_threshold;
        var n: usize = 0;
        var r: u16 = 0;
        const row_limit: u16 = if (draw_extra) term.grid.rows + 1 else term.grid.rows;
        while (r < row_limit) : (r += 1) {
            const cells = if (r < term.grid.rows)
                term.viewRowAt(base, r)
            else
                &[_]@import("../vt/cell.zig").Cell{};
            const y = oy + @as(f32, @floatFromInt(r)) * self.cell_h + y_shift;
            // Right half UV owed to the spacer cell of a preceding wide glyph.
            var wide_right: ?[2]f32 = null;
            var c: u16 = 0;
            while (c < term.grid.cols) : (c += 1) {
                const cell = if (c < cells.len) cells[c] else @import("../vt/cell.zig").Cell.blank;
                // Bold maps the 8 base ANSI colors to their bright variant.
                var fg_color = cell.fg;
                if (cell.attrs.bold) {
                    if (fg_color == .indexed and fg_color.indexed < 8)
                        fg_color = .{ .indexed = fg_color.indexed + 8 };
                }
                var fg = palette.resolve(fg_color, true);
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
                var flags: u32 = 0;
                if (cell.attrs.underline) flags |= inst.flag_underline;
                if (cell.attrs.strike) flags |= inst.flag_strike;
                if (cell.attrs.dim) flags |= inst.flag_dim;
                // A double-width glyph is drawn as two cells sampling the left
                // and right halves of a 2-cell-wide atlas raster.
                var uv: [2]f32 = undefined;
                if (wide_right) |ruv| {
                    uv = ruv;
                    wide_right = null;
                } else if (width(cell.cp) == 2) {
                    const left = self.atlas.wideSlot(cell.cp);
                    uv = Atlas.slotUV(left);
                    wide_right = Atlas.slotUV(left + 1);
                } else {
                    uv = self.atlas.uvOrigin(cell.cp);
                }
                out[n] = .{
                    .x = ox + @as(f32, @floatFromInt(c)) * self.cell_w,
                    .y = y,
                    .fg = fg.f32x4(),
                    .bg = bg.f32x4(),
                    .uv = uv,
                    .flags = flags,
                };
                n += 1;
            }
        }
        return n;
    }

    /// A block cursor at the terminal's cursor cell: the cell's own glyph with
    /// fg/bg swapped. Append after the cell instances so it draws on top.
    pub fn cursorInstance(self: *Renderer, term: *Terminal, ox: f32, oy: f32) CellInstance {
        const cx = @min(term.cx, term.grid.cols - 1);
        const cy = @min(term.cy, term.grid.rows - 1);
        const cell = term.grid.at(cy, cx);
        const fg = palette.resolve(cell.fg, true);
        const bg = palette.resolve(cell.bg, false);
        const x = ox + @as(f32, @floatFromInt(cx)) * self.cell_w;
        const y = oy + @as(f32, @floatFromInt(cy)) * self.cell_h;
        return switch (term.cursor_style) {
            // Block: the cell glyph with fg/bg swapped (drawn over the cell).
            .block => .{ .x = x, .y = y, .fg = bg.f32x4(), .bg = fg.f32x4(), .uv = self.atlas.uvOrigin(cell.cp) },
            // Bar / underline: a colored band; the shader discards the rest so the
            // already-rendered glyph stays visible. fg carries the cursor color.
            .bar => .{ .x = x, .y = y, .fg = fg.f32x4(), .bg = bg.f32x4(), .uv = self.atlas.uvOrigin(' '), .flags = inst.flag_cursor_bar },
            .underline => .{ .x = x, .y = y, .fg = fg.f32x4(), .bg = bg.f32x4(), .uv = self.atlas.uvOrigin(' '), .flags = inst.flag_cursor_underline },
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
    var rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [6]CellInstance = undefined;
    const n = rd.buildInstances(&t, 0, 0, 0, t.view_offset, &buf);
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
    var rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [1]CellInstance = undefined;
    _ = rd.buildInstances(&t, 100, 50, 0, t.view_offset, &buf);
    try std.testing.expectEqual(@as(f32, 100), buf[0].x);
    try std.testing.expectEqual(@as(f32, 50), buf[0].y);
}

test "buildInstances carries glyph uv and resolves color" {
    var t = try Terminal.init(std.testing.allocator, 1, 1);
    defer t.deinit();
    t.pen.fg = .{ .indexed = 1 };
    t.print('A');
    var rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [1]CellInstance = undefined;
    _ = rd.buildInstances(&t, 0, 0, 0, t.view_offset, &buf);
    try std.testing.expectEqual(rd.atlas.uvOrigin('A'), buf[0].uv);
    const red = palette.indexed(1).f32x4();
    try std.testing.expectEqual(red, buf[0].fg);
}

test "cursorInstance swaps colors at the cursor cell" {
    var t = try Terminal.init(std.testing.allocator, 2, 3);
    defer t.deinit();
    t.print('a'); // cursor advances to col 1, row 0
    var rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
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
    var rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    const ci = rd.cursorInstance(&t, 0, 0);
    try std.testing.expectEqual(@as(f32, 10), ci.x); // clamped to col 1 * cell_w
}

test "buildInstances swaps fg/bg on reverse" {
    var t = try Terminal.init(std.testing.allocator, 1, 1);
    defer t.deinit();
    t.pen.attrs.reverse = true;
    t.print('x');
    var rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [1]CellInstance = undefined;
    _ = rd.buildInstances(&t, 0, 0, 0, t.view_offset, &buf);
    try std.testing.expectEqual(palette.defaultBg().f32x4(), buf[0].fg);
    try std.testing.expectEqual(palette.defaultFg().f32x4(), buf[0].bg);
}

test "buildInstances y_shift slides row positions" {
    // 2 rows, 1 col; y_shift=-5 triggers draw_extra → 3 instances needed
    var t = try Terminal.init(std.testing.allocator, 2, 1);
    defer t.deinit();
    var rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    var buf: [3]CellInstance = undefined;
    const n = rd.buildInstances(&t, 0, 0, -5, t.view_offset, &buf);
    // draw_extra=true → row_limit=3; extra row r=2 at base=0 → logical=2 = totalLines=2 → blank
    try std.testing.expectEqual(@as(usize, 3), n);
    try std.testing.expectEqual(@as(f32, -5), buf[0].y); // row 0: 0*20 + (-5)
    try std.testing.expectEqual(@as(f32, 15), buf[1].y); // row 1: 1*20 + (-5)
    try std.testing.expectEqual(@as(f32, 35), buf[2].y); // row 2 (extra): 2*20 + (-5)
}

test "buildInstances extra row when y_shift is non-trivial" {
    var t = try Terminal.init(std.testing.allocator, 1, 1);
    defer t.deinit();
    var rd = Renderer{ .cell_w = 10, .cell_h = 20, .pad_x = 0, .pad_y = 0 };
    // y_shift=-5 (abs > 1e-3) → draw_extra=true → row_limit=2
    var buf: [2]CellInstance = undefined;
    const n = rd.buildInstances(&t, 0, 0, -5, t.view_offset, &buf);
    try std.testing.expectEqual(@as(usize, 2), n);
    // row r=1 is the extra row; its y = 1*20 + (-5) = 15
    try std.testing.expectEqual(@as(f32, 15), buf[1].y);
}
