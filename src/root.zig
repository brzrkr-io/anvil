//! Anvil terminal core: PTY + VT parser + cell grid.

pub const Pty = @import("pty.zig").Pty;
pub const Cell = @import("vt/cell.zig").Cell;
pub const Color = @import("vt/cell.zig").Color;
pub const Grid = @import("vt/grid.zig").Grid;
pub const Terminal = @import("vt/terminal.zig").Terminal;
pub const Parser = @import("vt/parser.zig").Parser;

test {
    @import("std").testing.refAllDecls(@This());
}
