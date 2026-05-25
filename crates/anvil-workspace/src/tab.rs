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
    /// True when this tab has received PTY output since it was last focused.
    /// Cleared on focus; set by the tick loop for background tabs.
    pub has_unread: bool,
    /// Animation phase: 0.0 = invisible (close complete), 1.0 = fully open.
    /// Driven by the tick loop toward `target_phase`.
    pub anim_phase: f32,
    /// Target for `anim_phase`: 1.0 when opening, 0.0 when closing.
    pub target_phase: f32,
}

impl Tab {
    /// Create a tab from an existing tree and registry.  The caller is
    /// responsible for creating the PTY for the initial pane.
    pub fn new(tree: PaneTree, registry: PaneRegistry) -> Self {
        Self {
            tree,
            registry,
            has_unread: false,
            anim_phase: 0.0,
            target_phase: 1.0,
        }
    }

    /// Clear the unread indicator (called when the tab gains focus).
    pub fn clear_unread(&mut self) {
        self.has_unread = false;
    }

    /// Create a tab with a single pane backed by a fresh `cols × rows`
    /// terminal.  No PTY is created — that is a platform concern.
    pub fn new_single_pane(cols: usize, rows: usize, scrollback: usize) -> Self {
        Self::new_single_pane_starting_at(1, cols, rows, scrollback)
    }

    /// Like [`new_single_pane`] but the pane gets `start_id` (or higher).
    /// The caller is responsible for choosing an id that doesn't collide
    /// with PaneIds in OTHER tabs — `self.ptys` in the App is keyed by
    /// PaneId across all tabs.
    pub fn new_single_pane_starting_at(
        start_id: PaneId,
        cols: usize,
        rows: usize,
        scrollback: usize,
    ) -> Self {
        let mut registry = PaneRegistry::default();
        registry.set_next_id_at_least(start_id);
        let first_id = registry.create_and_register(cols, rows, scrollback);
        let tree = PaneTree::init_single(first_id);
        Self {
            tree,
            registry,
            has_unread: false,
            anim_phase: 0.0,
            target_phase: 1.0,
        }
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

    /// Begin an animated close of the tab at `index`: set target_phase = 0 so
    /// the tick loop fades it out, then calls [`purge_closed_tabs`] to remove it.
    /// Also updates `active` immediately so the user sees the next tab.
    /// Returns `true` if a non-closing tab remains (app should not quit).
    pub fn begin_close_at(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return !self.tabs.is_empty();
        }
        // Count tabs that are not already closing.
        let live = self.tabs.iter().filter(|t| t.target_phase > 0.0).count();
        if live <= 1 {
            // Last live tab — skip animation; caller handles termination.
            return false;
        }
        self.tabs[index].target_phase = 0.0;
        let old_count = self.tabs.len();
        // Adjust active to point at the next non-closing tab.
        self.active = next_active_after_close(old_count, index, self.active);
        if self.tabs.get(self.active).is_some_and(|t| t.target_phase == 0.0) {
            if let Some(i) = self.tabs.iter().position(|t| t.target_phase > 0.0) {
                self.active = i;
            }
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.clear_unread();
        }
        true
    }

    /// Remove tabs whose `anim_phase` has reached 0 with `target_phase` == 0.
    /// Returns `true` if any tabs were removed.
    pub fn purge_closed_tabs(&mut self) -> bool {
        let before = self.tabs.len();
        let mut i = 0;
        while i < self.tabs.len() {
            if self.tabs[i].target_phase == 0.0 && self.tabs[i].anim_phase <= 0.0 {
                let old_count = self.tabs.len();
                self.tabs.remove(i);
                if self.active >= i && self.active > 0 {
                    self.active = next_active_after_close(old_count, i, self.active);
                }
            } else {
                i += 1;
            }
        }
        self.tabs.len() < before
    }

