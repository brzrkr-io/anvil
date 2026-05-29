const std = @import("std");

/// Read-only client for the local Caldera daemon (127.0.0.1:4175). Polls a few
/// GET endpoints on a background thread and exposes a plain-bytes Snapshot the
/// render thread copies under a mutex. The app must degrade cleanly when the
/// daemon is absent — `.offline` is a normal state, not an error.
const host = "127.0.0.1";
const port = 4175;

pub const Connection = enum { offline, live, disabled };

pub const RowKind = enum { run_passed, run_open, attn_warning, attn_error };

pub const Row = struct {
    kind: RowKind = .run_open,
    text: [120]u8 = undefined,
    len: usize = 0,

    pub fn slice(self: *const Row) []const u8 {
        return self.text[0..self.len];
    }

    fn set(self: *Row, kind: RowKind, comptime fmt: []const u8, args: anytype) void {
        self.kind = kind;
        const s = std.fmt.bufPrint(&self.text, fmt, args) catch self.text[0..self.text.len];
        self.len = s.len;
    }
};

const max_rows = 16;

pub const Snapshot = struct {
    conn: Connection = .offline,
    project: [64]u8 = undefined,
    project_len: usize = 0,
    runs: usize = 0, // count of run rows (they lead `rows`)
    rows: [max_rows]Row = undefined,
    row_count: usize = 0,

    pub fn projectName(self: *const Snapshot) []const u8 {
        return self.project[0..self.project_len];
    }

    fn pushRow(self: *Snapshot) ?*Row {
        if (self.row_count >= max_rows) return null;
        const r = &self.rows[self.row_count];
        self.row_count += 1;
        return r;
    }
};

var g_mutex: std.Thread.Mutex = .{};
var g_snap: Snapshot = .{};
var g_alloc: std.mem.Allocator = undefined;
var g_running = false;

/// Copy the latest snapshot. Safe to call from the render thread.
pub fn get(out: *Snapshot) void {
    g_mutex.lock();
    defer g_mutex.unlock();
    out.* = g_snap;
}

/// Spawn the background poller. No-op if already running.
pub fn start(alloc: std.mem.Allocator) void {
    if (g_running) return;
    g_alloc = alloc;
    g_running = true;
    _ = std.Thread.spawn(.{}, pollLoop, .{}) catch {
        g_running = false;
    };
}

fn pollLoop() void {
    while (g_running) {
        const snap = buildSnapshot();
        g_mutex.lock();
        g_snap = snap;
        g_mutex.unlock();
        std.Thread.sleep(2 * std.time.ns_per_s);
    }
}

/// Issue one GET and return the response body (everything after the header
/// terminator). Caller owns the returned slice.
fn fetch(alloc: std.mem.Allocator, path: []const u8) ![]u8 {
    var stream = try std.net.tcpConnectToHost(alloc, host, port);
    defer stream.close();
    var req: [256]u8 = undefined;
    const r = try std.fmt.bufPrint(&req, "GET {s} HTTP/1.1\r\nHost: {s}\r\nConnection: close\r\n\r\n", .{ path, host });
    try stream.writeAll(r);

    var raw: std.ArrayListUnmanaged(u8) = .empty;
    errdefer raw.deinit(alloc);
    var buf: [4096]u8 = undefined;
    while (true) {
        const n = try stream.read(&buf);
        if (n == 0) break;
        try raw.appendSlice(alloc, buf[0..n]);
    }
    const sep = std.mem.indexOf(u8, raw.items, "\r\n\r\n") orelse return error.BadResponse;
    const body = try alloc.dupe(u8, raw.items[sep + 4 ..]);
    raw.deinit(alloc);
    return body;
}

const ProjectResp = struct {
    project: struct {
        enabled: bool = false,
        project_name: []const u8 = "",
    } = .{},
};

const Event = struct { summary: []const u8 = "" };

const Run = struct {
    agent: []const u8 = "",
    current_step: []const u8 = "",
    backend_status: []const u8 = "",
    events: []Event = &.{},
};

const RunsResp = struct { agent_runs: []Run = &.{} };

const Attention = struct {
    severity: []const u8 = "",
    summary: []const u8 = "",
};

const ActivityResp = struct { attention: []Attention = &.{} };

fn parse(comptime T: type, alloc: std.mem.Allocator, body: []const u8) ?std.json.Parsed(T) {
    return std.json.parseFromSlice(T, alloc, body, .{ .ignore_unknown_fields = true }) catch null;
}

/// Build a fresh snapshot from the daemon. Never errors: a dead daemon yields
/// the default `.offline` snapshot.
fn buildSnapshot() Snapshot {
    var arena = std.heap.ArenaAllocator.init(g_alloc);
    defer arena.deinit();
    const a = arena.allocator();

    var s = Snapshot{};

    // Health: any failure means offline.
    const health = fetch(a, "/health") catch return s;
    if (std.mem.indexOf(u8, health, "\"ok\"") == null) return s;
    s.conn = .live;

    if (fetch(a, "/api/project")) |body| {
        if (parse(ProjectResp, a, body)) |p| {
            defer p.deinit();
            const name = p.value.project.project_name;
            const len = @min(name.len, s.project.len);
            @memcpy(s.project[0..len], name[0..len]);
            s.project_len = len;
            if (!p.value.project.enabled) s.conn = .disabled;
        }
    } else |_| {}

    if (fetch(a, "/api/agent-runs")) |body| {
        if (parse(RunsResp, a, body)) |p| {
            defer p.deinit();
            for (p.value.agent_runs) |run| {
                const row = s.pushRow() orelse break;
                const passed = std.mem.eql(u8, run.backend_status, "passed");
                const task = if (run.events.len > 0) run.events[0].summary else "";
                row.set(if (passed) .run_passed else .run_open, "{s}  {s}  {s}", .{ run.agent, run.current_step, task });
            }
            s.runs = s.row_count;
        }
    } else |_| {}

    if (fetch(a, "/api/activity")) |body| {
        if (parse(ActivityResp, a, body)) |p| {
            defer p.deinit();
            for (p.value.attention) |att| {
                const row = s.pushRow() orelse break;
                const err = std.mem.eql(u8, att.severity, "error");
                row.set(if (err) .attn_error else .attn_warning, "{s}", .{att.summary});
            }
        }
    } else |_| {}

    return s;
}

test "parse agent-runs body into rows" {
    const body =
        \\{"agent_runs":[{"agent":"terry","current_step":"closed_out","backend_status":"passed","events":[{"summary":"ship it"}]}]}
    ;
    const p = parse(RunsResp, std.testing.allocator, body).?;
    defer p.deinit();
    try std.testing.expectEqual(@as(usize, 1), p.value.agent_runs.len);
    try std.testing.expectEqualStrings("terry", p.value.agent_runs[0].agent);
    try std.testing.expectEqualStrings("ship it", p.value.agent_runs[0].events[0].summary);
}

test "parse activity attention, tolerating unknown fields" {
    const body =
        \\{"attention":[{"code":"x","severity":"warning","summary":"open run","extra":1}],"pending_approvals":[]}
    ;
    const p = parse(ActivityResp, std.testing.allocator, body).?;
    defer p.deinit();
    try std.testing.expectEqual(@as(usize, 1), p.value.attention.len);
    try std.testing.expectEqualStrings("warning", p.value.attention[0].severity);
}

test "Row.set truncates and exposes a slice" {
    var r = Row{};
    r.set(.run_passed, "{s}", .{"hello"});
    try std.testing.expectEqualStrings("hello", r.slice());
    try std.testing.expectEqual(RowKind.run_passed, r.kind);
}
