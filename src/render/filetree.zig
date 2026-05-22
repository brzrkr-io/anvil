//! File-tree panel renderer. Draws the left-edge panel in absolute pixel space
//! (independent of raster.x_offset, which must be 0 when this is called).
//!
//! Brand: Mineral palette — charcoal panel bg, ash separator, alloy-grey file
//! names, foreground for dir names, info-teal icons.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;
const color = @import("color.zig");
const FileTree = @import("../app/filetree.zig").FileTree;

/// Number of terminal columns the tree panel occupies.
pub const tree_cols: usize = 26;

// Brand color constants (Mineral palette).
/// ash: separator (#374046)
const ash: [3]u8 = .{ 0x37, 0x40, 0x46 };
/// alloy: muted text / file names (#86919a)
const alloy: [3]u8 = .{ 0x86, 0x91, 0x9a };
/// info/trace teal for dir icons (#2f7f86)
const info_teal: [3]u8 = .{ 0x2f, 0x7f, 0x86 };
/// charcoal: panel background (#161a1c)
const charcoal: [3]u8 = .{ 0x16, 0x1a, 0x1c };

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

    // Panel background: slightly lighter than background.
    const panel_bg = color.mix(theme.background, theme.foreground, 0.04);
    raster.fillPixelRect(pad_x, panel_top_px, panel_w_px, panel_h_px, panel_bg);

    // 1px right-edge border.
    raster.fillPixelRect(pad_x + panel_w_px - 1.0, panel_top_px, 1.0, panel_h_px, ash);

    // Draw entries.
    const content_rows = total_rows -| top_offset;
    var row_idx: usize = 0;
    var entry_idx: usize = 0;
    while (entry_idx < tree.count and row_idx < content_rows) : (entry_idx += 1) {
        const e = &tree.entries[entry_idx];
        const raster_row = top_offset + row_idx;

        // Indent: 1 col for each depth level, then icon at that col.
        const indent_cols: usize = @as(usize, e.depth) * 2;

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
        const name_max_col = tree_cols; // stop at panel edge
        if (name_start_col < name_max_col) {
            const name_color: [3]u8 = if (e.is_dir) theme.foreground else alloy;
            drawText(raster, font, name_start_col, raster_row, e.nameSlice(), name_color, name_max_col);
        }

        row_idx += 1;
    }

    _ = charcoal; // referenced in doc comment; kept for future use
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
