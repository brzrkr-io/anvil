//! Rasterizes the terminal grid into a BGRA8 bitmap with CoreGraphics +
//! CoreText. The bitmap is then uploaded as a texture by the Metal renderer.
//!
//! Coordinates: terminal row 0 is the top. The CoreGraphics context is y-up
//! (origin bottom-left), so a row's CG-y is measured down from `height`.

const std = @import("std");
const capi = @import("capi.zig");
const Font = @import("font.zig").Font;

pub const Raster = struct {
    alloc: std.mem.Allocator,
    pixels: []u8, // BGRA8, width*height*4
    ctx: capi.Ref, // CGContextRef over `pixels`
    space: capi.Ref, // CGColorSpaceRef
    width: usize,
    height: usize,
    pad_x: f64 = 0, // inset margin in device pixels, applied by cellRect
    pad_y: f64 = 0,
    y_shift_px: f64 = 0, // vertical pixel offset added to every cell (smooth scroll)

    pub fn init(alloc: std.mem.Allocator, width: usize, height: usize) !Raster {
        const space = capi.CGColorSpaceCreateDeviceRGB() orelse return error.ColorSpaceFailed;
        var self: Raster = .{
            .alloc = alloc,
            .pixels = &.{},
            .ctx = null,
            .space = space,
            .width = 0,
            .height = 0,
        };
        try self.resize(width, height);
        return self;
    }

    pub fn deinit(self: *Raster) void {
        if (self.ctx) |ctx| capi.CGContextRelease(ctx);
        capi.CGColorSpaceRelease(self.space);
        if (self.pixels.len > 0) self.alloc.free(self.pixels);
    }

    pub fn resize(self: *Raster, width: usize, height: usize) !void {
        if (width == self.width and height == self.height and self.ctx != null) return;
        const w = @max(width, 1);
        const h = @max(height, 1);
        const new_px = try self.alloc.alloc(u8, w * h * 4);
        const ctx = capi.CGBitmapContextCreate(
            new_px.ptr,
            w,
            h,
            8,
            w * 4,
            self.space,
            capi.bgra8_bitmap_info,
        ) orelse {
            self.alloc.free(new_px);
            return error.ContextFailed;
        };
        capi.CGContextSetTextMatrix(ctx, capi.identity);
        if (self.ctx) |old| capi.CGContextRelease(old);
        if (self.pixels.len > 0) self.alloc.free(self.pixels);
        self.pixels = new_px;
        self.ctx = ctx;
        self.width = w;
        self.height = h;
    }

    /// Fill the whole bitmap with one color.
    pub fn clear(self: *Raster, rgb: [3]u8) void {
        setFill(self.ctx, rgb);
        capi.CGContextFillRect(self.ctx, .{ .origin = .{}, .size = .{
            .width = @floatFromInt(self.width),
            .height = @floatFromInt(self.height),
        } });
    }

    /// Fill one cell's background.
    pub fn cellBg(self: *Raster, font: Font, col: usize, row: usize, rgb: [3]u8) void {
        setFill(self.ctx, rgb);
        capi.CGContextFillRect(self.ctx, self.cellRect(font, @floatFromInt(col), @floatFromInt(row)));
    }

    /// Fill a sub-rectangle of a cell. Used for bar/underline cursors.
    /// `fx` is the offset from the cell's left edge as a fraction of cell width;
    /// `fy` is the offset from the cell's BOTTOM edge as a fraction of cell height
    /// (CG context is y-up, so `fy=0` is the bottom of the cell, not the top).
    /// `fw`,`fh` are the width/height size fractions.
    /// `col` and `row` are fractional cell coordinates for sub-cell animation.
    pub fn cellInset(
        self: *Raster,
        font: Font,
        col: f64,
        row: f64,
        rgb: [3]u8,
        fx: f64,
        fy: f64,
        fw: f64,
        fh: f64,
    ) void {
        const r = self.cellRect(font, col, row);
        setFill(self.ctx, rgb);
        capi.CGContextFillRect(self.ctx, .{
            .origin = .{ .x = r.origin.x + r.size.width * fx, .y = r.origin.y + r.size.height * fy },
            .size = .{ .width = r.size.width * fw, .height = r.size.height * fh },
        });
    }

    /// Draw one glyph in a cell. `glyph` of 0 (missing glyph) draws nothing.
    pub fn cellGlyph(self: *Raster, font: Font, col: usize, row: usize, glyph: u16, rgb: [3]u8) void {
        if (glyph == 0) return;
        const rect = self.cellRect(font, @floatFromInt(col), @floatFromInt(row));
        setFill(self.ctx, rgb);
        var g = [_]capi.CGGlyph{glyph};
        var pos = [_]capi.CGPoint{.{
            .x = rect.origin.x,
            .y = rect.origin.y + font.metrics.descent,
        }};
        capi.CTFontDrawGlyphs(font.ct, &g, &pos, 1, self.ctx);
    }

    /// Draw a thin full-height vertical hairline at the LEFT edge of cell-column
    /// `col`. The strip is ≈2 device pixels wide. Intended for panel separators.
    pub fn colRule(self: *Raster, font: Font, col: usize, rgb: [3]u8) void {
        const cw = font.metrics.cell_w;
        const strip_w: f64 = 2.0;
        const left_x = self.pad_x + @as(f64, @floatFromInt(col)) * cw;
        setFill(self.ctx, rgb);
        capi.CGContextFillRect(self.ctx, .{
            .origin = .{ .x = left_x, .y = 0 },
            .size = .{ .width = strip_w, .height = @floatFromInt(self.height) },
        });
    }

    /// Draw a thin full-width horizontal hairline at the TOP edge of cell-row
    /// `row`. The strip is ≈2 device pixels tall and respects `y_shift_px` so
    /// it scrolls with the grid. Intended for prompt-start separator marks.
    pub fn rowRule(self: *Raster, font: Font, row: f64, rgb: [3]u8) void {
        const ch = font.metrics.cell_h;
        // Top edge of the row in CG coordinates (y-up, origin bottom-left).
        // cellRect places the cell at:
        //   y = height - pad_y - (row + 1) * ch + y_shift_px
        // The TOP of that cell (highest CG y) is one cell_h above that origin:
        //   top_y = height - pad_y - row * ch + y_shift_px
        // We fill a 2px strip just below the top edge (toward lower CG y).
        const strip_h: f64 = 2.0;
        const top_y = @as(f64, @floatFromInt(self.height)) - self.pad_y -
            row * ch + self.y_shift_px - strip_h;
        setFill(self.ctx, rgb);
        capi.CGContextFillRect(self.ctx, .{
            .origin = .{ .x = 0, .y = top_y },
            .size = .{ .width = @floatFromInt(self.width), .height = strip_h },
        });
    }

    fn cellRect(self: *Raster, font: Font, col: f64, row: f64) capi.CGRect {
        const cw = font.metrics.cell_w;
        const ch = font.metrics.cell_h;
        return .{
            .origin = .{
                .x = self.pad_x + col * cw,
                .y = @as(f64, @floatFromInt(self.height)) - self.pad_y - (row + 1.0) * ch + self.y_shift_px,
            },
            .size = .{ .width = cw, .height = ch },
        };
    }

    /// Fill an arbitrary rectangle in device-pixel coordinates.
    /// `px`, `py` are the top-left corner in raster space (y=0 is the top row
    /// of the bitmap). The CG context is y-up, so we convert here.
    pub fn fillPixelRect(self: *Raster, px: f64, py: f64, pw: f64, ph: f64, rgb: [3]u8) void {
        setFill(self.ctx, rgb);
        // CG y origin is bottom-left; raster py is from top.
        const cg_y = @as(f64, @floatFromInt(self.height)) - py - ph;
        capi.CGContextFillRect(self.ctx, .{
            .origin = .{ .x = px, .y = cg_y },
            .size = .{ .width = pw, .height = ph },
        });
    }

    /// Like `fillPixelRect`, but composites the fill over the existing content
    /// at `alpha` (0 = clear, 1 = opaque) — used for the translucent HUD card.
    pub fn fillPixelRectAlpha(self: *Raster, px: f64, py: f64, pw: f64, ph: f64, rgb: [3]u8, alpha: f64) void {
        capi.CGContextSetRGBFillColor(
            self.ctx,
            @as(f64, @floatFromInt(rgb[0])) / 255.0,
            @as(f64, @floatFromInt(rgb[1])) / 255.0,
            @as(f64, @floatFromInt(rgb[2])) / 255.0,
            alpha,
        );
        const cg_y = @as(f64, @floatFromInt(self.height)) - py - ph;
        capi.CGContextFillRect(self.ctx, .{
            .origin = .{ .x = px, .y = cg_y },
            .size = .{ .width = pw, .height = ph },
        });
    }

    /// The BGRA8 pixel buffer, ready for texture upload.
    pub fn bytes(self: *Raster) []const u8 {
        return self.pixels;
    }
};

