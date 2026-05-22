//! Developer-context HUD — a compact floating card in the top-right corner of
//! the terminal area. The terminal grid always occupies full width; the card is
//! drawn ON TOP of the grid content.
//!
//! Brand: Mineral palette, IBM Plex Mono (the raster font), alloy-grey labels,
//! semantic status colors (verified green / failure red / attention amber /
//! info teal). No decoration — compact, calm, operational.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;
/// Width of the HUD card in terminal columns.
pub const hud_cols: usize = 30;

/// Height of the HUD card in terminal rows (tall enough for all sections with
/// a couple of blank rows in reserve, plus 1 top padding row).
const card_rows: usize = 13;

// --- Brand color constants (Mineral palette, hex → RGB) ------------------

/// alloy: muted labels / metadata (#86919a)
const alloy: [3]u8 = .{ 0x86, 0x91, 0x9a };
/// status.verified: success / passing (#3f8a5b)
const verified: [3]u8 = .{ 0x3f, 0x8a, 0x5b };
/// status.failure: failed check (#b13a30)
const failure: [3]u8 = .{ 0xb1, 0x3a, 0x30 };
/// status.attention: dirty / warning (#b07a14)
const attention: [3]u8 = .{ 0xb0, 0x7a, 0x14 };
/// status.info / trace: mineral teal (#2f7f86)
const info_teal: [3]u8 = .{ 0x2f, 0x7f, 0x86 };

// --- Data ---------------------------------------------------------------

pub const GitState = enum { ok, dirty, no_repo };

pub const RunState = enum { idle, ok, failed };

/// All display data for one HUD frame. Plain data — no allocations.
pub const Hud = struct {
    // cwd section
    cwd: [64]u8 = undefined,
    cwd_len: usize = 0,

    // git section
    git: GitState = .no_repo,
    branch: [128]u8 = undefined,
    branch_len: usize = 0,
    git_dirty: u32 = 0,
    git_ahead: u32 = 0,
    git_behind: u32 = 0,

    // last-run section
    run: RunState = .idle,
    run_exit: i32 = 0,
    run_duration_ms: i64 = 0,

    pub fn branchSlice(self: *const Hud) []const u8 {
        return self.branch[0..self.branch_len];
    }

    pub fn cwdSlice(self: *const Hud) []const u8 {
        return self.cwd[0..self.cwd_len];
    }
};

// --- Formatting helpers (pure, unit-testable) ----------------------------

/// Format a duration in milliseconds as a compact human string into `buf`.
/// Returns the slice written (e.g. "0.3s", "1.2s", "72s").
pub fn formatDuration(buf: []u8, ms: i64) []const u8 {
    if (ms < 0) return std.fmt.bufPrint(buf, "0s", .{}) catch "0s";
    const s = @divTrunc(ms, 1000);
    const frac = @divTrunc(@mod(ms, 1000), 100); // tenths
    if (s < 10) {
        return std.fmt.bufPrint(buf, "{d}.{d}s", .{ s, frac }) catch "?s";
    }
    return std.fmt.bufPrint(buf, "{d}s", .{s}) catch "?s";
}

/// Format the last-run outcome as a compact status string into `buf`.
/// E.g. "ok · 1.2s", "failed 1 · 0.5s", "running…"
pub fn formatRunStatus(buf: []u8, run: RunState, exit_code: i32, duration_ms: i64) []const u8 {
    var dur_buf: [16]u8 = undefined;
    const dur = formatDuration(&dur_buf, duration_ms);
    return switch (run) {
        .idle => std.fmt.bufPrint(buf, "idle", .{}) catch "idle",
        .ok => std.fmt.bufPrint(buf, "ok \xc2\xb7 {s}", .{dur}) catch "ok",
        .failed => std.fmt.bufPrint(buf, "failed {d} \xc2\xb7 {s}", .{ exit_code, dur }) catch "failed",
    };
}

