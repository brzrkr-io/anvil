const std = @import("std");
const agent = @import("agent.zig");

const c = @cImport({
    @cInclude("sys/socket.h");
    @cInclude("netinet/in.h");
    @cInclude("arpa/inet.h");
    @cInclude("unistd.h");
});

/// Caldera adapter for the agent activity surface. Implements the
/// `agent.Provider` seam: polls a few GET endpoints on the local Caldera daemon
/// (127.0.0.1:4175) on a background thread and exposes the neutral
/// `agent.Snapshot` the render thread copies under a mutex. The app must
/// degrade cleanly when the daemon is absent — `.offline` is a normal state,
/// not an error. Caldera is just one provider; the surface is provider-generic.
const host = "127.0.0.1";
const port = 4175;

const max_events_per_run = 8;

/// The Caldera implementation of the agent provider seam.
pub const provider = agent.Provider{ .name = "caldera", .start = start, .get = get };

var g_mutex: std.c.pthread_mutex_t = std.c.PTHREAD_MUTEX_INITIALIZER;
var g_snap: agent.Snapshot = .{};
var g_alloc: std.mem.Allocator = undefined;
var g_running = false;

/// Copy the latest snapshot. Safe to call from the render thread.
pub fn get(out: *agent.Snapshot) void {
    _ = std.c.pthread_mutex_lock(&g_mutex);
    defer _ = std.c.pthread_mutex_unlock(&g_mutex);
    out.* = g_snap;
}

/// Spawn the background poller. No-op if already running.
pub fn start(alloc: std.mem.Allocator) void {
    if (g_running) return;
    g_alloc = alloc;
    g_running = true;
    const t = std.Thread.spawn(.{}, pollLoop, .{}) catch {
        g_running = false;
        return;
    };
    t.detach();
}

fn pollLoop() void {
    while (g_running) {
        const snap = buildSnapshot();
        _ = std.c.pthread_mutex_lock(&g_mutex);
        g_snap = snap;
        _ = std.c.pthread_mutex_unlock(&g_mutex);
        const ts = std.c.timespec{ .sec = 2, .nsec = 0 };
        _ = std.c.nanosleep(&ts, null);
    }
}

/// Issue one GET and return the response body (everything after the header
/// terminator). Caller owns the returned slice.
fn fetch(alloc: std.mem.Allocator, path: []const u8) ![]u8 {
    const fd = c.socket(c.AF_INET, c.SOCK_STREAM, 0);
    if (fd < 0) return error.Socket;
    defer _ = c.close(fd);

    // 200ms send/recv timeout so a missing daemon doesn't stall the poll thread.
    const tv = c.struct_timeval{ .tv_sec = 0, .tv_usec = 200_000 };
    _ = c.setsockopt(fd, c.SOL_SOCKET, c.SO_SNDTIMEO, &tv, @sizeOf(c.struct_timeval));
    _ = c.setsockopt(fd, c.SOL_SOCKET, c.SO_RCVTIMEO, &tv, @sizeOf(c.struct_timeval));

    var addr: c.struct_sockaddr_in = std.mem.zeroes(c.struct_sockaddr_in);
    addr.sin_family = c.AF_INET;
    addr.sin_port = c.htons(port);
    addr.sin_addr.s_addr = c.inet_addr("127.0.0.1");

    if (c.connect(fd, @ptrCast(&addr), @sizeOf(c.struct_sockaddr_in)) != 0)
        return error.ConnectFailed;

    var req: [256]u8 = undefined;
    const r = try std.fmt.bufPrint(&req, "GET {s} HTTP/1.1\r\nHost: {s}\r\nConnection: close\r\n\r\n", .{ path, host });
    if (c.write(fd, r.ptr, r.len) < 0) return error.WriteFailed;

    var raw: std.ArrayListUnmanaged(u8) = .empty;
    errdefer raw.deinit(alloc);
    var buf: [4096]u8 = undefined;
    while (true) {
        const n = c.read(fd, &buf, buf.len);
        if (n <= 0) break;
        try raw.appendSlice(alloc, buf[0..@intCast(n)]);
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
fn buildSnapshot() agent.Snapshot {
    var arena = std.heap.ArenaAllocator.init(g_alloc);
    defer arena.deinit();
    const a = arena.allocator();

    var s = agent.Snapshot{};

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
                const idx = s.row_count;
                const row = s.pushRow() orelse break;
                const passed = std.mem.eql(u8, run.backend_status, "passed");
                const task = if (run.events.len > 0) run.events[0].summary else "";
                row.set(if (passed) .run_passed else .run_open, "{s}  {s}  {s}", .{ run.agent, run.current_step, task });
                var d = &s.details[idx];
                d.* = .{};
                agent.RunDetail.setField(&d.agent, &d.agent_len, run.agent);
                agent.RunDetail.setField(&d.step, &d.step_len, run.current_step);
                agent.RunDetail.setField(&d.status, &d.status_len, run.backend_status);
                const n_ev = @min(run.events.len, max_events_per_run);
                for (run.events[0..n_ev], 0..) |ev, ei| {
                    d.events[ei].set(ev.summary);
                }
                d.event_count = n_ev;
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

test "buildSnapshot retains all capped event summaries in order" {
    const body =
        \\{"agent_runs":[{"agent":"ops","current_step":"running","backend_status":"open",
        \\ "events":[{"summary":"first"},{"summary":"second"},{"summary":"third"}]}]}
    ;
    const p = parse(RunsResp, std.testing.allocator, body).?;
    defer p.deinit();
    const run = p.value.agent_runs[0];
    var d = agent.RunDetail{};
    agent.RunDetail.setField(&d.agent, &d.agent_len, run.agent);
    agent.RunDetail.setField(&d.step, &d.step_len, run.current_step);
    agent.RunDetail.setField(&d.status, &d.status_len, run.backend_status);
    const n_ev = @min(run.events.len, max_events_per_run);
    for (run.events[0..n_ev], 0..) |ev, ei| d.events[ei].set(ev.summary);
    d.event_count = n_ev;

    try std.testing.expectEqualStrings("ops", d.agentSlice());
    try std.testing.expectEqualStrings("running", d.stepSlice());
    try std.testing.expectEqual(@as(usize, 3), d.event_count);
    try std.testing.expectEqualStrings("first", d.events[0].slice());
    try std.testing.expectEqualStrings("second", d.events[1].slice());
    try std.testing.expectEqualStrings("third", d.events[2].slice());
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
