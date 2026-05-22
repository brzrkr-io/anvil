//! Agent-panel — the primary developer-context surface for Anvil.
//!
//! Replaces the old `hud.zig` developer-context HUD. This module draws a
//! floating card (top-right corner by default) showing:
//!
//!   - A header row: status bullet + "agents" label + a one-line summary driven
//!     by `Snapshot.connection` and live run/approval counts.
//!   - Up to 3 priority rows (pending approvals → running runs → failure findings).
//!   - A footer: compressed local context (cwd · branch · last-run), the content
//!     from the old HUD demoted to a secondary role.
//!
//! The card layout and draw helpers are identical to the old `hud.zig` (card
//! background, 2-px border, UTF-8 glyph text) so there is no visual regression
//! for the frame chrome outside the content rows.
//!
//! Brand: Mineral palette, IBM Plex Mono (the raster font), alloy-grey labels,
//! semantic status colors (verified green / failure red / attention amber /
//! agent violet / info teal). No decoration — compact, calm, operational.
//!
//! AG1 phase: `Snapshot.connection` is hardcoded to `.not_installed` at the call
//! site. AG2 will populate the snapshot from the caldera-local API.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;
const poller = @import("../caldera/poller.zig");

pub const Snapshot = poller.Snapshot;
pub const Connection = poller.Connection;
pub const RunStatus = poller.RunStatus;
pub const AgentRunRow = poller.AgentRunRow;
pub const ApprovalRow = poller.ApprovalRow;
pub const FindingRow = poller.FindingRow;

// Re-export the Rect type for the docked placement variant.
pub const Rect = struct { x: f64, y: f64, w: f64, h: f64 };

/// Width of the agent-panel card in terminal columns.
pub const panel_cols: usize = 36;

/// Height of the card in terminal rows.
const card_rows: usize = 13;

// --- Brand color constants (Mineral palette, hex → RGB) ------------------
// These are module-level constants, matching the established pattern in
// hud.zig / tabbar.zig.  They are NOT added to the Theme struct because
// Theme is a terminal color contract (background, ANSI, surface, border)
// and expanding it would break the WCAG contrast tests.

/// alloy: muted labels / metadata (#86919a)
const alloy: [3]u8 = .{ 0x86, 0x91, 0x9a };
/// status.verified: success / passing (#3f8a5b)
const verified: [3]u8 = .{ 0x3f, 0x8a, 0x5b };
/// status.failure: failed check (#b13a30)
const failure: [3]u8 = .{ 0xb1, 0x3a, 0x30 };
/// status.attention: reviewable warning / pending action (#b07a14)
const attention: [3]u8 = .{ 0xb0, 0x7a, 0x14 };
/// status.info / trace: mineral teal (#2f7f86)
const info_teal: [3]u8 = .{ 0x2f, 0x7f, 0x86 };
/// status.agent: agent / automation / model activity — violet (#6a5fa3)
const agent_violet: [3]u8 = .{ 0x6a, 0x5f, 0xa3 };

// --- Data types ----------------------------------------------------------

pub const GitState = enum { ok, dirty, no_repo };
pub const RunState = enum { idle, ok, failed };

/// Local context: cwd, git, last-run. Was the `Hud` struct in hud.zig.
/// Used as the footer of the agent panel.
pub const LocalContext = struct {
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

    pub fn branchSlice(self: *const LocalContext) []const u8 {
        return self.branch[0..self.branch_len];
    }

    pub fn cwdSlice(self: *const LocalContext) []const u8 {
        return self.cwd[0..self.cwd_len];
    }
};

