const window = @import("platform/window.zig");

comptime {
    _ = @import("app.zig");
}

pub fn main() void {
    window.run();
}
