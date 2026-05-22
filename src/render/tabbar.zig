//! The low-profile terminal tab bar — one text-row tall, drawn into the raster.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;
const TabManager = @import("../app/tab.zig").TabManager;

/// Draw the tab bar across raster row 0. Each tab gets an equal-width segment;
/// the active segment is filled with the theme accent, others with ansi[8]
/// (a muted surface). Active-segment labels use theme.background for contrast;
/// inactive-segment labels use theme.foreground.
pub fn drawTabBar(raster: *Raster, font: Font, theme: Theme, tabs: *TabManager) void {
    const n = tabs.count();
    if (n < 2) return; // low-profile: no bar with a single tab

    const cell_w = font.metrics.cell_w;
    // Match the padded grid width: the bar spans the inset region, not the
    // raw bitmap. cellRect applies the same pad to each column it draws.
    const usable_w = @as(f64, @floatFromInt(raster.width)) - 2 * raster.pad_x;
    const total_cols: usize = @intFromFloat(@max(usable_w, 0) / cell_w);
    if (total_cols == 0) return;
    const seg_cols = @max(total_cols / n, 1);

    var label_buf: [256]u8 = undefined;
    var t: usize = 0;
    while (t < n) : (t += 1) {
        const start_col = t * seg_cols;
        const is_active = (t == tabs.active);
        const bg = if (is_active) theme.accent else theme.ansi[8];
        // Fill the segment background across row 0.
        var col = start_col;
        const end_col = if (t == n - 1) total_cols else start_col + seg_cols;
        while (col < end_col) : (col += 1) {
            raster.cellBg(font, col, 0, bg);
        }
        // Draw the label, truncated to the segment width minus a 1-cell pad.
        const label = tabs.tabs.items[t].label(&label_buf);
        const fg = if (is_active) theme.background else theme.foreground;
        var i: usize = 0;
        while (i < label.len and i + 1 < end_col - start_col) : (i += 1) {
            raster.cellGlyph(font, start_col + 1 + i, 0, font.glyph(label[i]), fg);
        }
    }
}

const testing = std.testing;

test "drawTabBar is a no-op below 2 tabs" {
    // A TabManager with 0 tabs: drawTabBar must return without touching the
    // raster. We assert by giving it a raster pre-filled with a sentinel and
    // checking it is unchanged.
    const RasterT = @import("raster.zig").Raster;
    const FontT = @import("font.zig").Font;
    const theme = @import("../config/theme.zig").mineral_dark;
    const TabManagerT = @import("../app/tab.zig").TabManager;

    const f = try FontT.init("Menlo", 26.0);
    defer f.deinit();
    var r = try RasterT.init(testing.allocator, 200, 80);
    defer r.deinit();
    r.clear(.{ 1, 2, 3 });

    var mgr = TabManagerT.init(testing.allocator);
    defer mgr.deinit();

    drawTabBar(&r, f, theme, &mgr); // 0 tabs -> no-op
    // pixel (5,5) still the sentinel
    const px = (5 * r.width + 5) * 4;
    try testing.expectEqual(@as(u8, 3), r.pixels[px + 0]); // B
    try testing.expectEqual(@as(u8, 1), r.pixels[px + 2]); // R
}
