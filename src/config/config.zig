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
    keybindings: Keybindings = .{},

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

/// Write the absolute config path into `buf`. Returns the slice, or null when
/// `$HOME` is unset (then the app simply runs on defaults).
pub fn resolvePath(buf: []u8) ?[]const u8 {
    const home_ptr = std.c.getenv("HOME") orelse return null;
    const home = std.mem.sliceTo(home_ptr, 0);
    return std.fmt.bufPrint(buf, "{s}/.config/caldera-console/config.zon", .{home}) catch null;
}

/// Read and parse the config file at `path`. A missing file or any read/parse
/// error yields `defaults` — running the app is never blocked by a bad config.
pub fn load(backing: std.mem.Allocator, path: []const u8) Loaded {
    // Need a null-terminated path for std.c.open.
    var path_z_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
    if (path.len >= path_z_buf.len) return defaults(backing);
    @memcpy(path_z_buf[0..path.len], path);
    path_z_buf[path.len] = 0;
    const path_z: [*:0]const u8 = path_z_buf[0..path.len :0];

    const fd = std.c.open(path_z, .{}, @as(c_uint, 0));
    if (fd < 0) {
        const e = std.posix.errno(fd);
        if (e != .NOENT)
            std.debug.print("caldera-console: cannot open config: {s}\n", .{@tagName(e)});
        return defaults(backing);
    }
    defer _ = std.c.close(fd);

    var buf: [1 << 20]u8 = undefined;
    var total: usize = 0;
    while (total < buf.len) {
        const n = std.c.read(fd, buf[total..].ptr, buf.len - total);
        if (n < 0) return defaults(backing);
        if (n == 0) break;
        total += @intCast(n);
    }
    if (total >= buf.len) {
        std.debug.print("caldera-console: config file too large, using defaults\n", .{});
        return defaults(backing);
    }
    buf[total] = 0;
    return parseSlice(backing, buf[0..total :0]) catch defaults(backing);
}

/// Polls the config file's modification time so the render loop can reload it
/// without a file-watcher thread. Cheap: one `stat` per poll.
pub const Watcher = struct {
    path: []const u8, // borrowed; must outlive the Watcher
    last_mtime: i128 = 0, // 0 = nothing seen yet (or no file)

    pub fn init(path: []const u8) Watcher {
        return .{ .path = path };
    }

    /// Current mtime of the file in nanoseconds, or 0 if it cannot be stat'd.
    fn mtime(self: *const Watcher) i128 {
        var path_z_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
        if (self.path.len >= path_z_buf.len) return 0;
        @memcpy(path_z_buf[0..self.path.len], self.path);
        path_z_buf[self.path.len] = 0;
        const path_z: [*:0]const u8 = path_z_buf[0..self.path.len :0];

        const fd = std.c.open(path_z, .{}, @as(c_uint, 0));
        if (fd < 0) return 0;
        defer _ = std.c.close(fd);

        var st: std.c.Stat = undefined;
        if (std.c.fstat(fd, &st) != 0) return 0;
        const ts = st.mtime();
        return @as(i128, ts.sec) * 1_000_000_000 + @as(i128, ts.nsec);
    }

    /// If the file changed since the last call, load and return the new config;
    /// otherwise return null. A parse failure still advances the recorded mtime
    /// so the error is reported once, not every poll.
    pub fn poll(self: *Watcher, backing: std.mem.Allocator) ?Loaded {
        const m = self.mtime();
        if (m == self.last_mtime) return null;
        self.last_mtime = m;
        if (m == 0) return defaults(backing); // file was removed -> defaults
        return load(backing, self.path);
    }
};

/// A parsed key chord: modifier flags plus one key codepoint (lowercased for
/// ASCII letters). The default tab shortcuts all use single-character keys.
pub const Chord = struct {
    cmd: bool = false,
    shift: bool = false,
    ctrl: bool = false,
    opt: bool = false,
    key: u21,
};

/// Parse a chord string like "cmd+shift+]" or "cmd+t". Modifier tokens are
/// cmd/shift/ctrl/opt (case-insensitive); the final token is a single key
/// character. Returns null on a malformed string.
pub fn parseChord(s: []const u8) ?Chord {
    var ch: Chord = .{ .key = 0 };
    var have_key = false;
    var it = std.mem.splitScalar(u8, s, '+');
    while (it.next()) |tok_raw| {
        const tok = std.mem.trim(u8, tok_raw, " ");
        if (tok.len == 0) return null;
        if (eqIgnoreCase(tok, "cmd")) {
            ch.cmd = true;
        } else if (eqIgnoreCase(tok, "shift")) {
            ch.shift = true;
        } else if (eqIgnoreCase(tok, "ctrl")) {
            ch.ctrl = true;
        } else if (eqIgnoreCase(tok, "opt")) {
            ch.opt = true;
        } else {
            // Must be the key — exactly one ASCII character, and last.
            if (have_key or tok.len != 1) return null;
            ch.key = std.ascii.toLower(tok[0]);
            have_key = true;
        }
    }
    if (!have_key) return null;
    return ch;
}

fn eqIgnoreCase(a: []const u8, b: []const u8) bool {
    return std.ascii.eqlIgnoreCase(a, b);
}

/// Chord strings for tab actions. Live-reloadable. Each is parsed via
/// `parseChord`; an unparseable string falls back to that field's default.
pub const Keybindings = struct {
    new_tab: []const u8 = "cmd+t",
    close_tab: []const u8 = "cmd+w",
    next_tab: []const u8 = "cmd+shift+]",
    prev_tab: []const u8 = "cmd+shift+[",
    tab_1: []const u8 = "cmd+1",
    tab_2: []const u8 = "cmd+2",
    tab_3: []const u8 = "cmd+3",
    tab_4: []const u8 = "cmd+4",
    tab_5: []const u8 = "cmd+5",
    tab_6: []const u8 = "cmd+6",
    tab_7: []const u8 = "cmd+7",
    tab_8: []const u8 = "cmd+8",
    tab_9: []const u8 = "cmd+9",
};

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
    var loaded = try parseSlice(testing.allocator, ".{ .scrollback = 0, .font = .{ .size = 0.0 }, .window = .{ .width = 1.0, .height = 1.0 } }");
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

