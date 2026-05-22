//! Renders a segment list to an ANSI prompt string. Two forms: `full` — a
//! two-line block with a mineral accent edge; `transient` — a single collapsed
//! line for past prompts. Colours are 24-bit ANSI; the accent/edge is mineral.

const std = @import("std");
const seg = @import("segments.zig");
const icons = @import("icons.zig");

const reset = "\x1b[0m";
const accent = "\x1b[38;2;47;127;134m"; // mineral #2f7f86
const accent_err = "\x1b[38;2;177;58;48m"; // failure #b13a30
const dim = "\x1b[38;2;125;135;145m";
const edge = "\u{258e}"; // ▎

fn stateColor(s: seg.State) []const u8 {
    return switch (s) {
        .normal => dim,
        .ok => "\x1b[38;2;63;138;91m",
        .warn => "\x1b[38;2;176;122;20m",
        .err => "\x1b[38;2;177;58;48m",
        .run => accent,
    };
}

pub const Options = struct {
    rich: bool,
    failed: bool, // last command exited non-zero
};

/// The full two-line prompt block. Caller owns the returned slice.
pub fn full(allocator: std.mem.Allocator, segments: []const seg.Segment, opts: Options) ![]u8 {
    var buf: std.ArrayList(u8) = .empty;
    errdefer buf.deinit(allocator);

    const edge_color = if (opts.failed) accent_err else accent;

    // Line 1: edge + segments.
    try buf.appendSlice(allocator, edge_color);
    try buf.appendSlice(allocator, edge);
    try buf.appendSlice(allocator, reset);
    try buf.appendSlice(allocator, " ");
    for (segments, 0..) |s, i| {
        if (i != 0) try buf.appendSlice(allocator, "  ");
        try buf.appendSlice(allocator, stateColor(s.state));
        try buf.appendSlice(allocator, icons.glyph(s.icon, opts.rich));
        try buf.appendSlice(allocator, " ");
        try buf.appendSlice(allocator, s.text);
        try buf.appendSlice(allocator, reset);
    }
    try buf.appendSlice(allocator, "\n");
    // Line 2: edge + prompt glyph.
    try buf.appendSlice(allocator, edge_color);
    try buf.appendSlice(allocator, edge);
    try buf.appendSlice(allocator, " \u{276f}");
    try buf.appendSlice(allocator, reset);
    try buf.appendSlice(allocator, " ");
    try buf.appendSlice(allocator, "\x1b]133;B\x07");

    return buf.toOwnedSlice(allocator);
}

/// The collapsed transient prompt — just the glyph, no edge, no context.
pub fn transient(allocator: std.mem.Allocator, opts: Options) ![]u8 {
    const color = if (opts.failed) accent_err else dim;
    return std.fmt.allocPrint(allocator, "{s}\u{276f}{s} ", .{ color, reset });
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
    try testing.expect(std.mem.indexOf(u8, out, "\n") != null); // two lines
    try testing.expect(std.mem.indexOf(u8, out, edge) != null); // edge present
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

test "ascii mode emits fallback glyphs" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = false, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, icons.glyph(.branch, true)) == null);
}

test "transient is a single line, no edge" {
    const out = try transient(testing.allocator, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "\n") == null);
    try testing.expect(std.mem.indexOf(u8, out, edge) == null);
}

test "full emits the OSC 133;B prompt-end mark" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "\x1b]133;B") != null);
}
