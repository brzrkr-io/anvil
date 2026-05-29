const std = @import("std");
const anvil = @import("anvil");
const posix = std.posix;

const c = @cImport({
    @cInclude("termios.h");
    @cInclude("sys/ioctl.h");
    @cInclude("unistd.h");
});

pub fn main(init: std.process.Init) !void {
    _ = init;

    var ws: c.struct_winsize = std.mem.zeroes(c.struct_winsize);
    _ = c.ioctl(1, c.TIOCGWINSZ, &ws);
    const rows: u16 = if (ws.ws_row != 0) @intCast(ws.ws_row) else 24;
    const cols: u16 = if (ws.ws_col != 0) @intCast(ws.ws_col) else 80;

    var pty = try anvil.Pty.spawn(rows, cols);
    defer pty.deinit();

    var orig: c.struct_termios = undefined;
    const have_tty = c.tcgetattr(0, &orig) == 0;
    if (have_tty) {
        var raw = orig;
        c.cfmakeraw(&raw);
        _ = c.tcsetattr(0, c.TCSANOW, &raw);
    }
    defer if (have_tty) {
        _ = c.tcsetattr(0, c.TCSANOW, &orig);
    };

    var fds = [_]posix.pollfd{
        .{ .fd = 0, .events = posix.POLL.IN, .revents = 0 },
        .{ .fd = pty.master, .events = posix.POLL.IN, .revents = 0 },
    };

    var buf: [8192]u8 = undefined;
    while (true) {
        _ = posix.poll(&fds, -1) catch break;

        if (fds[0].revents & (posix.POLL.IN | posix.POLL.HUP) != 0) {
            const n = c.read(0, @ptrCast(&buf), buf.len);
            if (n <= 0) {
                fds[0].fd = -1; // stop polling stdin, keep relaying shell
            } else {
                writeAll(pty.master, buf[0..@intCast(n)]) catch break;
            }
        }

        if (fds[1].revents & posix.POLL.IN != 0) {
            const n = c.read(pty.master, @ptrCast(&buf), buf.len);
            if (n <= 0) break;
            writeAll(1, buf[0..@intCast(n)]) catch break;
        }

        if (fds[1].revents & (posix.POLL.HUP | posix.POLL.ERR) != 0) break;
    }
}

fn writeAll(fd: c_int, bytes: []const u8) !void {
    var off: usize = 0;
    while (off < bytes.len) {
        const n = c.write(fd, @ptrCast(bytes.ptr + off), bytes.len - off);
        if (n < 0) return error.WriteFailed;
        off += @intCast(n);
    }
}
