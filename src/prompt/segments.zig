//! The prompt's segment model. A Segment is one unit on the context line:
//! an icon, a text value, and a state that drives its colour.

const std = @import("std");
const Icon = @import("icons.zig").Icon;

/// Drives the segment's colour at render time.
pub const State = enum { normal, ok, warn, err, run };

pub const Segment = struct {
    icon: Icon,
    /// Borrowed; must outlive the Segment (caller owns the backing memory).
    text: []const u8,
    state: State = .normal,
};

/// A fixed-capacity segment list — the prompt never shows more than this many,
/// and a stack buffer keeps `anvil-prompt` allocation-light on the hot path.
pub const max_segments = 12;

pub const List = struct {
    items: [max_segments]Segment = undefined,
    len: usize = 0,

    pub fn add(self: *List, seg: Segment) void {
        if (self.len >= max_segments) return;
        self.items[self.len] = seg;
        self.len += 1;
    }

    pub fn slice(self: *const List) []const Segment {
        return self.items[0..self.len];
    }
};

test "List.add appends until capacity" {
    var l = List{};
    try std.testing.expectEqual(@as(usize, 0), l.slice().len);
    l.add(.{ .icon = .branch, .text = "main" });
    try std.testing.expectEqual(@as(usize, 1), l.slice().len);
    try std.testing.expectEqualStrings("main", l.slice()[0].text);
}

test "List.add stops at capacity, never overflows" {
    var l = List{};
    var i: usize = 0;
    while (i < max_segments + 5) : (i += 1) l.add(.{ .icon = .repo, .text = "x" });
    try std.testing.expectEqual(max_segments, l.slice().len);
}
