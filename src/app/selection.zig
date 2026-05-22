//! Terminal text selection in content-coordinate space.
//! A content row is an absolute index over scrollback + active grid,
//! the same space used by `terminal.lineCount()` and `terminal.line(i)`.

const std = @import("std");

pub const Point = struct { row: usize, col: usize };

pub const Selection = struct {
    active: bool = false,
    anchor: Point = .{ .row = 0, .col = 0 },
    head: Point = .{ .row = 0, .col = 0 },

    pub fn clear(self: *Selection) void {
        self.active = false;
    }

    /// Returns ordered (start, end) so start <= end in reading order.
    pub fn ordered(self: Selection) struct { s: Point, e: Point } {
        const a = self.anchor;
        const h = self.head;
        if (a.row < h.row or (a.row == h.row and a.col <= h.col)) {
            return .{ .s = a, .e = h };
        }
        return .{ .s = h, .e = a };
    }

    /// Is the content cell at (row, col) inside the selection?
    /// The range is half-open: start.col..end.col on a single row,
    /// start.col..cols on the first row, 0..cols on middle rows,
    /// 0..end.col on the last row.
    pub fn contains(self: Selection, row: usize, col: usize) bool {
        if (!self.active) return false;
        const o = self.ordered();
        const s = o.s;
        const e = o.e;
        if (row < s.row or row > e.row) return false;
        if (s.row == e.row) return col >= s.col and col < e.col;
        if (row == s.row) return col >= s.col;
        if (row == e.row) return col < e.col;
        return true; // middle rows — entire row
    }
};

// --- unit tests ----------------------------------------------------------

test "inactive selection contains nothing" {
    const sel = Selection{};
    try std.testing.expect(!sel.contains(0, 0));
}

test "single-row selection, anchor before head" {
    const sel = Selection{
        .active = true,
        .anchor = .{ .row = 5, .col = 3 },
        .head = .{ .row = 5, .col = 7 },
    };
    try std.testing.expect(!sel.contains(5, 2));
    try std.testing.expect(sel.contains(5, 3));
    try std.testing.expect(sel.contains(5, 6));
    try std.testing.expect(!sel.contains(5, 7)); // half-open
    try std.testing.expect(!sel.contains(4, 5));
    try std.testing.expect(!sel.contains(6, 5));
}

test "single-row selection, reversed drag (anchor after head)" {
    const sel = Selection{
        .active = true,
        .anchor = .{ .row = 5, .col = 7 },
        .head = .{ .row = 5, .col = 3 },
    };
    try std.testing.expect(!sel.contains(5, 2));
    try std.testing.expect(sel.contains(5, 3));
    try std.testing.expect(sel.contains(5, 6));
    try std.testing.expect(!sel.contains(5, 7));
}

test "multi-line selection: first row from start.col, last row up to end.col" {
    const sel = Selection{
        .active = true,
        .anchor = .{ .row = 2, .col = 4 },
        .head = .{ .row = 5, .col = 10 },
    };
    // first row: col >= 4
    try std.testing.expect(!sel.contains(2, 3));
    try std.testing.expect(sel.contains(2, 4));
    try std.testing.expect(sel.contains(2, 100));
    // middle rows: all cols
    try std.testing.expect(sel.contains(3, 0));
    try std.testing.expect(sel.contains(4, 999));
    // last row: col < 10
    try std.testing.expect(sel.contains(5, 0));
    try std.testing.expect(sel.contains(5, 9));
    try std.testing.expect(!sel.contains(5, 10));
    // outside rows
    try std.testing.expect(!sel.contains(1, 0));
    try std.testing.expect(!sel.contains(6, 0));
}

test "multi-line selection reversed (anchor below head)" {
    const sel = Selection{
        .active = true,
        .anchor = .{ .row = 5, .col = 10 },
        .head = .{ .row = 2, .col = 4 },
    };
    try std.testing.expect(!sel.contains(2, 3));
    try std.testing.expect(sel.contains(2, 4));
    try std.testing.expect(sel.contains(3, 0));
    try std.testing.expect(sel.contains(5, 9));
    try std.testing.expect(!sel.contains(5, 10));
}

test "empty single-row selection (zero width)" {
    const sel = Selection{
        .active = true,
        .anchor = .{ .row = 3, .col = 5 },
        .head = .{ .row = 3, .col = 5 },
    };
    try std.testing.expect(!sel.contains(3, 5)); // zero-width: nothing selected
    try std.testing.expect(!sel.contains(3, 4));
}
