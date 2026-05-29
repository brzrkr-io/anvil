//! PTY seam: spawn the user's shell on a pseudo-terminal and expose the master
//! fd for reading/writing. This is the spine of the terminal — everything else
//! (VT parsing, grid, rendering) consumes the byte stream that flows through here.
//!
//! We use libc's `forkpty`, which does the fiddly work for us: open a pty pair,
//! fork, make the child the session leader, and wire the child's stdio to the
//! slave side. We just exec the shell in the child and keep the master fd.

const std = @import("std");

const c = @cImport({
    @cInclude("util.h"); // forkpty (declared here on macOS/BSD)
    @cInclude("unistd.h"); // execlp, _exit, close
    @cInclude("stdlib.h"); // getenv
    @cInclude("signal.h"); // kill, SIGHUP
    @cInclude("termios.h");
    @cInclude("sys/ioctl.h"); // struct winsize
});

pub const Pty = struct {
    /// Master side of the pty. Read shell output from it; write keystrokes to it.
    master: std.posix.fd_t,
    /// Child shell pid.
    pid: std.posix.pid_t,

    /// Spawn the user's `$SHELL` (fallback `/bin/zsh`) on a fresh pty sized
    /// `rows` x `cols`. Returns the master fd + child pid.
    pub fn spawn(rows: u16, cols: u16) !Pty {
        var master: c_int = -1;
        var ws = c.struct_winsize{
            .ws_row = rows,
            .ws_col = cols,
            .ws_xpixel = 0,
            .ws_ypixel = 0,
        };

        const pid = c.forkpty(&master, null, null, &ws);
        if (pid < 0) return error.ForkptyFailed;

        if (pid == 0) {
            // Child: replace ourselves with the shell. forkpty already made our
            // stdio the pty slave, so we only need to exec.
            childExec();
            unreachable; // childExec always execs or _exits
        }

        return .{ .master = @intCast(master), .pid = @intCast(pid) };
    }

    fn childExec() void {
        const env = c.getenv("SHELL");
        const shell: [*c]const u8 = if (env != null) env else "/bin/zsh";
        // argv[0] = shell path, then the C-variadic NULL terminator.
        _ = c.execlp(shell, shell, @as([*c]const u8, null));
        c._exit(127); // only reached if exec failed
    }

    /// Resize the pty (call on window resize). Tells the child via TIOCSWINSZ
    /// so programs like vim/htop redraw at the right size.
    pub fn resize(self: *Pty, rows: u16, cols: u16) void {
        var ws = c.struct_winsize{
            .ws_row = rows,
            .ws_col = cols,
            .ws_xpixel = 0,
            .ws_ypixel = 0,
        };
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