/// Where to position the card.
pub const Placement = union(enum) {
    /// Floating card in the top-right corner of the terminal area.
    floating: struct {
        total_cols: usize,
        total_rows: usize,
        top_offset: usize,
    },
    /// IDE mode: caller supplies an explicit pixel rect. Not exercised in AG1.
    docked: Rect,
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

/// Format ahead/behind counts as a compact string.
pub fn formatAheadBehind(buf: []u8, ahead: u32, behind: u32) []const u8 {
    if (ahead == 0 and behind == 0) return "";
    if (ahead > 0 and behind == 0)
        return std.fmt.bufPrint(buf, "\xe2\x86\x91{d}", .{ahead}) catch "";
    if (ahead == 0 and behind > 0)
        return std.fmt.bufPrint(buf, "\xe2\x86\x93{d}", .{behind}) catch "";
    return std.fmt.bufPrint(buf, "\xe2\x86\x91{d} \xe2\x86\x93{d}", .{ ahead, behind }) catch "";
}

/// Shorten a filesystem path to its last two components, prefixed with "…/".
pub fn formatCwd(buf: []u8, path: []const u8) []const u8 {
    if (path.len == 0) return "";
    var p = path;
    if (p.len > 1 and p[p.len - 1] == '/') p = p[0 .. p.len - 1];
    const last = std.mem.lastIndexOfScalar(u8, p, '/') orelse {
        return std.fmt.bufPrint(buf, "{s}", .{p}) catch p;
    };
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

/// Draw the agent-panel card.
///
/// `snap`     — current agent state (AG1: always connection = .not_installed).
/// `local`    — cwd / git / last-run for the footer row.
/// `placement`— where to position the card.
/// `expanded` — when true, a taller version with section headers is drawn.
///              In AG1, collapsed is the priority; expanded draws the same content.
pub fn draw(
    raster: *Raster,
    font: Font,
    theme: Theme,
    snap: *const Snapshot,
    local: *const LocalContext,
    placement: Placement,
    expanded: bool,
) void {
    _ = expanded; // collapsed view is the AG1 priority

    // Resolve card coordinates from placement.
    // Docked is defined but not exercised in AG1.
    switch (placement) {
        .docked => return, // not exercised yet
        .floating => {},
    }
    const f = placement.floating;
    if (f.total_rows == 0 or f.total_cols < panel_cols + 2) return;
    const card_col = f.total_cols - panel_cols - 2;
    const card_row = f.top_offset + 1;
    const available = f.total_rows;
    const actual_rows = @min(card_rows, available);
    if (actual_rows == 0) return;

    // --- Panel background & border (identical to old hud.zig) ---------------
    const cw = font.metrics.cell_w;
    const ch = font.metrics.cell_h;
    const left_px = raster.pad_x + @as(f64, @floatFromInt(card_col)) * cw;
    const top_px = raster.pad_y + @as(f64, @floatFromInt(card_row)) * ch;
    const card_w_px = @as(f64, @floatFromInt(panel_cols)) * cw;
    const card_h_px = @as(f64, @floatFromInt(actual_rows)) * ch;

    raster.fillPixelRect(left_px, top_px, card_w_px, card_h_px, theme.surface);

    const border: f64 = 2.0;
    raster.fillPixelRect(left_px, top_px, card_w_px, border, theme.border);
    raster.fillPixelRect(left_px, top_px + card_h_px - border, card_w_px, border, theme.border);
    raster.fillPixelRect(left_px, top_px, border, card_h_px, theme.border);
    raster.fillPixelRect(left_px + card_w_px - border, top_px, border, card_h_px, theme.border);

    // --- Content rows --------------------------------------------------------
    var row = card_row + 1; // one row breathing room at the top
    const max_row = card_row + actual_rows;

    // --- Header row: bullet + "agents" + summary ----------------------------
    if (row < max_row) {
        const bullet_color = headerBulletColor(snap);
        const summary = buildHeaderSummary(snap);
        drawAgentHeader(raster, font, card_col, row, bullet_color, summary, panel_cols);
        row += 1;
    }

    // --- Priority rows (up to 3): approvals → running → failures ------------
    var priority_count: usize = 0;

    // Pending approvals first.
    var ai: usize = 0;
    while (ai < snap.approvals_len and priority_count < 3 and row < max_row) : (ai += 1) {
        const ap = &snap.approvals[ai];
        var lbuf: [64]u8 = undefined;
        const connector = ap.connector[0..ap.connector_len];
        const label = std.fmt.bufPrint(&lbuf, "{s}", .{connector}) catch "approval";
        drawPriorityRow(raster, font, card_col, row, "\xe2\x96\xb8", attention, label, panel_cols);
        row += 1;
        priority_count += 1;
    }

    // Running runs.
    var ri: usize = 0;
    while (ri < snap.runs_len and priority_count < 3 and row < max_row) : (ri += 1) {
        const run = &snap.runs[ri];
        if (run.status != .running) continue;
        var lbuf: [64]u8 = undefined;
        const agent_name = run.agent[0..run.agent_len];
        const label = std.fmt.bufPrint(&lbuf, "{s}", .{agent_name}) catch "agent";
        // U+25C6 BLACK DIAMOND ◆
        drawPriorityRow(raster, font, card_col, row, "\xe2\x97\x86", agent_violet, label, panel_cols);
        row += 1;
        priority_count += 1;
    }

    // Failure findings.
    var fi: usize = 0;
    while (fi < snap.findings_len and priority_count < 3 and row < max_row) : (fi += 1) {
        const finding = &snap.findings[fi];
        if (finding.severity != .failure) continue;
        var lbuf: [64]u8 = undefined;
        const summary = finding.summary[0..finding.summary_len];
        const label = std.fmt.bufPrint(&lbuf, "{s}", .{summary}) catch "failure";
        // U+2717 BALLOT X ✗
        drawPriorityRow(raster, font, card_col, row, "\xe2\x9c\x97", failure, label, panel_cols);
        row += 1;
        priority_count += 1;
    }

    // Separator before the footer.
    if (row < max_row) {
        drawHairline(raster, font, card_col, row, theme, panel_cols);
    }
    row += 1;

    // --- Footer: Local context (cwd · branch · last-run) --------------------
    if (row < max_row) {
        drawLocalFooter(raster, font, theme, card_col, row, local, panel_cols);
    }
}

// --- Header helpers ------------------------------------------------------

/// Determine the bullet color from the current snapshot state.
fn headerBulletColor(snap: *const Snapshot) [3]u8 {
    return switch (snap.connection) {
        .not_installed, .no_project, .disabled, .offline, .error_state => alloy,
        .live => blk: {
            // Worst-state priority: failure > attention > agent-active > all-clear.
            if (snap.findings_len > 0) {
                for (snap.findings[0..snap.findings_len]) |f| {
                    if (f.severity == .failure) break :blk failure;
                }
            }
            if (snap.pending_approvals_count > 0) break :blk attention;
            if (snap.running_count > 0) break :blk agent_violet;
            break :blk verified;
        },
    };
}

/// Build the single-line summary that appears next to "agents" in the header.
/// Returns a slice into a static buffer — valid until the next call.
fn buildHeaderSummary(snap: *const Snapshot) []const u8 {
    const Static = struct {
        var buf: [80]u8 = undefined;
    };
    return switch (snap.connection) {
        .not_installed => "caldera-local not found",
        .no_project => "no .caldera in this repo",
        .disabled => "caldera disabled for this repo",
        .offline => "caldera-local not running",
        .error_state => "caldera api error",
        .live => blk: {
            if (snap.running_count == 0 and snap.pending_approvals_count == 0 and snap.attention_count == 0) {
                break :blk "no active runs";
            }
            var parts: [3][]const u8 = .{ "", "", "" };
            var n: usize = 0;
            if (snap.running_count > 0 and n < 3) {
                parts[n] = std.fmt.bufPrint(Static.buf[0..40], "{d} running", .{snap.running_count}) catch "?";
                n += 1;
            }
            if (snap.pending_approvals_count > 0 and n < 3) {
                const off: usize = if (n > 0) 41 else 0;
                parts[n] = std.fmt.bufPrint(Static.buf[off .. off + 39], "{d} approval{s}", .{
                    snap.pending_approvals_count,
                    if (snap.pending_approvals_count == 1) @as([]const u8, "") else "s",
                }) catch "?";
                n += 1;
            }
            if (snap.attention_count > 0 and n < 3) {
                const off: usize = 41 * n;
                parts[n] = std.fmt.bufPrint(Static.buf[off .. off + 39], "{d} attention", .{snap.attention_count}) catch "?";
                n += 1;
            }
            if (n == 0) break :blk "no active runs";
            if (n == 1) break :blk parts[0];
            // Join with " · " (UTF-8 middle dot: 0xC2 0xB7)
            var sb: [80]u8 = undefined;
            var pos: usize = 0;
            for (parts[0..n], 0..) |part, idx| {
                if (idx > 0) {
                    const sep = " \xc2\xb7 ";
                    if (pos + sep.len <= sb.len) {
                        @memcpy(sb[pos .. pos + sep.len], sep);
                        pos += sep.len;
                    }
                }
                const rem = @min(part.len, sb.len -| pos);
                @memcpy(sb[pos .. pos + rem], part[0..rem]);
                pos += rem;
            }
            @memcpy(Static.buf[0..pos], sb[0..pos]);
            break :blk Static.buf[0..pos];
        },
    };
}

// --- Row draw helpers ----------------------------------------------------

/// Draw the header row: bullet U+25CF + "agents" dim label + summary text.
/// The bullet is in `bullet_color`; the word "agents" is alloy; the summary
/// is `theme.foreground`-dim (alloy) for degraded states, foreground for live.
fn drawAgentHeader(
    raster: *Raster,
    font: Font,
    start_col: usize,
    row: usize,
    bullet_color: [3]u8,
    summary: []const u8,
    cols: usize,
) void {
    const max_col = start_col + cols - 1;
    // Col+1: bullet U+25CF
    raster.cellGlyph(font, start_col + 1, row, font.glyph(0x25CF), bullet_color);
    // Col+3: "agents" in alloy
    drawText(raster, font, start_col + 3, row, "agents", alloy, max_col);
    // Col+10: summary (3 spaces gap after "agents" which is 6 chars = col+3+6 = col+9, then gap)
    drawText(raster, font, start_col + 10, row, summary, alloy, max_col);
}

/// Draw a priority row: a glyph + a label.
/// `glyph_utf8` is the multi-byte UTF-8 string for the status icon.
fn drawPriorityRow(
    raster: *Raster,
    font: Font,
    start_col: usize,
    row: usize,
    glyph_utf8: []const u8,
    glyph_color: [3]u8,
    label: []const u8,
    cols: usize,
) void {
    const max_col = start_col + cols - 1;
    // Col+2: status glyph (1 codepoint)
    drawText(raster, font, start_col + 2, row, glyph_utf8, glyph_color, start_col + 4);
    // Col+4: label in alloy
    drawText(raster, font, start_col + 4, row, label, alloy, max_col);
}

/// Draw a horizontal hairline separator at the center of `row`.
fn drawHairline(
    raster: *Raster,
    font: Font,
    start_col: usize,
    row: usize,
    theme: Theme,
    cols: usize,
) void {
    const ch = font.metrics.cell_h;
    const cw = font.metrics.cell_w;
    const sep_y = raster.pad_y + (@as(f64, @floatFromInt(row)) + 0.5) * ch;
    const sep_x = raster.pad_x + @as(f64, @floatFromInt(start_col + 1)) * cw;
    const sep_w = @as(f64, @floatFromInt(cols - 2)) * cw;
    raster.fillPixelRect(sep_x, sep_y, sep_w, 1.0, theme.border);
}

/// Draw the Local footer: one dim row with cwd · branch · last-run.
fn drawLocalFooter(
    raster: *Raster,
    font: Font,
    theme: Theme,
    start_col: usize,
    row: usize,
    local: *const LocalContext,
    cols: usize,
) void {
    _ = theme;
    const max_col = start_col + cols - 1;

    // Build a compact line: "cwd · branch · run"
    var buf: [80]u8 = undefined;
    var pos: usize = 0;

    // cwd (last component only to save space)
    var cwd_buf: [40]u8 = undefined;
    const cwd_short = formatCwd(&cwd_buf, local.cwdSlice());
    // Use only the last path component for the footer (very compact).
    {
        const tail: []const u8 = if (std.mem.lastIndexOfScalar(u8, cwd_short, '/')) |sep|
            cwd_short[sep + 1 ..]
        else
            cwd_short;
        const rem = @min(tail.len, buf.len -| pos);
        @memcpy(buf[pos .. pos + rem], tail[0..rem]);
        pos += rem;
    }

    // · branch (if in a repo)
    if (local.git != .no_repo and local.branch_len > 0) {
        const sep = " \xc2\xb7 "; // U+00B7 middle dot
        const sl = @min(sep.len, buf.len -| pos);
        @memcpy(buf[pos .. pos + sl], sep[0..sl]);
        pos += sl;
        const br = local.branchSlice();
        const bl = @min(br.len, buf.len -| pos);
        @memcpy(buf[pos .. pos + bl], br[0..bl]);
        pos += bl;
    }

    // · run state
    {
        const sep = " \xc2\xb7 ";
        const sl = @min(sep.len, buf.len -| pos);
        @memcpy(buf[pos .. pos + sl], sep[0..sl]);
        pos += sl;
        var rbuf: [24]u8 = undefined;
        const rtxt = formatRunStatus(&rbuf, local.run, local.run_exit, local.run_duration_ms);
        const rl = @min(rtxt.len, buf.len -| pos);
        @memcpy(buf[pos .. pos + rl], rtxt[0..rl]);
        pos += rl;
    }

    drawText(raster, font, start_col + 2, row, buf[0..pos], alloy, max_col);
}

// --- Shared draw utilities -----------------------------------------------

/// Draw a UTF-8 string from cell `col`, one codepoint per cell, stopping at
/// `max_col`. Multi-byte sequences are decoded correctly.
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

/// Draw a section-header row (used internally for expanded view and footer label).
/// Returns the next row index.
fn drawSectionDot(
    raster: *Raster,
    font: Font,
    theme: Theme,
    start_col: usize,
    row: usize,
    label: []const u8,
    dot_color: [3]u8,
    cols: usize,
) usize {
    _ = theme;
    raster.cellGlyph(font, start_col + 1, row, font.glyph(0x25CF), dot_color);
    drawText(raster, font, start_col + 3, row, label, alloy, start_col + cols - 1);
    return row + 1;
}

/// Draw a value row indented under a section label. Returns the next row.
fn drawValueRow(
    raster: *Raster,
    font: Font,
    theme: Theme,
    start_col: usize,
    row: usize,
    text: []const u8,
    color_: [3]u8,
    cols: usize,
) usize {
    _ = theme;
    drawText(raster, font, start_col + 4, row, text, color_, start_col + cols - 1);
    return row + 1;
}

// Suppress "unused" warnings for helpers kept for expanded-view use in AG2.
const _keep = struct {
    const _a = drawSectionDot;
    const _b = drawValueRow;
    const _c = info_teal;
};

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

test "headerBulletColor: not_installed is alloy" {
    const snap = Snapshot{ .connection = .not_installed };
    const color = headerBulletColor(&snap);
    try testing.expectEqual(alloy, color);
}

test "headerBulletColor: live with no activity is verified" {
    const snap = Snapshot{ .connection = .live };
    const color = headerBulletColor(&snap);
    try testing.expectEqual(verified, color);
}

test "headerBulletColor: live with pending approval is attention" {
    const snap = Snapshot{ .connection = .live, .pending_approvals_count = 1 };
    const color = headerBulletColor(&snap);
    try testing.expectEqual(attention, color);
}

test "headerBulletColor: live with running count is agent_violet" {
    const snap = Snapshot{ .connection = .live, .running_count = 2 };
    const color = headerBulletColor(&snap);
    try testing.expectEqual(agent_violet, color);
}

test "buildHeaderSummary: not_installed" {
    const snap = Snapshot{ .connection = .not_installed };
    const s = buildHeaderSummary(&snap);
    try testing.expectEqualStrings("caldera-local not found", s);
}

test "buildHeaderSummary: live empty" {
    const snap = Snapshot{ .connection = .live };
    const s = buildHeaderSummary(&snap);
    try testing.expectEqualStrings("no active runs", s);
}

test "buildHeaderSummary: live with running" {
    const snap = Snapshot{ .connection = .live, .running_count = 3 };
    const s = buildHeaderSummary(&snap);
    try testing.expect(std.mem.indexOf(u8, s, "3") != null);
    try testing.expect(std.mem.indexOf(u8, s, "running") != null);
}
