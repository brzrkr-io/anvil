//! Terminal tabs: each `Tab` owns a `PaneTree` (layout) and a `PaneRegistry`
//! (pure state).  `TabManager` owns the tab list.
//!
//! PTY creation and reader-thread spawning are platform concerns; this module
//! is pure.

use crate::layout::{PaneId, PaneTree, SplitDir};
use crate::pane::PaneRegistry;

/// True when a tab bar should be drawn — only with 2+ tabs (low-profile rule).
pub fn bar_visible(count: usize) -> bool {
    count >= 2
}

/// Clamp an arbitrary index to `[0, count-1]`.  `count` is assumed ≥ 1.
pub fn clamp_index(count: usize, index: usize) -> usize {
    if count == 0 {
        return 0;
    }
    index.min(count - 1)
}

/// The active index after stepping `delta` (+1 / -1) with wraparound.
/// `count` is assumed ≥ 1.
pub fn wrap_index(count: usize, index: usize, delta: isize) -> usize {
    if count == 0 {
        return 0;
    }
    let c = count as isize;
    let i = (index as isize + delta).rem_euclid(c);
    i as usize
}

/// The active index after the tab at `closed` is removed from a list that had
/// `count` tabs (so `count-1` remain).  `active` is the index before removal.
///
/// Rule: if a tab before the active one closed, the active shifts down by one;
/// if the active tab itself closed, stay at the same slot unless it was the
/// last, then step back; tabs after the active are unaffected.
pub fn next_active_after_close(count: usize, closed: usize, active: usize) -> usize {
    if count <= 1 {
        return 0;
    }
    let remaining = count - 1;
    if closed < active {
        return active - 1;
    }
    if closed > active {
        return active;
    }
    // The active tab itself closed.
    active.min(remaining - 1)
}

/// Return the last component of `path`, ignoring a single trailing slash.
pub fn basename(path: &str) -> &str {
    let p = if path.len() > 1 && path.ends_with('/') {
        &path[..path.len() - 1]
    } else {
        path
    };
    match p.rfind('/') {
        Some(i) => &p[i + 1..],
        None => p,
    }
}

/// One terminal tab: owns a `PaneTree` (layout) and a `PaneRegistry` (pure
/// pane state).
///
/// The platform layer owns a parallel `HashMap<PaneId, Pty>` and
/// `HashMap<PaneId, ReaderThread>` to back each pane with a real shell.
pub struct Tab {
    pub tree: PaneTree,
    pub registry: PaneRegistry,
}

impl Tab {
    /// Create a tab from an existing tree and registry.  The caller is
    /// responsible for creating the PTY for the initial pane.
    pub fn new(tree: PaneTree, registry: PaneRegistry) -> Self {
        Self { tree, registry }
    }

    /// Create a tab with a single pane backed by a fresh `cols × rows`
    /// terminal.  No PTY is created — that is a platform concern.
    pub fn new_single_pane(cols: usize, rows: usize, scrollback: usize) -> Self {
        let mut registry = PaneRegistry::default();
        let first_id = registry.create_and_register(cols, rows, scrollback);
        let tree = PaneTree::init_single(first_id);
        Self { tree, registry }
    }

    /// The id of the focused pane.
    pub fn focused_id(&self) -> PaneId {
        self.tree.focused
    }

    /// Split the focused pane, adding a new pure pane in `dir`.
    /// The caller must subsequently create a PTY for the returned `PaneId`.
    pub fn split(
        &mut self,
        dir: SplitDir,
        cols: usize,
        rows: usize,
        scrollback: usize,
    ) -> Result<PaneId, crate::layout::LayoutError> {
        let new_id = self.registry.create_and_register(cols, rows, scrollback);
        self.tree.split(dir, new_id)?;
        Ok(new_id)
    }
}

/// A hard cap on tabs.
pub const MAX_TABS: usize = 32;

#[derive(Default)]
pub struct TabManager {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl TabManager {
    pub fn count(&self) -> usize {
        self.tabs.len()
    }

