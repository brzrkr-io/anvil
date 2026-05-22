//! Terminal tabs: each `Tab` owns a shell; `TabManager` owns the tab list.
//! This file starts with pure list/index helpers so the bookkeeping is unit
//! tested without spawning real shells.

const std = @import("std");
const Terminal = @import("../terminal/terminal.zig").Terminal;
const Pty = @import("../pty/pty.zig").Pty;
const pane_mod = @import("../workspace/pane.zig");
const Pane = pane_mod.Pane;
const PaneRegistry = pane_mod.PaneRegistry;
const layout_mod = @import("../workspace/layout.zig");
const PaneTree = layout_mod.PaneTree;

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

/// One terminal tab: owns a PaneTree (layout) and a PaneRegistry (runtime).
/// Currently always a single-leaf tree. Heap-allocate via `create` so the
/// address is stable.
pub const Tab = struct {
    alloc: std.mem.Allocator,
    tree: PaneTree,
    registry: PaneRegistry,

    /// Create a tab: a `cols x rows` terminal with `scrollback` history and a
    /// shell spawned in `cwd` (or the inherited default when `cwd` is null).
    /// The caller must call `startReader` once the Tab address is final.
    pub fn create(
        alloc: std.mem.Allocator,
        cols: usize,
        rows: usize,
        scrollback: usize,
        cwd: ?[]const u8,
    ) !*Tab {
        const self = try alloc.create(Tab);
        errdefer alloc.destroy(self);

        var registry = PaneRegistry{};
        errdefer registry.deinit(alloc);

        const first_id = try registry.createAndRegister(alloc, cols, rows, scrollback, cwd);

        const tree = try PaneTree.initSingle(alloc, first_id);
        errdefer {
            // tree.deinit() only frees nodes; registry.deinit() frees panes.
            var t = tree;
            t.deinit();
        }

        self.* = .{ .alloc = alloc, .tree = tree, .registry = registry };
        return self;
    }

    /// Spawn reader threads for all panes. Call exactly once, after `create`,
    /// when the Tab pointer is stable.
    pub fn startReader(self: *Tab) !void {
        var it = self.registry.map.valueIterator();
        while (it.next()) |pane_ptr| {
            try pane_ptr.*.startReader();
        }
    }

    /// Return the focused pane. Always valid after `create`.
    pub fn focusedPane(self: *Tab) *Pane {
        const id = self.tree.focused;
        return self.registry.get(id) orelse @panic("focused PaneId not in registry");
    }

    /// Stop all shells + reader threads, free all panes and the tree, free the Tab.
    pub fn deinit(self: *Tab) void {
        self.registry.deinit(self.alloc);
        self.tree.deinit();
        const alloc = self.alloc;
        alloc.destroy(self);
    }

    /// The tab's display label: shell title -> cwd basename -> "shell".
    /// Writes into `out` and returns the used slice.
    pub fn label(self: *const Tab, out: []u8) []const u8 {
        // Label from the focused pane's terminal.
        const id = self.tree.focused;
        const pane = self.registry.get(id) orelse return copyTrunc(out, "shell");
        const title = pane.terminal.title();
        if (title.len > 0) return copyTrunc(out, title);
        const cwd = pane.terminal.cwdPath();
        if (cwd.len > 0) return copyTrunc(out, basename(cwd));
        return copyTrunc(out, "shell");
    }
};

/// A hard cap on tabs — bounds the per-tab thread + 256 KiB buffer cost.
pub const max_tabs = 32;

/// File-scope alias used by TabManager.barVisible to avoid the ambiguous-reference
/// error that arises when the struct method and the module function share a name.
const barVisibleFn = barVisible;