/// Format ahead/behind counts as a compact string. E.g. "\xe2\x86\x91 2" or
/// "\xe2\x86\x913 \xe2\x86\x931" (up-arrow ahead, down-arrow behind).
pub fn formatAheadBehind(buf: []u8, ahead: u32, behind: u32) []const u8 {
    if (ahead == 0 and behind == 0) return "";
    if (ahead > 0 and behind == 0)
        return std.fmt.bufPrint(buf, "\xe2\x86\x91{d}", .{ahead}) catch "";
    if (ahead == 0 and behind > 0)
        return std.fmt.bufPrint(buf, "\xe2\x86\x93{d}", .{behind}) catch "";
    return std.fmt.bufPrint(buf, "\xe2\x86\x91{d} \xe2\x86\x93{d}", .{ ahead, behind }) catch "";
}

/// Shorten a filesystem path to its last two components, prefixed with "…/".
/// E.g. "/Users/foo/projects/anvil" → "…/projects/anvil".
/// If the path has ≤2 components (or is empty), it is returned as-is into `buf`.
pub fn formatCwd(buf: []u8, path: []const u8) []const u8 {
    if (path.len == 0) return "";
    // Strip trailing slash.
    var p = path;
    if (p.len > 1 and p[p.len - 1] == '/') p = p[0 .. p.len - 1];
    // Find the last slash.
    const last = std.mem.lastIndexOfScalar(u8, p, '/') orelse {
        return std.fmt.bufPrint(buf, "{s}", .{p}) catch p;
    };
    // Find the second-to-last slash.
    if (last == 0) {
        return std.fmt.bufPrint(buf, "{s}", .{p}) catch p;
    }
    const prev = std.mem.lastIndexOfScalar(u8, p[0..last], '/') orelse {
        return std.fmt.bufPrint(buf, "{s}", .{p}) catch p;
    };
    const tail = p[prev + 1 ..];
    return std.fmt.bufPrint(buf, "\xe2\x80\xa6/{s}", .{tail}) catch p;
}

// --- Draw ----------------------------------------------------------------

