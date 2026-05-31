// IPC server for a running Anvil window.  Each window process binds its own
// Unix domain socket at $TMPDIR/anvil-<uid>-<pid>.sock, so `anvil split|tab|
// run|pipe` can drive an ALREADY-RUNNING window from the shell.
//
// Per-window routing: every window runs its own listener (one socket per pid),
// preserving the process-per-window crash isolation.  The client scans the
// directory and targets the newest-by-mtime socket — the front window — or an
// explicit `--window <pid>`.  A window bumps its socket mtime on becoming
// active (touchFocus), so "front window" follows focus.  Sockets whose owner
// has died are reaped on the failed connect and the scan retried.
//
// Threading: the listener runs on its own thread.  It MUST NOT call any
// app.zig exports directly — those assume the AppKit main thread and touch
// render state.  Instead the listener pushes Command values onto a bounded
// queue; app.zig drains the queue at the top of anvil_poll() on the main
// thread.

const std = @import("std");
const pane = @import("workspace/pane_tree.zig");

const c = @cImport({
    @cInclude("sys/socket.h");
    @cInclude("sys/un.h");
    @cInclude("unistd.h");
    @cInclude("errno.h");
    @cInclude("fcntl.h");
    @cInclude("dirent.h");
    @cInclude("sys/stat.h");
    @cInclude("sys/time.h");
    @cInclude("signal.h");
});

// ---------------------------------------------------------------------------
// Command queue — crossed from listener thread to main thread.
// ---------------------------------------------------------------------------

const path_max = 1024;

pub const TabArg = struct {
    path: [path_max]u8 = undefined,
    len: usize = 0,
    has_path: bool = false,
};

pub const RunArg = struct {
    cmd: [path_max]u8 = undefined,
    len: usize = 0,
};

pub const ViewArg = struct {
    path: [path_max]u8 = undefined,
    len: usize = 0,
};

pub const Command = union(enum) {
    split: pane.Axis,
    tab: TabArg,
    run: RunArg,
    view: ViewArg,
};

var q_mutex: std.c.pthread_mutex_t = std.c.PTHREAD_MUTEX_INITIALIZER;
var queue: [32]Command = undefined;
var q_len: usize = 0;

fn push(cmd: Command) void {
    _ = std.c.pthread_mutex_lock(&q_mutex);
    defer _ = std.c.pthread_mutex_unlock(&q_mutex);
    if (q_len < queue.len) {
        queue[q_len] = cmd;
        q_len += 1;
    }
}

/// Drain all pending commands into `out`.  Returns the count copied.
/// Safe to call from the AppKit main thread only.
pub fn takeCommands(out: []Command) usize {
    _ = std.c.pthread_mutex_lock(&q_mutex);
    defer _ = std.c.pthread_mutex_unlock(&q_mutex);
    const n = @min(q_len, out.len);
    @memcpy(out[0..n], queue[0..n]);
    q_len = 0;
    return n;
}

// ---------------------------------------------------------------------------
// Socket paths
//
// Each window process owns its OWN socket: <tmpdir>anvil-<uid>-<pid>.sock.
// This keeps the process-per-window model (and its crash isolation) while
// letting the CLI reach ANY window, not just the first.  The client scans the
// directory, picks a target (newest mtime = front window, or an explicit
// --window <pid>), and reaps sockets whose owner has died.
// ---------------------------------------------------------------------------

const sock_max = 108; // sun_path limit

var sock_path_buf: [sock_max]u8 = undefined; // this process's server socket
var sock_path_len: usize = 0;
var sock_path_z: [sock_max + 1]u8 = undefined; // null-terminated copy for the signal handler

/// SIGTERM/SIGINT handler: unlink our socket so it doesn't linger, then exit.
/// unlink + _exit are async-signal-safe.
fn onTerm(_: c_int) callconv(.c) void {
    if (sock_path_len > 0) _ = c.unlink(&sock_path_z);
    std.c._exit(0);
}

fn tmpDir() [*:0]const u8 {
    return std.c.getenv("TMPDIR") orelse "/tmp/";
}

