//! Renders a segment list to an ANSI prompt string. Every escape sequence is
//! wrapped in the shell's zero-width markers (`%{ %}` for zsh, `\001 \002` for
//! bash) — without that the shell miscounts the prompt's visible width and
//! typed input lands in the wrong column.

const std = @import("std");
const seg = @import("segments.zig");
const icons = @import("icons.zig");

pub const Shell = enum { plain, zsh, bash };

const reset = "\x1b[0m";
const edge = "\u{258e}"; // ▎

const Palette = struct {
    accent: []const u8,
    accent_err: []const u8,
    bright: []const u8,
    dim: []const u8,
    rule_color: []const u8,
    git: []const u8,
    tool: []const u8,
    infra: []const u8,
    warn: []const u8,
    ok: []const u8,
};

const dark_palette: Palette = .{
    .accent = "\x1b[38;2;84;183;192m", // #54b7c0 mineral
    .accent_err = "\x1b[38;2;212;74;63m", // #d44a3f
    .bright = "\x1b[38;2;216;219;226m", // #d8dbe2 anchor
    .dim = "\x1b[38;2;125;135;145m", // #7d8791
    .rule_color = "\x1b[38;2;74;80;96m", // #4a5060
    .git = "\x1b[38;2;127;176;135m", // #7fb087
    .tool = "\x1b[38;2;157;146;207m", // #9d92cf
    .infra = "\x1b[38;2;111;159;196m", // #6f9fc4
    .warn = "\x1b[38;2;199;154;62m", // #c79a3e
    .ok = "\x1b[38;2;90;168;115m", // #5aa873
};

const light_palette: Palette = .{
    .accent = "\x1b[38;2;44;122;130m", // #2c7a82 mineral
    .accent_err = "\x1b[38;2;181;68;58m", // #b5443a
    .bright = "\x1b[38;2;27;31;36m", // #1b1f24 ink — the strong anchor on light
    .dim = "\x1b[38;2;93;102;113m", // #5d6671 readable mid grey
    .rule_color = "\x1b[38;2;205;211;213m", // #cdd3d5 faint light separator
    .git = "\x1b[38;2;60;125;84m", // #3c7d54
    .tool = "\x1b[38;2;98;85;143m", // #62558f
    .infra = "\x1b[38;2;63;108;149m", // #3f6c95
    .warn = "\x1b[38;2;148;100;16m", // #946410
    .ok = "\x1b[38;2;60;125;84m", // #3c7d54
};

/// A segment's colour: an attention state (dirty / failed) wins; otherwise the
/// colour is keyed to the segment's type.
fn segColor(p: *const Palette, s: seg.Segment) []const u8 {
    switch (s.state) {
        .warn => return p.warn,
        .err => return p.accent_err,
        .ok => return p.ok,
        .run => return p.accent,
        .normal => {},
    }
    return switch (s.icon) {
        .repo => p.bright,
        .branch => p.git,
        .toolchain => p.tool,
        .container, .cluster => p.infra,
        else => p.dim,
    };
}

pub const Options = struct {
    rich: bool,
    failed: bool, // last command exited non-zero
    width: usize = 0, // terminal columns, for the separator rule
    shell: Shell = .plain,
    light: bool = false, // use light palette instead of dark
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
    return ruleWithPalette(a, width, shell, &dark_palette);
}

fn ruleWithPalette(a: std.mem.Allocator, width: usize, shell: Shell, p: *const Palette) ![]u8 {
    const w: usize = if (width <= 1) 79 else @min(width - 1, 511);
    var buf: Buf = .empty;
    errdefer buf.deinit(a);
    try esc(&buf, a, shell, p.rule_color);
    var i: usize = 0;
    while (i < w) : (i += 1) try buf.appendSlice(a, "\u{2500}");
    try esc(&buf, a, shell, reset);
    try buf.appendSlice(a, "\n");
    return buf.toOwnedSlice(a);
}

/// The full two-line prompt block, with the separator rule as its first line.
pub fn full(a: std.mem.Allocator, segments: []const seg.Segment, opts: Options) ![]u8 {
    const p: *const Palette = if (opts.light) &light_palette else &dark_palette;
    var buf: Buf = .empty;
    errdefer buf.deinit(a);
    const sh = opts.shell;

    const r = try ruleWithPalette(a, opts.width, sh, p);
    defer a.free(r);
    try buf.appendSlice(a, r);

    const edge_color = if (opts.failed) p.accent_err else p.accent;
    // Line 1: edge + segments. The cwd anchor is bright; the rest take their
    // state colour.
    try esc(&buf, a, sh, edge_color);
    try buf.appendSlice(a, edge);
    try esc(&buf, a, sh, reset);
    try buf.appendSlice(a, "  ");
    for (segments, 0..) |s, idx| {
        if (idx != 0) try buf.appendSlice(a, "   ");
        try esc(&buf, a, sh, segColor(p, s));
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
    const p: *const Palette = if (opts.light) &light_palette else &dark_palette;
    var buf: Buf = .empty;
    errdefer buf.deinit(a);
    const col = if (opts.failed) p.accent_err else p.dim;
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

test "full uses the failure colour when the last command failed" {
    const s = sampleSegs();
    const ok = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(ok);
    const bad = try full(testing.allocator, &s, .{ .rich = true, .failed = true });
    defer testing.allocator.free(bad);
    try testing.expect(std.mem.indexOf(u8, bad, dark_palette.accent_err) != null);
    try testing.expect(std.mem.indexOf(u8, ok, dark_palette.accent_err) == null);
}

test "the cwd anchor renders bright (dark palette)" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, dark_palette.bright) != null);
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

test "light palette: full uses light bright and not dark bright" {
    const s = sampleSegs();
    const out = try full(testing.allocator, &s, .{ .rich = true, .failed = false, .light = true });
    defer testing.allocator.free(out);
    try testing.expect(std.mem.indexOf(u8, out, light_palette.bright) != null);
    try testing.expect(std.mem.indexOf(u8, out, dark_palette.bright) == null);
}