/// Draw the HUD as a floating card in the top-right corner of the terminal
/// area. `total_cols` is the full terminal column count; `total_rows` is the
/// full visible row count (grid rows only, tab bar excluded); `top_offset` is
/// the number of rows taken by the tab bar above the grid.
///
/// The card sits 1 column in from the right edge and 1 row below the tab bar.
/// It is `hud_cols` wide and `card_rows` tall.
pub fn draw(
    raster: *Raster,
    font: Font,
    theme: Theme,
    hud: Hud,
    total_cols: usize,
    total_rows: usize,
    top_offset: usize,
) void {
    if (total_rows == 0 or total_cols < hud_cols + 2) return;

    // Card top-left cell: 2 column margin from the right edge, 1 row below tab bar.
    const card_col = total_cols - hud_cols - 2;
    const card_row = top_offset + 1;

    // How many rows are actually available below the tab bar.
    const available = total_rows; // total_rows already excludes top_offset rows
    const actual_card_rows = @min(card_rows, available);
    if (actual_card_rows == 0) return;

    // --- Panel background — a translucent frosted card --------------------
    // Card pixel extents (raster device-pixel space, y=0 at top).
    const cw = font.metrics.cell_w;
    const ch = font.metrics.cell_h;
    const left_px = raster.pad_x + @as(f64, @floatFromInt(card_col)) * cw;
    const top_px = raster.pad_y + @as(f64, @floatFromInt(card_row)) * ch;
    const card_w_px = @as(f64, @floatFromInt(hud_cols)) * cw;
    const card_h_px = @as(f64, @floatFromInt(actual_card_rows)) * ch;

    // Fully opaque raised card using surface tone.
    raster.fillPixelRect(left_px, top_px, card_w_px, card_h_px, theme.surface);

    // --- Border (2-device-pixel strips around the card) -------------------
    const border: f64 = 2.0;
    // Top edge
    raster.fillPixelRect(left_px, top_px, card_w_px, border, theme.border);
    // Bottom edge
    raster.fillPixelRect(left_px, top_px + card_h_px - border, card_w_px, border, theme.border);
    // Left edge
    raster.fillPixelRect(left_px, top_px, border, card_h_px, theme.border);
    // Right edge
    raster.fillPixelRect(left_px + card_w_px - border, top_px, border, card_h_px, theme.border);

    // --- Content rows (top-to-bottom inside card) --------------------------
    // Start one row into the card for top breathing room.
    var row = card_row + 1;
    const max_row = card_row + actual_card_rows;

    // --- cwd section -------------------------------------------------------
    if (row < max_row) {
        row = drawSectionDot(raster, font, theme, card_col, row, "cwd", info_teal);
    }
    if (row < max_row) {
        var cwdbuf: [80]u8 = undefined;
        const cwdtxt = formatCwd(&cwdbuf, hud.cwdSlice());
        row = drawValueRow(raster, font, theme, card_col, row, cwdtxt, theme.foreground);
    }

    // blank row with center hairline
    if (row < max_row) {
        const sep_y = raster.pad_y + (@as(f64, @floatFromInt(row)) + 0.5) * ch;
        const sep_x = raster.pad_x + @as(f64, @floatFromInt(card_col + 1)) * cw;
        const sep_w = @as(f64, @floatFromInt(hud_cols - 2)) * cw;
        raster.fillPixelRect(sep_x, sep_y, sep_w, 1.0, theme.border);
    }
    row += 1;

    // --- git section -------------------------------------------------------
    if (row < max_row) {
        row = drawSectionDot(raster, font, theme, card_col, row, "git", info_teal);
    }
    switch (hud.git) {
        .no_repo => {
            if (row < max_row) {
                row = drawValueRow(raster, font, theme, card_col, row, "not a repo", alloy);
            }
        },
        .ok, .dirty => {
            if (row < max_row) {
                const branch = hud.branchSlice();
                row = drawValueRow(raster, font, theme, card_col, row, branch, theme.foreground);
            }
            if (row < max_row and hud.git_dirty > 0) {
                var dbuf: [32]u8 = undefined;
                const dtxt = std.fmt.bufPrint(&dbuf, "{d} dirty", .{hud.git_dirty}) catch "";
                row = drawValueRow(raster, font, theme, card_col, row, dtxt, attention);
            }
            if (row < max_row) {
                var abbuf: [32]u8 = undefined;
                const abtxt = formatAheadBehind(&abbuf, hud.git_ahead, hud.git_behind);
                if (abtxt.len > 0) {
                    row = drawValueRow(raster, font, theme, card_col, row, abtxt, alloy);
                }
            }
        },
    }

    // blank row with center hairline
    if (row < max_row) {
        const sep_y = raster.pad_y + (@as(f64, @floatFromInt(row)) + 0.5) * ch;
        const sep_x = raster.pad_x + @as(f64, @floatFromInt(card_col + 1)) * cw;
        const sep_w = @as(f64, @floatFromInt(hud_cols - 2)) * cw;
        raster.fillPixelRect(sep_x, sep_y, sep_w, 1.0, theme.border);
    }
    row += 1;

    // --- last-run section --------------------------------------------------
    if (row < max_row) {
        row = drawSectionDot(raster, font, theme, card_col, row, "last run", info_teal);
    }
    if (row < max_row) {
        var rbuf: [48]u8 = undefined;
        const rtxt = formatRunStatus(&rbuf, hud.run, hud.run_exit, hud.run_duration_ms);
        const run_color: [3]u8 = switch (hud.run) {
            .idle => alloy,
            .ok => verified,
            .failed => failure,
        };
        _ = drawValueRow(raster, font, theme, card_col, row, rtxt, run_color);
    }
}

// --- Internal draw helpers -----------------------------------------------

/// Draw a UTF-8 string from cell `col`, one cell per codepoint, stopping at
/// `max_col`. The HUD's text (status glyphs, the `·` separator, the ↑/↓
/// arrows) is multi-byte UTF-8 — it must be decoded to codepoints, not walked
/// byte-by-byte, or each byte renders as a separate mojibake glyph.
fn drawText(
    raster: *Raster,
    font: Font,
    col: usize,
    row: usize,
    text: []const u8,
    color_: [3]u8,
    max_col: usize,
) void {
    var cx = col;
    const view = std.unicode.Utf8View.init(text) catch return;
    var it = view.iterator();
    while (it.nextCodepoint()) |cp| {
        if (cx >= max_col) break;
        raster.cellGlyph(font, cx, row, font.glyph(cp), color_);
        cx += 1;
    }
}

