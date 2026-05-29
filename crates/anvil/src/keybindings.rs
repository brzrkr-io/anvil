use anvil_config::{Chord, parse_chord};

// ── Keybindings ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub(crate) struct Keybindings {
    pub(crate) new_tab: Option<Chord>,
    pub(crate) close_tab: Option<Chord>,
    pub(crate) next_tab: Option<Chord>,
    pub(crate) prev_tab: Option<Chord>,
    pub(crate) jump: [Option<Chord>; 9],
    pub(crate) search_open: Option<Chord>,
    /// Cmd+Opt+Shift+F: open search bar scoped to the current block (moved from Cmd+Shift+F).
    pub(crate) search_open_block: Option<Chord>,
    /// Cmd+Shift+F: open the project-wide search overlay.
    pub(crate) project_search_open: Option<Chord>,
    pub(crate) search_next: Option<Chord>,
    pub(crate) search_prev: Option<Chord>,
    /// Cmd+Opt+R: toggle regex mode while the search bar is open.
    pub(crate) search_regex_toggle: Option<Chord>,
    pub(crate) hud_toggle: Option<Chord>,
    pub(crate) cheatsheet: Option<Chord>,
    pub(crate) split_right: Option<Chord>,
    pub(crate) split_down: Option<Chord>,
    pub(crate) close_pane: Option<Chord>,
    pub(crate) focus_left: Option<Chord>,
    pub(crate) focus_right: Option<Chord>,
    pub(crate) focus_up: Option<Chord>,
    pub(crate) focus_down: Option<Chord>,
    pub(crate) fold_block: Option<Chord>,
    pub(crate) toggle_theme: Option<Chord>,
    pub(crate) layout_mode_toggle: Option<Chord>,
    pub(crate) left_dock_toggle: Option<Chord>,
    pub(crate) recent_files: Option<Chord>,
    pub(crate) editor_new: Option<Chord>,
    /// Cmd+T: open workspace symbol search overlay (O1).
    pub(crate) workspace_symbol_search: Option<Chord>,
    /// Cmd+R: open buffer symbol search overlay (O2).
    pub(crate) buffer_symbol_search: Option<Chord>,
}

impl Keybindings {
    pub(crate) fn from_config(kb: &anvil_config::Keybindings) -> Self {
        let jump_strs = [
            &kb.tab_1, &kb.tab_2, &kb.tab_3, &kb.tab_4, &kb.tab_5, &kb.tab_6, &kb.tab_7, &kb.tab_8,
            &kb.tab_9,
        ];
        let mut jump = [None; 9];
        for (i, s) in jump_strs.iter().enumerate() {
            jump[i] = parse_chord(s);
        }
        Self {
            new_tab: parse_chord(&kb.new_tab),
            close_tab: parse_chord(&kb.close_tab),
            next_tab: parse_chord(&kb.next_tab),
            prev_tab: parse_chord(&kb.prev_tab),
            jump,
            search_open: parse_chord(&kb.search_open),
            search_open_block: parse_chord("cmd+opt+shift+f"),
            project_search_open: parse_chord(&kb.project_search),
            search_next: parse_chord(&kb.search_next),
            search_prev: parse_chord(&kb.search_prev),
            search_regex_toggle: parse_chord("cmd+opt+r"),
            hud_toggle: parse_chord(&kb.hud_toggle),
            cheatsheet: parse_chord(&kb.cheatsheet_toggle),
            split_right: parse_chord(&kb.split_right),
            split_down: parse_chord(&kb.split_down),
            close_pane: parse_chord(&kb.close_pane),
            focus_left: parse_chord(&kb.focus_left),
            focus_right: parse_chord(&kb.focus_right),
            focus_up: parse_chord(&kb.focus_up),
            focus_down: parse_chord(&kb.focus_down),
            fold_block: parse_chord(&kb.fold_block),
            toggle_theme: parse_chord(&kb.toggle_theme),
            layout_mode_toggle: parse_chord(&kb.layout_mode_toggle),
            left_dock_toggle: parse_chord(&kb.left_dock_toggle),
            recent_files: parse_chord(&kb.recent_files),
            editor_new: parse_chord(&kb.editor_new),
            workspace_symbol_search: parse_chord("cmd+t"),
            buffer_symbol_search: parse_chord("cmd+r"),
        }
    }
}
