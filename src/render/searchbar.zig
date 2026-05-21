//! The in-terminal search bar — one text row at the bottom of the window.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;
const Search = @import("../terminal/search.zig").Search;

/// Draw the search bar across the bottom raster row. `bottom_row` is the cell
/// row index of that last row. Shows a "find:" prefix, the query, and a
/// `current/total` match counter.
pub fn drawSearchBar(
    raster: *Raster,
    font: Font,
    theme: Theme,
    search: *const Search,
    bottom_row: usize,
) void {
    const cell_w = font.metrics.cell_w;
    const total_cols: usize = @intFromFloat(@as(f64, @floatFromInt(raster.width)) / cell_w);
    if (total_cols == 0) return;

    // Bar background across the whole bottom row.
    var c: usize = 0;
    while (c < total_cols) : (c += 1) raster.cellBg(font, c, bottom_row, theme.ansi[8]);

    // Compose the bar text: "find: <query>" left-aligned, "<cur>/<total>" right.
    // Compute the counter first so its length is known when capping the left text.
    var count_buf: [32]u8 = undefined;
    const cur = if (search.count() == 0) 0 else search.current + 1;
    const counter = std.fmt.bufPrint(&count_buf, "{d}/{d}", .{ cur, search.count() }) catch "";

    var line_buf: [512]u8 = undefined;
    const text = std.fmt.bufPrint(&line_buf, "find: {s}", .{search.query()}) catch "find:";
    // Left text must not reach the counter; leave at least a 1-column gap.
    // Guard against usize underflow when the counter alone fills the window.
    if (counter.len + 1 < total_cols) {
        const left_limit = total_cols - counter.len - 1;
        var i: usize = 0;
        while (i < text.len and i < left_limit) : (i += 1) {
            raster.cellGlyph(font, i, bottom_row, font.glyph(text[i]), theme.foreground);
        }
    }

    // Fix: use <= so a counter whose length exactly equals total_cols still fits.
    if (counter.len <= total_cols) {
        const start = total_cols - counter.len;
        for (counter, 0..) |ch, j| {
            raster.cellGlyph(font, start + j, bottom_row, font.glyph(ch), theme.foreground);
        }
    }
}

const testing = std.testing;

test "drawSearchBar fills the bottom row background" {
    const f = try Font.init("Menlo", 26.0);
    defer f.deinit();
    var r = try Raster.init(testing.allocator, 400, 200);
    defer r.deinit();
    r.clear(.{ 0, 0, 0 });

    var s = Search.init(testing.allocator);
    defer s.deinit();

    const theme = @import("../config/theme.zig").mineral_dark;
    const cell_h: usize = @intFromFloat(f.metrics.cell_h);
    const bottom_row: usize = (200 / cell_h) - 1;
    drawSearchBar(&r, f, theme, &s, bottom_row);

    // A pixel inside the bottom row now carries the bar background (ansi[8]).
    const px_y: usize = bottom_row * cell_h + cell_h / 2;
    const px = (px_y * r.width + 4) * 4;
    try testing.expectEqual(theme.ansi[8][2], r.pixels[px + 0]); // B channel == ansi8 blue
}
