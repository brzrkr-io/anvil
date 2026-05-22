//! Renders a segment list to an ANSI prompt string. Every escape sequence is
//! wrapped in the shell's zero-width markers (`%{ %}` for zsh, `\001 \002` for
//! bash) — without that the shell miscounts the prompt's visible width and
//! typed input lands in the wrong column.
//!
//! Colors are emitted as indexed ANSI colors (`\x1b[38;5;Nm`) so the terminal
//! re-resolves them through the active theme palette on every frame. A theme
//! switch therefore recolors all prompts in scrollback automatically.

const std = @import("std");
const seg = @import("segments.zig");
const icons = @import("icons.zig");

pub const Shell = enum { plain, zsh, bash };

const reset = "\x1b[0m";
const edge = "\u{258e}"; // ▎

// Indexed ANSI colors — resolved through the active theme each frame.
const anchor = "\x1b[39m"; // default foreground — cwd anchor; flips with theme
const accent = "\x1b[38;5;6m"; // ANSI 6 = mineral/cyan — edge glyph, prompt glyph
const accent_err = "\x1b[38;5;1m"; // ANSI 1 = red — error state
const dim = "\x1b[38;5;8m"; // ANSI 8 = readable dim grey
const git_color = "\x1b[38;5;2m"; // ANSI 2 = green
const tool_color = "\x1b[38;5;5m"; // ANSI 5 = magenta — toolchain
const infra_color = "\x1b[38;5;4m"; // ANSI 4 = blue — container/cluster
const warn_color = "\x1b[38;5;3m"; // ANSI 3 = yellow/amber — attention
const ok_color = "\x1b[38;5;2m"; // ANSI 2 = green

/// A segment's colour: an attention state (dirty / failed) wins; otherwise the
/// colour is keyed to the segment's type.
fn segColor(s: seg.Segment) []const u8 {
    switch (s.state) {
        .warn => return warn_color,
        .err => return accent_err,
        .ok => return ok_color,
        .run => return accent,
        .normal => {},
    }
    return switch (s.icon) {
        .repo => anchor,
        .branch => git_color,
        .toolchain => tool_color,
        .container, .cluster => infra_color,
        else => dim,
    };
}

pub const Options = struct {
    rich: bool,
    failed: bool, // last command exited non-zero
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

/// The full two-line prompt block. The separator rule is now drawn by the
/// renderer from OSC 133;A marks, so it is always full-width and theme-colored.
pub fn full(a: std.mem.Allocator, segments: []const seg.Segment, opts: Options) ![]u8 {
    var buf: Buf = .empty;
    errdefer buf.deinit(a);
    const sh = opts.shell;

    const edge_color = if (opts.failed) accent_err else accent;
    // Line 1: edge + segments. The cwd anchor uses the default fg; others take
    // their state colour.
    try esc(&buf, a, sh, edge_color);
    try buf.appendSlice(a, edge);
    try esc(&buf, a, sh, reset);
    try buf.appendSlice(a, "  ");
    for (segments, 0..) |s, idx| {
        if (idx != 0) try buf.appendSlice(a, "   ");
        try esc(&buf, a, sh, segColor(s));
        // In rich mode the segment leads with its icon glyph (segment colour).
        if (opts.rich) {
            try buf.appendSlice(a, icons.glyph(s.icon, true));
            try buf.appendSlice(a, " ");
        }
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
    const col = if (opts.failed) accent_err else dim;
    try esc(&buf, a, opts.shell, col);
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

test "full uses indexed accent_err colour on failure" {
    const s = sampleSegs();
    const ok = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(ok);
    const bad = try full(testing.allocator, &s, .{ .rich = true, .failed = true });
    defer testing.allocator.free(bad);
    try testing.expect(std.mem.indexOf(u8, bad, accent_err) != null);
    try testing.expect(std.mem.indexOf(u8, ok, accent_err) == null);
}

test "the cwd anchor uses the default-fg indexed color" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, anchor) != null);
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

test "full does not contain a rule line (renderer draws the separator)" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    // The box-drawing horizontal bar character must not appear in the prompt text.
    try testing.expect(std.mem.indexOf(u8, out, "\u{2500}") == null);
}

test "zsh mode wraps escape sequences in zero-width markers" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false, .shell = .zsh });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, "%{") != null);
    try testing.expect(std.mem.indexOf(u8, out, "%}") != null);
}

test "transient uses indexed dim colour when not failed" {
    const out = try transient(testing.allocator, .{ .rich = false, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, dim) != null);
}

test "transient uses indexed accent_err colour when failed" {
    const out = try transient(testing.allocator, .{ .rich = false, .failed = true });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, accent_err) != null);
}
