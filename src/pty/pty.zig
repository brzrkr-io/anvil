//! PTY layer: spawns a real shell in a pseudo-terminal and lets the caller
//! read its output, write input, and resize it. macOS/libc only.

const std = @import("std");
const posix = std.posix;

const c = @cImport({
    @cInclude("util.h"); // openpty, login_tty
    @cInclude("sys/ioctl.h"); // TIOCSWINSZ, struct winsize
    @cInclude("termios.h");
    @cInclude("unistd.h"); // execve, close, _exit, fork
    @cInclude("signal.h"); // kill, SIGHUP
    @cInclude("sys/wait.h"); // waitpid
});

/// The shell used when `$SHELL` is unset.
const default_shell = "/bin/zsh";

/// The terminal type advertised to the child process.
const term_value = "xterm-256color";

pub const Pty = struct {
    master: posix.fd_t,
    child: posix.pid_t,

    /// Spawn a pseudo-terminal running the program at `path` with the given
    /// `argv`. `argv[0]` is the conventional program name passed to the
    /// process — it may differ from `path` (e.g. a login shell whose `argv[0]`
    /// is `-zsh` while `path` is `/bin/zsh`).
    pub fn spawnExec(
        alloc: std.mem.Allocator,
        path: []const u8,
        argv: []const []const u8,
        cols: u16,
        rows: u16,
    ) !Pty {
        std.debug.assert(argv.len > 0);

        var master: c_int = undefined;
        var slave: c_int = undefined;
        var size = winsize(cols, rows);

        if (c.openpty(&master, &slave, null, null, &size) != 0) {
            return error.OpenPtyFailed;
        }
        errdefer _ = c.close(master);

        const argv_z = try toCStringArray(alloc, argv);
        defer freeCStringArray(alloc, argv_z);

        const env_z = try buildChildEnv(alloc);
        defer freeCStringArray(alloc, env_z);

        const pid = c.fork();
        if (pid < 0) return error.ForkFailed;
        if (pid == 0) {
            childExec(slave, path, argv_z, env_z);
        }

        // Parent: the slave fd belongs to the child now.
        _ = c.close(slave);
        return .{ .master = master, .child = pid };
    }

    /// Spawn `argv` in a new pseudo-terminal sized cols x rows. The executed
    /// program is `argv[0]`.
    pub fn spawn(
        alloc: std.mem.Allocator,
        argv: []const []const u8,
        cols: u16,
        rows: u16,
    ) !Pty {
        std.debug.assert(argv.len > 0);
        return spawnExec(alloc, argv[0], argv, cols, rows);
    }

    /// Spawn the user's login shell (`$SHELL`, fallback `/bin/zsh`). A login
    /// shell is signalled by a single `argv[0]` of `-` + the shell basename
    /// (e.g. `-zsh`); the executable path stays the real path.
    pub fn spawnShell(alloc: std.mem.Allocator, cols: u16, rows: u16) !Pty {
        const shell = lookupEnv("SHELL") orelse default_shell;
        const arg0 = try std.fmt.allocPrint(alloc, "-{s}", .{std.fs.path.basename(shell)});
        defer alloc.free(arg0);

        return spawnExec(alloc, shell, &.{arg0}, cols, rows);
    }

    /// Read available output from the child. Blocking. Returns bytes read;
    /// 0 (and `error.Eof` on macOS EIO) once the child exits.
    pub fn read(self: *Pty, buf: []u8) !usize {
        const n = posix.read(self.master, buf) catch |err| switch (err) {
            // macOS reports a read on a master whose child has exited as EIO.
            error.InputOutput => return error.Eof,
            else => return err,
        };
        if (n == 0) return error.Eof;
        return n;
    }

    /// Write input bytes to the child.
    pub fn write(self: *Pty, bytes: []const u8) !usize {
        const n = c.write(self.master, bytes.ptr, bytes.len);
        if (n < 0) return error.WriteFailed;
        return @intCast(n);
    }

    /// Resize the pty (TIOCSWINSZ) so the child sees SIGWINCH.
    pub fn resize(self: *Pty, cols: u16, rows: u16) void {
        var size = winsize(cols, rows);
        _ = c.ioctl(self.master, c.TIOCSWINSZ, &size);
    }

    /// Close the master fd and terminate the child, reaping the zombie.
    pub fn deinit(self: *Pty) void {
        _ = c.kill(self.child, c.SIGHUP);
        _ = c.close(self.master);
        _ = c.waitpid(self.child, null, 0);
        self.master = -1;
        self.child = -1;
    }
};

/// Build a `struct winsize` with the given character dimensions and zero
/// pixel dimensions (the child computes pixels from the font).
fn winsize(cols: u16, rows: u16) c.struct_winsize {
    return .{
        .ws_col = cols,
        .ws_row = rows,
        .ws_xpixel = 0,
        .ws_ypixel = 0,
    };
}

/// Look up an environment variable in the parent process. Returns a slice
/// borrowed from libc's `environ` — valid for the lifetime of the process.
fn lookupEnv(name: []const u8) ?[]const u8 {
    var i: usize = 0;
    while (std.c.environ[i]) |entry| : (i += 1) {
        const pair = std.mem.span(entry);
        const eq = std.mem.indexOfScalar(u8, pair, '=') orelse continue;
        if (std.mem.eql(u8, pair[0..eq], name)) return pair[eq + 1 ..];
    }
    return null;
}