/// This process's server socket path: <tmpdir>anvil-<uid>-<pid>.sock.
fn buildServerSockPath(buf: []u8) []const u8 {
    const uid = std.c.getuid();
    const pid = c.getpid();
    return std.fmt.bufPrint(buf, "{s}anvil-{d}-{d}.sock", .{ std.mem.span(tmpDir()), uid, pid }) catch buf[0..0];
}

/// Filename prefix shared by all of this user's window sockets ("anvil-<uid>-").
fn sockPrefix(buf: []u8) []const u8 {
    return std.fmt.bufPrint(buf, "anvil-{d}-", .{std.c.getuid()}) catch buf[0..0];
}

// ---------------------------------------------------------------------------
// Request parsing
// ---------------------------------------------------------------------------

fn parseRequest(line: []const u8) ?Command {
    const trimmed = std.mem.trimEnd(u8, line, "\n\r");
    if (std.mem.eql(u8, trimmed, "split h")) return Command{ .split = .x };
    if (std.mem.eql(u8, trimmed, "split v")) return Command{ .split = .y };
    if (std.mem.eql(u8, trimmed, "tab")) return Command{ .tab = .{} };
    if (std.mem.startsWith(u8, trimmed, "tab ")) {
        const path = trimmed[4..];
        var arg = TabArg{ .has_path = true };
        const n = @min(path.len, path_max);
        @memcpy(arg.path[0..n], path[0..n]);
        arg.len = n;
        return Command{ .tab = arg };
    }
    if (std.mem.startsWith(u8, trimmed, "run ")) {
        const cmd = trimmed[4..];
        if (cmd.len == 0) return null;
        var arg = RunArg{};
        const n = @min(cmd.len, path_max);
        @memcpy(arg.cmd[0..n], cmd[0..n]);
        arg.len = n;
        return Command{ .run = arg };
    }
    if (std.mem.startsWith(u8, trimmed, "view ")) {
        const vpath = trimmed[5..];
        if (vpath.len == 0) return null;
        var arg = ViewArg{};
        const n = @min(vpath.len, path_max);
        @memcpy(arg.path[0..n], vpath[0..n]);
        arg.len = n;
        return Command{ .view = arg };
    }
    return null;
}

// ---------------------------------------------------------------------------
// Listener loop
// ---------------------------------------------------------------------------

var server_fd: c_int = -1;

fn listenLoop() void {
    while (true) {
        const client = c.accept(server_fd, null, null);
        if (client < 0) break;
        var req_buf: [1024]u8 = undefined;
        var req_len: usize = 0;
        // Read until newline or buffer full.
        while (req_len < req_buf.len) {
            const n = c.read(client, req_buf[req_len..].ptr, 1);
            if (n <= 0) break;
            req_len += 1;
            if (req_buf[req_len - 1] == '\n') break;
        }
        const line = req_buf[0..req_len];
        if (parseRequest(line)) |cmd| {
            push(cmd);
            _ = c.write(client, "ok\n", 3);
        } else {
            const msg = "err unknown verb\n";
            _ = c.write(client, msg.ptr, msg.len);
        }
        _ = c.close(client);
    }
}

// ---------------------------------------------------------------------------
// Server start  (called once from app.zig after GUI is ready)
// ---------------------------------------------------------------------------

