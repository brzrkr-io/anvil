//! Terminal emulation types and VT state machine.

pub mod cell;
pub mod grid;
pub mod parser;
pub mod scrollback;
pub mod search;
pub mod terminal;

pub use cell::{Attrs, Cell, Color};
pub use grid::{Grid, Modes, ScrollRegion};
pub use parser::{Handler, Parser};
pub use scrollback::{DEFAULT_CAPACITY, Scrollback};
pub use search::{MAX_MATCHES as SEARCH_MAX_MATCHES, Match, MatchKind, Search};
pub use terminal::{
    Block, BlockState, Cursor, CursorShape, DiffKind, DirtySet, LastRun, PrivateModes, PromptMark,
    PromptMarkKind, Terminal,
};
