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
    theme_variant: [32]u8 = variantDefault(),
    theme_variant_len: usize = "mineral".len,
    font_size: f32 = 13,
    padding_x: f32 = 8,
    padding_y: f32 = 6,
    cursor_style: CursorStyle = .block,
    cursor_blink: bool = true,
    cursor_smooth: bool = true,
    scroll_smooth: bool = true,
    background_opacity: f32 = 1.0,

    pub fn themeVariant(self: *const Config) []const u8 {
        return self.theme_variant[0..self.theme_variant_len];
    }
};

fn variantDefault() [32]u8 {
    var buf = [_]u8{0} ** 32;
    @memcpy(buf[0.."mineral".len], "mineral");
    return buf;
}

pub const ParseResult = struct {
    cfg: Config,
    err: [128]u8 = undefined,
    err_len: usize = 0,

    pub fn errMsg(self: *const ParseResult) []const u8 {
        return self.err[0..self.err_len];
    }
};

/// Parse config text, collecting the first error into the result.
/// Unknown keys, bad enum values, unparseable floats, and out-of-range values
/// are all errors when the key is present. Missing file is not an error.
pub fn parseFull(text: []const u8) ParseResult {
    var result = ParseResult{ .cfg = Config{} };
    var lines = std.mem.splitScalar(u8, text, '\n');
    var line_num: usize = 0;
    while (lines.next()) |raw| {
        line_num += 1;
        const line = trim(stripComment(raw));
        if (line.len == 0 or line[0] == '[') continue;
        const eq = std.mem.indexOfScalar(u8, line, '=') orelse continue;
        const key = trim(line[0..eq]);
        const val = unquote(trim(line[eq + 1 ..]));
        if (result.err_len == 0) {
            applyKey(&result.cfg, key, val, &result.err, &result.err_len, line_num);
        } else {
            applyKey(&result.cfg, key, val, null, null, line_num);
        }
    }
    return result;
}

/// Parse config text. Always succeeds; unrecognized lines are skipped.
/// Preserved for compatibility with existing tests and callers that don't need errors.
pub fn parse(text: []const u8) Config {
    return parseFull(text).cfg;
}