pub const TabManager = struct {
    alloc: std.mem.Allocator,
    tabs: std.ArrayList(*Tab),
    active: usize = 0,

    pub fn init(alloc: std.mem.Allocator) TabManager {
        return .{ .alloc = alloc, .tabs = std.ArrayList(*Tab).empty };
    }

    /// Deinit and free every tab, then the list.
    pub fn deinit(self: *TabManager) void {
        for (self.tabs.items) |tab| tab.deinit();
        self.tabs.deinit(self.alloc);
    }

    pub fn count(self: *const TabManager) usize {
        return self.tabs.items.len;
    }

    pub fn current(self: *TabManager) *Tab {
        return self.tabs.items[self.active];
    }

    pub fn barVisible(self: *const TabManager) bool {
        return barVisibleFn(self.tabs.items.len);
    }

    /// Create a tab, start its reader thread, append it, and make it active.
    /// A no-op (logged) once `max_tabs` is reached.
    pub fn newTab(self: *TabManager, cols: usize, rows: usize, scrollback: usize, cwd: ?[]const u8) !void {
        if (self.tabs.items.len >= max_tabs) {
            std.debug.print("anvil: tab limit ({d}) reached\n", .{max_tabs});
            return;
        }
        const tab = try Tab.create(self.alloc, cols, rows, scrollback, cwd);
        errdefer tab.deinit();
        try tab.startReader();
        try self.tabs.append(self.alloc, tab);
        self.active = self.tabs.items.len - 1;
    }

    /// Close the active tab. Returns true if tabs remain, false if the list is
    /// now empty (the caller should then terminate the app).
    pub fn closeActive(self: *TabManager) bool {
        return self.closeAt(self.active);
    }

    /// Close the tab at `index`. Returns true if tabs remain.
    pub fn closeAt(self: *TabManager, index: usize) bool {
        if (index >= self.tabs.items.len) return self.tabs.items.len > 0;
        const old_count = self.tabs.items.len;
        const tab = self.tabs.orderedRemove(index);
        tab.deinit();
        if (self.tabs.items.len == 0) return false;
        self.active = nextActiveAfterClose(old_count, index, self.active);
        return true;
    }

    pub fn switchTo(self: *TabManager, index: usize) void {
        self.active = clampIndex(self.tabs.items.len, index);
    }

    pub fn next(self: *TabManager) void {
        self.active = wrapIndex(self.tabs.items.len, self.active, 1);
    }

    pub fn prev(self: *TabManager) void {
        self.active = wrapIndex(self.tabs.items.len, self.active, -1);
    }
};

fn copyTrunc(out: []u8, src: []const u8) []const u8 {
    const n = @min(out.len, src.len);
    @memcpy(out[0..n], src[0..n]);
    return out[0..n];
}

/// The last path component of `path`, ignoring a single trailing slash.
fn basename(path: []const u8) []const u8 {
    var p = path;
    if (p.len > 1 and p[p.len - 1] == '/') p = p[0 .. p.len - 1];
    if (std.mem.lastIndexOfScalar(u8, p, '/')) |i| return p[i + 1 ..];
    return p;
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

test "basename extracts the last path component" {
    try testing.expectEqualStrings("anvil", basename("/Users/x/anvil"));
    try testing.expectEqualStrings("anvil", basename("/Users/x/anvil/"));
    try testing.expectEqualStrings("x", basename("x"));
    try testing.expectEqualStrings("", basename("/"));
}

test "label falls back to \"shell\" with no title or cwd" {
    var t = try Terminal.init(testing.allocator, 20, 5, 100);
    defer t.deinit();
    // A fresh terminal has neither an OSC title nor an OSC cwd.
    // Build a minimal Pane on the stack (no PTY, no reader thread).
    var pane = Pane{
        .alloc = testing.allocator,
        .id = 1,
        .terminal = t,
        .pty = undefined,
    };
    // Build a minimal registry pointing at the pane.
    var registry = PaneRegistry{};
    defer registry.map.deinit(testing.allocator);
    try registry.map.put(testing.allocator, 1, &pane);
    // Build a minimal tree.
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    var tab = Tab{ .alloc = testing.allocator, .tree = tree, .registry = registry };
    var buf: [64]u8 = undefined;
    try testing.expectEqualStrings("shell", tab.label(&buf));
}

test "TabManager index logic: switch, next, prev, close" {
    // Build a manager with 3 placeholder tab pointers (never started, never
    // PTY-backed) so only the index bookkeeping is exercised.
    var mgr = TabManager.init(testing.allocator);
    defer mgr.tabs.deinit(testing.allocator); // free the list only, not the fakes

    var fake: [3]Tab = undefined; // addresses only; fields never read
    for (&fake) |*f| try mgr.tabs.append(testing.allocator, f);
    mgr.active = 0;

    mgr.next();
    try testing.expectEqual(@as(usize, 1), mgr.active);
    mgr.prev();
    mgr.prev();
    try testing.expectEqual(@as(usize, 2), mgr.active); // wrapped
    mgr.switchTo(99);
    try testing.expectEqual(@as(usize, 2), mgr.active); // clamped
    mgr.switchTo(0);
    try testing.expectEqual(@as(usize, 0), mgr.active);

    // Removing index 0 while active=0: helper says stay at slot 0.
    _ = mgr.tabs.orderedRemove(0);
    mgr.active = nextActiveAfterClose(3, 0, 0);
    try testing.expectEqual(@as(usize, 0), mgr.active);
    try testing.expectEqual(@as(usize, 2), mgr.count());
}