fn setFill(ctx: capi.Ref, rgb: [3]u8) void {
    capi.CGContextSetRGBFillColor(
        ctx,
        @as(f64, @floatFromInt(rgb[0])) / 255.0,
        @as(f64, @floatFromInt(rgb[1])) / 255.0,
        @as(f64, @floatFromInt(rgb[2])) / 255.0,
        1.0,
    );
}

fn pixelAt(r: *Raster, x: usize, y: usize) [3]u8 {
    const i = (y * r.width + x) * 4;
    return .{ r.pixels[i + 2], r.pixels[i + 1], r.pixels[i + 0] }; // BGRA -> RGB
}

test "clear fills the bitmap" {
    var r = try Raster.init(std.testing.allocator, 64, 48);
    defer r.deinit();
    r.clear(.{ 10, 20, 30 });
    try std.testing.expectEqual([3]u8{ 10, 20, 30 }, pixelAt(&r, 5, 5));
    try std.testing.expectEqual([3]u8{ 10, 20, 30 }, pixelAt(&r, 60, 40));
}

test "cellBg and glyph draw onto the bitmap" {
    const f = try Font.init("Menlo", 26.0);
    defer f.deinit();
    var r = try Raster.init(std.testing.allocator, 400, 200);
    defer r.deinit();
    r.clear(.{ 0, 0, 0 });
    r.cellBg(f, 1, 1, .{ 80, 0, 0 });
    // a pixel inside cell (1,1) should now carry the bg color
    const cx: usize = @intFromFloat(f.metrics.cell_w * 1.5);
    const cy: usize = @intFromFloat(f.metrics.cell_h * 1.5);
    try std.testing.expectEqual([3]u8{ 80, 0, 0 }, pixelAt(&r, cx, cy));
    // a white 'A' glyph lights up pixels in the cell
    r.cellGlyph(f, 1, 1, f.glyph('A'), .{ 255, 255, 255 });
    var lit = false;
    var y: usize = @intFromFloat(f.metrics.cell_h);
    const y_end: usize = @intFromFloat(f.metrics.cell_h * 2);
    while (y < y_end) : (y += 1) {
        var x: usize = @intFromFloat(f.metrics.cell_w);
        const x_end: usize = @intFromFloat(f.metrics.cell_w * 2);
        while (x < x_end) : (x += 1) {
            if (pixelAt(&r, x, y)[0] > 100) lit = true;
        }
    }
    try std.testing.expect(lit);
}

