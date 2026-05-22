//! Renders a segment list to an ANSI prompt string. Two forms: `full` — a
//! two-line block with a mineral accent edge; `transient` — a single collapsed
//! line for past prompts. Text-forward: colour and spacing carry the design,
//! no icon glyphs (the terminal font has no coverage for them yet).

const std = @import("std");
const seg = @import("segments.zig");

const reset = "\x1b[0m";
const accent = "\x1b[38;2;70;165;173m"; // mineral #46a5ad
const accent_err = "\x1b[38;2;196;86;75m"; // failure #c4564b
const bright = "\x1b[38;2;223;225;232m"; // soft white — the cwd anchor
const dim = "\x1b[38;2;125;135;145m";
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
};

/// The full two-line prompt block. Caller owns the returned slice.
pub fn full(allocator: std.mem.Allocator, segments: []const seg.Segment, opts: Options) ![]u8 {
    var buf: std.ArrayList(u8) = .empty;
    errdefer buf.deinit(allocator);

    // A faint separator rule is the prompt's first line — drawn as part of the
    // prompt so the terminal renders it reliably, capping each command block.
    const r = try rule(allocator, opts.width);
    defer allocator.free(r);
    try buf.appendSlice(allocator, r);

    const edge_color = if (opts.failed) accent_err else accent;

    // Line 1: edge + segments. The cwd anchor is bright; the rest take their
    // state colour.
    try buf.appendSlice(allocator, edge_color);
    try buf.appendSlice(allocator, edge);
    try buf.appendSlice(allocator, reset);
    try buf.appendSlice(allocator, "  ");
    for (segments, 0..) |s, i| {
        if (i != 0) try buf.appendSlice(allocator, "   ");
        try buf.appendSlice(allocator, if (i == 0) bright else stateColor(s.state));
        try buf.appendSlice(allocator, s.text);
        try buf.appendSlice(allocator, reset);
    }
    try buf.appendSlice(allocator, "\n");
    // Line 2: edge + prompt glyph, aligned under the segments.
    try buf.appendSlice(allocator, edge_color);
    try buf.appendSlice(allocator, edge);
    try buf.appendSlice(allocator, reset);
    try buf.appendSlice(allocator, "  ");
    try buf.appendSlice(allocator, edge_color);
    try buf.appendSlice(allocator, "\u{203a}");
    try buf.appendSlice(allocator, reset);
    try buf.appendSlice(allocator, " ");
    try buf.appendSlice(allocator, "\x1b]133;B\x07");

    return buf.toOwnedSlice(allocator);
}

/// The collapsed transient prompt — just the glyph, no edge, no context.
pub fn transient(allocator: std.mem.Allocator, opts: Options) ![]u8 {
    const color = if (opts.failed) accent_err else dim;
    return std.fmt.allocPrint(allocator, "{s}\u{203a}{s} ", .{ color, reset });
}

/// A faint, full-width horizontal rule — printed above each prompt so the
/// scrollback reads as distinct command blocks. `width` is the terminal column
/// count; 0 falls back to 80. The returned slice ends with a newline.
pub fn rule(allocator: std.mem.Allocator, width: usize) ![]u8 {
    // One column short of the full width — filling the exact width triggers the
    // terminal's deferred-wrap edge case and the prompt overwrites the rule's
    // row. Leading \r and trailing \r\n keep the rule on its own clean line.
    const w: usize = if (width <= 1) 79 else @min(width - 1, 511);
    var buf: std.ArrayList(u8) = .empty;
    errdefer buf.deinit(allocator);
    try buf.appendSlice(allocator, "\x1b[38;2;74;80;96m"); // a quiet separator grey
    var i: usize = 0;
    while (i < w) : (i += 1) try buf.appendSlice(allocator, "\u{2500}");
    try buf.appendSlice(allocator, reset);
    try buf.appendSlice(allocator, "\n");
    return buf.toOwnedSlice(allocator);
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

test "the cwd anchor renders bright" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, bright) != null);
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

test "rule is a faint full-width line ending in a newline" {
    const out = try rule(testing.allocator, 12);
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "\u{2500}") != null);
    try testing.expect(std.mem.endsWith(u8, out, "\n"));
}