pub fn start() void {
    const path = buildServerSockPath(&sock_path_buf);
    sock_path_len = path.len;

    const fd = c.socket(c.AF_UNIX, c.SOCK_STREAM, 0);
    if (fd < 0) return;

    var addr: c.struct_sockaddr_un = std.mem.zeroes(c.struct_sockaddr_un);
    addr.sun_family = c.AF_UNIX;
    const copy_n = @min(path.len, addr.sun_path.len - 1);
    @memcpy(addr.sun_path[0..copy_n], path[0..copy_n]);
    addr.sun_path[copy_n] = 0;

    // Our path is pid-unique, so a leftover file can only be a crashed prior
    // run that reused our pid — unlink it, then bind.
    var z: [sock_max + 1]u8 = undefined;
    @memcpy(z[0..copy_n], path[0..copy_n]);
    z[copy_n] = 0;
    _ = c.unlink(&z);

    if (c.bind(fd, @ptrCast(&addr), @sizeOf(c.struct_sockaddr_un)) != 0) {
        _ = c.close(fd);
        return;
    }
    if (c.listen(fd, 8) != 0) {
        _ = c.close(fd);
        return;
    }

    // Keep a null-terminated copy and unlink the socket on SIGTERM/SIGINT so it
    // doesn't linger for the client to reap.
    @memcpy(sock_path_z[0..copy_n], path[0..copy_n]);
    sock_path_z[copy_n] = 0;
    _ = c.signal(c.SIGTERM, onTerm);
    _ = c.signal(c.SIGINT, onTerm);

    server_fd = fd;
    const t = std.Thread.spawn(.{}, listenLoop, .{}) catch {
        _ = c.close(fd);
        server_fd = -1;
        return;
    };
    t.detach();
}

/// Mark this window as most-recently-focused by bumping its socket's mtime.
/// Called from the AppKit main thread when the app becomes active.
pub fn touchFocus() void {
    if (sock_path_len == 0) return;
    var z: [sock_max + 1]u8 = undefined;
    @memcpy(z[0..sock_path_len], sock_path_buf[0..sock_path_len]);
    z[sock_path_len] = 0;
    _ = c.utimes(&z, null); // null → set atime/mtime to now
}

// ---------------------------------------------------------------------------
// Client mode  (called from main.zig before opening a window)
// ---------------------------------------------------------------------------

/// Extract the pid from a socket filename "anvil-<uid>-<pid>.sock".
fn pidFromName(name: []const u8, prefix: []const u8) ?u32 {
    if (!std.mem.startsWith(u8, name, prefix)) return null;
    if (!std.mem.endsWith(u8, name, ".sock")) return null;
    const mid = name[prefix.len .. name.len - ".sock".len];
    return std.fmt.parseInt(u32, mid, 10) catch null;
}

/// Choose a target socket path into `out`.  With `target_pid` set, returns that
/// window's socket (or null if absent).  Otherwise returns the newest-by-mtime
/// socket — the most recently focused window.  Returns null when none exist.
fn pickSocket(out: []u8, target_pid: ?u32) ?[]const u8 {
    var pfx_buf: [64]u8 = undefined;
    const prefix = sockPrefix(&pfx_buf);
    const tmp = std.mem.span(tmpDir());

    const dirp = c.opendir(tmpDir()) orelse return null;
    defer _ = c.closedir(dirp);

    var best_name: [256]u8 = undefined;
    var best_len: usize = 0;
    var best_mtime: i128 = std.math.minInt(i128);

    while (c.readdir(dirp)) |ent| {
        const name = std.mem.sliceTo(@as([*:0]const u8, @ptrCast(&ent.*.d_name)), 0);
        const pid = pidFromName(name, prefix) orelse continue;
        if (target_pid) |want| {
            if (pid != want) continue;
        }

        // Full path for stat / candidacy.
        var path_buf: [sock_max]u8 = undefined;
        const path = std.fmt.bufPrint(&path_buf, "{s}{s}", .{ tmp, name }) catch continue;
        var pz: [sock_max + 1]u8 = undefined;
        @memcpy(pz[0..path.len], path);
        pz[path.len] = 0;

        var st: c.struct_stat = undefined;
        if (c.stat(&pz, &st) != 0) continue;
        const mt: i128 = @as(i128, st.st_mtimespec.tv_sec) * std.time.ns_per_s + st.st_mtimespec.tv_nsec;

        if (target_pid != null or mt > best_mtime) {
            best_mtime = mt;
            @memcpy(best_name[0..name.len], name);
            best_len = name.len;
            if (target_pid != null) break;
        }
    }

    if (best_len == 0) return null;
    return std.fmt.bufPrint(out, "{s}{s}", .{ tmp, best_name[0..best_len] }) catch null;
}

