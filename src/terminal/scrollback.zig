//! Deep terminal scrollback: a ring buffer of *trimmed* rows.
//!
//! Each pushed row is copied into a fresh allocation sized to only its used
//! width — trailing blank cells are dropped. Most terminal lines are short,
//! so this keeps memory tiny even at very large capacities. When the ring is
//! full the oldest row is freed and evicted to make room.

const std = @import("std");
const Cell = @import("cell.zig").Cell;

/// Default ring capacity in rows. Deliberately enormous — trimmed storage
/// makes this cheap — and trivially raised by callers that want even more.
pub const default_capacity: usize = 100_000;

pub const Scrollback = struct {
    alloc: std.mem.Allocator,

    /// Ring of trimmed rows. `rows[i]` owns its `[]Cell` allocation.
    rows: []?[]Cell,
    /// Index of the oldest row within `rows`.
    head: usize = 0,
    /// Number of rows currently stored (0..rows.len).
    count: usize = 0,

    /// Create a scrollback holding up to `row_capacity` rows. A capacity of 0
    /// is raised to 1 so the ring math always has somewhere to put a row.
    pub fn init(alloc: std.mem.Allocator, row_capacity: usize) !Scrollback {
        const cap = @max(row_capacity, 1);
        const rows = try alloc.alloc(?[]Cell, cap);
        @memset(rows, null);
        return .{ .alloc = alloc, .rows = rows };
    }

    /// Free every retained row and the ring itself.
    pub fn deinit(self: *Scrollback) void {
        for (self.rows) |maybe_row| {
            if (maybe_row) |row| self.alloc.free(row);
        }
        self.alloc.free(self.rows);
        self.* = undefined;
    }

    /// Number of rows currently held.
    pub fn len(self: *const Scrollback) usize {
        return self.count;
    }

    /// Maximum number of rows the ring can hold.
    pub fn capacity(self: *const Scrollback) usize {
        return self.rows.len;
    }

    /// Copy `row` into the ring, trimming trailing blank cells. When the ring
    /// is full the oldest row is freed and evicted. A push silently no-ops if
    /// the trimmed copy cannot be allocated — scrollback is best-effort.
    pub fn push(self: *Scrollback, row: []const Cell) void {
        const used = trimmedLen(row);
        const copy = self.alloc.alloc(Cell, used) catch return;
        @memcpy(copy, row[0..used]);

        const slot = (self.head + self.count) % self.rows.len;
        if (self.count == self.rows.len) {
            // Ring full: the slot we are about to write holds the oldest row.
            if (self.rows[slot]) |old| self.alloc.free(old);
            self.head = (self.head + 1) % self.rows.len;
        } else {
            self.count += 1;
        }
        self.rows[slot] = copy;
    }

    /// Borrow row `index` counting from the oldest (0) to the newest
    /// (`len() - 1`). The slice may be shorter than the grid width — callers
    /// pad with blanks when rendering. Returns an empty slice if out of range.
    pub fn get(self: *const Scrollback, index_from_oldest: usize) []const Cell {
        if (index_from_oldest >= self.count) return &.{};
        const slot = (self.head + index_from_oldest) % self.rows.len;
        return self.rows[slot] orelse &.{};
    }

    /// Length of `row` with trailing blank cells removed.
    fn trimmedLen(row: []const Cell) usize {
        var n = row.len;
        while (n > 0 and row[n - 1].isBlank()) n -= 1;
        return n;
    }
};

// --- tests -----------------------------------------------------------------

const testing = std.testing;

/// Build a row of `width` cells whose first `text.len` cells carry `text`.
fn makeRow(buf: []Cell, width: usize, text: []const u8) []Cell {
    for (buf[0..width], 0..) |*cell, i| {
        cell.* = .{};
        if (i < text.len) cell.cp = text[i];
    }
    return buf[0..width];
}

test "push then get round-trips trimmed content" {
    var sb = try Scrollback.init(testing.allocator, 8);
    defer sb.deinit();

    var buf: [80]Cell = undefined;
    sb.push(makeRow(&buf, 80, "hello"));

    try testing.expectEqual(@as(usize, 1), sb.len());
    const row = sb.get(0);
    // Trailing blanks dropped: the row shrank to its 5 used cells.
    try testing.expectEqual(@as(usize, 5), row.len);
    try testing.expectEqual(@as(u21, 'h'), row[0].cp);
    try testing.expectEqual(@as(u21, 'o'), row[4].cp);
}

test "an all-blank row trims to length zero" {
    var sb = try Scrollback.init(testing.allocator, 4);
    defer sb.deinit();

    var buf: [40]Cell = undefined;
    sb.push(makeRow(&buf, 40, ""));
    try testing.expectEqual(@as(usize, 1), sb.len());
    try testing.expectEqual(@as(usize, 0), sb.get(0).len);
}

test "ring evicts the oldest row at capacity" {
    var sb = try Scrollback.init(testing.allocator, 3);
    defer sb.deinit();

    var buf: [10]Cell = undefined;
    sb.push(makeRow(&buf, 10, "A"));
    sb.push(makeRow(&buf, 10, "B"));
    sb.push(makeRow(&buf, 10, "C"));
    try testing.expectEqual(@as(usize, 3), sb.len());
    try testing.expectEqual(@as(u21, 'A'), sb.get(0)[0].cp);

    // Pushing a fourth row evicts "A"; the window slides forward.
    sb.push(makeRow(&buf, 10, "D"));
    try testing.expectEqual(@as(usize, 3), sb.len());
    try testing.expectEqual(@as(u21, 'B'), sb.get(0)[0].cp);
    try testing.expectEqual(@as(u21, 'C'), sb.get(1)[0].cp);
    try testing.expectEqual(@as(u21, 'D'), sb.get(2)[0].cp);
}

test "get out of range returns an empty slice" {
    var sb = try Scrollback.init(testing.allocator, 4);
    defer sb.deinit();
    try testing.expectEqual(@as(usize, 0), sb.get(0).len);

    var buf: [10]Cell = undefined;
    sb.push(makeRow(&buf, 10, "x"));
    try testing.expectEqual(@as(usize, 0), sb.get(1).len);
    try testing.expectEqual(@as(usize, 0), sb.get(99).len);
}

test "many pushes past capacity keep memory bounded and ordering correct" {
    var sb = try Scrollback.init(testing.allocator, 5);
    defer sb.deinit();

    var buf: [4]Cell = undefined;
    var i: u8 = 0;
    while (i < 100) : (i += 1) {
        buf[0] = .{ .cp = i };
        sb.push(buf[0..1]);
    }
    try testing.expectEqual(@as(usize, 5), sb.len());
    // The retained window is the last five values: 95..99.
    try testing.expectEqual(@as(u21, 95), sb.get(0)[0].cp);
    try testing.expectEqual(@as(u21, 99), sb.get(4)[0].cp);
}

test "default capacity constant is the deep value" {
    try testing.expectEqual(@as(usize, 100_000), default_capacity);
}

test "zero capacity is raised to one" {
    var sb = try Scrollback.init(testing.allocator, 0);
    defer sb.deinit();
    try testing.expectEqual(@as(usize, 1), sb.capacity());

    var buf: [4]Cell = undefined;
    sb.push(makeRow(&buf, 4, "z"));
    try testing.expectEqual(@as(usize, 1), sb.len());
    try testing.expectEqual(@as(u21, 'z'), sb.get(0)[0].cp);
}
