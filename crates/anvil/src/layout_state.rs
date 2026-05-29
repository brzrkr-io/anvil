use std::path::PathBuf;

use anvil_workspace::layout::PaneId;
use anvil_workspace::mode::LayoutMode;
use anvil_workspace::tab::{Tab, TabManager};

// ── Explorer support types (items 4, 6, 7, 8) ────────────────────────────────

/// Which surface has keyboard focus for key routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FocusTarget {
    #[default]
    Editor,
    Explorer,
    Terminal,
}

pub(crate) fn normalize_input_state_for_layout_mode(
    mode: LayoutMode,
    focus_target: &mut FocusTarget,
    selected_explorer_row: &mut Option<usize>,
    explorer_filter: &mut Option<String>,
) {
    *focus_target = match mode {
        LayoutMode::Terminal => FocusTarget::Terminal,
        LayoutMode::Ide => FocusTarget::Editor,
    };
    *selected_explorer_row = None;
    *explorer_filter = None;
}

pub(crate) fn normalize_tabs_for_layout_mode(mode: LayoutMode, tabs: &mut TabManager) {
    for tab in &mut tabs.tabs {
        match mode {
            LayoutMode::Terminal => {
                tab.hide_to_terminal_surface();
            }
            LayoutMode::Ide => {
                tab.ensure_ide_editor_surface();
            }
        }
    }
}

pub(crate) fn parse_layout_mode_env_override(value: Option<&str>) -> Option<LayoutMode> {
    match value {
        Some("ide") => Some(LayoutMode::Ide),
        Some("terminal") => Some(LayoutMode::Terminal),
        _ => None,
    }
}

pub(crate) fn sync_terminal_pane_scroll_to_model(pane: &mut anvil_workspace::pane::Pane) {
    let Some(terminal) = pane.terminal.as_ref() else {
        return;
    };
    let offset = terminal.viewport_offset() as f32;
    pane.scroll_pos = offset;
    pane.scroll_target = offset;
    pane.scroll_vel = 0.0;
    if offset == 0.0 {
        pane.unseen_baseline = None;
    }
}

pub(crate) trait LayoutTransitionSlot {
    fn clear_for_layout_transition(&mut self) -> bool;
}

impl LayoutTransitionSlot for bool {
    fn clear_for_layout_transition(&mut self) -> bool {
        let was_active = *self;
        *self = false;
        was_active
    }
}

impl<T> LayoutTransitionSlot for Option<T> {
    fn clear_for_layout_transition(&mut self) -> bool {
        let was_active = self.is_some();
        *self = None;
        was_active
    }
}

pub(crate) fn clear_layout_transition_slot<T: LayoutTransitionSlot>(slot: &mut T) -> bool {
    slot.clear_for_layout_transition()
}

pub(crate) fn drawer_tracking_from_tab<F>(
    tab: &Tab,
    mut is_live_pty: F,
    previous_active: Option<PaneId>,
) -> (Vec<PaneId>, usize)
where
    F: FnMut(PaneId) -> bool,
{
    let ids: Vec<PaneId> = tab
        .terminal_pane_ids()
        .into_iter()
        .filter(|id| is_live_pty(*id))
        .collect();
    let active = previous_active
        .and_then(|id| ids.iter().position(|candidate| *candidate == id))
        .unwrap_or(0);
    (ids, active)
}

/// Inline rename state for an Explorer row (item 6).
pub struct RenameState {
    /// Absolute path of the entry being renamed.
    pub old_path: PathBuf,
    /// Current text in the rename input field (starts as the basename).
    pub input: String,
}

/// Ghost-row creation state for new-file / new-folder (item 7).
pub struct NewItemState {
    /// Directory in which the new entry will be created.
    pub parent_dir: PathBuf,
    /// Current text typed by the user (empty on open).
    pub input: String,
    /// True → create directory; false → create file.
    pub is_dir: bool,
}

/// Pending delete confirmation state (item 8).
pub struct DeleteConfirm {
    /// Absolute path of the item to delete.
    pub path: PathBuf,
}