/// Send `"<verb> <arg>\n"` to a running window and return the reply: true on
/// `ok`, false on `err ...`.  With `target_pid` null, targets the front
/// (newest-mtime) window; sockets whose owner has died are reaped and the scan
/// retried.  Prints to stderr and returns false when no window is reachable.
pub fn tryClient(verb: []const u8, arg: ?[]const u8, target_pid: ?u32) bool {
    var attempt: usize = 0;
    while (attempt < 64) : (attempt += 1) {
        var path_buf: [sock_max]u8 = undefined;
        const path = pickSocket(&path_buf, target_pid) orelse {
            const msg = if (target_pid != null)
                "anvil: no window with that pid\n"
            else
                "anvil: no running Anvil window\n";
            _ = std.c.write(2, msg.ptr, msg.len);
            return false;
        };

        const fd = c.socket(c.AF_UNIX, c.SOCK_STREAM, 0);
        if (fd < 0) return false;

        var addr: c.struct_sockaddr_un = std.mem.zeroes(c.struct_sockaddr_un);
        addr.sun_family = c.AF_UNIX;
        const n = @min(path.len, addr.sun_path.len - 1);
        @memcpy(addr.sun_path[0..n], path[0..n]);
        addr.sun_path[n] = 0;

        if (c.connect(fd, @ptrCast(&addr), @sizeOf(c.struct_sockaddr_un)) != 0) {
            _ = c.close(fd);
            // Stale socket: reap it and rescan.
            var z: [sock_max + 1]u8 = undefined;
            @memcpy(z[0..path.len], path);
            z[path.len] = 0;
            _ = c.unlink(&z);
            if (target_pid != null) {
                const msg = "anvil: that window is not running\n";
                _ = std.c.write(2, msg.ptr, msg.len);
                return false;
            }
            continue;
        }
        defer _ = c.close(fd);

        // Build request line: "verb arg\n" or "verb\n".
        var req_buf: [1024 + 16]u8 = undefined;
        const req = if (arg) |a|
            std.fmt.bufPrint(&req_buf, "{s} {s}\n", .{ verb, a }) catch return false
        else
            std.fmt.bufPrint(&req_buf, "{s}\n", .{verb}) catch return false;
        _ = c.write(fd, req.ptr, req.len);

        // Read reply.
        var rep: [64]u8 = undefined;
        const nr = c.read(fd, &rep, rep.len);
        if (nr <= 0) return false;
        const reply = rep[0..@intCast(nr)];
        if (std.mem.startsWith(u8, reply, "ok")) return true;
        _ = std.c.write(2, reply.ptr, reply.len);
        return false;
    }
    return false;
}

