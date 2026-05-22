//! Terminal emulation types and VT state machine.

pub mod cell;
pub mod grid;
pub mod scrollback;

pub use cell::{Attrs, Cell, Color};
pub use grid::{Grid, Modes, ScrollRegion};
pub use scrollback::{DEFAULT_CAPACITY, Scrollback};
