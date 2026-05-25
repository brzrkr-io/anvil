//! Native editor pane state and registry — NE4.
//!
//! `EditorPane` is the per-pane view state for a native editor.  `EditorPaneRegistry`
//! holds both the per-pane view state and the underlying `Buffer`s, keyed by
//! `PaneId` and `BufferId` respectively.  It lives alongside `PaneRegistry` on `Tab`.

use std::collections::HashMap;

use anvil_editor::{Buffer, BufferId, Cursor, Position};

use crate::layout::PaneId;
use crate::selection::Selection;

/// Per-pane view state for a native editor pane.
pub struct EditorPane {
    pub buffer_id: BufferId,
    pub cursor: Cursor,
    pub selection: Selection,
    pub scroll_pos: f32,
    pub scroll_target: f32,
    pub scroll_vel: f32,
}

/// Registry of all native editor panes and their buffers for one `Tab`.
///
/// `panes` maps a `PaneId` to its `EditorPane` view state.
/// `buffers` maps a `BufferId` to the underlying `Buffer`.
/// `next_buffer_id` is a monotonic counter for allocating fresh `BufferId`s.
pub struct EditorPaneRegistry {
    panes: HashMap<PaneId, EditorPane>,
    buffers: HashMap<BufferId, Buffer>,
    next_buffer_id: BufferId,
}

impl Default for EditorPaneRegistry {
    fn default() -> Self {
        Self {
            panes: HashMap::new(),
            buffers: HashMap::new(),
            next_buffer_id: 1,
        }
    }
}

impl EditorPaneRegistry {
    /// Allocate a fresh `Buffer`, register an `EditorPane` for `pane_id`,
    /// and return the new `BufferId`.
    pub fn new_pane(&mut self, pane_id: PaneId) -> BufferId {
        let buffer_id = self.next_buffer_id;
        self.next_buffer_id += 1;
        let origin = Position { line: 0, col: 0 };
        let pane = EditorPane {
            buffer_id,
            cursor: Cursor {
                pos: origin,
                anchor: origin,
            },
            selection: Selection::default(),
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
        };
        self.panes.insert(pane_id, pane);
        self.buffers.insert(buffer_id, Buffer::new());
        buffer_id
    }

    /// Look up the `EditorPane` for `pane_id`.
    pub fn get_pane(&self, pane_id: PaneId) -> Option<&EditorPane> {
        self.panes.get(&pane_id)
    }

    /// Look up the `EditorPane` mutably for `pane_id`.
    pub fn get_pane_mut(&mut self, pane_id: PaneId) -> Option<&mut EditorPane> {
        self.panes.get_mut(&pane_id)
    }

    /// Look up the `Buffer` by `buffer_id`.
    pub fn get_buffer(&self, buffer_id: BufferId) -> Option<&Buffer> {
        self.buffers.get(&buffer_id)
    }

    /// Look up the `Buffer` mutably by `buffer_id`.
    pub fn get_buffer_mut(&mut self, buffer_id: BufferId) -> Option<&mut Buffer> {
        self.buffers.get_mut(&buffer_id)
    }

    /// Remove the `EditorPane` for `pane_id` and drop its buffer.
    ///
    /// No-op if `pane_id` is not registered.
    pub fn remove_pane(&mut self, pane_id: PaneId) {
        if let Some(pane) = self.panes.remove(&pane_id) {
            self.buffers.remove(&pane.buffer_id);
        }
    }

    /// Number of registered editor panes.
    pub fn count(&self) -> usize {
        self.panes.len()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_pane_registry_new_pane_returns_buffer_id() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(1);
        // Buffer id must be a non-zero sentinel (our counter starts at 1).
        assert!(bid > 0);
        // The pane must be findable.
        assert!(reg.get_pane(1).is_some());
    }

    #[test]
    fn editor_pane_registry_get_buffer_round_trip() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(42);
        // Buffer must be present and empty.
        let buf = reg
            .get_buffer(bid)
            .expect("buffer should exist after new_pane");
        assert_eq!(buf.char_count(), 0);
    }

    #[test]
    fn editor_pane_registry_remove_pane_drops_buffer() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(7);
        assert_eq!(reg.count(), 1);
        reg.remove_pane(7);
        assert_eq!(reg.count(), 0);
        // Buffer is gone.
        assert!(reg.get_buffer(bid).is_none());
    }

    #[test]
    fn editor_pane_registry_multiple_panes_independent() {
        let mut reg = EditorPaneRegistry::default();
        let bid1 = reg.new_pane(1);
        let bid2 = reg.new_pane(2);
        assert_ne!(bid1, bid2, "each pane must get a unique buffer id");
        assert_eq!(reg.count(), 2);
        // Remove one; other stays.
        reg.remove_pane(1);
        assert_eq!(reg.count(), 1);
        assert!(reg.get_pane(2).is_some());
        assert!(reg.get_buffer(bid2).is_some());
    }
}
