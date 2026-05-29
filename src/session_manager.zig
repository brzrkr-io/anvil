const std = @import("std");
const Session = @import("session.zig").Session;
const pane = @import("workspace/pane_tree.zig");

/// Owns the live terminal sessions and their split layout, and tracks which
/// session has focus. Every C-ABI export routes through the focused session.
pub const SessionManager = struct {
    alloc: std.mem.Allocator,
    sessions: std.ArrayListUnmanaged(Session) = .empty,
    tree: ?pane.PaneTree = null,
    focused: usize = 0, // session id
    next_id: usize = 0,

    pub fn deinit(self: *SessionManager) void {
        for (self.sessions.items) |*s| s.deinit();
        self.sessions.deinit(self.alloc);
        if (self.tree) |*t| t.deinit();
    }

    /// Spawn the first session and seed the pane tree.
    pub fn spawnFirst(self: *SessionManager, rows: u16, cols: u16) !void {
        const id = try self.add(rows, cols);
        self.tree = pane.PaneTree.init(self.alloc, id);
        self.focused = id;
    }

    /// Split the focused pane along `axis`; the new session takes focus.
    pub fn splitFocused(self: *SessionManager, axis: pane.Axis, rows: u16, cols: u16) !void {
        var tree = &(self.tree orelse return);
        const id = try self.add(rows, cols);
        try tree.split(self.focused, axis, id);
        self.focused = id;
    }

    /// Close the focused pane, collapsing the layout. No-op on the last pane.
    pub fn closeFocused(self: *SessionManager) void {
        var tree = &(self.tree orelse return);
        if (tree.count() <= 1) return;
        const dead = self.focused;
        tree.close(dead);
        self.focused = tree.anyLeaf();
        self.removeSession(dead);
    }

    /// Move focus to the nearest pane in `dir` within `rect` (device px).
    pub fn focusNeighbor(self: *SessionManager, rect: pane.Rect, dir: pane.Dir, buf: []pane.PaneRect) void {
        const tree = &(self.tree orelse return);
        if (tree.neighbor(rect, self.focused, dir, buf)) |id| self.focused = id;
    }

    pub fn byId(self: *SessionManager, id: usize) ?*Session {
        for (self.sessions.items) |*s| {
            if (s.id == id) return s;
        }
        return null;
    }

    pub fn focusedSession(self: *SessionManager) ?*Session {
        return self.byId(self.focused);
    }

    pub fn count(self: *const SessionManager) usize {
        return self.sessions.items.len;
    }

    fn add(self: *SessionManager, rows: u16, cols: u16) !usize {
        const id = self.next_id;
        var s = try Session.init(self.alloc, rows, cols);
        s.id = id;
        try self.sessions.append(self.alloc, s);
        self.next_id += 1;
        return id;
    }

    fn removeSession(self: *SessionManager, id: usize) void {
        for (self.sessions.items, 0..) |*s, i| {
            if (s.id == id) {
                var dead = self.sessions.swapRemove(i);
                dead.deinit();
                return;
            }
        }
    }
};

test "spawnFirst seeds one focused session and tree" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    try std.testing.expectEqual(@as(usize, 1), mgr.count());
    try std.testing.expect(mgr.focusedSession() != null);
    try std.testing.expectEqual(@as(usize, 1), mgr.tree.?.count());
}

test "splitFocused adds a pane and shifts focus" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    const first = mgr.focused;
    try mgr.splitFocused(.x, 24, 40);
    try std.testing.expectEqual(@as(usize, 2), mgr.count());
    try std.testing.expectEqual(@as(usize, 2), mgr.tree.?.count());
    try std.testing.expect(mgr.focused != first);
}

test "closeFocused removes the pane and re-homes focus" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    const first = mgr.focused;
    try mgr.splitFocused(.x, 24, 40);
    mgr.closeFocused();
    try std.testing.expectEqual(@as(usize, 1), mgr.count());
    try std.testing.expectEqual(first, mgr.focused);
}

test "closeFocused is a no-op on the last pane" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    mgr.closeFocused();
    try std.testing.expectEqual(@as(usize, 1), mgr.count());
}
