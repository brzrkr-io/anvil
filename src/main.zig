const std = @import("std");
const window = @import("platform/window.zig");
const cli = @import("cli.zig");
const app = @import("app.zig");
const ipc = @import("ipc.zig");

comptime {
    _ = app;
}

pub fn main(init: std.process.Init.Minimal) void {
    var it = std.process.Args.Iterator.init(init.args);
    _ = it.skip(); // skip argv[0]

    var buf: [512][*:0]const u8 = undefined;
    var argc: usize = 0;
    while (it.next()) |a| : (argc += 1) {
        if (argc >= buf.len) break;
        buf[argc] = a.ptr;
    }

    const args = cli.parse(buf[0..argc]);

    switch (args.mode) {
        .help => {
            _ = std.c.write(1, cli.help_text.ptr, cli.help_text.len);
            return;
        },
        .version => {
            const msg = "anvil " ++ cli.version_string ++ "\n";
            _ = std.c.write(1, msg.ptr, msg.len);
            return;
        },
        .list => {
            ipc.listWindows();
            return;
        },
        .client => {
            if (std.mem.eql(u8, args.verb, "pipe")) {
                _ = ipc.runPipe(args.window_pid);
            } else if (std.mem.eql(u8, args.verb, "open")) {
                if (args.verb_arg) |file| {
                    const editor_c = std.c.getenv("EDITOR") orelse "vi";
                    const editor = std.mem.span(editor_c);
                    var obuf: [1024]u8 = undefined;
                    const ocmd = std.fmt.bufPrint(&obuf, "{s} '{s}'", .{ editor, file }) catch return;
                    _ = ipc.tryClient("run", ocmd, args.window_pid);
                }
            } else if (std.mem.eql(u8, args.verb, "run")) {
                var jbuf: [1024]u8 = undefined;
                var jlen: usize = 0;
                for (args.run_args, 0..) |tok, ti| {
                    const t = std.mem.span(tok);
                    if (ti > 0 and jlen < jbuf.len) {
                        jbuf[jlen] = ' ';
                        jlen += 1;
                    }
                    const n = @min(t.len, jbuf.len - jlen);
                    @memcpy(jbuf[jlen..][0..n], t[0..n]);
                    jlen += n;
                }
                _ = ipc.tryClient("run", if (jlen > 0) jbuf[0..jlen] else null, args.window_pid);
            } else {
                _ = ipc.tryClient(args.verb, args.verb_arg, args.window_pid);
            }
            return;
        },
        .dump => {
            if (args.dump_path) |p| {
                var path_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
                @memcpy(path_buf[0..p.len], p);
                path_buf[p.len] = 0;
                window.dump(@ptrCast(path_buf[0..p.len :0]), 1600, 1000);
            }
            return;
        },
        .run => {
            if (args.fresh) app.suppress_persist = true;
            if (args.start_dir) |dir| {
                var z_buf: [std.fs.max_path_bytes + 1]u8 = undefined;
                @memcpy(z_buf[0..dir.len], dir);
                z_buf[dir.len] = 0;
                const z: [*:0]const u8 = @ptrCast(z_buf[0..dir.len :0]);
                if (std.c.access(z, 0) == 0) {
                    app.start_cwd = dir;
                } else {
                    const msg = "anvil: path not found: ";
                    _ = std.c.write(2, msg.ptr, msg.len);
                    _ = std.c.write(2, dir.ptr, dir.len);
                    _ = std.c.write(2, ", using default\n", 16);
                }
            }
            window.run();
        },
    }
}

test {
    _ = @import("cli.zig");
}
