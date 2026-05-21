//! Terminal tabs: each `Tab` owns a shell; `TabManager` owns the tab list.
//! This file starts with pure list/index helpers so the bookkeeping is unit
//! tested without spawning real shells.

const std = @import("std");
const Terminal = @import("../terminal/terminal.zig").Terminal;
const Pty = @import("../pty/pty.zig").Pty;

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

/// One terminal tab: a shell, its model, and the PTY->main-thread handoff.
/// Heap-allocate via `create` so the address is stable for the reader thread.
pub const Tab = struct {
    alloc: std.mem.Allocator,
    terminal: Terminal,
    pty: Pty,

    // PTY -> main handoff. The reader thread appends to `buf`; the 60 Hz tick
    // drains it. Same design as M1's module globals, now per-tab.
    buf: [256 * 1024]u8 = undefined,
    len: usize = 0,
    mutex: std.atomic.Mutex = .unlocked, // match M1's lock type exactly
    dead: bool = false,
    closing: bool = false,
    reader: ?std.Thread = null,

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
        // Separate statements with their own errdefer — a struct literal would
        // leak the Terminal if spawnIn failed after Terminal.init succeeded.
        var terminal = try Terminal.init(alloc, cols, rows, scrollback);
        errdefer terminal.deinit();
        const pty = try spawnIn(alloc, @intCast(cols), @intCast(rows), cwd);
        self.* = .{ .alloc = alloc, .terminal = terminal, .pty = pty };
        return self;
    }

    /// Spawn the reader thread. Call exactly once, after `create`, when the
    /// Tab pointer is stable.
    pub fn startReader(self: *Tab) !void {
        self.reader = try std.Thread.spawn(.{}, readerLoop, .{self});
    }

    /// Stop the shell + reader thread, free the terminal, free the Tab.
    pub fn deinit(self: *Tab) void {
        self.lock();
        self.closing = true;
        self.unlock();
        self.pty.deinit(); // closes the master fd -> read() returns Eof
        if (self.reader) |t| t.join(); // inner-loop escape (closing) or Eof -> thread exits
        self.terminal.deinit();
        const alloc = self.alloc;
        alloc.destroy(self);
    }

    fn lock(self: *Tab) void {
        while (!self.mutex.tryLock()) std.Thread.yield() catch {};
    }
    fn unlock(self: *Tab) void {
        self.mutex.unlock();
    }

    /// Move newly-read PTY bytes out of the handoff buffer into `dst`.
    /// Returns the slice of `dst` that was filled. Call from the main thread.
    pub fn drain(self: *Tab, dst: []u8) []u8 {
        self.lock();
        defer self.unlock();
        const n = @min(self.len, dst.len);
        @memcpy(dst[0..n], self.buf[0..n]);
        self.len = 0;
        return dst[0..n];
    }

    /// True once the shell has exited.
    pub fn isDead(self: *Tab) bool {
        self.lock();
        defer self.unlock();
        return self.dead;
    }

    /// The tab's display label: shell title -> cwd basename -> "shell".
    /// Writes into `out` and returns the used slice.
    pub fn label(self: *const Tab, out: []u8) []const u8 {
        const title = self.terminal.title();
        if (title.len > 0) return copyTrunc(out, title);
        const cwd = self.terminal.cwd();
        if (cwd.len > 0) return copyTrunc(out, basename(cwd));
        return copyTrunc(out, "shell");
    }
};

/// PTY reader thread body — blocking reads appended to the tab's handoff buffer.
fn readerLoop(tab: *Tab) void {
    var local: [64 * 1024]u8 = undefined;
    outer: while (true) {
        const n = tab.pty.read(&local) catch break;
        if (n == 0) break;
        var off: usize = 0;
        while (off < n) {
            tab.lock();
            if (tab.closing) {
                tab.unlock();
                break :outer;
            }
            const space = tab.buf.len - tab.len;
            if (space == 0) {
                tab.unlock();
                std.Thread.yield() catch {};
                continue;
            }
            const take = @min(space, n - off);
            @memcpy(tab.buf[tab.len..][0..take], local[off..][0..take]);
            tab.len += take;
            tab.unlock();
            off += take;
        }
    }
    tab.lock();
    tab.dead = true;
    tab.unlock();
}

/// Spawn a shell, in `cwd` when given. Pty has no cwd parameter, so when a cwd
/// is requested this temporarily chdir's the process around the spawn (the
/// child inherits cwd at fork). Tab creation is main-thread-only, so the
/// transient process-wide chdir is safe.
/// Uses std.c.chdir / std.c.getcwd — std.process.changeCurDir does not exist
/// in Zig 0.16 (the process API requires an Io instance in that version).
fn spawnIn(alloc: std.mem.Allocator, cols: u16, rows: u16, cwd: ?[]const u8) !Pty {
    if (cwd) |dir| {
        // Save the current working directory. Only proceed with the chdir-spawn-
        // restore dance when getcwd succeeds; if it fails we cannot guarantee a
        // restore, so fall through to the no-cwd spawn below.
        var saved_buf: [std.fs.max_path_bytes]u8 = undefined;
        const saved_ptr = std.c.getcwd(&saved_buf, saved_buf.len);
        if (saved_ptr != null) {
            // Null-terminate the requested dir for the C call.
            var dir_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
            if (dir.len >= dir_buf.len) return error.NameTooLong;
            @memcpy(dir_buf[0..dir.len], dir);
            dir_buf[dir.len] = 0;
            _ = std.c.chdir(@ptrCast(dir_buf[0..dir.len :0])); // best-effort
            const pty = Pty.spawnShell(alloc, cols, rows);
            // Restore the saved cwd (guaranteed to have been saved).
            const saved = std.mem.span(@as([*:0]const u8, @ptrCast(&saved_buf)));
            var restore_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
            if (saved.len < restore_buf.len) {
                @memcpy(restore_buf[0..saved.len], saved);
                restore_buf[saved.len] = 0;
                _ = std.c.chdir(@ptrCast(restore_buf[0..saved.len :0]));
            }
            return pty;
        }
    }
    return Pty.spawnShell(alloc, cols, rows);
}

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
    /// A no-op (logged) once `max_tabs` is reached. The reader starts *before*
    /// the append so a failed append/startReader never leaves a freed tab in
    /// the list (the heap pointer from `create` is already stable).
    pub fn newTab(self: *TabManager, cols: usize, rows: usize, scrollback: usize, cwd: ?[]const u8) !void {
        if (self.tabs.items.len >= max_tabs) {
            std.debug.print("caldera-console: tab limit ({d}) reached\n", .{max_tabs});
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
    try testing.expectEqualStrings("caldera-console", basename("/Users/x/caldera-console"));
    try testing.expectEqualStrings("caldera-console", basename("/Users/x/caldera-console/"));
    try testing.expectEqualStrings("x", basename("x"));
    try testing.expectEqualStrings("", basename("/"));
}

test "label falls back to \"shell\" with no title or cwd" {
    var t = try Terminal.init(testing.allocator, 20, 5, 100);
    defer t.deinit();
    // A fresh terminal has neither an OSC title nor an OSC cwd.
    var tab = Tab{ .alloc = testing.allocator, .terminal = t, .pty = undefined };
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
