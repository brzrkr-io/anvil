const std = @import("std");
const Cell = @import("cell.zig").Cell;

/// Ring buffer of scrolled-off rows. Owns each line; drops the oldest at cap.
pub const Scrollback = struct {
    alloc: std.mem.Allocator,
    buf: [][]Cell,
    start: usize = 0,
    count: usize = 0,
    /// Total lines ever pushed, including evicted ones. Gives each line a stable
    /// absolute id (`pushed - count + index`) that survives eviction — used for
    /// prompt marks (OSC 133).
    pushed: usize = 0,

    pub fn init(alloc: std.mem.Allocator, cap: usize) !Scrollback {
        return .{ .alloc = alloc, .buf = try alloc.alloc([]Cell, cap) };
    }

    pub fn deinit(self: *Scrollback) void {
        var i: usize = 0;
        while (i < self.count) : (i += 1) self.alloc.free(self.at(i));
        self.alloc.free(self.buf);
    }

    pub fn clear(self: *Scrollback) void {
        var i: usize = 0;
        while (i < self.count) : (i += 1) self.alloc.free(self.at(i));
        self.count = 0;
        self.start = 0;
        self.pushed = 0;
    }

    pub fn push(self: *Scrollback, src: []const Cell) void {
        const line = self.alloc.dupe(Cell, src) catch return;
        self.pushed += 1;
        if (self.count < self.buf.len) {
            self.buf[(self.start + self.count) % self.buf.len] = line;
            self.count += 1;
        } else {
            self.alloc.free(self.buf[self.start]);
            self.buf[self.start] = line;
            self.start = (self.start + 1) % self.buf.len;
        }
    }

    /// Remove and return the newest line. Caller owns and must free it.
    pub fn pop(self: *Scrollback) ?[]Cell {
        if (self.count == 0) return null;
        self.count -= 1;
        self.pushed -= 1;
        return self.at(self.count);
    }

    pub fn len(self: *const Scrollback) usize {
        return self.count;
    }

    pub fn at(self: *const Scrollback, i: usize) []Cell {
        return self.buf[(self.start + i) % self.buf.len];
    }
};

test "push grows then drops oldest at cap" {
    var sb = try Scrollback.init(std.testing.allocator, 2);
    defer sb.deinit();
    var a = [_]Cell{.{ .cp = 'a' }};
    var b = [_]Cell{.{ .cp = 'b' }};
    var c = [_]Cell{.{ .cp = 'c' }};
    sb.push(&a);
    sb.push(&b);
    try std.testing.expectEqual(@as(usize, 2), sb.len());
    try std.testing.expectEqual(@as(u21, 'a'), sb.at(0)[0].cp);
    sb.push(&c); // evicts 'a'
    try std.testing.expectEqual(@as(usize, 2), sb.len());
    try std.testing.expectEqual(@as(u21, 'b'), sb.at(0)[0].cp);
    try std.testing.expectEqual(@as(u21, 'c'), sb.at(1)[0].cp);
}
