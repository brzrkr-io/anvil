//! Pane: one terminal viewport (or native editor stub).
//!
//! ## Pane content
//!
//! A pane holds either a terminal (PTY-backed) or a native editor pane.
//! The `terminal` field is `Option<Terminal>`: `Some` for terminal panes,
//! `None` for native editor panes.  The `editor_id` field is `Some(BufferId)`
//! for native editor panes, `None` for terminal panes.
//!
//! Call sites must branch on which variant is present:
//!
//! ```ignore
//! if let Some(terminal) = &pane.terminal { /* terminal path */ }
//! if let Some(bid) = pane.editor_id { /* editor path */ }
//! ```
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

use anvil_editor::BufferId;
use anvil_term::Terminal;

use crate::layout::PaneId;
use crate::selection::Selection;

/// Maximum number of simultaneously folded blocks per pane.
const MAX_FOLDED: usize = 32;

/// Per-pane view state and terminal emulator (or native editor stub).  No PTY, no threads.
pub struct Pane {
    pub id: PaneId,
    /// Terminal emulator state.  `Some` for terminal panes, `None` for native
    /// editor panes.  Always check before use:
    /// `if let Some(terminal) = &pane.terminal { … }`.
    pub terminal: Option<Terminal>,
    /// Buffer identifier for native editor panes.  `Some` when this pane holds
    /// a native editor; `None` for terminal panes.
    pub editor_id: Option<BufferId>,

    // Per-pane view state animated by the main thread.
    pub scroll_pos: f32,
    /// Easing target for smooth-scroll momentum release.
    pub scroll_target: f32,
    /// Accumulated wheel delta since last settle; cleared when easing stops.
    pub scroll_vel: f32,
    pub cursor_ax: f32,
    pub cursor_ay: f32,
    pub selection: Selection,

    // Folded blocks, keyed by absolute command_line. Bounded.
    pub folded: [usize; MAX_FOLDED],
    pub folded_count: usize,

    /// Living-scrollback indicator: snapshot of `terminal.line_count()` taken
    /// the moment the user first scrolled away from live bottom.  `None` when
    /// pinned to live (scroll_pos == 0).  When set and terminal has grown,
    /// `unseen_rows()` returns the number of rows that arrived while scrolled.
    pub unseen_baseline: Option<usize>,
}

impl Pane {
    /// Create a new terminal pane with a `cols × rows` terminal and `scrollback` history.
    /// The caller registers the returned `Pane` in a [`PaneRegistry`] and
    /// separately spawns a PTY + reader thread identified by the same `id`.
    pub fn new(id: PaneId, cols: usize, rows: usize, scrollback: usize) -> Self {
        let terminal = Terminal::new(cols, rows, scrollback);
        Self {
            id,
            terminal: Some(terminal),
            editor_id: None,
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
            cursor_ax: 0.0,
            cursor_ay: 0.0,
            selection: Selection::default(),
            folded: [0; MAX_FOLDED],
            folded_count: 0,
            unseen_baseline: None,
        }
    }

    /// Create a new native editor pane.  No PTY is created.
    /// The caller registers the returned `Pane` in a [`PaneRegistry`] and
    /// the `buffer_id` in an [`EditorPaneRegistry`].
    pub fn new_editor(id: PaneId, buffer_id: BufferId) -> Self {
        Self {
            id,
            terminal: None,
            editor_id: Some(buffer_id),
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
            cursor_ax: 0.0,
            cursor_ay: 0.0,
            selection: Selection::default(),
            folded: [0; MAX_FOLDED],
            folded_count: 0,
            unseen_baseline: None,
        }
    }

    /// Toggle fold state for the block whose command starts at `cmd_line`.
    /// If already folded, unfolds it.  If not folded, adds it (up to the cap).
    pub fn toggle_fold(&mut self, cmd_line: usize) {
        for i in 0..self.folded_count {
            if self.folded[i] == cmd_line {
                // Remove by swapping with the last element.
                self.folded_count -= 1;
                self.folded[i] = self.folded[self.folded_count];
                return;
            }
        }
        if self.folded_count < MAX_FOLDED {
            self.folded[self.folded_count] = cmd_line;
            self.folded_count += 1;
        }
    }

    /// Returns true if the block whose command starts at `cmd_line` is folded.
    pub fn is_folded(&self, cmd_line: usize) -> bool {
        self.folded[..self.folded_count].contains(&cmd_line)
    }

    /// Returns a reference to the terminal if this is a terminal pane.
    pub fn terminal(&self) -> Option<&Terminal> {
        self.terminal.as_ref()
    }

    /// Returns a mutable reference to the terminal if this is a terminal pane.
    pub fn terminal_mut(&mut self) -> Option<&mut Terminal> {
        self.terminal.as_mut()
    }

    /// Number of content rows that arrived while the user was scrolled away
    /// from live bottom.  Returns 0 when pinned to live, when nothing new
    /// has arrived, or when this is a native editor pane (no scrollback).
    pub fn unseen_rows(&self) -> usize {
        let terminal = match &self.terminal {
            Some(t) => t,
            None => return 0,
        };
        match self.unseen_baseline {
            Some(baseline) => terminal.line_count().saturating_sub(baseline),
            None => 0,
        }
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
    /// Bump the internal id counter so the next `create_and_register` call
    /// returns at least `id`. Used by the App to keep pane IDs globally
    /// unique across tabs — `self.ptys` is keyed by `PaneId`, so collisions
    /// across tabs would overwrite live PTYs.
    pub fn set_next_id_at_least(&mut self, id: PaneId) {
        if self.next_id < id {
            self.next_id = id;
        }
    }

    /// Return the `PaneId` that the next `create_and_register*` call will use,
    /// without advancing the counter.  Used by [`Tab::split_native_editor`] to
    /// pre-allocate a `BufferId` in the `EditorPaneRegistry` before calling
    /// into the registry.
    pub fn peek_next_id(&self) -> PaneId {
        self.next_id
    }

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

    /// Allocate a fresh `PaneId`, create a native editor `Pane` (no terminal),
    /// register it, and return the id.  The caller must separately register
    /// a `Buffer` in the tab's `EditorPaneRegistry` using this id.
    pub fn create_and_register_editor(&mut self, buffer_id: anvil_editor::BufferId) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        let pane = Pane::new_editor(id, buffer_id);
        self.map.insert(id, pane);
        id
    }

