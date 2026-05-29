const std = @import("std");
const Session = @import("session.zig").Session;

/// Owns the live terminal sessions and tracks which one has focus. Every
/// C-ABI export routes through the focused session.
pub const SessionManager = struct {
    alloc: std.mem.Allocator,
    sessions: std.ArrayListUnmanaged(Session) = .empty,
    focused: usize = 0,

    pub fn deinit(self: *SessionManager) void {
        for (self.sessions.items) |*s| s.deinit();
        self.sessions.deinit(self.alloc);
    }

    pub fn spawn(self: *SessionManager, rows: u16, cols: u16) !void {
        const s = try Session.init(self.alloc, rows, cols);
        try self.sessions.append(self.alloc, s);
        self.focused = self.sessions.items.len - 1;
    }

    pub fn focusedSession(self: *SessionManager) ?*Session {
        if (self.focused >= self.sessions.items.len) return null;
        return &self.sessions.items[self.focused];
    }
};

test "spawn adds a focused session" {
    var mgr = SessionManager{ .alloc = std.testing.allocator };
    defer mgr.deinit();
    try mgr.spawn(24, 80);
    try std.testing.expectEqual(@as(usize, 1), mgr.sessions.items.len);
    try std.testing.expect(mgr.focusedSession() != null);
}
