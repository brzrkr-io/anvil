//! Keyboard-shortcut cheatsheet overlay. Draws a centered modal card listing
//! every shortcut grouped by category.
//!
//! Brand: Mineral palette — near-opaque charcoal card, ash border, alloy group
//! headers, accent (mineral teal) chords, foreground descriptions. No decoration.
//!
//! Call `draw` from renderFrame *last* (on top of grid, HUD, tree, tab bar).

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;

// --- Brand color constants (Mineral palette) --------------------------------

/// ash: border hairline (#374046)
const ash: [3]u8 = .{ 0x37, 0x40, 0x46 };
/// alloy: muted labels / group headers (#86919a)
const alloy: [3]u8 = .{ 0x86, 0x91, 0x9a };

// --- Shortcut data ----------------------------------------------------------

pub const Row = union(enum) {
    /// A group header label.
    header: []const u8,
    /// A shortcut: chord string + description.
    shortcut: struct { chord: []const u8, desc: []const u8 },
};

/// Static shortcut list. Pure data — no allocations, testable.
// Modifiers are spelled out (Cmd / Ctrl / Shift) — the ⌘ ⌃ ⇧ glyphs are not
// in the terminal font and render blank.
pub const rows: []const Row = &[_]Row{
    .{ .header = "Tabs" },
    .{ .shortcut = .{ .chord = "Cmd T", .desc = "new tab" } },
    .{ .shortcut = .{ .chord = "Cmd W", .desc = "close tab" } },
    .{ .shortcut = .{ .chord = "Ctrl Tab", .desc = "next tab" } },
    .{ .shortcut = .{ .chord = "Ctrl Shift Tab", .desc = "previous tab" } },
    .{ .shortcut = .{ .chord = "Cmd 1-9", .desc = "jump to tab" } },
    .{ .header = "Panels" },
    .{ .shortcut = .{ .chord = "Cmd K", .desc = "command palette" } },
    .{ .shortcut = .{ .chord = "Cmd J", .desc = "toggle HUD" } },
    .{ .shortcut = .{ .chord = "Cmd E", .desc = "toggle file tree" } },
    .{ .shortcut = .{ .chord = "Cmd /", .desc = "this cheatsheet" } },
    .{ .header = "Search" },
    .{ .shortcut = .{ .chord = "Cmd F", .desc = "search" } },
    .{ .shortcut = .{ .chord = "Cmd G", .desc = "next match" } },
    .{ .shortcut = .{ .chord = "Cmd Shift G", .desc = "previous match" } },
    .{ .header = "Navigation" },
    .{ .shortcut = .{ .chord = "Cmd Up", .desc = "previous command" } },
    .{ .shortcut = .{ .chord = "Cmd Down", .desc = "next command" } },
    .{ .header = "Selection" },
    .{ .shortcut = .{ .chord = "drag", .desc = "select text" } },
    .{ .shortcut = .{ .chord = "Cmd C", .desc = "copy" } },
    .{ .shortcut = .{ .chord = "Cmd-click", .desc = "open path or URL" } },
};

/// Card width in terminal columns. Wide enough for the longest row.
pub const card_cols: usize = 52;

/// Card height in terminal rows (title + blank + all rows + 1 footer + 1 padding).
pub const card_rows: usize = rows.len + 4;

// --- Draw -------------------------------------------------------------------

