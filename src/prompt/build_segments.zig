//! Turns detected context + git info into the ordered Segment list the
//! renderer draws. This is the adaptive core: a segment appears only when the
//! context calls for it.

const std = @import("std");
const seg = @import("segments.zig");
const ctx = @import("context.zig");
const git = @import("git.zig");

pub const Inputs = struct {
    cwd_base: []const u8, // basename of the working directory
    context: ctx.Context,
    git_info: ?git.Info,
    exit_code: u8,
    /// scratch buffer the assembled segment texts borrow from
    scratch: []u8,
};

fn langText(l: ctx.Lang) ?[]const u8 {
    return switch (l) {
        .none => null,
        .zig => "zig",
        .node => "node",
        .python => "python",
        .rust => "rust",
        .go => "go",
    };
}

/// Build the active segment list. Texts are slices into `in.scratch`.
pub fn assemble(in: Inputs) seg.List {
    var list = seg.List{};
    var off: usize = 0;

    // cwd — always.
    list.add(.{ .icon = .repo, .text = in.cwd_base });

    // git — when in a repo.
    if (in.git_info) |g| {
        const dirty_suffix = if (g.dirty > 0) blk: {
            if (std.fmt.bufPrint(in.scratch[off..], "{s} \u{25cf}{d}", .{ g.branch, g.dirty })) |s| {
                off += s.len;
                break :blk s;
            } else |_| {
                break :blk g.branch;
            }
        } else g.branch;
        list.add(.{
            .icon = .branch,
            .text = dirty_suffix,
            .state = if (g.dirty > 0) .warn else .normal,
        });
    }

    // toolchain — when a language is detected.
    if (langText(in.context.lang)) |lt| {
        list.add(.{ .icon = .toolchain, .text = lt });
    }

    // container / cluster — when present.
    if (in.context.has_container) list.add(.{ .icon = .container, .text = "docker" });
    if (in.context.has_k8s) list.add(.{ .icon = .cluster, .text = "k8s" });

    // failure — only on a non-zero exit.
    if (in.exit_code != 0) {
        const exit_text = if (std.fmt.bufPrint(in.scratch[off..], "{d}", .{in.exit_code})) |s| blk: {
            off += s.len;
            break :blk s;
        } else |_| "?";
        list.add(.{ .icon = .err, .text = exit_text, .state = .err });
    }

    return list;
}

const testing = std.testing;
const icons = @import("icons.zig");

test "assemble: clean repo shows cwd + branch only" {
    var scratch: [256]u8 = undefined;
    const list = assemble(.{
        .cwd_base = "caldera-console",
        .context = .{ .in_git = true },
        .git_info = .{ .branch = "main" },
        .exit_code = 0,
        .scratch = &scratch,
    });
    try testing.expectEqual(@as(usize, 2), list.slice().len);
    try testing.expectEqual(seg.Segment{ .icon = .repo, .text = "caldera-console" }, list.slice()[0]);
}

test "assemble: dirty repo marks the git segment warn" {
    var scratch: [256]u8 = undefined;
    const list = assemble(.{
        .cwd_base = "x",
        .context = .{ .in_git = true },
        .git_info = .{ .branch = "main", .dirty = 3 },
        .exit_code = 0,
        .scratch = &scratch,
    });
    try testing.expectEqual(seg.State.warn, list.slice()[1].state);
    try testing.expect(std.mem.indexOf(u8, list.slice()[1].text, "3") != null);
}

test "assemble: a node+docker dir surfaces toolchain and container" {
    var scratch: [256]u8 = undefined;
    const list = assemble(.{
        .cwd_base = "app",
        .context = .{ .lang = .node, .has_container = true },
        .git_info = null,
        .exit_code = 0,
        .scratch = &scratch,
    });
    var saw_tool = false;
    var saw_dk = false;
    for (list.slice()) |s| {
        if (s.icon == .toolchain) saw_tool = true;
        if (s.icon == .container) saw_dk = true;
    }
    try testing.expect(saw_tool and saw_dk);
}

test "assemble: a non-zero exit adds an err segment" {
    var scratch: [256]u8 = undefined;
    const list = assemble(.{
        .cwd_base = "x",
        .context = .{},
        .git_info = null,
        .exit_code = 127,
        .scratch = &scratch,
    });
    const last = list.slice()[list.slice().len - 1];
    try testing.expectEqual(icons.Icon.err, last.icon);
    try testing.expectEqualStrings("127", last.text);
}
