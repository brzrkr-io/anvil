const std = @import("std");
const window = @import("platform/window.zig");

comptime {
    _ = @import("app.zig");
}

pub fn main(init: std.process.Init.Minimal) void {
    var it = std.process.Args.Iterator.init(init.args);
    _ = it.skip();
    while (it.next()) |a| {
        if (std.mem.eql(u8, a, "--dump")) {
            if (it.next()) |path| {
                window.dump(path.ptr, 1600, 1000);
                return;
            }
        }
    }
    window.run();
}