/// Draw the cheatsheet as a centered modal card. Must be called last in
/// renderFrame so it renders on top of all other UI elements.
pub fn draw(
    raster: *Raster,
    font: Font,
    theme: Theme,
    total_cols: usize,
    total_rows: usize,
) void {
    if (total_rows < card_rows + 2 or total_cols < card_cols + 2) return;

    // Center the card.
    const card_col = (total_cols - card_cols) / 2;
    const card_row = (total_rows - card_rows) / 2;

    const cw = font.metrics.cell_w;
    const ch = font.metrics.cell_h;
    const left_px = raster.pad_x + @as(f64, @floatFromInt(card_col)) * cw;
    const top_px = raster.pad_y + @as(f64, @floatFromInt(card_row)) * ch;
    const card_w_px = @as(f64, @floatFromInt(card_cols)) * cw;
    const card_h_px = @as(f64, @floatFromInt(card_rows)) * ch;

    // Near-opaque panel background — this is a focused modal.
    raster.fillPixelRectAlpha(left_px, top_px, card_w_px, card_h_px, theme.background, 0.96);

    // 1px ash border on all four edges.
    const b: f64 = 1.0;
    raster.fillPixelRect(left_px, top_px, card_w_px, b, ash);
    raster.fillPixelRect(left_px, top_px + card_h_px - b, card_w_px, b, ash);
    raster.fillPixelRect(left_px, top_px, b, card_h_px, ash);
    raster.fillPixelRect(left_px + card_w_px - b, top_px, b, card_h_px, ash);

    // Content rows inside the card.
    const max_col = card_col + card_cols - 1; // leave 1-col right margin

    // Row 0: title.
    const title = "Keyboard Shortcuts";
    drawText(raster, font, card_col + 2, card_row, title, theme.foreground, max_col);

    // Row 1: dim hint.
    const hint = "Cmd / or Esc to close";
    drawText(raster, font, card_col + 2, card_row + 1, hint, alloy, max_col);

    // Row 2: blank separator before content.
    // (nothing drawn)

    // Rows 3+: content.
    var r: usize = card_row + 3;
    for (rows) |row| {
        switch (row) {
            .header => |label| {
                drawText(raster, font, card_col + 2, r, label, alloy, max_col);
                r += 1;
            },
            .shortcut => |s| {
                // Chord: left-aligned in accent color at col+2.
                drawText(raster, font, card_col + 2, r, s.chord, theme.accent, max_col);
                // Description: starts at a fixed column in foreground, clear
                // of the widest chord ("Ctrl Shift Tab").
                const desc_col = card_col + 19;
                if (desc_col < max_col) {
                    drawText(raster, font, desc_col, r, s.desc, theme.foreground, max_col);
                }
                r += 1;
            },
        }
    }
}

// --- Internal helpers -------------------------------------------------------

/// Draw a UTF-8 string from cell `col`, one codepoint per cell, stopping at
/// `max_col`. The chord strings contain multi-byte UTF-8 (⌘, ⇧, ↑, ↓ etc.) and
/// must be decoded to codepoints, not walked byte-by-byte.
fn drawText(
    raster: *Raster,
    font: Font,
    col: usize,
    row: usize,
    text: []const u8,
    color: [3]u8,
    max_col: usize,
) void {
    var cx = col;
    const view = std.unicode.Utf8View.init(text) catch return;
    var it = view.iterator();
    while (it.nextCodepoint()) |cp| {
        if (cx >= max_col) break;
        raster.cellGlyph(font, cx, row, font.glyph(cp), color);
        cx += 1;
    }
}

// --- Tests ------------------------------------------------------------------

const testing = std.testing;

test "cheatsheet rows list is non-empty" {
    try testing.expect(rows.len > 0);
}

test "cheatsheet rows contain at least one header" {
    var found_header = false;
    for (rows) |row| {
        if (row == .header) {
            found_header = true;
            break;
        }
    }
    try testing.expect(found_header);
}

test "cheatsheet rows contain at least one shortcut" {
    var found_shortcut = false;
    for (rows) |row| {
        if (row == .shortcut) {
            found_shortcut = true;
            break;
        }
    }
    try testing.expect(found_shortcut);
}

test "all shortcut chords and descs are non-empty" {
    for (rows) |row| {
        switch (row) {
            .header => |label| try testing.expect(label.len > 0),
            .shortcut => |s| {
                try testing.expect(s.chord.len > 0);
                try testing.expect(s.desc.len > 0);
            },
        }
    }
}

test "cheatsheet rows are valid UTF-8" {
    for (rows) |row| {
        switch (row) {
            .header => |label| try testing.expect(std.unicode.utf8ValidateSlice(label)),
            .shortcut => |s| {
                try testing.expect(std.unicode.utf8ValidateSlice(s.chord));
                try testing.expect(std.unicode.utf8ValidateSlice(s.desc));
            },
        }
    }
}

test "card_rows covers all content rows" {
    // card_rows = rows.len + 4  (title + hint + blank + rows + 1 footer pad)
    try testing.expectEqual(rows.len + 4, card_rows);
}
