//! Anvil terminal core. VT parser + grid land here in M1.

pub const Pty = @import("pty.zig").Pty;

test {
    @import("std").testing.refAllDecls(@This());
}
