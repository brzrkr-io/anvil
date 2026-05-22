//! File-tree panel renderer. Draws the left-edge panel in absolute pixel space
//! (independent of raster.x_offset, which must be 0 when this is called).
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
/// `raster.x_offset` must be 0 when this is called.
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
