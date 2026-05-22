//! File-tree panel renderer. Draws the left-edge panel in absolute pixel space
//! (raster.origin_x and raster.origin_y must be 0 when this is called).
//!
//! Brand: Mineral palette — theme.surface panel bg, theme.border separator,
//! alloy-grey file names, foreground for dir names, info-teal icons.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;
const FileTree = @import("../app/filetree.zig").FileTree;
const color = @import("color.zig");

/// Number of terminal columns the tree panel occupies.
pub const tree_cols: usize = 26;

// Brand color constants (Mineral palette).
/// alloy: muted text / file names (#86919a)
const alloy: [3]u8 = .{ 0x86, 0x91, 0x9a };
/// info/trace teal for dir icons (#2f7f86)
const info_teal: [3]u8 = .{ 0x2f, 0x7f, 0x86 };

// Nerd Font icon codepoints.
const icon_folder_closed: u21 = 0xf07b;
const icon_folder_open: u21 = 0xf07c;
const icon_file: u21 = 0xf15b;

/// Draw the file-tree panel into the left `tree_cols` columns of the raster.
/// `total_rows` is the full visible row count (including top bar).
/// `top_offset` is the number of rows taken by the tab bar (0 or 1).
/// `raster.origin_x` and `raster.origin_y` must be 0 when this is called.
pub fn draw(
    raster: *Raster,
    font: Font,
    theme: Theme,
    tree: *const FileTree,
    total_rows: usize,
    top_offset: usize,
) void {
    if (total_rows == 0) return;

    const cw = font.metrics.cell_w;
    const ch = font.metrics.cell_h;

    // Panel pixel extents (raster device-pixel space, y=0 at top).
    const pad_x = raster.pad_x;
    const pad_y = raster.pad_y;
    const panel_w_px = @as(f64, @floatFromInt(tree_cols)) * cw;
    const panel_top_px = pad_y + @as(f64, @floatFromInt(top_offset)) * ch;
    const panel_h_px = @as(f64, @floatFromInt(total_rows -| top_offset)) * ch;

    // Panel background: solid surface tone — a clearly defined sidebar.
    raster.fillPixelRect(pad_x, panel_top_px, panel_w_px, panel_h_px, theme.surface);

    // 1px right-edge border.
    raster.fillPixelRect(pad_x + panel_w_px - 1.0, panel_top_px, 1.0, panel_h_px, theme.border);

    // Header row: "FILES" label in info_teal, 2-col left pad; border below.
    const header_raster_row = top_offset;
    if (total_rows > top_offset) {
        drawText(raster, font, 2, header_raster_row, "FILES", info_teal, tree_cols - 1);
        const header_rule_y = pad_y + @as(f64, @floatFromInt(header_raster_row + 1)) * ch;
        raster.fillPixelRect(pad_x, header_rule_y, panel_w_px - 1.0, 1.0, theme.border);
    }

    // Draw entries (start one row below the header).
    const content_rows = total_rows -| top_offset;
    var row_idx: usize = 1;
    var entry_idx: usize = 0;
    while (entry_idx < tree.count and row_idx < content_rows) : (entry_idx += 1) {
        const e = &tree.entries[entry_idx];
        const raster_row = top_offset + row_idx;

        // Selected row: fill full panel width with a subtle accent tint.
        if (tree.selected_idx) |sel| {
            if (entry_idx == sel) {
                const row_top_px = pad_y + @as(f64, @floatFromInt(raster_row)) * ch;
                raster.fillPixelRect(pad_x, row_top_px, panel_w_px, ch, color.mix(theme.background, theme.accent, 0.18));
            }
        }

        // Indent: 1 col inner left padding, then depth, then icon.
        const indent_cols: usize = 1 + @as(usize, e.depth) * 2;

        // Icon codepoint and color.
        const icon_cp: u21 = if (e.is_dir)
            (if (e.expanded) icon_folder_open else icon_folder_closed)
        else
            icon_file;
        const icon_color: [3]u8 = if (e.is_dir) info_teal else alloy;

        // Draw icon if it fits within the panel.
        const icon_col = indent_cols;
        if (icon_col < tree_cols) {
            raster.cellGlyph(font, icon_col, raster_row, font.glyph(icon_cp), icon_color);
        }

        // Draw name starting one col after the icon.
        const name_start_col = indent_cols + 2;
        const name_max_col = tree_cols - 1; // leave 1-col right margin
        if (name_start_col < name_max_col) {
            const name_color: [3]u8 = if (e.is_dir) theme.foreground else alloy;
            drawText(raster, font, name_start_col, raster_row, e.nameSlice(), name_color, name_max_col);
        }

        row_idx += 1;
    }
}

