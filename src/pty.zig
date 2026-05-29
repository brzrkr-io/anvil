const std = @import("std");

const c = @cImport({
    @cInclude("util.h");
    @cInclude("unistd.h");
    @cInclude("stdlib.h");
    @cInclude("signal.h");
    @cInclude("termios.h");
    @cInclude("sys/ioctl.h");
});

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
        const env = c.getenv("SHELL");
        const shell: [*c]const u8 = if (env != null) env else "/bin/zsh";
        _ = c.execlp(shell, shell, @as([*c]const u8, null));
        c._exit(127);
    }

    pub fn resize(self: *Pty, rows: u16, cols: u16) void {
        var ws = c.struct_winsize{ .ws_row = rows, .ws_col = cols, .ws_xpixel = 0, .ws_ypixel = 0 };
        _ = c.ioctl(self.master, c.TIOCSWINSZ, &ws);
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
