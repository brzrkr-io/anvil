//! Git status for the prompt. `parseStatus` (pure, tested) interprets the
//! output of `git status --porcelain=v1 --branch`; `query` runs git as a
//! subprocess and feeds it to the parser.

const std = @import("std");

pub const Info = struct {
    branch: []const u8, // borrowed from the caller-provided buffer
    dirty: u32 = 0,
    ahead: u32 = 0,
    behind: u32 = 0,
};

/// Parse `git status --porcelain=v1 --branch` output. The branch name is a
/// slice into `text`. Returns null if no branch header line is present.
pub fn parseStatus(text: []const u8) ?Info {
    var info: ?Info = null;
    var lines = std.mem.splitScalar(u8, text, '\n');
    while (lines.next()) |line| {
        if (line.len == 0) continue;
        if (std.mem.startsWith(u8, line, "## ")) {
            info = parseBranchLine(line[3..]);
        } else {
            if (info) |*i| i.dirty += 1;
        }
    }
    return info;
}

fn parseBranchLine(rest: []const u8) Info {
    // e.g. "main...origin/main [ahead 1, behind 2]"  or  "main"
    var branch_end: usize = rest.len;
    if (std.mem.indexOf(u8, rest, "...")) |i| branch_end = i;
    if (std.mem.indexOfScalar(u8, rest, ' ')) |i| branch_end = @min(branch_end, i);
    var info = Info{ .branch = rest[0..branch_end] };
    if (std.mem.indexOf(u8, rest, "ahead ")) |i| {
        info.ahead = readNum(rest[i + 6 ..]);
    }
    if (std.mem.indexOf(u8, rest, "behind ")) |i| {
        info.behind = readNum(rest[i + 7 ..]);
    }
    return info;
}

fn readNum(s: []const u8) u32 {
    var n: u32 = 0;
    for (s) |ch| {
        if (ch < '0' or ch > '9') break;
        n = n * 10 + (ch - '0');
    }
    return n;
}

/// Run git in `cwd` and return its status, or null if not a repo / git fails /
/// it errors. `out_buf` backs the returned branch slice.
///
/// Uses `std.process.run` (Zig 0.16 API) with a `Threaded` io backed by
/// `c_allocator` so that process spawn can allocate argv buffers.
pub fn query(allocator: std.mem.Allocator, cwd: []const u8, out_buf: []u8) ?Info {
    var threaded = std.Io.Threaded.init(std.heap.c_allocator, .{});
    const io = threaded.io();
    const result = std.process.run(allocator, io, .{
        .argv = &.{ "git", "status", "--porcelain=v1", "--branch" },
        .cwd = .{ .path = cwd },
    }) catch return null;
    defer allocator.free(result.stdout);
    defer allocator.free(result.stderr);

    switch (result.term) {
        .exited => |code| if (code != 0) return null,
        else => return null,
    }

    const parsed = parseStatus(result.stdout) orelse return null;
    if (parsed.branch.len > out_buf.len) return null;
    @memcpy(out_buf[0..parsed.branch.len], parsed.branch);
    return .{
        .branch = out_buf[0..parsed.branch.len],
        .dirty = parsed.dirty,
        .ahead = parsed.ahead,
        .behind = parsed.behind,
    };
}

const testing = std.testing;

test "parseStatus reads branch and dirty count" {
    const out =
        "## main...origin/main\n" ++
        " M src/a.zig\n" ++
        "?? new.txt\n";
    const info = parseStatus(out).?;
    try testing.expectEqualStrings("main", info.branch);
    try testing.expectEqual(@as(u32, 2), info.dirty);
    try testing.expectEqual(@as(u32, 0), info.ahead);
}

test "parseStatus reads ahead and behind" {
    const out = "## main...origin/main [ahead 3, behind 1]\n";
    const info = parseStatus(out).?;
    try testing.expectEqualStrings("main", info.branch);
    try testing.expectEqual(@as(u32, 3), info.ahead);
    try testing.expectEqual(@as(u32, 1), info.behind);
}

test "parseStatus handles a branch with no upstream" {
    const info = parseStatus("## feature/x\n").?;
    try testing.expectEqualStrings("feature/x", info.branch);
    try testing.expectEqual(@as(u32, 0), info.dirty);
}

test "parseStatus returns null without a branch header" {
    try testing.expect(parseStatus("") == null);
    try testing.expect(parseStatus("?? stray.txt\n") == null);
}
