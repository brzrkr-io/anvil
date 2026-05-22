//! caldera-prompt — renders the Caldera shell prompt. Invoked by the shell on
//! every prompt draw. Emits ANSI to stdout.

const std = @import("std");
const icons = @import("icons.zig");
const segments = @import("segments.zig");

pub fn main() void {
    _ = icons;
    _ = segments;
    std.debug.print("caldera-prompt\n", .{});
}
