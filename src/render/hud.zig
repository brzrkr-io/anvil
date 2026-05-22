//! Developer-context HUD panel — drawn into the rightmost N columns of the
//! raster. The terminal grid is already narrowed by `hud_cols + 1` columns
//! (the +1 is the separator gutter) when the HUD is visible.
//!
//! Brand: Mineral palette, IBM Plex Mono (the raster font), alloy-grey labels,
//! semantic status colors (verified green / failure red / attention amber /
//! info teal). No decoration — compact, calm, operational.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;

/// Width of the HUD panel in terminal columns. The separator gutter takes one
/// additional column, so the terminal grid is narrowed by `hud_cols + 1`.
pub const hud_cols: usize = 30;

// --- Brand color constants (Mineral palette, hex → RGB) ------------------

/// alloy: muted labels / metadata (#86919a)
const alloy: [3]u8 = .{ 0x86, 0x91, 0x9a };
/// ash: separator hairline (#374046)
const ash: [3]u8 = .{ 0x37, 0x40, 0x46 };
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

    // system section
    mem_pct: u8 = 0, // 0-100

    pub fn branchSlice(self: *const Hud) []const u8 {
        return self.branch[0..self.branch_len];
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

// --- Draw ----------------------------------------------------------------

/// Draw the HUD panel starting at raster column `start_col`. `total_rows` is
/// the full cell-row count of the visible area (tab bar rows already excluded
/// by the caller — pass the grid rows count so we draw into the right region,
/// adding `top_offset` for the tab bar).
pub fn draw(
    raster: *Raster,
    font: Font,
    theme: Theme,
    hud: Hud,
    start_col: usize,
    total_rows: usize,
    top_offset: usize,
) void {
    if (total_rows == 0) return;

    // Separator gutter: one thin vertical rule in the column just before start_col.
    const sep_col = start_col -| 1;
    const sep_rgb = ash;
    raster.colRule(font, sep_col, sep_rgb);

    // Fill the HUD background (panel surface = charcoal-ish mix).
    // We re-use theme.ansi[8] (the muted bg slot) as the panel surface — it is
    // slightly lifted from the terminal background, which provides quiet contrast.
    const panel_bg = theme.ansi[0]; // darkest ansi slot = deep surface
    var c = start_col;
    while (c < start_col + hud_cols) : (c += 1) {
        var r = top_offset;
        while (r < top_offset + total_rows) : (r += 1) {
            raster.cellBg(font, c, r, panel_bg);
        }
    }

    // Current raster row cursor — draw rows top-to-bottom inside the grid area.
    var row = top_offset;

    // --- git section -------------------------------------------------------
    row = drawSectionDot(raster, font, theme, start_col, row, "git", info_teal);
    switch (hud.git) {
        .no_repo => {
            row = drawValueRow(raster, font, theme, start_col, row, "not a repo", alloy);
        },
        .ok, .dirty => {
            const branch = hud.branchSlice();
            row = drawValueRow(raster, font, theme, start_col, row, branch, theme.foreground);
            // dirty count line
            if (hud.git_dirty > 0) {
                var dbuf: [32]u8 = undefined;
                const dtxt = std.fmt.bufPrint(&dbuf, "{d} dirty", .{hud.git_dirty}) catch "";
                row = drawValueRow(raster, font, theme, start_col, row, dtxt, attention);
            }
            // ahead/behind line
            var abbuf: [32]u8 = undefined;
            const abtxt = formatAheadBehind(&abbuf, hud.git_ahead, hud.git_behind);
            if (abtxt.len > 0) {
                row = drawValueRow(raster, font, theme, start_col, row, abtxt, alloy);
            }
        },
    }

    // blank row between sections
    row += 1;

    // --- last-run section --------------------------------------------------
    row = drawSectionDot(raster, font, theme, start_col, row, "last run", info_teal);
    {
        var rbuf: [48]u8 = undefined;
        const rtxt = formatRunStatus(&rbuf, hud.run, hud.run_exit, hud.run_duration_ms);
        const run_color: [3]u8 = switch (hud.run) {
            .idle => alloy,
            .ok => verified,
            .failed => failure,
        };
        row = drawValueRow(raster, font, theme, start_col, row, rtxt, run_color);
    }

    // blank row between sections
    row += 1;

    // --- system section (only if still in bounds) --------------------------
    if (row < top_offset + total_rows) {
        var mbuf: [24]u8 = undefined;
        const mtxt = std.fmt.bufPrint(&mbuf, "mem {d}%", .{hud.mem_pct}) catch "mem ?";
        _ = drawSectionDot(raster, font, theme, start_col, row, mtxt, info_teal);
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
    // U+25CF BLACK CIRCLE — the status bullet, in the section's status color.
    raster.cellGlyph(font, start_col, row, font.glyph(0x25CF), dot_color);
    // Label in alloy, two cols in (bullet, gap, label).
    drawText(raster, font, start_col + 2, row, label, alloy, start_col + hud_cols);
    return row + 1;
}

/// Draw one value row, indented two cols to align under the section label.
/// Returns the next row index.
fn drawValueRow(
    raster: *Raster,
    font: Font,
    theme: Theme,
    start_col: usize,
    row: usize,
    text: []const u8,
    color: [3]u8,
) usize {
    _ = theme;
    drawText(raster, font, start_col + 2, row, text, color, start_col + hud_cols);
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
