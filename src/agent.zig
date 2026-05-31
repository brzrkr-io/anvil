const std = @import("std");

/// Provider-independent model for the agent activity surface (the right context
/// drawer: RUNS / TRACE / AGENT). Any backend — Caldera, Claude Code, Codex,
/// aider, … — fills this same `Snapshot`; the render path never names a
/// specific provider. A provider is registered with `use` and polled through
/// `start` / `get`. With no provider the surface stays `.offline`, a normal
/// state rather than an error.
pub const Connection = enum { offline, live, disabled };

pub const RowKind = enum { run_passed, run_open, attn_warning, attn_error };

pub const Row = struct {
    kind: RowKind = .run_open,
    text: [120]u8 = undefined,
    len: usize = 0,

    pub fn slice(self: *const Row) []const u8 {
        return self.text[0..self.len];
    }

    pub fn set(self: *Row, kind: RowKind, comptime fmt: []const u8, args: anytype) void {
        self.kind = kind;
        const s = std.fmt.bufPrint(&self.text, fmt, args) catch self.text[0..self.text.len];
        self.len = s.len;
    }
};

const max_rows = 16;
const max_events_per_run = 8;
const event_summary_max = 64;
const run_field_max = 48;

pub const EventSummary = struct {
    text: [event_summary_max]u8 = undefined,
    len: usize = 0,

    pub fn slice(self: *const EventSummary) []const u8 {
        return self.text[0..self.len];
    }

    pub fn set(self: *EventSummary, s: []const u8) void {
        const n = @min(s.len, event_summary_max);
        @memcpy(self.text[0..n], s[0..n]);
        self.len = n;
    }
};

pub const RunDetail = struct {
    agent: [run_field_max]u8 = undefined,
    agent_len: usize = 0,
    step: [run_field_max]u8 = undefined,
    step_len: usize = 0,
    status: [run_field_max]u8 = undefined,
    status_len: usize = 0,
    events: [max_events_per_run]EventSummary = undefined,
    event_count: usize = 0,

    pub fn agentSlice(self: *const RunDetail) []const u8 {
        return self.agent[0..self.agent_len];
    }
    pub fn stepSlice(self: *const RunDetail) []const u8 {
        return self.step[0..self.step_len];
    }
    pub fn statusSlice(self: *const RunDetail) []const u8 {
        return self.status[0..self.status_len];
    }

    pub fn setField(buf: []u8, len_out: *usize, s: []const u8) void {
        const n = @min(s.len, buf.len);
        @memcpy(buf[0..n], s[0..n]);
        len_out.* = n;
    }
};

pub const Snapshot = struct {
    conn: Connection = .offline,
    project: [64]u8 = undefined,
    project_len: usize = 0,
    runs: usize = 0, // count of run rows (they lead `rows`)
    rows: [max_rows]Row = undefined,
    row_count: usize = 0,
    details: [max_rows]RunDetail = undefined,

    pub fn projectName(self: *const Snapshot) []const u8 {
        return self.project[0..self.project_len];
    }

    pub fn pushRow(self: *Snapshot) ?*Row {
        if (self.row_count >= max_rows) return null;
        const r = &self.rows[self.row_count];
        self.row_count += 1;
        return r;
    }
};

/// An agent backend. `start` spawns its poller; `get` copies the latest
/// snapshot (both safe to call from the render thread per the provider's own
/// synchronization).
pub const Provider = struct {
    name: []const u8,
    start: *const fn (std.mem.Allocator) void,
    get: *const fn (*Snapshot) void,
};

var g_provider: ?Provider = null;

/// Register the active provider. Last writer wins; Caldera is registered as the
/// default at startup, but any adapter can replace it.
pub fn use(p: Provider) void {
    g_provider = p;
}

/// Start the active provider's poller. No-op when none is registered.
pub fn start(alloc: std.mem.Allocator) void {
    if (g_provider) |p| p.start(alloc);
}

/// Copy the latest snapshot from the active provider, or a default `.offline`
/// snapshot when none is registered.
pub fn get(out: *Snapshot) void {
    if (g_provider) |p| p.get(out) else out.* = .{};
}

test "no provider yields an offline snapshot" {
    g_provider = null;
    var s = Snapshot{};
    s.conn = .live;
    get(&s);
    try std.testing.expectEqual(Connection.offline, s.conn);
}

test "registered provider feeds the snapshot through get" {
    const Stub = struct {
        fn start(_: std.mem.Allocator) void {}
        fn get(out: *Snapshot) void {
            out.* = .{};
            out.conn = .live;
            const row = out.pushRow().?;
            row.set(.run_passed, "{s}", .{"ok"});
            out.runs = out.row_count;
        }
    };
    use(.{ .name = "stub", .start = Stub.start, .get = Stub.get });
    defer g_provider = null;
    var s = Snapshot{};
    get(&s);
    try std.testing.expectEqual(Connection.live, s.conn);
    try std.testing.expectEqual(@as(usize, 1), s.runs);
    try std.testing.expectEqualStrings("ok", s.rows[0].slice());
}

test "Row.set truncates and exposes a slice" {
    var r = Row{};
    r.set(.run_passed, "{s}", .{"hello"});
    try std.testing.expectEqualStrings("hello", r.slice());
    try std.testing.expectEqual(RowKind.run_passed, r.kind);
}
