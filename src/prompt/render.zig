//! Renders a segment list to an ANSI prompt string. Every escape sequence is
//! wrapped in the shell's zero-width markers (`%{ %}` for zsh, `\001 \002` for
//! bash) — without that the shell miscounts the prompt's visible width and
//! typed input lands in the wrong column.

const std = @import("std");
const seg = @import("segments.zig");

pub const Shell = enum { plain, zsh, bash };

const reset = "\x1b[0m";
const accent = "\x1b[38;2;70;165;173m"; // mineral #46a5ad
const accent_err = "\x1b[38;2;196;86;75m"; // failure #c4564b
const bright = "\x1b[38;2;223;225;232m"; // soft white — the cwd anchor
const dim = "\x1b[38;2;125;135;145m";
const rule_color = "\x1b[38;2;74;80;96m"; // a quiet separator grey
const edge = "\u{258e}"; // ▎

fn stateColor(s: seg.State) []const u8 {
    return switch (s) {
        .normal => dim,
        .ok => "\x1b[38;2;90;168;115m",
        .warn => "\x1b[38;2;199;154;62m",
        .err => accent_err,
        .run => accent,
    };
}

pub const Options = struct {
    rich: bool,
    failed: bool, // last command exited non-zero
    width: usize = 0, // terminal columns, for the separator rule
    shell: Shell = .plain,
};

const Buf = std.ArrayList(u8);

/// Append a non-printing escape sequence, wrapped in the shell's zero-width
/// markers so the shell counts only the visible glyphs.
fn esc(buf: *Buf, a: std.mem.Allocator, shell: Shell, seq: []const u8) !void {
    switch (shell) {
        .plain => try buf.appendSlice(a, seq),
        .zsh => {
            try buf.appendSlice(a, "%{");
            try buf.appendSlice(a, seq);
            try buf.appendSlice(a, "%}");
        },
        .bash => {
            try buf.appendSlice(a, "\x01");
            try buf.appendSlice(a, seq);
            try buf.appendSlice(a, "\x02");
        },
    }
}

/// A faint, full-width horizontal rule — printed as the prompt's first line so
/// the scrollback reads as distinct command blocks. `width` is the terminal
/// column count; the rule stops one column short to avoid the wrap edge case.
pub fn rule(a: std.mem.Allocator, width: usize, shell: Shell) ![]u8 {
    const w: usize = if (width <= 1) 79 else @min(width - 1, 511);
    var buf: Buf = .empty;
    errdefer buf.deinit(a);
    try esc(&buf, a, shell, rule_color);
    var i: usize = 0;
    while (i < w) : (i += 1) try buf.appendSlice(a, "\u{2500}");
    try esc(&buf, a, shell, reset);
    try buf.appendSlice(a, "\n");
    return buf.toOwnedSlice(a);
}

/// The full two-line prompt block, with the separator rule as its first line.
pub fn full(a: std.mem.Allocator, segments: []const seg.Segment, opts: Options) ![]u8 {
    var buf: Buf = .empty;
    errdefer buf.deinit(a);
    const sh = opts.shell;

    const r = try rule(a, opts.width, sh);
    defer a.free(r);
    try buf.appendSlice(a, r);

    const edge_color = if (opts.failed) accent_err else accent;
    // Line 1: edge + segments. The cwd anchor is bright; the rest take their
    // state colour.
    try esc(&buf, a, sh, edge_color);
    try buf.appendSlice(a, edge);
    try esc(&buf, a, sh, reset);
    try buf.appendSlice(a, "  ");
    for (segments, 0..) |s, idx| {
        if (idx != 0) try buf.appendSlice(a, "   ");
        try esc(&buf, a, sh, if (idx == 0) bright else stateColor(s.state));
        try buf.appendSlice(a, s.text);
        try esc(&buf, a, sh, reset);
    }
    try buf.appendSlice(a, "\n");
    // Line 2: edge + prompt glyph, aligned under the segments.
    try esc(&buf, a, sh, edge_color);
    try buf.appendSlice(a, edge);
    try esc(&buf, a, sh, reset);
    try buf.appendSlice(a, "  ");
    try esc(&buf, a, sh, edge_color);
    try buf.appendSlice(a, "\u{203a}");
    try esc(&buf, a, sh, reset);
    try buf.appendSlice(a, " ");
    try esc(&buf, a, sh, "\x1b]133;B\x07");
    return buf.toOwnedSlice(a);
}

/// The collapsed transient prompt — just the glyph.
pub fn transient(a: std.mem.Allocator, opts: Options) ![]u8 {
    var buf: Buf = .empty;
    errdefer buf.deinit(a);
    const color = if (opts.failed) accent_err else dim;
    try esc(&buf, a, opts.shell, color);
    try buf.appendSlice(a, "\u{203a}");
    try esc(&buf, a, opts.shell, reset);
    try buf.appendSlice(a, " ");
    return buf.toOwnedSlice(a);
}

const testing = std.testing;

fn sampleSegs() [2]seg.Segment {
    return .{
        .{ .icon = .repo, .text = "caldera-console" },
        .{ .icon = .branch, .text = "main", .state = .warn },
    };
}

test "full renders two lines with the accent edge" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "\n") != null);
    try testing.expect(std.mem.indexOf(u8, out, edge) != null);
    try testing.expect(std.mem.indexOf(u8, out, "caldera-console") != null);
    try testing.expect(std.mem.indexOf(u8, out, "main") != null);
}

test "full uses the failure colour when the last command failed" {
    const s = sampleSegs();
    const ok = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(ok);
    const bad = try full(testing.allocator, &s, .{ .rich = true, .failed = true });
    defer testing.allocator.free(bad);
    try testing.expect(std.mem.indexOf(u8, bad, accent_err) != null);
    try testing.expect(std.mem.indexOf(u8, ok, accent_err) == null);
}

test "the cwd anchor renders bright" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, bright) != null);
}

test "transient is a single line" {
    const out = try transient(testing.allocator, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "\n") == null);
}

test "full emits the OSC 133;B prompt-end mark" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "\x1b]133;B") != null);
}

test "rule is a faint full-width line ending in a newline" {
    const out = try rule(testing.allocator, 12, .plain);
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "\u{2500}") != null);
    try testing.expect(std.mem.endsWith(u8, out, "\n"));
}

test "zsh mode wraps escape sequences in zero-width markers" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false, .shell = .zsh });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "%{") != null);
    try testing.expect(std.mem.indexOf(u8, out, "%}") != null);
}
