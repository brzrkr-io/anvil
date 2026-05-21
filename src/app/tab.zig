//! Terminal tabs: each `Tab` owns a shell; `TabManager` owns the tab list.
//! This file starts with pure list/index helpers so the bookkeeping is unit
//! tested without spawning real shells.

const std = @import("std");

/// True when a tab bar should be drawn — only with 2+ tabs (low-profile rule).
pub fn barVisible(count: usize) bool {
    return count >= 2;
}

/// Clamp an arbitrary index to `[0, count-1]`. `count` is assumed >= 1.
pub fn clampIndex(count: usize, index: usize) usize {
    if (count == 0) return 0;
    return @min(index, count - 1);
}

/// The active index after stepping `delta` (+1 / -1) with wraparound.
/// `count` is assumed >= 1.
pub fn wrapIndex(count: usize, index: usize, delta: isize) usize {
    if (count == 0) return 0;
    const c: isize = @intCast(count);
    var i: isize = @as(isize, @intCast(index)) + delta;
    i = @mod(i, c); // Zig @mod gives a non-negative result for positive c
    return @intCast(i);
}

/// The active index after the tab at `closed` is removed from a list that had
/// `count` tabs (so `count-1` remain). `active` is the index before removal.
/// Rule: if a tab before the active one closed, the active shifts down by one;
/// if the active tab itself closed, stay at the same slot (now the next tab)
/// unless it was the last, then step back; tabs after the active are unaffected.
pub fn nextActiveAfterClose(count: usize, closed: usize, active: usize) usize {
    if (count <= 1) return 0;
    const remaining = count - 1;
    if (closed < active) return active - 1;
    if (closed > active) return active;
    // The active tab itself closed.
    return @min(active, remaining - 1);
}

const testing = std.testing;

test "barVisible only at 2+ tabs" {
    try testing.expect(!barVisible(0));
    try testing.expect(!barVisible(1));
    try testing.expect(barVisible(2));
    try testing.expect(barVisible(9));
}

test "clampIndex pins to range" {
    try testing.expectEqual(@as(usize, 2), clampIndex(3, 2));
    try testing.expectEqual(@as(usize, 2), clampIndex(3, 99));
    try testing.expectEqual(@as(usize, 0), clampIndex(1, 5));
}

test "wrapIndex wraps both directions" {
    try testing.expectEqual(@as(usize, 1), wrapIndex(3, 0, 1));
    try testing.expectEqual(@as(usize, 0), wrapIndex(3, 2, 1)); // wrap forward
    try testing.expectEqual(@as(usize, 2), wrapIndex(3, 0, -1)); // wrap backward
    try testing.expectEqual(@as(usize, 0), wrapIndex(1, 0, 1)); // single tab
}

test "nextActiveAfterClose handles every position" {
    // 3 tabs, active = 1.
    try testing.expectEqual(@as(usize, 0), nextActiveAfterClose(3, 0, 1)); // closed before active
    try testing.expectEqual(@as(usize, 1), nextActiveAfterClose(3, 2, 1)); // closed after active
    try testing.expectEqual(@as(usize, 1), nextActiveAfterClose(3, 1, 1)); // closed the active (middle)
    // closing the active *last* tab steps back
    try testing.expectEqual(@as(usize, 1), nextActiveAfterClose(3, 2, 2));
    // closing down to one tab
    try testing.expectEqual(@as(usize, 0), nextActiveAfterClose(2, 0, 0));
}