test "resize keeps a usable context" {
    var r = try Raster.init(std.testing.allocator, 32, 32);
    defer r.deinit();
    try r.resize(128, 64);
    try std.testing.expectEqual(@as(usize, 128), r.width);
    r.clear(.{ 7, 7, 7 });
    try std.testing.expectEqual([3]u8{ 7, 7, 7 }, pixelAt(&r, 100, 50));
}

test "cellInset fills a sub-rectangle of a cell" {
    const f = try Font.init("Menlo", 26.0);
    defer f.deinit();
    var r = try Raster.init(std.testing.allocator, 400, 200);
    defer r.deinit();
    r.clear(.{ 0, 0, 0 });
    // A left bar: 15% width, full height, of cell (2,1).
    r.cellInset(f, 2.0, 1.0, .{ 90, 0, 0 }, 0.0, 0.0, 0.15, 1.0);
    const lx: usize = @intFromFloat(f.metrics.cell_w * 2.0 + 1);
    const ly: usize = @intFromFloat(f.metrics.cell_h * 1.5);
    try std.testing.expectEqual([3]u8{ 90, 0, 0 }, pixelAt(&r, lx, ly));
    // The cell's right half stays clear.
    const rx: usize = @intFromFloat(f.metrics.cell_w * 2.8);
    try std.testing.expectEqual([3]u8{ 0, 0, 0 }, pixelAt(&r, rx, ly));
}

