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
                .x = @as(f64, @floatFromInt(col)) * cw,
                .y = @as(f64, @floatFromInt(self.height)) - @as(f64, @floatFromInt(row + 1)) * ch,
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