    pub fn current(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn current_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn bar_visible(&self) -> bool {
        bar_visible(self.tabs.len())
    }

    /// Add `tab` and make it active.  No-op once `MAX_TABS` is reached.
    pub fn push(&mut self, tab: Tab) {
        if self.tabs.len() >= MAX_TABS {
            return;
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    /// Close the active tab.  Returns `true` if tabs remain.
    pub fn close_active(&mut self) -> bool {
        self.close_at(self.active)
    }

    /// Close the tab at `index`.  Returns `true` if tabs remain.
    pub fn close_at(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return !self.tabs.is_empty();
        }
        let old_count = self.tabs.len();
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            return false;
        }
        self.active = next_active_after_close(old_count, index, self.active);
        true
    }

    pub fn switch_to(&mut self, index: usize) {
        self.active = clamp_index(self.tabs.len(), index);
    }

    pub fn next(&mut self) {
        self.active = wrap_index(self.tabs.len(), self.active, 1);
    }

    pub fn prev(&mut self) {
        self.active = wrap_index(self.tabs.len(), self.active, -1);
    }
}

// ---------------------------------------------------------------------------
// Tests  (6 Zig tests → 6 Rust tests; label/PTY tests not applicable here)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_visible_only_at_two_plus_tabs() {
        assert!(!bar_visible(0));
        assert!(!bar_visible(1));
        assert!(bar_visible(2));
        assert!(bar_visible(9));
    }

    #[test]
    fn clamp_index_pins_to_range() {
        assert_eq!(clamp_index(3, 2), 2);
        assert_eq!(clamp_index(3, 99), 2);
        assert_eq!(clamp_index(1, 5), 0);
    }

    #[test]
    fn wrap_index_wraps_both_directions() {
        assert_eq!(wrap_index(3, 0, 1), 1);
        assert_eq!(wrap_index(3, 2, 1), 0); // wrap forward
        assert_eq!(wrap_index(3, 0, -1), 2); // wrap backward
        assert_eq!(wrap_index(1, 0, 1), 0); // single tab
    }

    #[test]
    fn next_active_after_close_handles_every_position() {
        // 3 tabs, active = 1.
        assert_eq!(next_active_after_close(3, 0, 1), 0); // closed before active
        assert_eq!(next_active_after_close(3, 2, 1), 1); // closed after active
        assert_eq!(next_active_after_close(3, 1, 1), 1); // closed the active (middle)
        // closing the active *last* tab steps back
        assert_eq!(next_active_after_close(3, 2, 2), 1);
        // closing down to one tab
        assert_eq!(next_active_after_close(2, 0, 0), 0);
    }

    #[test]
    fn basename_extracts_last_path_component() {
        assert_eq!(basename("/Users/x/anvil"), "anvil");
        assert_eq!(basename("/Users/x/anvil/"), "anvil");
        assert_eq!(basename("x"), "x");
        assert_eq!(basename("/"), "");
    }

    #[test]
    fn tab_manager_index_logic_switch_next_prev_close() {
        let mut mgr = TabManager::default();

        // Build 3 tabs with tiny terminals (1×1, 0 scrollback).
        for _ in 0..3 {
            let tab = Tab::new_single_pane(1, 1, 0);
            mgr.push(tab);
        }
        mgr.active = 0;

        mgr.next();
        assert_eq!(mgr.active, 1);
        mgr.prev();
        mgr.prev();
        assert_eq!(mgr.active, 2); // wrapped
        mgr.switch_to(99);
        assert_eq!(mgr.active, 2); // clamped
        mgr.switch_to(0);
        assert_eq!(mgr.active, 0);

        // Remove index 0 while active=0: stays at slot 0.
        mgr.tabs.remove(0);
        mgr.active = next_active_after_close(3, 0, 0);
        assert_eq!(mgr.active, 0);
        assert_eq!(mgr.count(), 2);
    }
}
