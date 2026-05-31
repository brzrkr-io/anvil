//! Anvil terminal core: PTY + VT parser + cell grid + render logic.

pub const Pty = @import("pty.zig").Pty;
pub const Cell = @import("vt/cell.zig").Cell;
pub const Color = @import("vt/cell.zig").Color;
pub const Grid = @import("vt/grid.zig").Grid;
pub const Terminal = @import("vt/terminal.zig").Terminal;
pub const Parser = @import("vt/parser.zig").Parser;
pub const palette = @import("render/palette.zig");
pub const Renderer = @import("render/renderer.zig").Renderer;
pub const CellInstance = @import("render/instance.zig").CellInstance;
pub const Atlas = @import("render/atlas.zig").Atlas;

test {
    const std = @import("std");
    std.testing.refAllDecls(@This());
    _ = @import("render/palette.zig");
    _ = @import("render/renderer.zig");
    _ = @import("render/atlas.zig");
    _ = @import("render/theme.zig");
    _ = @import("session.zig");
    _ = @import("session_manager.zig");
    _ = @import("workspace/pane_tree.zig");
    _ = @import("palette.zig");
    _ = @import("config.zig");
    _ = @import("search.zig");
    _ = @import("regex.zig");
    _ = @import("keys.zig");
    _ = @import("session_persist.zig");
    _ = @import("context_chip.zig");
    _ = @import("copy_mode.zig");
    _ = @import("caldera.zig");
    _ = @import("ipc.zig");
    _ = @import("syntax.zig");
    _ = @import("fileview.zig");
    _ = @import("editor.zig");
}
