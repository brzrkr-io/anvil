//! Pane: one terminal viewport.
//!
//! ## PTY seam
//!
//! In the Zig implementation `Pane` owned a `Pty` and spawned a reader thread.
//! In this crate `Pane` is **pure** — it owns only the terminal state and
//! per-pane view state.  The PTY + reader thread are a platform concern handled
//! in a later phase (`crates/anvil-platform`).
//!
//! The expected platform-layer design:
//!
//! ```text
//! Platform layer owns:
//!   HashMap<PaneId, Pty>           — one PTY file descriptor per pane
//!   HashMap<PaneId, ReaderThread>  — one reader thread per pane
//!
//! anvil-workspace owns:
//!   PaneRegistry<HashMap<PaneId, Pane>>  — pure terminal state + view state
//! ```
//!
//! The platform creates a `Pane` via [`Pane::new`], registers it in a
//! [`PaneRegistry`], and separately tracks the `Pty` by the same `PaneId`.
//! When the 60 Hz tick drains PTY bytes, the platform calls
//! `pane.terminal_mut().process(bytes)` directly — no lock needed because
//! all mutations happen on the main thread.

use anvil_term::Terminal;

use crate::layout::PaneId;
use crate::selection::Selection;

/// Per-pane view state and terminal emulator.  No PTY, no threads.
pub struct Pane {
    pub id: PaneId,
    pub terminal: Terminal,

    // Per-pane view state animated by the main thread.
    pub scroll_pos: f32,
    pub overscroll: f32,
    pub overscroll_target: f32,
    pub cursor_ax: f32,
    pub cursor_ay: f32,
    pub selection: Selection,
}

impl Pane {
    /// Create a new pane with a `cols × rows` terminal and `scrollback` history.
    /// The caller registers the returned `Pane` in a [`PaneRegistry`] and
    /// separately spawns a PTY + reader thread identified by the same `id`.
    pub fn new(id: PaneId, cols: usize, rows: usize, scrollback: usize) -> Self {
        let terminal = Terminal::new(cols, rows, scrollback);
        Self {
            id,
            terminal,
            scroll_pos: 0.0,
            overscroll: 0.0,
            overscroll_target: 0.0,
            cursor_ax: 0.0,
            cursor_ay: 0.0,
            selection: Selection::default(),
        }
    }

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    pub fn terminal_mut(&mut self) -> &mut Terminal {
        &mut self.terminal
    }
}

/// A registry of all pure `Pane`s owned by one tab.
pub struct PaneRegistry {
    map: std::collections::HashMap<PaneId, Pane>,
    next_id: PaneId,
}

impl Default for PaneRegistry {
    fn default() -> Self {
        Self {
            map: std::collections::HashMap::new(),
            next_id: 1,
        }
    }
}

impl PaneRegistry {
    /// Allocate a fresh `PaneId`, create the `Pane`, register it, and return
    /// the id.  The caller must separately create the PTY and reader thread
    /// keyed by the returned id.
    pub fn create_and_register(&mut self, cols: usize, rows: usize, scrollback: usize) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        let pane = Pane::new(id, cols, rows, scrollback);
        self.map.insert(id, pane);
        id
    }

    /// Look up a pane by id.
    pub fn get(&self, id: PaneId) -> Option<&Pane> {
        self.map.get(&id)
    }

    /// Look up a pane mutably by id.
    pub fn get_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.map.get_mut(&id)
    }

    /// Remove and drop the pane with `id`.  No-op if not present.
    pub fn remove(&mut self, id: PaneId) {
        self.map.remove(&id);
    }

    /// Number of registered panes.
    pub fn count(&self) -> usize {
        self.map.len()
    }
}
