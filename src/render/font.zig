//! A monospace font for the terminal grid, backed by CoreText. All metrics
//! are in device pixels — construct with the pixel size you intend to render.

const std = @import("std");
const capi = @import("capi.zig");

pub const Metrics = struct {
    /// Advance width of one cell, device px (ceil'd to a whole pixel).
    cell_w: f64,
    /// Line height of one cell, device px.
    cell_h: f64,
    /// Baseline offset from the cell top, device px.
    ascent: f64,
    descent: f64,
};

pub const Font = struct {
    ct: capi.Ref, // CTFontRef
    metrics: Metrics,

    /// `name` is a Core Text font name (e.g. "Menlo"). `pixel_size` is the
    /// rasterization size in device pixels (point size * backing scale).
    pub fn init(name: [:0]const u8, pixel_size: f64) !Font {
        const cf_name = capi.CFStringCreateWithCString(
            null,
            name.ptr,
            capi.kCFStringEncodingUTF8,
        ) orelse return error.FontNameFailed;
        defer capi.CFRelease(cf_name);

        const ct = capi.CTFontCreateWithName(cf_name, pixel_size, null) orelse
            return error.FontCreateFailed;

        const ascent = capi.CTFontGetAscent(ct);
        const descent = capi.CTFontGetDescent(ct);
        const leading = capi.CTFontGetLeading(ct);

        // Cell width = advance of 'M' (the font is monospace, so any glyph
        // would do; 'M' is a safe, always-present choice).
        var ch = [_]u16{'M'};
        var gl = [_]capi.CGGlyph{0};
        _ = capi.CTFontGetGlyphsForCharacters(ct, &ch, &gl, 1);
        var adv = [_]capi.CGSize{.{}};
        _ = capi.CTFontGetAdvancesForGlyphs(ct, capi.kCTFontOrientationDefault, &gl, &adv, 1);

        return .{
            .ct = ct,
            .metrics = .{
                .cell_w = @ceil(adv[0].width),
                .cell_h = @ceil(ascent + descent + leading),
                .ascent = ascent,
                .descent = descent,
            },
        };
    }

    pub fn deinit(self: Font) void {
        capi.CFRelease(self.ct);
    }

    /// Try each name in order; return the first that loads with non-zero cell width.
    /// Falls back to `"Menlo"` as the last resort (always present on macOS).
    pub fn initFirstAvailable(names: []const [:0]const u8, pixel_size: f64) !Font {
        for (names) |name| {
            const f = init(name, pixel_size) catch continue;
            if (f.metrics.cell_w > 0) return f;
            f.deinit();
        }
        return error.NoFontAvailable;
    }

    /// The glyph index for a Unicode codepoint. Returns 0 (the font's missing
    /// glyph) when the font has no glyph for it.
    pub fn glyph(self: Font, cp: u21) capi.CGGlyph {
        var chars: [2]u16 = undefined;
        var glyphs = [_]capi.CGGlyph{ 0, 0 };
        var n: capi.CFIndex = 1;
        if (cp <= 0xFFFF) {
            chars[0] = @intCast(cp);
        } else {
            const v: u32 = @as(u32, cp) - 0x10000;
            chars[0] = @intCast(0xD800 + (v >> 10));
            chars[1] = @intCast(0xDC00 + (v & 0x3FF));
            n = 2;
        }
        _ = capi.CTFontGetGlyphsForCharacters(self.ct, &chars, &glyphs, n);
        return glyphs[0];
    }
};

test "brand mono font stack loads with sane metrics" {
    // IBM Plex Mono is the brand primary; SFMono-Regular is the first fallback.
    // Menlo is the last-resort (always present on macOS).
    const names = [_][:0]const u8{ "IBMPlexMono", "SFMono-Regular", "Menlo" };
    const f = try Font.initFirstAvailable(&names, 26.0);
    defer f.deinit();
    try std.testing.expect(f.metrics.cell_w > 0);
    try std.testing.expect(f.metrics.cell_h > 0);
    try std.testing.expect(f.metrics.ascent > 0);
    // Monospace at 26px: cell taller than wide, both within a sane range.
    try std.testing.expect(f.metrics.cell_h > f.metrics.cell_w);
    try std.testing.expect(f.metrics.cell_w < 64 and f.metrics.cell_h < 64);
}

test "glyph lookup resolves common characters" {
    const names = [_][:0]const u8{ "IBMPlexMono", "SFMono-Regular", "Menlo" };
    const f = try Font.initFirstAvailable(&names, 26.0);
    defer f.deinit();
    try std.testing.expect(f.glyph('A') != 0);
    try std.testing.expect(f.glyph('z') != 0);
    try std.testing.expect(f.glyph('0') != 0);
}
