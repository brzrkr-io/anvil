//! Shell integration setup: write the embedded zsh/bash scripts to a runtime
//! dir and export the env vars that wire spawned shells to them.
//!
//! `Pty.buildChildEnv` copies `environ` into every child, so env vars exported
//! here (before any tab spawns) are inherited by every shell.

const std = @import("std");

// setenv/unsetenv are standard POSIX but not declared in Zig 0.16's std.c.
extern "c" fn setenv(name: [*:0]const u8, value: [*:0]const u8, overwrite: c_int) c_int;
extern "c" fn unsetenv(name: [*:0]const u8) c_int;

const integration_zsh = @embedFile("../shell/caldera-integration.zsh");
const integration_bash = @embedFile("../shell/caldera-integration.bash");
const zdotdir_zshenv = @embedFile("../shell/zdotdir-zshenv.zsh");

/// Resolve `~/.cache/caldera-console/shell` into `buf`. Null when `$HOME`
/// is unset.
fn runtimeDir(buf: []u8) ?[]const u8 {
    const home = std.c.getenv("HOME") orelse return null;
    const h = std.mem.span(home);
    return std.fmt.bufPrint(buf, "{s}/.cache/caldera-console/shell", .{h}) catch null;
}

/// Create every directory along `path` (like `mkdir -p`). Best-effort.
fn mkdirP(path: []const u8) void {
    var i: usize = 1;
    while (i < path.len) : (i += 1) {
        if (path[i] != '/') continue;
        var seg: [std.fs.max_path_bytes]u8 = undefined;
        if (i >= seg.len) return;
        @memcpy(seg[0..i], path[0..i]);
        seg[i] = 0;
        _ = std.c.mkdir(seg[0..i :0].ptr, 0o755);
    }
    var full: [std.fs.max_path_bytes]u8 = undefined;
    if (path.len >= full.len) return;
    @memcpy(full[0..path.len], path);
    full[path.len] = 0;
    _ = std.c.mkdir(full[0..path.len :0].ptr, 0o755);
}

/// Write `content` to `dir/name`. Returns false on any failure.
fn writeFile(dir: []const u8, name: []const u8, content: []const u8) bool {
    var pbuf: [std.fs.max_path_bytes]u8 = undefined;
    const path = std.fmt.bufPrintZ(&pbuf, "{s}/{s}", .{ dir, name }) catch return false;
    const fd = std.c.open(path.ptr, .{ .ACCMODE = .WRONLY, .CREAT = true, .TRUNC = true }, @as(c_uint, 0o644));
    if (fd < 0) return false;
    defer _ = std.c.close(fd);
    var off: usize = 0;
    while (off < content.len) {
        const n = std.c.write(fd, content[off..].ptr, content.len - off);
        if (n <= 0) return false;
        off += @intCast(n);
    }
    return true;
}

/// Set up shell integration. Writes the scripts and exports the wiring env
/// vars. When `enabled` is false, exports only the harmless markers and skips
/// `ZDOTDIR`. Any filesystem failure is logged and degrades to "no
/// integration" — never fatal. Call once at startup, before any tab spawns.
pub fn setup(enabled: bool) void {
    var dbuf: [std.fs.max_path_bytes]u8 = undefined;
    const dir = runtimeDir(&dbuf) orelse {
        std.debug.print("caldera-console: shell integration: $HOME unset, skipped\n", .{});
        return;
    };
    mkdirP(dir);

    const ok_zsh = writeFile(dir, "caldera-integration.zsh", integration_zsh);
    const ok_bash = writeFile(dir, "caldera-integration.bash", integration_bash);
    const ok_env = writeFile(dir, ".zshenv", zdotdir_zshenv);
    if (!(ok_zsh and ok_bash and ok_env)) {
        std.debug.print("caldera-console: shell integration: write failed, skipped\n", .{});
        return;
    }

    // Markers — always exported; harmless to any shell.
    _ = setenv("CALDERA_CONSOLE", "1", 1);
    var bbuf: [std.fs.max_path_bytes]u8 = undefined;
    if (std.fmt.bufPrintZ(&bbuf, "{s}/caldera-integration.bash", .{dir})) |bash_path| {
        _ = setenv("CALDERA_SHELL_INTEGRATION", bash_path.ptr, 1);
    } else |_| {}

    if (!enabled) return;

    // zsh auto-injection: point ZDOTDIR at our dir, after stashing the real one.
    const real = std.c.getenv("ZDOTDIR");
    if (real) |r| {
        _ = setenv("CALDERA_REAL_ZDOTDIR", r, 1);
    }
    var zbuf: [std.fs.max_path_bytes]u8 = undefined;
    if (std.fmt.bufPrintZ(&zbuf, "{s}/caldera-integration.zsh", .{dir})) |zsh_path| {
        _ = setenv("CALDERA_SHELL_INTEGRATION_ZSH", zsh_path.ptr, 1);
    } else |_| {}
    var dz: [std.fs.max_path_bytes]u8 = undefined;
    if (std.fmt.bufPrintZ(&dz, "{s}", .{dir})) |dirz| {
        _ = setenv("ZDOTDIR", dirz.ptr, 1);
    } else |_| {}
}

const testing = std.testing;

test "runtimeDir resolves under HOME" {
    // Save and override HOME.
    const saved = std.c.getenv("HOME");
    _ = setenv("HOME", "/tmp/caldera-shell-test", 1);
    defer if (saved) |s| {
        _ = setenv("HOME", s, 1);
    };
    var buf: [std.fs.max_path_bytes]u8 = undefined;
    const dir = runtimeDir(&buf).?;
    try testing.expectEqualStrings("/tmp/caldera-shell-test/.cache/caldera-console/shell", dir);
}

test "setup writes the scripts and exports markers" {
    const saved_home = std.c.getenv("HOME");
    _ = setenv("HOME", "/tmp/caldera-shell-test", 1);
    defer if (saved_home) |s| {
        _ = setenv("HOME", s, 1);
    };

    setup(true);

    // The three files exist and are non-empty.
    var buf: [std.fs.max_path_bytes]u8 = undefined;
    const dir = runtimeDir(&buf).?;
    var pbuf: [std.fs.max_path_bytes]u8 = undefined;
    inline for (.{ "caldera-integration.zsh", "caldera-integration.bash", ".zshenv" }) |name| {
        const path = try std.fmt.bufPrintZ(&pbuf, "{s}/{s}", .{ dir, name });
        const fd = std.c.open(path.ptr, .{ .ACCMODE = .RDONLY }, @as(c_uint, 0));
        try testing.expect(fd >= 0);
        _ = std.c.close(fd);
    }

    // Markers exported.
    try testing.expect(std.c.getenv("CALDERA_CONSOLE") != null);
    try testing.expect(std.c.getenv("ZDOTDIR") != null);
}

test "setup(false) does not export ZDOTDIR" {
    const saved_home = std.c.getenv("HOME");
    _ = setenv("HOME", "/tmp/caldera-shell-test-2", 1);
    defer if (saved_home) |s| {
        _ = setenv("HOME", s, 1);
    };
    // Clear any ZDOTDIR a prior test set.
    _ = unsetenv("ZDOTDIR");

    setup(false);
    try testing.expect(std.c.getenv("CALDERA_CONSOLE") != null); // marker still set
    try testing.expect(std.c.getenv("ZDOTDIR") == null); // but no injection
}
