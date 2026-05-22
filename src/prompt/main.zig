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
    shell: render.Shell = .plain,
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
        } else if (std.mem.eql(u8, arg, "--shell")) {
            if (it.next()) |v| {
                if (std.mem.eql(u8, v, "zsh")) {
                    a.shell = .zsh;
                } else if (std.mem.eql(u8, v, "bash")) {
                    a.shell = .bash;
                }
            }
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

/// Read the theme hint file pointed to by CALDERA_THEME_FILE. Returns true
/// when the file content starts with "light". Defaults to false (dark) when
/// the env var is unset or the file is missing/unreadable.
fn themeIsLight() bool {
    const path = std.c.getenv("CALDERA_THEME_FILE") orelse return false;
    const fd = std.c.open(path, .{ .ACCMODE = .RDONLY }, @as(c_uint, 0));
    if (fd < 0) return false;
    defer _ = std.c.close(fd);
    var buf: [16]u8 = undefined;
    const n = std.c.read(fd, &buf, buf.len);
    if (n <= 0) return false;
    return std.mem.startsWith(u8, buf[0..@intCast(n)], "light");
}

pub fn main(p: std.process.Init.Minimal) void {
    var arena = std.heap.ArenaAllocator.init(std.heap.c_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const args = parseArgs(p);
    // Rich glyphs only inside Caldera.
    const rich = std.c.getenv("CALDERA_CONSOLE") != null;
    const opts = render.Options{ .rich = rich, .failed = args.exit_code != 0, .width = args.width, .shell = args.shell, .light = themeIsLight() };

    if (args.rule) {
        const s = render.rule(alloc, args.width, args.shell) catch return;
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