/// Print running windows to stdout, newest-focused first.  Liveness is probed
/// by connecting; sockets whose owner has died are reaped.
pub fn listWindows() void {
    var pfx_buf: [64]u8 = undefined;
    const prefix = sockPrefix(&pfx_buf);
    const tmp = std.mem.span(tmpDir());

    const Win = struct { pid: u32, mtime: i128 };
    var wins: [64]Win = undefined;
    var nw: usize = 0;

    const dirp = c.opendir(tmpDir()) orelse {
        const m = "no running Anvil windows\n";
        _ = std.c.write(1, m.ptr, m.len);
        return;
    };
    defer _ = c.closedir(dirp);

    while (c.readdir(dirp)) |ent| {
        if (nw >= wins.len) break;
        const name = std.mem.sliceTo(@as([*:0]const u8, @ptrCast(&ent.*.d_name)), 0);
        const pid = pidFromName(name, prefix) orelse continue;

        var path_buf: [sock_max]u8 = undefined;
        const path = std.fmt.bufPrint(&path_buf, "{s}{s}", .{ tmp, name }) catch continue;
        var pz: [sock_max + 1]u8 = undefined;
        @memcpy(pz[0..path.len], path);
        pz[path.len] = 0;

        // Probe liveness; reap dead sockets.
        const fd = c.socket(c.AF_UNIX, c.SOCK_STREAM, 0);
        if (fd < 0) continue;
        var addr: c.struct_sockaddr_un = std.mem.zeroes(c.struct_sockaddr_un);
        addr.sun_family = c.AF_UNIX;
        const cn = @min(path.len, addr.sun_path.len - 1);
        @memcpy(addr.sun_path[0..cn], path[0..cn]);
        addr.sun_path[cn] = 0;
        if (c.connect(fd, @ptrCast(&addr), @sizeOf(c.struct_sockaddr_un)) != 0) {
            _ = c.close(fd);
            _ = c.unlink(&pz);
            continue;
        }
        _ = c.close(fd);

        var st: c.struct_stat = undefined;
        const mt: i128 = if (c.stat(&pz, &st) == 0)
            @as(i128, st.st_mtimespec.tv_sec) * std.time.ns_per_s + st.st_mtimespec.tv_nsec
        else
            0;
        wins[nw] = .{ .pid = pid, .mtime = mt };
        nw += 1;
    }

    if (nw == 0) {
        const m = "no running Anvil windows\n";
        _ = std.c.write(1, m.ptr, m.len);
        return;
    }

    // Insertion sort by mtime descending (newest focus first).
    var i: usize = 1;
    while (i < nw) : (i += 1) {
        var j = i;
        while (j > 0 and wins[j].mtime > wins[j - 1].mtime) : (j -= 1) {
            const t = wins[j];
            wins[j] = wins[j - 1];
            wins[j - 1] = t;
        }
    }

    var out: [64]u8 = undefined;
    for (wins[0..nw], 0..) |w, idx| {
        const tag = if (idx == 0) " (front)" else "";
        const line = std.fmt.bufPrint(&out, "{d}{s}\n", .{ w.pid, tag }) catch continue;
        _ = std.c.write(1, line.ptr, line.len);
    }
}

// ---------------------------------------------------------------------------
// Pipe mode  (`cmd | anvil pipe` → new pager pane in a running window)
// ---------------------------------------------------------------------------

/// Build the shell line a piped pane runs: open `file` in $PAGER (default
/// `less -R`), then delete it.  Written into `buf`; returns the slice.
fn pipeCommand(buf: []u8, file: []const u8) []const u8 {
    const pager_c = std.c.getenv("PAGER") orelse "less -R";
    const pager = std.mem.span(pager_c);
    return std.fmt.bufPrint(buf, "sh -c '{s} {s}; rm {s}'", .{ pager, file, file }) catch buf[0..0];
}

