const std = @import("std");
const Session = @import("session.zig").Session;
const pane = @import("workspace/pane_tree.zig");

const max_panes = 64;

/// Owns the live terminal sessions and the tab layouts, and tracks the active
/// tab plus the focused session within it. Every C-ABI export routes through
/// the focused session.
pub const SessionManager = struct {
    alloc: std.mem.Allocator,
    sessions: std.ArrayListUnmanaged(Session) = .empty,
    tabs: std.ArrayListUnmanaged(pane.PaneTree) = .empty,
    active_tab: usize = 0,
    focused: usize = 0, // session id within the active tab
    next_id: usize = 0,

    pub fn deinit(self: *SessionManager) void {
        for (self.sessions.items) |*s| s.deinit();
        self.sessions.deinit(self.alloc);
        for (self.tabs.items) |*t| t.deinit();
        self.tabs.deinit(self.alloc);
    }

    pub fn activeTree(self: *SessionManager) ?*pane.PaneTree {
        if (self.active_tab >= self.tabs.items.len) return null;
        return &self.tabs.items[self.active_tab];
    }

    /// Spawn the first session and seed the first tab.
    pub fn spawnFirst(self: *SessionManager, rows: u16, cols: u16) !void {
        const id = try self.add(rows, cols);
        try self.tabs.append(self.alloc, pane.PaneTree.init(self.alloc, id));
        self.active_tab = 0;
        self.focused = id;
    }

    /// Open a new tab with a fresh session, made active.
    pub fn newTab(self: *SessionManager, rows: u16, cols: u16) !void {
        const id = try self.add(rows, cols);
        try self.tabs.append(self.alloc, pane.PaneTree.init(self.alloc, id));
        self.active_tab = self.tabs.items.len - 1;
        self.focused = id;
    }

    /// Move to another tab by signed offset, wrapping. Focus follows.
    pub fn cycleTab(self: *SessionManager, delta: i32) void {
        const n = self.tabs.items.len;
        if (n == 0) return;
        const cur: i64 = @intCast(self.active_tab);
        const nx = @mod(cur + delta, @as(i64, @intCast(n)));
        self.active_tab = @intCast(nx);
        self.focused = self.activeTree().?.anyLeaf();
    }

    /// Close the active tab, killing its sessions. No-op on the last tab.
    pub fn closeTab(self: *SessionManager) void {
        if (self.tabs.items.len <= 1) return;
        var tree = self.tabs.orderedRemove(self.active_tab);
        var ids: [max_panes]usize = undefined;
        const n = tree.leaves(&ids);
        for (ids[0..n]) |id| self.removeSession(id);
        tree.deinit();
        if (self.active_tab >= self.tabs.items.len) self.active_tab = self.tabs.items.len - 1;
        self.focused = self.activeTree().?.anyLeaf();
    }

    /// Split the focused pane along `axis`; the new session takes focus.
    pub fn splitFocused(self: *SessionManager, axis: pane.Axis, rows: u16, cols: u16) !void {
        const tree = self.activeTree() orelse return;
        const id = try self.add(rows, cols);
        try tree.split(self.focused, axis, id);
        self.focused = id;
    }

    /// Close the focused pane. If it is the tab's last pane, close the tab.
    pub fn closeFocused(self: *SessionManager) void {
        const tree = self.activeTree() orelse return;
        if (tree.count() <= 1) {
            self.closeTab();
            return;
        }
        const dead = self.focused;
        tree.close(dead);
        self.focused = tree.anyLeaf();
        self.removeSession(dead);
    }

    /// Move focus to the nearest pane in `dir` within `rect` (device px).
    pub fn focusNeighbor(self: *SessionManager, rect: pane.Rect, dir: pane.Dir, buf: []pane.PaneRect) void {
        const tree = self.activeTree() orelse return;
        if (tree.neighbor(rect, self.focused, dir, buf)) |id| self.focused = id;
    }

    /// Grow the focused pane toward `dir` within the active tab.
    pub fn resizeFocused(self: *SessionManager, dir: pane.Dir, step: f32) void {
        const tree = self.activeTree() orelse return;
        tree.resize(self.focused, dir, step);
    }

    /// Reset the active tab's splits to even 50/50.
    pub fn balanceActive(self: *SessionManager) void {
        const tree = self.activeTree() orelse return;
        tree.balance();
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

test "spawnFirst seeds one tab, session, and focus" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    try std.testing.expectEqual(@as(usize, 1), mgr.count());
    try std.testing.expectEqual(@as(usize, 1), mgr.tabs.items.len);
    try std.testing.expect(mgr.focusedSession() != null);
}

test "splitFocused adds a pane in the active tab and shifts focus" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    const first = mgr.focused;
    try mgr.splitFocused(.x, 24, 40);
    try std.testing.expectEqual(@as(usize, 2), mgr.count());
    try std.testing.expectEqual(@as(usize, 2), mgr.activeTree().?.count());
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

test "newTab opens and focuses a second tab" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    try mgr.newTab(24, 80);
    try std.testing.expectEqual(@as(usize, 2), mgr.tabs.items.len);
    try std.testing.expectEqual(@as(usize, 1), mgr.active_tab);
    try std.testing.expectEqual(@as(usize, 2), mgr.count());
}

test "cycleTab wraps and follows focus" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    const tab0_focus = mgr.focused;
    try mgr.newTab(24, 80);
    mgr.cycleTab(1); // wrap back to tab 0
    try std.testing.expectEqual(@as(usize, 0), mgr.active_tab);
    try std.testing.expectEqual(tab0_focus, mgr.focused);
}

test "closeTab kills its sessions and keeps at least one tab" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawnFirst(24, 80);
    try mgr.newTab(24, 80);
    try mgr.splitFocused(.x, 24, 40); // tab 1 has two panes
    try std.testing.expectEqual(@as(usize, 3), mgr.count());
    mgr.closeTab(); // closes active tab 1 (two sessions)
    try std.testing.expectEqual(@as(usize, 1), mgr.tabs.items.len);
    try std.testing.expectEqual(@as(usize, 1), mgr.count());
    mgr.closeTab(); // no-op on last tab
    try std.testing.expectEqual(@as(usize, 1), mgr.tabs.items.len);
}
