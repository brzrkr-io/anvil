const std = @import("std");

const c = @cImport({
    @cInclude("util.h");
    @cInclude("unistd.h");
    @cInclude("stdlib.h");
    @cInclude("signal.h");
    @cInclude("termios.h");
    @cInclude("fcntl.h");
    @cInclude("sys/ioctl.h");
});

pub const ReadResult = union(enum) {
    data: usize,
    would_block,
    eof,
};

pub const Pty = struct {
    master: std.posix.fd_t,
    pid: std.posix.pid_t,

    pub fn spawn(rows: u16, cols: u16) !Pty {
        var master: c_int = -1;
        var ws = c.struct_winsize{ .ws_row = rows, .ws_col = cols, .ws_xpixel = 0, .ws_ypixel = 0 };
        const pid = c.forkpty(&master, null, null, &ws);
        if (pid < 0) return error.ForkptyFailed;
        if (pid == 0) {
            childExec();
            unreachable;
        }
        return .{ .master = @intCast(master), .pid = @intCast(pid) };
    }

    fn childExec() void {
        _ = c.setenv("TERM", "xterm-256color", 1);
        _ = c.setenv("COLORTERM", "truecolor", 1); // signal 24-bit to nvim/etc.
        // Launched from Finder/Dock the cwd is "/"; start the shell in $HOME.
        const home = c.getenv("HOME");
        if (home != null) _ = c.chdir(home);
        const env = c.getenv("SHELL");
        const shell: [*c]const u8 = if (env != null) env else "/bin/zsh";
        _ = c.execlp(shell, shell, @as([*c]const u8, null));
        c._exit(127);
    }

    pub fn resize(self: *Pty, rows: u16, cols: u16) void {
        var ws = c.struct_winsize{ .ws_row = rows, .ws_col = cols, .ws_xpixel = 0, .ws_ypixel = 0 };
        _ = c.ioctl(self.master, c.TIOCSWINSZ, &ws);
    }

    pub fn setNonblock(self: *Pty) void {
        const flags = c.fcntl(self.master, c.F_GETFL, @as(c_int, 0));
        _ = c.fcntl(self.master, c.F_SETFL, flags | c.O_NONBLOCK);
    }

    pub fn read(self: *Pty, buf: []u8) ReadResult {
        const n = c.read(self.master, buf.ptr, buf.len);
        if (n > 0) return .{ .data = @intCast(n) };
        if (n == 0) return .eof;
        return .would_block;
    }

    pub fn write(self: *Pty, bytes: []const u8) void {
        var off: usize = 0;
        while (off < bytes.len) {
            const n = c.write(self.master, bytes.ptr + off, bytes.len - off);
            if (n <= 0) return;
            off += @intCast(n);
        }
    }

    pub fn deinit(self: *Pty) void {
        _ = c.close(self.master);
        _ = c.kill(self.pid, c.SIGHUP);
    }
};

test "Pty has master + pid fields" {
    try std.testing.expect(@hasField(Pty, "master"));
    try std.testing.expect(@hasField(Pty, "pid"));
}

test "Pty round-trips bytes and reports eof on shell exit" {
    var p = try Pty.spawn(24, 80);
    defer p.deinit();
    p.write("exit\n");
    var buf: [4096]u8 = undefined;
    var total: usize = 0;
    while (true) switch (p.read(&buf)) {
        .data => |n| total += n,
        .eof => break,
        .would_block => {},
    };
    try std.testing.expect(total > 0);
}
