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

pub const Config = struct {
    theme: ThemeMode = .system,
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

test "parse falls back to defaults on junk" {
    const cfg = parse("this is not valid\n=oops\n");
    try std.testing.expectEqual(ThemeMode.system, cfg.theme);
}