/// Stream stdin to a temp file and open it in a pager pane via the `run`
/// verb.  Refuses when stdin is a terminal (nothing was piped in).
pub fn runPipe(target_pid: ?u32) bool {
    if (c.isatty(0) == 1) {
        const msg = "anvil pipe: nothing on stdin (pipe a command into it)\n";
        _ = std.c.write(2, msg.ptr, msg.len);
        return false;
    }

    // Temp file: $TMPDIR/anvil-pipe-<pid>.txt
    var file_buf: [256]u8 = undefined;
    const tmpdir_c = std.c.getenv("TMPDIR") orelse "/tmp/";
    const tmpdir = std.mem.span(tmpdir_c);
    const file = std.fmt.bufPrint(&file_buf, "{s}anvil-pipe-{d}.txt", .{ tmpdir, c.getpid() }) catch return false;

    var z: [257]u8 = undefined;
    @memcpy(z[0..file.len], file);
    z[file.len] = 0;
    const fd = c.open(&z, c.O_CREAT | c.O_WRONLY | c.O_TRUNC, @as(c_uint, 0o600));
    if (fd < 0) return false;

    var io_buf: [8192]u8 = undefined;
    while (true) {
        const n = c.read(0, &io_buf, io_buf.len);
        if (n <= 0) break;
        var off: usize = 0;
        while (off < n) {
            const w = c.write(fd, io_buf[@intCast(off)..].ptr, @intCast(@as(isize, n) - @as(isize, @intCast(off))));
            if (w <= 0) break;
            off += @intCast(w);
        }
    }
    _ = c.close(fd);

    var cmd_buf: [path_max]u8 = undefined;
    const cmd = pipeCommand(&cmd_buf, file);
    return tryClient("run", cmd, target_pid);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test "buildServerSockPath includes uid and pid" {
    var buf: [sock_max]u8 = undefined;
    const path = buildServerSockPath(&buf);
    try std.testing.expect(path.len > 0);
    try std.testing.expect(std.mem.endsWith(u8, path, ".sock"));
    var id_buf: [32]u8 = undefined;
    const uid_str = std.fmt.bufPrint(&id_buf, "anvil-{d}-", .{std.c.getuid()}) catch unreachable;
    try std.testing.expect(std.mem.indexOf(u8, path, uid_str) != null);
}

test "pidFromName parses pid, rejects non-matching" {
    try std.testing.expectEqual(@as(?u32, 12345), pidFromName("anvil-501-12345.sock", "anvil-501-"));
    try std.testing.expectEqual(@as(?u32, null), pidFromName("anvil-501-12345.sock", "anvil-999-"));
    try std.testing.expectEqual(@as(?u32, null), pidFromName("anvil-501-.sock", "anvil-501-"));
    try std.testing.expectEqual(@as(?u32, null), pidFromName("anvil-501-12.txt", "anvil-501-"));
}

test "parseRequest: split h and v" {
    const h = parseRequest("split h\n").?;
    try std.testing.expectEqual(pane.Axis.x, h.split);
    const v = parseRequest("split v\n").?;
    try std.testing.expectEqual(pane.Axis.y, v.split);
}

test "parseRequest: tab with and without path" {
    const bare = parseRequest("tab\n").?;
    try std.testing.expect(!bare.tab.has_path);
    const with_path = parseRequest("tab /my/dir\n").?;
    try std.testing.expect(with_path.tab.has_path);
    try std.testing.expectEqualStrings("/my/dir", with_path.tab.path[0..with_path.tab.len]);
}

test "parseRequest: run with command" {
    const r = parseRequest("run echo hi\n").?;
    try std.testing.expectEqualStrings("echo hi", r.run.cmd[0..r.run.len]);
}

test "pipeCommand embeds pager and file, deletes after" {
    var buf: [path_max]u8 = undefined;
    const cmd = pipeCommand(&buf, "/tmp/anvil-pipe-9.txt");
    try std.testing.expect(std.mem.startsWith(u8, cmd, "sh -c '"));
    try std.testing.expect(std.mem.indexOf(u8, cmd, "/tmp/anvil-pipe-9.txt") != null);
    try std.testing.expect(std.mem.indexOf(u8, cmd, "; rm /tmp/anvil-pipe-9.txt'") != null);
}

test "parseRequest: bare run returns null" {
    try std.testing.expect(parseRequest("run\n") == null);
    try std.testing.expect(parseRequest("run \n") == null);
}

test "parseRequest: view with path" {
    const r = parseRequest("view /x\n").?;
    try std.testing.expectEqualStrings("/x", r.view.path[0..r.view.len]);
}

test "parseRequest: bare view returns null" {
    try std.testing.expect(parseRequest("view\n") == null);
    try std.testing.expect(parseRequest("view \n") == null);
}

test "parseRequest: unknown verb returns null" {
    try std.testing.expect(parseRequest("frob /x\n") == null);
    try std.testing.expect(parseRequest("\n") == null);
}

test "takeCommands drains queue" {
    _ = std.c.pthread_mutex_lock(&q_mutex);
    queue[0] = Command{ .split = .x };
    q_len = 1;
    _ = std.c.pthread_mutex_unlock(&q_mutex);

    var out: [32]Command = undefined;
    const n = takeCommands(&out);
    try std.testing.expectEqual(@as(usize, 1), n);
    try std.testing.expectEqual(pane.Axis.x, out[0].split);
    try std.testing.expectEqual(@as(usize, 0), q_len);
}
