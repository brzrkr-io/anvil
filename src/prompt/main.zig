//! caldera-prompt — renders the Caldera shell prompt. Invoked by the shell on
//! every prompt draw. Args: --exit <n>, --transient. Emits ANSI to stdout.

const std = @import("std");
const ctx = @import("context.zig");
const git = @import("git.zig");
const render = @import("render.zig");
const build_segments = @import("build_segments.zig");

const Args = struct {
    exit_code: u8 = 0,
    transient: bool = false,
    rule: bool = false,
    width: usize = 0,
};

fn parseArgs(p: std.process.Init.Minimal) Args {
    var a = Args{};
    var it = std.process.Args.Iterator.init(p.args);
    _ = it.next(); // argv[0]
    while (it.next()) |arg| {
        if (std.mem.eql(u8, arg, "--transient")) {
            a.transient = true;
        } else if (std.mem.eql(u8, arg, "--rule")) {
            a.rule = true;
        } else if (std.mem.eql(u8, arg, "--exit")) {
            if (it.next()) |v| a.exit_code = std.fmt.parseInt(u8, v, 10) catch 0;
        } else if (std.mem.eql(u8, arg, "--width")) {
            if (it.next()) |v| a.width = std.fmt.parseInt(usize, v, 10) catch 0;
        }
    }
    return a;
}

fn basename(path: []const u8) []const u8 {
    if (std.mem.lastIndexOfScalar(u8, path, '/')) |i| {
        if (i + 1 < path.len) return path[i + 1 ..];
    }
    return path;
}

fn writeAll(s: []const u8) void {
    var off: usize = 0;
    while (off < s.len) {
        const n = std.c.write(1, s[off..].ptr, s.len - off);
        if (n <= 0) break;
        off += @intCast(n);
    }
}

pub fn main(p: std.process.Init.Minimal) void {
    var arena = std.heap.ArenaAllocator.init(std.heap.c_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const args = parseArgs(p);
    // Rich glyphs only inside Caldera.
    const rich = std.c.getenv("CALDERA_CONSOLE") != null;
    const opts = render.Options{ .rich = rich, .failed = args.exit_code != 0, .width = args.width };

    if (args.rule) {
        const s = render.rule(alloc, args.width) catch return;
        writeAll(s);
        return;
    }

    if (args.transient) {
        const s = render.transient(alloc, opts) catch return;
        writeAll(s);
        return;
    }

    // cwd
    var cwd_buf: [std.fs.max_path_bytes]u8 = undefined;
    const cwd_ptr = std.c.getcwd(&cwd_buf, cwd_buf.len) orelse return;
    const cwd = std.mem.span(@as([*:0]const u8, @ptrCast(cwd_ptr)));

    const context = ctx.detect(cwd);
    var branch_buf: [256]u8 = undefined;
    const git_info: ?git.Info = if (context.in_git)
        git.query(alloc, cwd, &branch_buf)
    else
        null;

    var scratch: [512]u8 = undefined;
    const list = build_segments.assemble(.{
        .cwd_base = basename(cwd),
        .context = context,
        .git_info = git_info,
        .exit_code = args.exit_code,
        .scratch = &scratch,
    });

    const s = render.full(alloc, list.slice(), opts) catch return;
    writeAll(s);
}

test {
    _ = @import("icons.zig");
    _ = @import("segments.zig");
    _ = @import("context.zig");
    _ = @import("git.zig");
    _ = @import("render.zig");
    _ = @import("build_segments.zig");
}