/// Runs only in the forked child. Attaches the slave as the controlling tty,
/// then replaces the process image. Never returns on success; on any failure
/// the child exits non-zero so the parent sees a dead pipe.
fn childExec(
    slave: c_int,
    path: []const u8,
    argv: [:null]const ?[*:0]const u8,
    envp: [:null]const ?[*:0]const u8,
) noreturn {
    // login_tty makes the slave the controlling tty and dups it to 0/1/2.
    if (c.login_tty(slave) != 0) c._exit(127);

    var path_buf: [std.fs.max_path_bytes:0]u8 = undefined;
    if (path.len >= path_buf.len) c._exit(127);
    @memcpy(path_buf[0..path.len], path);
    path_buf[path.len] = 0;

    _ = std.c.execve(&path_buf, argv.ptr, envp.ptr);
    c._exit(127); // execve only returns on failure.
}

/// Duplicate `argv` into a null-terminated array of owned C strings.
fn toCStringArray(
    alloc: std.mem.Allocator,
    argv: []const []const u8,
) ![:null]?[*:0]const u8 {
    const out = try alloc.allocSentinel(?[*:0]const u8, argv.len, null);
    var filled: usize = 0;
    errdefer freePartial(alloc, out, filled);
    for (argv) |arg| {
        out[filled] = try alloc.dupeZ(u8, arg);
        filled += 1;
    }
    return out;
}

/// Copy the parent environment, forcing `TERM=xterm-256color`, into a
/// null-terminated array of owned C strings.
fn buildChildEnv(alloc: std.mem.Allocator) ![:null]?[*:0]const u8 {
    var entries: std.ArrayList([]const u8) = .empty;
    defer {
        for (entries.items) |e| alloc.free(e);
        entries.deinit(alloc);
    }

    var i: usize = 0;
    while (std.c.environ[i]) |entry| : (i += 1) {
        const pair = std.mem.span(entry);
        if (std.mem.startsWith(u8, pair, "TERM=")) continue;
        try entries.append(alloc, try alloc.dupe(u8, pair));
    }
    try entries.append(alloc, try std.fmt.allocPrint(
        alloc,
        "TERM={s}",
        .{term_value},
    ));

    return toCStringArray(alloc, entries.items);
}

/// Free a null-terminated C-string array produced by `toCStringArray`.
fn freeCStringArray(alloc: std.mem.Allocator, array: [:null]?[*:0]const u8) void {
    freePartial(alloc, array, array.len);
}

/// Free the first `count` strings of `array`, then the array itself.
fn freePartial(
    alloc: std.mem.Allocator,
    array: [:null]?[*:0]const u8,
    count: usize,
) void {
    for (array[0..count]) |entry| {
        if (entry) |s| alloc.free(std.mem.span(s));
    }
    alloc.free(array);
}

// -- tests -------------------------------------------------------------------

/// Drain a pty into `out` until the child exits, capped at `out.len`.
fn drainToEof(pty: *Pty, out: []u8) usize {
    var total: usize = 0;
    while (total < out.len) {
        const n = pty.read(out[total..]) catch break;
        if (n == 0) break;
        total += n;
    }
    return total;
}

test "spawn echo and read its output" {
    const alloc = std.testing.allocator;
    var pty = try Pty.spawn(alloc, &.{ "/bin/echo", "hello" }, 80, 24);
    defer pty.deinit();

    var buf: [256]u8 = undefined;
    const total = drainToEof(&pty, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf[0..total], "hello") != null);
}

test "write input is echoed back through the pty" {
    const alloc = std.testing.allocator;
    var pty = try Pty.spawn(alloc, &.{"/bin/cat"}, 80, 24);
    defer pty.deinit();

    _ = try pty.write("ping\n");

    var buf: [256]u8 = undefined;
    var total: usize = 0;
    // cat echoes the line back; read until we see it or hit EOF.
    while (total < buf.len) {
        const n = pty.read(buf[total..]) catch break;
        if (n == 0) break;
        total += n;
        if (std.mem.indexOf(u8, buf[0..total], "ping") != null) break;
    }

    try std.testing.expect(std.mem.indexOf(u8, buf[0..total], "ping") != null);
}

test "resize does not crash" {
    const alloc = std.testing.allocator;
    var pty = try Pty.spawn(alloc, &.{"/bin/cat"}, 80, 24);
    defer pty.deinit();

    pty.resize(120, 40);
    pty.resize(80, 24);
}

test "spawnShell starts an interactive login shell" {
    const alloc = std.testing.allocator;
    var pty = try Pty.spawnShell(alloc, 80, 24);
    defer pty.deinit();

    // An interactive shell stays alive and runs the commands we send it.
    // (A misformed login argv makes the shell exit before this runs.)
    _ = try pty.write("printf CALDERA_PTY_OK\n");
    _ = try pty.write("exit\n");

    var buf: [16384]u8 = undefined;
    const total = drainToEof(&pty, &buf);
    try std.testing.expect(
        std.mem.indexOf(u8, buf[0..total], "CALDERA_PTY_OK") != null,
    );
}

test "child environment advertises xterm-256color" {
    const alloc = std.testing.allocator;
    var pty = try Pty.spawn(
        alloc,
        &.{ "/bin/sh", "-c", "printf %s \"$TERM\"" },
        80,
        24,
    );
    defer pty.deinit();

    var buf: [256]u8 = undefined;
    const total = drainToEof(&pty, &buf);

    try std.testing.expect(
        std.mem.indexOf(u8, buf[0..total], term_value) != null,
    );
}
