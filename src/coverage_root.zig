//! Test root for the `coverage` build step (`zig build coverage`).
//!
//! kcov deadlocks while tracing the child processes the pty tests spawn on
//! macOS, so the coverage report is built from this root, which imports every
//! module except `pty/`. The pty module is still exercised by `zig build
//! test`, whose root is `main.zig`.

test {
    _ = @import("config/config.zig");
    _ = @import("config/theme.zig");
    _ = @import("render/color.zig");
    _ = @import("render/font.zig");
    _ = @import("render/raster.zig");
    _ = @import("render/capi.zig");
    _ = @import("render/metal.zig");
    _ = @import("app/keys.zig");
    _ = @import("terminal/terminal.zig");
    _ = @import("terminal/parser.zig");
    _ = @import("terminal/grid.zig");
    _ = @import("terminal/scrollback.zig");
    _ = @import("terminal/cell.zig");
}