test "cellInset underline fills the cell bottom" {
    const f = try Font.init("Menlo", 26.0);
    defer f.deinit();
    var r = try Raster.init(std.testing.allocator, 400, 200);
    defer r.deinit();
    r.clear(.{ 0, 0, 0 });
    // Underline: full width, bottom 12% of cell height, at cell (2, 1).
    // fy=0 is the cell bottom in CG (y-up), so this fills the lowest strip.
    r.cellInset(f, 2.0, 1.0, .{ 0, 0, 200 }, 0.0, 0.0, 1.0, 0.12);

    // The bitmap is y-down: the cell's bottom (CG) maps to the LARGEST
    // bitmap-y rows of the cell. Cell (2,1) occupies bitmap rows [ch, 2*ch).
    // A pixel one row from the cell's bottom edge (bitmap row 2*ch - 2)
    // should carry the fill color.
    const ux: usize = @intFromFloat(f.metrics.cell_w * 2.5);
    const bot_y: usize = @intFromFloat(f.metrics.cell_h * 2.0 - 2);
    try std.testing.expectEqual([3]u8{ 0, 0, 200 }, pixelAt(&r, ux, bot_y));

    // A pixel in the upper half of the same cell (bitmap row ~1.5*ch)
    // should still be clear — it is above the underline strip.
    const mid_y: usize = @intFromFloat(f.metrics.cell_h * 1.5);
    try std.testing.expectEqual([3]u8{ 0, 0, 0 }, pixelAt(&r, ux, mid_y));
}

test "rowRule draws a strip at the top of a cell row" {
    const f = try Font.init("Menlo", 26.0);
    defer f.deinit();
    const w: usize = 400;
    const h: usize = 300;
    var r = try Raster.init(std.testing.allocator, w, h);
    defer r.deinit();
    r.clear(.{ 0, 0, 0 });
    // Draw a rule at raster row 1 (the second row from the top of the viewport).
    r.rowRule(f, 1.0, .{ 200, 100, 50 });
    // The strip sits at the top edge of row 1. In the bitmap (y-down), row 1
    // starts at bitmap-y = cell_h (row 0 occupies [0, cell_h)).
    // The rule strip is 2px tall just at that boundary.
    const strip_bitmap_y: usize = @intFromFloat(f.metrics.cell_h);
    // A pixel in the middle of the strip's x-span should carry the rule color.
    const mid_x: usize = w / 2;
    try std.testing.expectEqual([3]u8{ 200, 100, 50 }, pixelAt(&r, mid_x, strip_bitmap_y));
    // A pixel well into the interior of row 1 (well below the top edge) should
    // still be the cleared background.
    const inner_y: usize = @intFromFloat(f.metrics.cell_h * 1.5);
    try std.testing.expectEqual([3]u8{ 0, 0, 0 }, pixelAt(&r, mid_x, inner_y));
}

test "y_shift_px shifts cellBg upward in the bitmap" {
    const f = try Font.init("Menlo", 26.0);
    defer f.deinit();
    const w: usize = 400;
    const h: usize = 200;
    var r_base = try Raster.init(std.testing.allocator, w, h);
    defer r_base.deinit();
    var r_shift = try Raster.init(std.testing.allocator, w, h);
    defer r_shift.deinit();

    // Sample near the vertical center of cell (1, 1) with no shift.
    r_base.clear(.{ 0, 0, 0 });
    r_base.cellBg(f, 1, 1, .{ 255, 0, 0 });
    const cx: usize = @intFromFloat(f.metrics.cell_w * 1.5);
    // Use a row near the BOTTOM of cell (1,1): bitmap-y ≈ 1.9 * cell_h.
    // Without shift this is inside the cell; with a full-cell shift upward
    // the cell moves entirely out of this region.
    const cy: usize = @intFromFloat(f.metrics.cell_h * 1.9);
    const base_px = pixelAt(&r_base, cx, cy);

    // Same cell with a positive shift equal to a full cell height: the cell
    // moves UP by one full cell so it now occupies bitmap rows [0, cell_h).
    // The previously sampled row (at ~1.9 * cell_h) is now well below the
    // shifted cell and should be clear (background).
    const shift: f64 = f.metrics.cell_h; // shift a full cell upward
    r_shift.clear(.{ 0, 0, 0 });
    r_shift.y_shift_px = shift;
    r_shift.cellBg(f, 1, 1, .{ 255, 0, 0 });
    r_shift.y_shift_px = 0;
    const shift_px = pixelAt(&r_shift, cx, cy);

    // Without shift the sample row is inside the cell; with the cell shifted
    // up by a full cell height, that row is now outside the cell.
    try std.testing.expectEqual([3]u8{ 255, 0, 0 }, base_px);
    try std.testing.expectEqual([3]u8{ 0, 0, 0 }, shift_px);
}
