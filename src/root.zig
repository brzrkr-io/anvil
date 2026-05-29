//! Anvil terminal core (the reusable library module).
//!
//! Ghostty-style: the terminal *logic* lives here, independent of any window or
//! GPU. Today that's just the PTY seam; M1 adds the VT parser + cell grid, M2
//! adds the AppKit/Metal front-end that consumes this module.

pub const Pty = @import("pty.zig").Pty;

test {
    @import("std").testing.refAllDecls(@This());
}
