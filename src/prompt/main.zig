//! caldera-prompt — renders the Caldera shell prompt. Invoked by the shell on
//! every prompt draw. Emits ANSI to stdout.

const std = @import("std");

pub fn main() void {
    std.debug.print("caldera-prompt\n", .{});
}
