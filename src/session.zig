const std = @import("std");
const Terminal = @import("vt/terminal.zig").Terminal;
const Parser = @import("vt/parser.zig").Parser;
const Pty = @import("pty.zig").Pty;

/// One terminal session: a VT emulator, its parser, and the PTY feeding it.
pub const Session = struct {
    id: usize = 0,
    term: Terminal,
    parser: Parser = .{},
    pty: Pty,
    exited: bool = false,

    pub fn init(alloc: std.mem.Allocator, rows: u16, cols: u16) !Session {
        return initWithCwd(alloc, rows, cols, null);
    }

    pub fn initWithCwd(alloc: std.mem.Allocator, rows: u16, cols: u16, cwd: ?[*:0]const u8) !Session {
        var term = try Terminal.init(alloc, rows, cols);
        errdefer term.deinit();
        var pty = try Pty.spawnCwd(rows, cols, cwd);
        pty.setNonblock();
        return .{ .term = term, .pty = pty };
    }

    pub fn deinit(self: *Session) void {
        self.pty.deinit();
        self.term.deinit();
    }

    pub fn resize(self: *Session, rows: u16, cols: u16) !void {
        try self.term.resize(rows, cols);
        self.pty.resize(rows, cols);
    }

    pub const PollResult = struct { alive: bool, consumed: bool };

    /// Drain pending shell output into the terminal.
    pub fn poll(self: *Session) PollResult {
        var buf: [8192]u8 = undefined;
        var consumed = false;
        while (true) {
            switch (self.pty.read(&buf)) {
                .data => |n| {
                    consumed = true;
                    self.parser.feed(&self.term, buf[0..n]);
                    if (self.term.reply_len > 0) {
                        self.pty.write(self.term.reply_buf[0..self.term.reply_len]);
                        self.term.reply_len = 0;
                    }
                },
                .would_block => return .{ .alive = true, .consumed = consumed },
                .eof => return .{ .alive = false, .consumed = consumed },
            }
        }
    }

    /// Re-fork the PTY and reset the terminal. Called after the shell exits.
    pub fn respawn(self: *Session) !void {
        self.pty.deinit();
        self.pty = try Pty.spawn(self.term.grid.rows, self.term.grid.cols);
        self.pty.setNonblock();
        self.term.reset();
        self.parser = .{};
        self.exited = false;
    }

    /// Send input to the shell; typing jumps to the live view and clears selection.
    pub fn write(self: *Session, bytes: []const u8) void {
        if (self.term.view_offset != 0) self.term.view_offset = 0;
        self.term.clearSelection();
        self.pty.write(bytes);
    }
};

test "poll: consumed=true on data, alive=false on eof" {
    var s = try Session.init(std.testing.allocator, 24, 80);
    defer s.term.deinit();
    s.pty.write("exit\n");
    var got_consumed = false;
    var got_eof = false;
    var iters: usize = 0;
    while (iters < 200) : (iters += 1) {
        const r = s.poll();
        if (r.consumed) got_consumed = true;
        if (!r.alive) {
            got_eof = true;
            break;
        }
        const ts = std.c.timespec{ .sec = 0, .nsec = 10 * std.time.ns_per_ms };
        _ = std.c.nanosleep(&ts, null);
    }
    try std.testing.expect(got_consumed);
    try std.testing.expect(got_eof);
}