fn applyKey(cfg: *Config, key: []const u8, val: []const u8, err: ?*[128]u8, err_len: ?*usize, line_num: usize) void {
    if (std.mem.eql(u8, key, "theme")) {
        if (std.mem.eql(u8, val, "system")) {
            cfg.theme = .system;
            return;
        }
        if (std.mem.eql(u8, val, "light")) {
            cfg.theme = .light;
            return;
        }
        if (std.mem.eql(u8, val, "dark")) {
            cfg.theme = .dark;
            return;
        }
        setErr(err, err_len, "config.toml: invalid theme value '{s}' (line {})", .{ val, line_num });
    } else if (std.mem.eql(u8, key, "font_size")) {
        if (std.fmt.parseFloat(f32, val)) |v| {
            if (v >= 6 and v <= 72) {
                cfg.font_size = v;
                return;
            }
            setErr(err, err_len, "config.toml: font_size {s} out of range 6-72 (line {})", .{ val, line_num });
        } else |_| {
            setErr(err, err_len, "config.toml: invalid font_size '{s}' (line {})", .{ val, line_num });
        }
    } else if (std.mem.eql(u8, key, "padding_x")) {
        if (std.fmt.parseFloat(f32, val)) |v| {
            if (v >= 0 and v <= 200) {
                cfg.padding_x = v;
                return;
            }
            setErr(err, err_len, "config.toml: padding_x {s} out of range 0-200 (line {})", .{ val, line_num });
        } else |_| {
            setErr(err, err_len, "config.toml: invalid padding_x '{s}' (line {})", .{ val, line_num });
        }
    } else if (std.mem.eql(u8, key, "padding_y")) {
        if (std.fmt.parseFloat(f32, val)) |v| {
            if (v >= 0 and v <= 200) {
                cfg.padding_y = v;
                return;
            }
            setErr(err, err_len, "config.toml: padding_y {s} out of range 0-200 (line {})", .{ val, line_num });
        } else |_| {
            setErr(err, err_len, "config.toml: invalid padding_y '{s}' (line {})", .{ val, line_num });
        }
    } else if (std.mem.eql(u8, key, "cursor_style")) {
        if (std.mem.eql(u8, val, "block")) {
            cfg.cursor_style = .block;
            return;
        }
        if (std.mem.eql(u8, val, "underline")) {
            cfg.cursor_style = .underline;
            return;
        }
        if (std.mem.eql(u8, val, "bar")) {
            cfg.cursor_style = .bar;
            return;
        }
        setErr(err, err_len, "config.toml: invalid cursor_style '{s}' (line {})", .{ val, line_num });
    } else if (std.mem.eql(u8, key, "cursor_blink")) {
        if (std.mem.eql(u8, val, "true")) {
            cfg.cursor_blink = true;
            return;
        }
        if (std.mem.eql(u8, val, "false")) {
            cfg.cursor_blink = false;
            return;
        }
        setErr(err, err_len, "config.toml: invalid cursor_blink '{s}' (line {})", .{ val, line_num });
    } else if (std.mem.eql(u8, key, "cursor_smooth")) {
        if (std.mem.eql(u8, val, "true")) {
            cfg.cursor_smooth = true;
            return;
        }
        if (std.mem.eql(u8, val, "false")) {
            cfg.cursor_smooth = false;
            return;
        }
        setErr(err, err_len, "config.toml: invalid cursor_smooth '{s}' (line {})", .{ val, line_num });
    } else if (std.mem.eql(u8, key, "scroll_smooth")) {
        if (std.mem.eql(u8, val, "true")) {
            cfg.scroll_smooth = true;
            return;
        }
        if (std.mem.eql(u8, val, "false")) {
            cfg.scroll_smooth = false;
            return;
        }
        setErr(err, err_len, "config.toml: invalid scroll_smooth '{s}' (line {})", .{ val, line_num });
    } else if (std.mem.eql(u8, key, "background_opacity")) {
        if (std.fmt.parseFloat(f32, val)) |v| {
            if (v >= 0.0 and v <= 1.0) {
                cfg.background_opacity = v;
                return;
            }
            setErr(err, err_len, "config.toml: background_opacity {s} out of range 0.0-1.0 (line {})", .{ val, line_num });
        } else |_| {
            setErr(err, err_len, "config.toml: invalid background_opacity '{s}' (line {})", .{ val, line_num });
        }
    } else if (std.mem.eql(u8, key, "theme_variant")) {
        const n = @min(val.len, cfg.theme_variant.len);
        @memcpy(cfg.theme_variant[0..n], val[0..n]);
        cfg.theme_variant_len = n;
    } else {
        setErr(err, err_len, "config.toml: unknown key '{s}' (line {})", .{ key, line_num });
    }
}