/// Draw a section-header row: a filled bullet + a dim label.
/// Returns the next row index.
fn drawSectionDot(
    raster: *Raster,
    font: Font,
    theme: Theme,
    start_col: usize,
    row: usize,
    label: []const u8,
    dot_color: [3]u8,
) usize {
    _ = theme;
    // U+25CF BLACK CIRCLE — the status bullet, 1 col inner padding from panel edge.
    raster.cellGlyph(font, start_col + 1, row, font.glyph(0x25CF), dot_color);
    // Label in alloy, three cols in (pad, bullet, gap, label).
    drawText(raster, font, start_col + 3, row, label, alloy, start_col + hud_cols - 1);
    return row + 1;
}

/// Draw one value row, indented one extra column under the section label.
/// Returns the next row index.
fn drawValueRow(
    raster: *Raster,
    font: Font,
    theme: Theme,
    start_col: usize,
    row: usize,
    text: []const u8,
    color_: [3]u8,
) usize {
    _ = theme;
    drawText(raster, font, start_col + 4, row, text, color_, start_col + hud_cols - 1);
    return row + 1;
}

// --- Tests ---------------------------------------------------------------

const testing = std.testing;

test "formatDuration sub-second" {
    var buf: [16]u8 = undefined;
    const s = formatDuration(&buf, 350);
    try testing.expectEqualStrings("0.3s", s);
}

test "formatDuration seconds with tenths" {
    var buf: [16]u8 = undefined;
    const s = formatDuration(&buf, 1250);
    try testing.expectEqualStrings("1.2s", s);
}

test "formatDuration large value" {
    var buf: [16]u8 = undefined;
    const s = formatDuration(&buf, 72000);
    try testing.expectEqualStrings("72s", s);
}

test "formatDuration negative clamps to zero" {
    var buf: [16]u8 = undefined;
    const s = formatDuration(&buf, -100);
    try testing.expectEqualStrings("0s", s);
}

test "formatRunStatus ok" {
    var buf: [64]u8 = undefined;
    const s = formatRunStatus(&buf, .ok, 0, 1200);
    // "ok · 1.2s" — the · is UTF-8 0xC2 0xB7
    try testing.expect(std.mem.startsWith(u8, s, "ok"));
    try testing.expect(std.mem.indexOf(u8, s, "1.2s") != null);
}

test "formatRunStatus failed with exit code" {
    var buf: [64]u8 = undefined;
    const s = formatRunStatus(&buf, .failed, 127, 500);
    try testing.expect(std.mem.indexOf(u8, s, "failed") != null);
    try testing.expect(std.mem.indexOf(u8, s, "127") != null);
}

test "formatRunStatus idle" {
    var buf: [64]u8 = undefined;
    const s = formatRunStatus(&buf, .idle, 0, 0);
    try testing.expectEqualStrings("idle", s);
}

test "formatAheadBehind ahead only" {
    var buf: [16]u8 = undefined;
    const s = formatAheadBehind(&buf, 2, 0);
    try testing.expect(std.mem.indexOf(u8, s, "2") != null);
}

test "formatAheadBehind both" {
    var buf: [32]u8 = undefined;
    const s = formatAheadBehind(&buf, 3, 1);
    try testing.expect(std.mem.indexOf(u8, s, "3") != null);
    try testing.expect(std.mem.indexOf(u8, s, "1") != null);
}

test "formatAheadBehind neither returns empty" {
    var buf: [16]u8 = undefined;
    const s = formatAheadBehind(&buf, 0, 0);
    try testing.expectEqualStrings("", s);
}

test "formatCwd last two components" {
    var buf: [80]u8 = undefined;
    const s = formatCwd(&buf, "/Users/foo/projects/anvil");
    // Should contain "projects/anvil" prefixed with ellipsis
    try testing.expect(std.mem.indexOf(u8, s, "projects/anvil") != null);
}

test "formatCwd short path returned as-is" {
    var buf: [80]u8 = undefined;
    const s = formatCwd(&buf, "/anvil");
    try testing.expectEqualStrings("/anvil", s);
}

test "formatCwd empty" {
    var buf: [80]u8 = undefined;
    const s = formatCwd(&buf, "");
    try testing.expectEqualStrings("", s);
}
