//! Pane: one terminal viewport — owns a Terminal, a Pty, and the
//! PTY->main-thread reader handoff. Heap-allocated via `create` so the
//! address is stable for the reader thread.
//!
//! Per-pane view state (scroll position, animated cursor, selection) lives
//! here so that when multiple panes exist each has its own independent state.

const std = @import("std");
const Terminal = @import("../terminal/terminal.zig").Terminal;
const Pty = @import("../pty/pty.zig").Pty;
const Selection = @import("../app/selection.zig").Selection;
const layout = @import("layout.zig");
pub const PaneId = layout.PaneId;

pub const Pane = struct {
    alloc: std.mem.Allocator,
    id: PaneId,
    terminal: Terminal,
    pty: Pty,

    // PTY -> main handoff. The reader thread appends to `buf`; the 60 Hz tick
    // drains it. Same design as the original Tab handoff, now per-pane.
    buf: [256 * 1024]u8 = undefined,
    len: usize = 0,
    mutex: std.atomic.Mutex = .unlocked,
    dead: bool = false,
    closing: bool = false,
    reader: ?std.Thread = null,

    // Per-pane view state — defaulted here, animated by the main thread.
    scroll_pos: f32 = 0,
    overscroll: f32 = 0,
    overscroll_target: f32 = 0,
    cursor_ax: f32 = 0,
    cursor_ay: f32 = 0,
    selection: Selection = .{},

    /// Create a pane: a `cols x rows` terminal with `scrollback` history and
    /// a shell spawned in `cwd` (or the inherited default when `cwd` is null).
    /// The caller must call `startReader` once the Pane address is final.
    pub fn create(
        alloc: std.mem.Allocator,
        id: PaneId,
        cols: usize,
        rows: usize,
        scrollback: usize,
        cwd: ?[]const u8,
    ) !*Pane {
        const self = try alloc.create(Pane);
        errdefer alloc.destroy(self);
        var terminal = try Terminal.init(alloc, cols, rows, scrollback);
        errdefer terminal.deinit();
        const pty = try spawnIn(alloc, @intCast(cols), @intCast(rows), cwd);
        self.* = .{ .alloc = alloc, .id = id, .terminal = terminal, .pty = pty };
        return self;
    }

    /// Spawn the reader thread. Call exactly once, after `create`, when the
    /// Pane pointer is stable.
    pub fn startReader(self: *Pane) !void {
        self.reader = try std.Thread.spawn(.{}, readerLoop, .{self});
    }

    /// Stop the shell + reader thread, free the terminal, free the Pane.
    pub fn deinit(self: *Pane) void {
        self.lock();
        self.closing = true;
        self.unlock();
        self.pty.deinit(); // closes the master fd -> read() returns Eof
        if (self.reader) |t| t.join();
        self.terminal.deinit();
        const alloc = self.alloc;
        alloc.destroy(self);
    }

    fn lock(self: *Pane) void {
        while (!self.mutex.tryLock()) std.Thread.yield() catch {};
    }
    fn unlock(self: *Pane) void {
        self.mutex.unlock();
    }

    /// Move newly-read PTY bytes out of the handoff buffer into `dst`.
    /// Returns the slice of `dst` that was filled. Call from the main thread.
    pub fn drain(self: *Pane, dst: []u8) []u8 {
        self.lock();
        defer self.unlock();
        const n = @min(self.len, dst.len);
        @memcpy(dst[0..n], self.buf[0..n]);
        self.len = 0;
        return dst[0..n];
    }

    /// True once the shell has exited.
    pub fn isDead(self: *Pane) bool {
        self.lock();
        defer self.unlock();
        return self.dead;
    }
};

/// PTY reader thread body — blocking reads appended to the pane's handoff buffer.
fn readerLoop(pane: *Pane) void {
    var local: [64 * 1024]u8 = undefined;
    outer: while (true) {
        const n = pane.pty.read(&local) catch break;
        if (n == 0) break;
        var off: usize = 0;
        while (off < n) {
            pane.lock();
            if (pane.closing) {
                pane.unlock();
                break :outer;
            }
            const space = pane.buf.len - pane.len;
            if (space == 0) {
                pane.unlock();
                std.Thread.yield() catch {};
                continue;
            }
            const take = @min(space, n - off);
            @memcpy(pane.buf[pane.len..][0..take], local[off..][0..take]);
            pane.len += take;
            pane.unlock();
            off += take;
        }
    }
    pane.lock();
    pane.dead = true;
    pane.unlock();
}

/// A registry of all panes owned by one tab. `remove` is the ONLY path that
/// calls `Pane.deinit` — callers must not deinit panes directly.
pub const PaneRegistry = struct {
    map: std.AutoHashMapUnmanaged(PaneId, *Pane) = .{},
    next_id: PaneId = 1,

    /// Allocate a fresh PaneId, create the Pane, register it, and return the id.
    /// The caller must call `startReader` on the returned pane when its address
    /// is stable (i.e., after `Tab.create` returns).
    pub fn createAndRegister(
        self: *PaneRegistry,
        alloc: std.mem.Allocator,
        cols: usize,
        rows: usize,
        scrollback: usize,
        cwd: ?[]const u8,
    ) !PaneId {
        const id = self.next_id;
        self.next_id += 1;
        const pane = try Pane.create(alloc, id, cols, rows, scrollback, cwd);
        errdefer pane.deinit();
        try self.map.put(alloc, id, pane);
        return id;
    }

    /// Look up a pane by id. Returns null if not found.
    pub fn get(self: *const PaneRegistry, id: PaneId) ?*Pane {
        return self.map.get(id);
    }

    /// Deinit and remove the pane with `id`. No-op if `id` is not present.
    pub fn remove(self: *PaneRegistry, id: PaneId) void {
        if (self.map.fetchRemove(id)) |kv| {
            kv.value.deinit();
        }
    }

    /// Deinit all panes and free the map. Call from Tab.deinit.
    pub fn deinit(self: *PaneRegistry, alloc: std.mem.Allocator) void {
        var it = self.map.valueIterator();
        while (it.next()) |pane_ptr| {
            pane_ptr.*.deinit();
        }
        self.map.deinit(alloc);
    }
};

/// Spawn a shell in `cwd` when given. Temporarily chdir's the process around
/// the spawn so the child inherits the cwd at fork. Tab/pane creation is
/// main-thread-only, so the transient process-wide chdir is safe.
fn spawnIn(alloc: std.mem.Allocator, cols: u16, rows: u16, cwd: ?[]const u8) !Pty {
    if (cwd) |dir| {
        var saved_buf: [std.fs.max_path_bytes]u8 = undefined;
        const saved_ptr = std.c.getcwd(&saved_buf, saved_buf.len);
        if (saved_ptr != null) {
            var dir_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
            if (dir.len >= dir_buf.len) return error.NameTooLong;
            @memcpy(dir_buf[0..dir.len], dir);
            dir_buf[dir.len] = 0;
            _ = std.c.chdir(@ptrCast(dir_buf[0..dir.len :0]));
            const pty = Pty.spawnShell(alloc, cols, rows);
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
