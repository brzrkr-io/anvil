const std = @import("std");

// std.fs file I/O is mid-migration in Zig 0.16; use libc like the rest of the
// app (see pty.zig).
const c = @cImport({
    @cInclude("stdio.h");
    @cInclude("sys/stat.h");
});

/// User configuration loaded from `$HOME/.config/anvil/config.toml`. Only a
/// flat subset of TOML is understood: `key = value` lines, `#` comments, and
/// double-quoted or bare values. Unknown keys and `[tables]` are ignored, so
/// the file can grow without breaking older builds. Missing file or parse
/// errors fall back to defaults — a bad config never stops the app.
pub const ThemeMode = enum { system, light, dark };
pub const CursorStyle = enum { block, underline, bar };

pub const Config = struct {
    theme: ThemeMode = .system,
    font_size: f32 = 13,
    padding_x: f32 = 8,
    padding_y: f32 = 6,
    cursor_style: CursorStyle = .block,
    cursor_blink: bool = true,
};

/// Parse config text. Always succeeds; unrecognized lines are skipped.
pub fn parse(text: []const u8) Config {
    var cfg = Config{};
    var lines = std.mem.splitScalar(u8, text, '\n');
    while (lines.next()) |raw| {
        const line = trim(stripComment(raw));
        if (line.len == 0 or line[0] == '[') continue;
        const eq = std.mem.indexOfScalar(u8, line, '=') orelse continue;
        const key = trim(line[0..eq]);
        const val = unquote(trim(line[eq + 1 ..]));
        applyKey(&cfg, key, val);
    }
    return cfg;
}

fn applyKey(cfg: *Config, key: []const u8, val: []const u8) void {
    if (std.mem.eql(u8, key, "theme")) {
        if (std.mem.eql(u8, val, "system")) cfg.theme = .system;
        if (std.mem.eql(u8, val, "light")) cfg.theme = .light;
        if (std.mem.eql(u8, val, "dark")) cfg.theme = .dark;
    } else if (std.mem.eql(u8, key, "font_size")) {
        if (std.fmt.parseFloat(f32, val)) |v| {
            if (v >= 6 and v <= 72) cfg.font_size = v;
        } else |_| {}
    } else if (std.mem.eql(u8, key, "padding_x")) {
        if (std.fmt.parseFloat(f32, val)) |v| {
            if (v >= 0 and v <= 200) cfg.padding_x = v;
        } else |_| {}
    } else if (std.mem.eql(u8, key, "padding_y")) {
        if (std.fmt.parseFloat(f32, val)) |v| {
            if (v >= 0 and v <= 200) cfg.padding_y = v;
        } else |_| {}
    } else if (std.mem.eql(u8, key, "cursor_style")) {
        if (std.mem.eql(u8, val, "block")) cfg.cursor_style = .block;
        if (std.mem.eql(u8, val, "underline")) cfg.cursor_style = .underline;
        if (std.mem.eql(u8, val, "bar")) cfg.cursor_style = .bar;
    } else if (std.mem.eql(u8, key, "cursor_blink")) {
        if (std.mem.eql(u8, val, "true")) cfg.cursor_blink = true;
        if (std.mem.eql(u8, val, "false")) cfg.cursor_blink = false;
    }
}

fn stripComment(line: []const u8) []const u8 {
    if (std.mem.indexOfScalar(u8, line, '#')) |h| return line[0..h];
    return line;
}

fn trim(s: []const u8) []const u8 {
    return std.mem.trim(u8, s, " \t\r");
}

fn unquote(s: []const u8) []const u8 {
    if (s.len >= 2 and s[0] == '"' and s[s.len - 1] == '"') return s[1 .. s.len - 1];
    return s;
}

/// Read and parse the config at `path`. Returns defaults if the file is
/// missing or unreadable. Config files are tiny; a 64 KiB read cap is plenty.
pub fn load(path: [:0]const u8) Config {
    const f = c.fopen(path.ptr, "rb") orelse return .{};
    defer _ = c.fclose(f);
    var buf: [1 << 16]u8 = undefined;
    const n = c.fread(&buf, 1, buf.len, f);
    return parse(buf[0..n]);
}

/// Last-modified time of `path` in nanoseconds, or null if it can't be stat'd.
/// Used to detect edits for live reload without re-reading the file.
pub fn mtime(path: [:0]const u8) ?i128 {
    var st: c.struct_stat = undefined;
    if (c.stat(path.ptr, &st) != 0) return null;
    const ts = st.st_mtimespec;
    return @as(i128, ts.tv_sec) * std.time.ns_per_s + ts.tv_nsec;
}

test "parse reads theme, ignoring comments and unknown keys" {
    const cfg = parse(
        \\# Anvil config
        \\theme = "dark"
        \\font_size = 14   # not yet supported
        \\[future]
        \\something = true
    );
    try std.testing.expectEqual(ThemeMode.dark, cfg.theme);
}

test "parse tolerates bare values and whitespace" {
    const cfg = parse("  theme=light  ");
    try std.testing.expectEqual(ThemeMode.light, cfg.theme);
}

test "parse reads font size, padding, and cursor options" {
    const cfg = parse(
        \\font_size = 15.5
        \\padding_x = 12
        \\padding_y = 4
        \\cursor_style = "bar"
        \\cursor_blink = false
    );
    try std.testing.expectEqual(@as(f32, 15.5), cfg.font_size);
    try std.testing.expectEqual(@as(f32, 12), cfg.padding_x);
    try std.testing.expectEqual(@as(f32, 4), cfg.padding_y);
    try std.testing.expectEqual(CursorStyle.bar, cfg.cursor_style);
    try std.testing.expect(!cfg.cursor_blink);
}

test "parse rejects out-of-range font size" {
    const cfg = parse("font_size = 999");
    try std.testing.expectEqual(@as(f32, 13), cfg.font_size); // default kept
}

test "parse falls back to defaults on junk" {
    const cfg = parse("this is not valid\n=oops\n");
    try std.testing.expectEqual(ThemeMode.system, cfg.theme);
}