    /// Look up a pane by id.
    pub fn get(&self, id: PaneId) -> Option<&Pane> {
        self.map.get(&id)
    }

    /// Iterate over registered panes in stable id order.
    pub fn iter(&self) -> impl Iterator<Item = (PaneId, &Pane)> {
        let mut panes: Vec<(PaneId, &Pane)> =
            self.map.iter().map(|(&id, pane)| (id, pane)).collect();
        panes.sort_by_key(|(id, _)| *id);
        panes.into_iter()
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
        assert_eq!(p.scroll_target, 0.0);
        assert_eq!(p.scroll_vel, 0.0);
        assert_eq!(p.cursor_ax, 0.0);
        assert_eq!(p.cursor_ay, 0.0);
        assert!(!p.selection.active);
        assert_eq!(p.folded_count, 0);
    }

    // ── Fold state ────────────────────────────────────────────────────────────

    #[test]
    fn toggle_fold_folds_and_unfolds_round_trip() {
        let mut p = Pane::new(1, 80, 24, 0);
        assert!(!p.is_folded(10));
        p.toggle_fold(10);
        assert!(p.is_folded(10));
        p.toggle_fold(10);
        assert!(!p.is_folded(10));
    }

    #[test]
    fn registry_iter_returns_panes_in_id_order() {
        let mut reg = PaneRegistry::default();
        let first = reg.create_and_register(80, 24, 0);
        let second = reg.create_and_register_editor(7);

        let ids: Vec<PaneId> = reg.iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![first, second]);
    }

    #[test]
    fn toggle_fold_multiple_distinct_blocks() {
        let mut p = Pane::new(1, 80, 24, 0);
        p.toggle_fold(5);
        p.toggle_fold(20);
        assert!(p.is_folded(5));
        assert!(p.is_folded(20));
        assert!(!p.is_folded(0));
        assert_eq!(p.folded_count, 2);
    }

    #[test]
    fn toggle_fold_unfold_leaves_other_blocks_intact() {
        let mut p = Pane::new(1, 80, 24, 0);
        p.toggle_fold(1);
        p.toggle_fold(2);
        p.toggle_fold(3);
        p.toggle_fold(2); // unfold middle
        assert!(p.is_folded(1));
        assert!(!p.is_folded(2));
        assert!(p.is_folded(3));
        assert_eq!(p.folded_count, 2);
    }

    #[test]
    fn toggle_fold_saturates_at_cap() {
        let mut p = Pane::new(1, 80, 24, 0);
        for i in 0..MAX_FOLDED {
            p.toggle_fold(i + 100);
        }
        assert_eq!(p.folded_count, MAX_FOLDED);
        // Extra toggle beyond cap is silently dropped.
        p.toggle_fold(999);
        assert_eq!(p.folded_count, MAX_FOLDED);
        assert!(!p.is_folded(999));
    }

    // ── unseen_rows ───────────────────────────────────────────────────────────

    #[test]
    fn unseen_rows_zero_when_no_baseline() {
        let p = Pane::new(1, 80, 24, 0);
        assert_eq!(p.unseen_rows(), 0);
    }

    #[test]
    fn unseen_rows_zero_when_baseline_equals_current() {
        let mut p = Pane::new(1, 80, 24, 0);
        let base = p.terminal().unwrap().line_count();
        p.unseen_baseline = Some(base);
        assert_eq!(p.unseen_rows(), 0);
    }

    #[test]
    fn unseen_rows_counts_lines_added_after_baseline() {
        let mut p = Pane::new(1, 80, 24, 100);
        let base = p.terminal().unwrap().line_count();
        p.unseen_baseline = Some(base);
        // Feed more lines than the active grid height so rows push into scrollback,
        // which increases line_count() beyond the baseline.
        for _ in 0..30 {
            p.terminal_mut().unwrap().feed(b"line of output\r\n");
        }
        assert!(p.unseen_rows() > 0);
    }

    // ── Pane::terminal / terminal_mut ─────────────────────────────────────────

    #[test]
    fn pane_terminal_accessor_returns_correct_dimensions() {
        let p = Pane::new(1, 80, 24, 0);
        assert_eq!(p.terminal().unwrap().cols(), 80);
        assert_eq!(p.terminal().unwrap().rows(), 24);
    }

    #[test]
    fn pane_terminal_mut_allows_write() {
        let mut p = Pane::new(1, 80, 24, 0);
        p.terminal_mut().unwrap().feed(b"hi");
        // No panic, terminal consumed the bytes.
        assert_eq!(p.terminal().unwrap().cols(), 80);
    }

    // ── Pane::new_editor ─────────────────────────────────────────────────────

    #[test]
    fn pane_new_editor_has_no_terminal() {
        let p = Pane::new_editor(99, 7);
        assert!(p.terminal.is_none());
        assert_eq!(p.editor_id, Some(7));
    }

    #[test]
    fn pane_new_editor_unseen_rows_zero() {
        let p = Pane::new_editor(1, 42);
        assert_eq!(p.unseen_rows(), 0);
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
        assert_eq!(pane.terminal().unwrap().cols(), 80);
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