test "load of a missing file yields defaults" {
    var loaded = load(testing.allocator, "/nonexistent/caldera-test-config.zon");
    defer loaded.deinit();
    try testing.expectEqual(@as(usize, 100_000), loaded.config.scrollback);
}

test "Watcher detects a change and reloads" {
    const io = std.testing.io;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();

    var path_buf: [std.fs.max_path_bytes]u8 = undefined;
    const path_len = try tmp.dir.realPath(io, &path_buf);
    const dir_path = path_buf[0..path_len];

    var full_buf: [std.fs.max_path_bytes]u8 = undefined;
    const full = try std.fmt.bufPrint(&full_buf, "{s}/config.zon", .{dir_path});

    try tmp.dir.writeFile(io, .{ .sub_path = "config.zon", .data = ".{ .scrollback = 11 }" });
    var w = Watcher.init(full);

    var first = w.poll(testing.allocator) orelse return error.ExpectedReload;
    defer first.deinit();
    try testing.expectEqual(@as(usize, 11), first.config.scrollback);

    // No change -> no reload.
    try testing.expect(w.poll(testing.allocator) == null);
}

test "Watcher file removed returns defaults" {
    const io = std.testing.io;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();

    var path_buf: [std.fs.max_path_bytes]u8 = undefined;
    const path_len = try tmp.dir.realPath(io, &path_buf);
    const dir_path = path_buf[0..path_len];

    var full_buf: [std.fs.max_path_bytes]u8 = undefined;
    const full = try std.fmt.bufPrint(&full_buf, "{s}/config.zon", .{dir_path});

    try tmp.dir.writeFile(io, .{ .sub_path = "config.zon", .data = ".{ .scrollback = 42 }" });
    var w = Watcher.init(full);

    // First poll: file exists, gets loaded config.
    var first = w.poll(testing.allocator) orelse return error.ExpectedReload;
    defer first.deinit();
    try testing.expectEqual(@as(usize, 42), first.config.scrollback);

    // Delete the file.
    try tmp.dir.deleteFile(io, "config.zon");

    // Second poll: file removed -> m == 0 branch -> returns defaults.
    var second = w.poll(testing.allocator) orelse return error.ExpectedDefaults;
    defer second.deinit();
    try testing.expectEqual(@as(usize, 100_000), second.config.scrollback);
}

test "Watcher parse failure advances mtime so it is not re-reported" {
    const io = std.testing.io;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();

    var path_buf: [std.fs.max_path_bytes]u8 = undefined;
    const path_len = try tmp.dir.realPath(io, &path_buf);
    const dir_path = path_buf[0..path_len];

    var full_buf: [std.fs.max_path_bytes]u8 = undefined;
    const full = try std.fmt.bufPrint(&full_buf, "{s}/config.zon", .{dir_path});

    try tmp.dir.writeFile(io, .{ .sub_path = "config.zon", .data = ".{ .scrollback = 77 }" });
    var w = Watcher.init(full);

    // First poll: valid content loads successfully.
    var first = w.poll(testing.allocator) orelse return error.ExpectedReload;
    defer first.deinit();
    try testing.expectEqual(@as(usize, 77), first.config.scrollback);

    // Sleep 10 ms to ensure the second write gets a distinct mtime.
    const delay = std.c.timespec{ .sec = 0, .nsec = 10_000_000 };
    _ = std.c.nanosleep(&delay, null);

    // Overwrite with malformed ZON.
    try tmp.dir.writeFile(io, .{ .sub_path = "config.zon", .data = ".{ .scrollback =" });

    // Second poll: parse fails, falls back to defaults — but mtime is advanced.
    var second = w.poll(testing.allocator) orelse return error.ExpectedDefaults;
    defer second.deinit();
    try testing.expectEqual(@as(usize, 100_000), second.config.scrollback);

    // Third poll: file unchanged, mtime was already recorded -> must return null.
    try testing.expect(w.poll(testing.allocator) == null);
}

test "parseChord parses modifiers and key" {
    const c = parseChord("cmd+shift+]").?;
    try testing.expect(c.cmd and c.shift and !c.ctrl and !c.opt);
    try testing.expectEqual(@as(u21, ']'), c.key);

    const t = parseChord("cmd+t").?;
    try testing.expect(t.cmd);
    try testing.expectEqual(@as(u21, 't'), t.key);

    // Case-insensitive, letter lowercased.
    const u = parseChord("CMD+T").?;
    try testing.expectEqual(@as(u21, 't'), u.key);
}

test "parseChord rejects malformed strings" {
    try testing.expect(parseChord("") == null);
    try testing.expect(parseChord("cmd+") == null);
    try testing.expect(parseChord("cmd+ab") == null); // key not single char
    try testing.expect(parseChord("cmd") == null); // no key
    try testing.expect(parseChord("cmd+t+w") == null); // two keys
}

test "config parses a keybindings override" {
    var loaded = try parseSlice(testing.allocator, ".{ .keybindings = .{ .new_tab = \"ctrl+n\" } }");
    defer loaded.deinit();
    try testing.expectEqualStrings("ctrl+n", loaded.config.keybindings.new_tab);
    try testing.expectEqualStrings("cmd+w", loaded.config.keybindings.close_tab); // default
}
