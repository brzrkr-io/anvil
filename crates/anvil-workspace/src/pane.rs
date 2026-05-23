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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Pane::new ─────────────────────────────────────────────────────────────

    #[test]
    fn pane_new_sets_id_and_zero_view_state() {
        let p = Pane::new(7, 80, 24, 0);
        assert_eq!(p.id, 7);
        assert_eq!(p.scroll_pos, 0.0);
        assert_eq!(p.overscroll, 0.0);
        assert_eq!(p.overscroll_target, 0.0);
        assert_eq!(p.cursor_ax, 0.0);
        assert_eq!(p.cursor_ay, 0.0);
        assert!(!p.selection.active);
    }

    // ── Pane::terminal / terminal_mut ─────────────────────────────────────────

    #[test]
    fn pane_terminal_accessor_returns_correct_dimensions() {
        let p = Pane::new(1, 80, 24, 0);
        assert_eq!(p.terminal().cols(), 80);
        assert_eq!(p.terminal().rows(), 24);
    }

    #[test]
    fn pane_terminal_mut_allows_write() {
        let mut p = Pane::new(1, 80, 24, 0);
        p.terminal_mut().feed(b"hi");
        // No panic, terminal consumed the bytes.
        assert_eq!(p.terminal().cols(), 80);
    }

    // ── PaneRegistry operations ───────────────────────────────────────────────

    #[test]
    fn registry_create_and_register_increments_id() {
        let mut reg = PaneRegistry::default();
        let id1 = reg.create_and_register(80, 24, 0);
        let id2 = reg.create_and_register(40, 12, 0);
        assert_ne!(id1, id2);
        assert_eq!(reg.count(), 2);
    }

    #[test]
    fn registry_get_returns_correct_pane() {
        let mut reg = PaneRegistry::default();
        let id = reg.create_and_register(80, 24, 0);
        let pane = reg.get(id).unwrap();
        assert_eq!(pane.id, id);
        assert_eq!(pane.terminal().cols(), 80);
    }

    #[test]
    fn registry_get_missing_id_returns_none() {
        let reg = PaneRegistry::default();
        assert!(reg.get(999).is_none());
    }

    #[test]
    fn registry_get_mut_returns_correct_pane() {
        let mut reg = PaneRegistry::default();
        let id = reg.create_and_register(80, 24, 0);
        let pane = reg.get_mut(id).unwrap();
        pane.scroll_pos = 3.5;
        assert_eq!(reg.get(id).unwrap().scroll_pos, 3.5);
    }

    #[test]
    fn registry_get_mut_missing_id_returns_none() {
        let mut reg = PaneRegistry::default();
        assert!(reg.get_mut(999).is_none());
    }

    #[test]
    fn registry_remove_drops_pane() {
        let mut reg = PaneRegistry::default();
        let id = reg.create_and_register(80, 24, 0);
        assert_eq!(reg.count(), 1);
        reg.remove(id);
        assert_eq!(reg.count(), 0);
        assert!(reg.get(id).is_none());
    }

    #[test]
    fn registry_remove_missing_id_is_noop() {
        let mut reg = PaneRegistry::default();
        reg.remove(999); // should not panic
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn registry_count_reflects_creates_and_removes() {
        let mut reg = PaneRegistry::default();
        assert_eq!(reg.count(), 0);
        let a = reg.create_and_register(1, 1, 0);
        let b = reg.create_and_register(1, 1, 0);
        assert_eq!(reg.count(), 2);
        reg.remove(a);
        assert_eq!(reg.count(), 1);
        reg.remove(b);
        assert_eq!(reg.count(), 0);
    }
}
