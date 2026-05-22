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
        capi.CGContextFillRect(self.ctx, self.cellRect(font, col, row));
    }

    /// Fill a sub-rectangle of a cell. Used for bar/underline cursors.
    /// `fx` is the offset from the cell's left edge as a fraction of cell width;
    /// `fy` is the offset from the cell's BOTTOM edge as a fraction of cell height
    /// (CG context is y-up, so `fy=0` is the bottom of the cell, not the top).
    /// `fw`,`fh` are the width/height size fractions.
    pub fn cellInset(
        self: *Raster,
        font: Font,
        col: usize,
        row: usize,
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
        const rect = self.cellRect(font, col, row);
        setFill(self.ctx, rgb);
        var g = [_]capi.CGGlyph{glyph};
        var pos = [_]capi.CGPoint{.{
            .x = rect.origin.x,
            .y = rect.origin.y + font.metrics.descent,
        }};
        capi.CTFontDrawGlyphs(font.ct, &g, &pos, 1, self.ctx);
    }

    fn cellRect(self: *Raster, font: Font, col: usize, row: usize) capi.CGRect {
        const cw = font.metrics.cell_w;
        const ch = font.metrics.cell_h;
        return .{
            .origin = .{
                .x = self.pad_x + @as(f64, @floatFromInt(col)) * cw,
                .y = @as(f64, @floatFromInt(self.height)) - self.pad_y - @as(f64, @floatFromInt(row + 1)) * ch,
            },
            .size = .{ .width = cw, .height = ch },
        };
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
    r.cellInset(f, 2, 1, .{ 90, 0, 0 }, 0.0, 0.0, 0.15, 1.0);
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
    r.cellInset(f, 2, 1, .{ 0, 0, 200 }, 0.0, 0.0, 1.0, 0.12);

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