fn setErr(err: ?*[128]u8, err_len: ?*usize, comptime fmt: []const u8, args: anytype) void {
    const buf = err orelse return;
    const len = err_len orelse return;
    const s = std.fmt.bufPrint(buf, fmt, args) catch buf[0..buf.len];
    len.* = s.len;
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

/// Read and parse the config at `path`. Returns defaults + empty error if file
/// is missing. Returns parsed config + first error if file exists but is invalid.
pub fn loadFull(path: [:0]const u8) ParseResult {
    const f = c.fopen(path.ptr, "rb") orelse return .{ .cfg = .{} };
    defer _ = c.fclose(f);
    var buf: [1 << 16]u8 = undefined;
    const n = c.fread(&buf, 1, buf.len, f);
    return parseFull(buf[0..n]);
}

/// Convenience wrapper; discards error information.
pub fn load(path: [:0]const u8) Config {
    return loadFull(path).cfg;
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

test "parse reads cursor_smooth; defaults true" {
    const cfg_default = parse("theme = dark\n");
    try std.testing.expect(cfg_default.cursor_smooth);
    const cfg_off = parse("cursor_smooth = false\n");
    try std.testing.expect(!cfg_off.cursor_smooth);
    const cfg_on = parse("cursor_smooth = true\n");
    try std.testing.expect(cfg_on.cursor_smooth);
}

test "parse reads scroll_smooth; defaults true" {
    const cfg_default = parse("theme = dark\n");
    try std.testing.expect(cfg_default.scroll_smooth);
    const cfg_off = parse("scroll_smooth = false\n");
    try std.testing.expect(!cfg_off.scroll_smooth);
    const cfg_on = parse("scroll_smooth = true\n");
    try std.testing.expect(cfg_on.scroll_smooth);
}

test "parse rejects out-of-range font size" {
    const cfg = parse("font_size = 999");
    try std.testing.expectEqual(@as(f32, 13), cfg.font_size); // default kept
}

test "parse falls back to defaults on junk" {
    const cfg = parse("this is not valid\n=oops\n");
    try std.testing.expectEqual(ThemeMode.system, cfg.theme);
}

test "parseFull: bad value yields non-empty error" {
    const r = parseFull("theme = \"darkk\"\n");
    try std.testing.expect(r.err_len > 0);
    try std.testing.expect(std.mem.indexOf(u8, r.errMsg(), "darkk") != null);
}

test "parseFull: unknown key yields non-empty error" {
    const r = parseFull("unknown_key = foo\n");
    try std.testing.expect(r.err_len > 0);
    try std.testing.expect(std.mem.indexOf(u8, r.errMsg(), "unknown_key") != null);
}

test "parseFull: valid config yields empty error" {
    const r = parseFull("theme = dark\nfont_size = 14\n");
    try std.testing.expectEqual(@as(usize, 0), r.err_len);
}

test "parseFull: missing file is not an error (load returns empty error)" {
    const r = parseFull("");
    try std.testing.expectEqual(@as(usize, 0), r.err_len);
}

test "parseFull: out-of-range font_size yields non-empty error" {
    const r = parseFull("font_size = 999\n");
    try std.testing.expect(r.err_len > 0);
    try std.testing.expect(std.mem.indexOf(u8, r.errMsg(), "font_size") != null);
}

test "parse reads theme_variant" {
    const cfg = parse("theme_variant = \"mineral-high\"\n");
    try std.testing.expectEqualStrings("mineral-high", cfg.themeVariant());
}

test "parse theme_variant defaults to mineral" {
    const cfg = parse("theme = dark\n");
    try std.testing.expectEqualStrings("mineral", cfg.themeVariant());
}

test "parse theme_variant unknown value is stored verbatim (caller resolves)" {
    const cfg = parse("theme_variant = \"unknown-variant\"\n");
    try std.testing.expectEqualStrings("unknown-variant", cfg.themeVariant());
}

test "parseFull: theme_variant does not yield an error" {
    const r = parseFull("theme_variant = mineral-high\n");
    try std.testing.expectEqual(@as(usize, 0), r.err_len);
    try std.testing.expectEqualStrings("mineral-high", r.cfg.themeVariant());
}

test "parse reads background_opacity; defaults 1.0" {
    const def = parse("theme = dark\n");
    try std.testing.expectEqual(@as(f32, 1.0), def.background_opacity);
    const cfg = parse("background_opacity = 0.85\n");
    try std.testing.expectEqual(@as(f32, 0.85), cfg.background_opacity);
}

test "parseFull: background_opacity out of range yields error" {
    const r = parseFull("background_opacity = 1.5\n");
    try std.testing.expect(r.err_len > 0);
    try std.testing.expect(std.mem.indexOf(u8, r.errMsg(), "background_opacity") != null);
}