/// Map a click's y-coordinate (measured from the top of the view, in points)
/// to a zero-based entry index into the file-tree list.
///
/// Returns `null` when the click lands on the header row or above/below the
/// visible tree content. Returns the entry index otherwise — the caller is
/// responsible for bounds-checking against the actual entry count.
///
/// Parameters:
///   `click_y_from_top` — y in points measured from the top of the view (NOT AppKit's
///                         y-up convention; callers must convert).
///   `tree_top`         — y in points where the tree content area begins
///                         (= top_bar_height_pt + pad_pt).
///   `cell_h`           — row height in points.
///   `header_rows`      — number of non-entry header rows at the top of the
///                         panel (always 1: the "FILES" label row).
pub fn treeRowAtClick(
    click_y_from_top: f64,
    tree_top: f64,
    cell_h: f64,
    header_rows: usize,
) ?usize {
    if (cell_h <= 0) return null;
    // Must be past the header rows.
    const header_h = @as(f64, @floatFromInt(header_rows)) * cell_h;
    if (click_y_from_top < tree_top + header_h) return null;
    const raw_row: usize = @intFromFloat((click_y_from_top - tree_top) / cell_h);
    if (raw_row < header_rows) return null;
    return raw_row - header_rows;
}

test "treeRowAtClick maps click-y to entry index" {
    const testing = std.testing;
    const tree_top: f64 = 30.0; // e.g. 1 tab row * 20px + 10px pad
    const cell_h: f64 = 20.0;
    const header_rows: usize = 1;

    // Click inside the header row → null.
    try testing.expectEqual(@as(?usize, null), treeRowAtClick(tree_top + 5.0, tree_top, cell_h, header_rows));
    // Click exactly at the header top boundary → null.
    try testing.expectEqual(@as(?usize, null), treeRowAtClick(tree_top, tree_top, cell_h, header_rows));
    // Click in the first entry row (just past header) → index 0.
    try testing.expectEqual(@as(?usize, 0), treeRowAtClick(tree_top + cell_h + 1.0, tree_top, cell_h, header_rows));
    // Click near the bottom of the first entry row → index 0.
    try testing.expectEqual(@as(?usize, 0), treeRowAtClick(tree_top + cell_h * 2.0 - 1.0, tree_top, cell_h, header_rows));
    // Click in the second entry row → index 1.
    try testing.expectEqual(@as(?usize, 1), treeRowAtClick(tree_top + cell_h * 2.0 + 1.0, tree_top, cell_h, header_rows));
    // Click above the tree_top → null.
    try testing.expectEqual(@as(?usize, null), treeRowAtClick(tree_top - 1.0, tree_top, cell_h, header_rows));
    // Zero cell_h → null (guard against division by zero).
    try testing.expectEqual(@as(?usize, null), treeRowAtClick(100.0, tree_top, 0.0, header_rows));
}

/// Draw a UTF-8 string from cell `col`, one codepoint per cell, stopping at `max_col`.
/// Must decode codepoints, not walk bytes, to handle multi-byte UTF-8 name chars.
fn drawText(
    raster: *Raster,
    font: Font,
    col: usize,
    row: usize,
    text: []const u8,
    text_color: [3]u8,
    max_col: usize,
) void {
    var cx = col;
    const view = std.unicode.Utf8View.init(text) catch return;
    var it = view.iterator();
    while (it.nextCodepoint()) |cp| {
        if (cx >= max_col) break;
        raster.cellGlyph(font, cx, row, font.glyph(cp), text_color);
        cx += 1;
    }
}
