//! User configuration, loaded from ~/.config/caldera-console/config.zon.
//!
//! The file is ZON (Zig Object Notation). Parsing allocates strings into an
//! arena owned by the returned `Loaded` value — `std.zon.parse.free` cannot be
//! used here because absent fields keep struct-default string *literals*, and
//! freeing static memory is illegal. `Loaded.deinit` frees the arena.

const std = @import("std");

pub const CursorStyle = enum { block, bar, underline };

/// Optional per-color overrides applied on top of the chosen base theme.
/// Every field is optional; an absent field keeps the base theme's value.
pub const Overrides = struct {
    background: ?[]const u8 = null,
    foreground: ?[]const u8 = null,
    accent: ?[]const u8 = null,
    ansi: Ansi = .{},

    pub const Ansi = struct {
        black: ?[]const u8 = null,
        red: ?[]const u8 = null,
        green: ?[]const u8 = null,
        yellow: ?[]const u8 = null,
        blue: ?[]const u8 = null,
        magenta: ?[]const u8 = null,
        cyan: ?[]const u8 = null,
        white: ?[]const u8 = null,
        bright_black: ?[]const u8 = null,
        bright_red: ?[]const u8 = null,
        bright_green: ?[]const u8 = null,
        bright_yellow: ?[]const u8 = null,
        bright_blue: ?[]const u8 = null,
        bright_magenta: ?[]const u8 = null,
        bright_cyan: ?[]const u8 = null,
        bright_white: ?[]const u8 = null,
    };
};

pub const Config = struct {
    scrollback: usize = 100_000,
    font: FontCfg = .{},
    cursor: CursorCfg = .{},
    window: WindowCfg = .{},
    theme: []const u8 = "mineral-dark",
    theme_overrides: Overrides = .{},

    pub const FontCfg = struct {
        family: []const u8 = "IBM Plex Mono",
        size: f64 = 14.0,
    };
    pub const CursorCfg = struct {
        style: CursorStyle = .block,
        blink: bool = true,
    };
    pub const WindowCfg = struct {
        width: f64 = 1024.0,
        height: f64 = 640.0,
    };

    /// Pull every out-of-range value back to a usable minimum.
    fn clamp(self: *Config) void {
        if (self.scrollback < 1) self.scrollback = 1;
        if (!(self.font.size >= 4.0)) self.font.size = 14.0; // also catches NaN
        if (!(self.window.width >= 200.0)) self.window.width = 1024.0;
        if (!(self.window.height >= 150.0)) self.window.height = 640.0;
    }
};

/// A parsed config plus the arena that owns its strings. Always `deinit` it.
pub const Loaded = struct {
    arena: std.heap.ArenaAllocator,
    config: Config,

    pub fn deinit(self: *Loaded) void {
        self.arena.deinit();
    }
};

/// Defaults with an empty arena — used for a missing file or a parse failure.
pub fn defaults(backing: std.mem.Allocator) Loaded {
    return .{ .arena = std.heap.ArenaAllocator.init(backing), .config = .{} };
}

/// Parse a ZON source string into a `Loaded`. On any ZON error the error text
/// is printed to stderr and `error.ParseFailed` is returned (the caller then
/// falls back to `defaults`).
pub fn parseSlice(backing: std.mem.Allocator, source: [:0]const u8) error{ParseFailed}!Loaded {
    var arena = std.heap.ArenaAllocator.init(backing);
    errdefer arena.deinit();
    const a = arena.allocator();

    var diag: std.zon.parse.Diagnostics = .{};
    var cfg = std.zon.parse.fromSliceAlloc(Config, a, source, &diag, .{
        .free_on_error = false, // the arena cleans up wholesale
    }) catch {
        std.debug.print("caldera-console: config parse error:\n{f}", .{diag});
        return error.ParseFailed;
    };
    cfg.clamp();
    return .{ .arena = arena, .config = cfg };
}

const testing = std.testing;

test "parses a full config" {
    const src =
        \\.{
        \\    .scrollback = 5000,
        \\    .font = .{ .family = "Menlo", .size = 16.0 },
        \\    .cursor = .{ .style = .bar, .blink = false },
        \\    .window = .{ .width = 800.0, .height = 600.0 },
        \\    .theme = "mineral-light",
        \\    .theme_overrides = .{ .accent = "#3aa0a8" },
        \\}
    ;
    var loaded = try parseSlice(testing.allocator, src);
    defer loaded.deinit();
    const c = loaded.config;
    try testing.expectEqual(@as(usize, 5000), c.scrollback);
    try testing.expectEqualStrings("Menlo", c.font.family);
    try testing.expectEqual(@as(f64, 16.0), c.font.size);
    try testing.expectEqual(CursorStyle.bar, c.cursor.style);
    try testing.expectEqual(false, c.cursor.blink);
    try testing.expectEqualStrings("mineral-light", c.theme);
    try testing.expectEqualStrings("#3aa0a8", c.theme_overrides.accent.?);
}

test "a partial config keeps defaults for absent fields" {
    var loaded = try parseSlice(testing.allocator, ".{ .scrollback = 200 }");
    defer loaded.deinit();
    try testing.expectEqual(@as(usize, 200), loaded.config.scrollback);
    try testing.expectEqualStrings("IBM Plex Mono", loaded.config.font.family);
    try testing.expectEqual(CursorStyle.block, loaded.config.cursor.style);
    try testing.expectEqualStrings("mineral-dark", loaded.config.theme);
}

test "malformed ZON returns ParseFailed" {
    try testing.expectError(error.ParseFailed, parseSlice(testing.allocator, ".{ .scrollback ="));
    try testing.expectError(error.ParseFailed, parseSlice(testing.allocator, ".{ .nonsense = 1 }"));
}

test "out-of-range values are clamped" {
    var loaded = try parseSlice(testing.allocator,
        ".{ .scrollback = 0, .font = .{ .size = 0.0 }, .window = .{ .width = 1.0, .height = 1.0 } }");
    defer loaded.deinit();
    try testing.expectEqual(@as(usize, 1), loaded.config.scrollback);
    try testing.expectEqual(@as(f64, 14.0), loaded.config.font.size);
    try testing.expectEqual(@as(f64, 1024.0), loaded.config.window.width);
    try testing.expectEqual(@as(f64, 640.0), loaded.config.window.height);
}

test "defaults has an empty config" {
    var loaded = defaults(testing.allocator);
    defer loaded.deinit();
    try testing.expectEqual(@as(usize, 100_000), loaded.config.scrollback);
}