    pub fn switch_to(&mut self, index: usize) {
        self.active = clamp_index(self.tabs.len(), index);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.clear_unread();
        }
    }

    pub fn next(&mut self) {
        self.active = wrap_index(self.tabs.len(), self.active, 1);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.clear_unread();
        }
    }

    pub fn prev(&mut self) {
        self.active = wrap_index(self.tabs.len(), self.active, -1);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.clear_unread();
        }
    }

    /// Move the tab at `from` to position `to`, preserving the active tab by
    /// identity.  If `from == to` or either index is out of bounds, this is a
    /// no-op.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        let n = self.tabs.len();
        if from == to || from >= n || to >= n {
            return;
        }
        // Remember which index is currently active before we shuffle.
        let active_idx = self.active;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        // Fix up active: the moved element lands at `to`; other elements shift.
        self.active = if active_idx == from {
            to
        } else if from < active_idx && to >= active_idx {
            active_idx - 1
        } else if from > active_idx && to <= active_idx {
            active_idx + 1
        } else {
            active_idx
        };
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

    // ── Tab::new (direct constructor) ────────────────────────────────────────

    #[test]
    fn tab_new_sets_tree_and_registry() {
        use crate::pane::PaneRegistry;
        let registry = PaneRegistry::default();
        let tree = crate::layout::PaneTree::init_single(42);
        let tab = Tab::new(tree, registry);
        assert_eq!(tab.focused_id(), 42);
    }

    // ── Tab::split ────────────────────────────────────────────────────────────

    #[test]
    fn tab_split_horizontal_adds_pane() {
        let mut tab = Tab::new_single_pane(80, 24, 0);
        let id = tab.focused_id();
        let new_id = tab
            .split(crate::layout::SplitDir::Horizontal, 40, 24, 0)
            .unwrap();
        assert_ne!(new_id, id);
        // Both panes are in the registry.
        assert!(tab.registry.get(id).is_some());
        assert!(tab.registry.get(new_id).is_some());
    }

    #[test]
    fn tab_split_vertical_adds_pane() {
        let mut tab = Tab::new_single_pane(80, 24, 0);
        let new_id = tab
            .split(crate::layout::SplitDir::Vertical, 80, 12, 0)
            .unwrap();
        assert!(tab.registry.get(new_id).is_some());
    }

    // ── TabManager::current / current_mut ─────────────────────────────────────

    #[test]
    fn tab_manager_current_returns_none_when_empty() {
        let mgr = TabManager::default();
        assert!(mgr.current().is_none());
    }

    #[test]
    fn tab_manager_current_returns_active_tab() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(80, 24, 0));
        assert!(mgr.current().is_some());
    }

    #[test]
    fn tab_manager_current_mut_returns_active_tab() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(80, 24, 0));
        assert!(mgr.current_mut().is_some());
    }

    // ── TabManager::push MAX_TABS cap ─────────────────────────────────────────

    #[test]
    fn tab_manager_push_stops_at_max_tabs() {
        let mut mgr = TabManager::default();
        for _ in 0..MAX_TABS + 5 {
            mgr.push(Tab::new_single_pane(1, 1, 0));
        }
        assert_eq!(mgr.count(), MAX_TABS);
    }

    // ── TabManager::close_at edge cases ───────────────────────────────────────

    #[test]
    fn close_at_out_of_bounds_returns_nonempty() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        // index out of bounds; tabs are not empty so returns true
        assert!(mgr.close_at(99));
        assert_eq!(mgr.count(), 1);
    }

    #[test]
    fn close_at_last_tab_returns_false() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        assert!(!mgr.close_at(0));
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn close_active_removes_and_adjusts_index() {
        let mut mgr = TabManager::default();
        for _ in 0..3 {
            mgr.push(Tab::new_single_pane(1, 1, 0));
        }
        mgr.active = 1;
        assert!(mgr.close_active());
        assert_eq!(mgr.count(), 2);
    }

    // ── clamp_index edge: count == 0 ─────────────────────────────────────────

    #[test]
    fn clamp_index_zero_count_returns_zero() {
        assert_eq!(clamp_index(0, 5), 0);
    }

    // ── wrap_index edge: count == 0 ──────────────────────────────────────────

    #[test]
    fn wrap_index_zero_count_returns_zero() {
        assert_eq!(wrap_index(0, 0, 1), 0);
        assert_eq!(wrap_index(0, 0, -1), 0);
    }

    // ── next_active_after_close edge: count <= 1 ─────────────────────────────

    #[test]
    fn next_active_after_close_count_one_returns_zero() {
        assert_eq!(next_active_after_close(1, 0, 0), 0);
        assert_eq!(next_active_after_close(0, 0, 0), 0);
    }

    // ── TabManager::bar_visible ───────────────────────────────────────────────

    #[test]
    fn tab_manager_bar_visible_false_for_one_tab() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        assert!(!mgr.bar_visible());
    }

    #[test]
    fn tab_manager_bar_visible_true_for_two_tabs() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.push(Tab::new_single_pane(1, 1, 0));
        assert!(mgr.bar_visible());
    }

    // ── has_unread / clear_unread ─────────────────────────────────────────────

    #[test]
    fn tab_clear_unread_resets_flag() {
        let mut tab = Tab::new_single_pane(80, 24, 0);
        tab.has_unread = true;
        tab.clear_unread();
        assert!(!tab.has_unread);
    }

    #[test]
    fn tab_manager_switch_to_clears_unread() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.active = 0;
        mgr.tabs[1].has_unread = true;
        mgr.switch_to(1);
        assert!(!mgr.tabs[1].has_unread);
    }

    #[test]
    fn tab_manager_next_clears_unread() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.active = 0;
        mgr.tabs[1].has_unread = true;
        mgr.next();
        assert_eq!(mgr.active, 1);
        assert!(!mgr.tabs[1].has_unread);
    }

    #[test]
    fn tab_manager_prev_clears_unread() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.active = 1;
        mgr.tabs[0].has_unread = true;
        mgr.prev();
        assert_eq!(mgr.active, 0);
        assert!(!mgr.tabs[0].has_unread);
    }

    // ── anim_phase / target_phase ─────────────────────────────────────────────

    #[test]
    fn tab_new_starts_animating_in() {
        let tab = Tab::new_single_pane(80, 24, 0);
        assert_eq!(tab.anim_phase, 0.0);
        assert_eq!(tab.target_phase, 1.0);
    }

    #[test]
    fn begin_close_at_sets_target_and_moves_active() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.active = 1;
        // Manually advance anim_phase so tabs are "live"
        mgr.tabs[0].anim_phase = 1.0;
        mgr.tabs[1].anim_phase = 1.0;

        let remains = mgr.begin_close_at(1);
        assert!(remains, "should remain after closing non-last tab");
        assert_eq!(mgr.tabs[1].target_phase, 0.0);
        assert_eq!(mgr.active, 0, "active should move to remaining tab");
        // Tab count unchanged until purge
        assert_eq!(mgr.count(), 2);
    }

    #[test]
    fn begin_close_at_last_live_returns_false() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.tabs[0].anim_phase = 1.0;
        let remains = mgr.begin_close_at(0);
        assert!(!remains, "last live tab should return false");
    }

    // ── TabManager::move_tab ──────────────────────────────────────────────────

    #[test]
    fn move_tab_forward() {
        // tabs: [0, 1, 2, 3], active=1 → move tab 1 to 3 → [0, 2, 3, 1], active=3
        let mut mgr = TabManager::default();
        for _ in 0..4 {
            mgr.push(Tab::new_single_pane(1, 1, 0));
        }
        mgr.active = 1;
        mgr.move_tab(1, 3);
        assert_eq!(mgr.active, 3, "active must follow the moved tab");
        assert_eq!(mgr.count(), 4);
    }

    #[test]
    fn move_tab_backward() {
        // tabs: [0, 1, 2, 3], active=3 → move tab 3 to 1 → [0, 3, 1, 2], active=1
        let mut mgr = TabManager::default();
        for _ in 0..4 {
            mgr.push(Tab::new_single_pane(1, 1, 0));
        }
        mgr.active = 3;
        mgr.move_tab(3, 1);
        assert_eq!(mgr.active, 1, "active must follow the moved tab");
        assert_eq!(mgr.count(), 4);
    }

    #[test]
    fn move_tab_noop_same_index() {
        let mut mgr = TabManager::default();
        for _ in 0..3 {
            mgr.push(Tab::new_single_pane(1, 1, 0));
        }
        mgr.active = 1;
        mgr.move_tab(1, 1);
        assert_eq!(mgr.active, 1);
        assert_eq!(mgr.count(), 3);
    }

    #[test]
    fn move_tab_non_active_follows_active() {
        // tabs: [0, 1, 2], active=1. Move tab 0 past active (0→2).
        // Active was at index 1; after remove(0) it shifts to 0; after insert(2) it stays 0.
        // expected active = 1 - 1 = 0 (from < active && to >= active branch).
        let mut mgr = TabManager::default();
        for _ in 0..3 {
            mgr.push(Tab::new_single_pane(1, 1, 0));
        }
        mgr.active = 1;
        mgr.move_tab(0, 2);
        assert_eq!(mgr.active, 0, "active index adjusts when a tab before it moves past it");
    }

    #[test]
    fn move_tab_out_of_bounds_is_noop() {
        let mut mgr = TabManager::default();
        for _ in 0..2 {
            mgr.push(Tab::new_single_pane(1, 1, 0));
        }
        mgr.active = 0;
        mgr.move_tab(0, 5); // to out of bounds
        assert_eq!(mgr.active, 0);
        assert_eq!(mgr.count(), 2);
        mgr.move_tab(5, 0); // from out of bounds
        assert_eq!(mgr.active, 0);
    }

    #[test]
    fn purge_closed_tabs_removes_finished_tabs() {
        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.push(Tab::new_single_pane(1, 1, 0));
        mgr.tabs[0].anim_phase = 1.0;
        mgr.tabs[1].anim_phase = 1.0;
        mgr.begin_close_at(0);
        // Simulate phase reaching 0
        mgr.tabs[0].anim_phase = 0.0;
        let removed = mgr.purge_closed_tabs();
        assert!(removed);
        assert_eq!(mgr.count(), 1);
    }
}
