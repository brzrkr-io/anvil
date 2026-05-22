//! caldera-prompt — renders the Caldera shell prompt. Invoked by the shell on
//! every prompt draw. Emits ANSI to stdout.

const std = @import("std");
const icons = @import("icons.zig");
const segments = @import("segments.zig");
const context = @import("context.zig");
const git = @import("git.zig");
const render = @import("render.zig");
const build_segments = @import("build_segments.zig");

pub fn main() void {
    _ = icons;
    _ = segments;
    _ = context;
    std.debug.print("caldera-prompt\n", .{});
}

// Pull sub-module tests into the test binary.
test {
    _ = icons;
    _ = segments;
    _ = context;
    _ = git;
    _ = render;
    _ = build_segments;
}
