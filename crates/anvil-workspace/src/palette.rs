//! Command-palette controller: the static command catalog, id→action mapping,
//! and the summon/dismiss + ready-handshake state machine.  Pure — no platform
//! I/O.

/// A native action a palette command runs.  The host maps each to real work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    ThemeDark,
    ThemeLight,
    ThemeSystem,
    ConfigReload,
    ClearScreen,
    ScrollTop,
    ScrollBottom,
    AppQuit,
    HudToggle,
    CheatsheetShow,
    // Dynamic: switch to tab by index.
    SwitchTab(usize),
    // Layout modes.
    LayoutTerminal,
    LayoutIde,
    // Agent actions (only shown when Caldera is Live).
    AgentApprove,
    AgentStart,
    // Editor pane.
    NewEditorPane,
    // Open selected file in nvim (Cmd+Shift+N).
    OpenInNvim,
}

pub struct Entry {
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: Option<&'static str>,
    pub action: Action,
}

/// The M3 command set.
pub const CATALOG: &[Entry] = &[
    Entry {
        id: "theme.dark",
        title: "Switch to Ember Dark",
        subtitle: None,
        action: Action::ThemeDark,
    },
    Entry {
        id: "theme.light",
        title: "Switch to Ember Light",
        subtitle: None,
        action: Action::ThemeLight,
    },
    Entry {
        id: "theme.system",
        title: "Follow System Appearance",
        subtitle: Some("Use macOS light/dark appearance"),
        action: Action::ThemeSystem,
    },
    Entry {
        id: "config.reload",
        title: "Reload Config",
        subtitle: None,
        action: Action::ConfigReload,
    },
    Entry {
        id: "terminal.clear",
        title: "Clear Screen",
        subtitle: None,
        action: Action::ClearScreen,
    },
    Entry {
        id: "scroll.top",
        title: "Scroll to Top",
        subtitle: None,
        action: Action::ScrollTop,
    },
    Entry {
        id: "scroll.bottom",
        title: "Scroll to Bottom",
        subtitle: None,
        action: Action::ScrollBottom,
    },
    Entry {
        id: "app.quit",
        title: "Quit Anvil",
        subtitle: None,
        action: Action::AppQuit,
    },
    Entry {
        id: "hud.toggle",
        title: "Toggle HUD",
        subtitle: Some("Show or hide the developer context panel"),
        action: Action::HudToggle,
    },
    Entry {
        id: "cheatsheet.show",
        title: "Keyboard Shortcuts",
        subtitle: Some("Show the keyboard shortcut cheatsheet"),
        action: Action::CheatsheetShow,
    },
    Entry {
        id: "layout.mode:terminal",
        title: "Switch to Straight Terminal",
        subtitle: Some("Hide IDE chrome and focus the preserved terminal (⌘⇧E)"),
        action: Action::LayoutTerminal,
    },
    Entry {
        id: "layout.mode:ide",
        title: "Switch to IDE Surface",
        subtitle: Some("Show Explorer, native editor, and bottom terminal drawer (⌘⇧E)"),
        action: Action::LayoutIde,
    },
    Entry {
        id: "editor.new",
        title: "New Editor Pane",
        subtitle: Some("Open the native editor in a new pane (⌘E)"),
        action: Action::NewEditorPane,
    },
    Entry {
        id: "editor.nvim",
        title: "Open with nvim",
        subtitle: Some("Open selected file in nvim in a new terminal pane (⌘⇧N)"),
        action: Action::OpenInNvim,
    },
];

/// Look up the action for a command id, or `None` if unknown.
///
/// Prefix-routed dynamic IDs are resolved first:
/// - `"tab.switch:{usize}"` → `Action::SwitchTab(n)`
/// - `"layout.mode:terminal"` → `Action::LayoutTerminal`
/// - `"layout.mode:ide"` → `Action::LayoutIde`
/// - `"agent.approve"` → `Action::AgentApprove`
/// - `"agent.start"` → `Action::AgentStart`
pub fn action_for_id(id: &str) -> Option<Action> {
    if let Some(rest) = id.strip_prefix("tab.switch:") {
        return rest.parse::<usize>().ok().map(Action::SwitchTab);
    }
    match id {
        "layout.mode:terminal" => return Some(Action::LayoutTerminal),
        "layout.mode:ide" => return Some(Action::LayoutIde),
        "agent.approve" => return Some(Action::AgentApprove),
        "agent.start" => return Some(Action::AgentStart),
        _ => {}
    }
    CATALOG.iter().find(|e| e.id == id).map(|e| e.action)
}

/// Tracks palette visibility and the webview ready-handshake.  A summon before
/// the webview signals `ready` is deferred and flushed on `on_ready`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Palette {
    pub visible: bool,
    pub ready: bool,
    pub pending_show: bool,
}

impl Palette {
    /// Mark the palette summoned.  Returns `true` if the host should send the
    /// `show` message now; `false` if it must wait for `on_ready`.
    pub fn summon(&mut self) -> bool {
        self.visible = true;
        if self.ready {
            return true;
        }
        self.pending_show = true;
        false
    }

    /// The webview finished loading.  Returns `true` if a deferred `show`
    /// should be sent now.
    pub fn on_ready(&mut self) -> bool {
        self.ready = true;
        if self.pending_show {
            self.pending_show = false;
            return true;
        }
        false
    }

    /// Mark the palette dismissed.
    pub fn dismiss(&mut self) {
        self.visible = false;
        self.pending_show = false;
    }
}

// ---------------------------------------------------------------------------
// Tests  (7 Zig tests → 7 Rust tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_map_to_actions() {
        assert_eq!(action_for_id("theme.dark"), Some(Action::ThemeDark));
        assert_eq!(action_for_id("theme.light"), Some(Action::ThemeLight));
        assert_eq!(action_for_id("theme.system"), Some(Action::ThemeSystem));
        assert_eq!(action_for_id("app.quit"), Some(Action::AppQuit));
        assert_eq!(action_for_id("scroll.top"), Some(Action::ScrollTop));
    }

    #[test]
    fn theme_catalog_names_ember_modes_and_system_appearance() {
        let dark = CATALOG.iter().find(|e| e.id == "theme.dark").unwrap();
        let light = CATALOG.iter().find(|e| e.id == "theme.light").unwrap();
        let system = CATALOG.iter().find(|e| e.id == "theme.system").unwrap();
        assert_eq!(dark.title, "Switch to Ember Dark");
        assert_eq!(light.title, "Switch to Ember Light");
        assert_eq!(system.title, "Follow System Appearance");
    }

    #[test]
    fn unknown_id_has_no_action() {
        assert_eq!(action_for_id("nope.nope"), None);
    }

    #[test]
    fn dynamic_tab_switch_ids_parse() {
        assert_eq!(action_for_id("tab.switch:0"), Some(Action::SwitchTab(0)));
        assert_eq!(action_for_id("tab.switch:3"), Some(Action::SwitchTab(3)));
        assert_eq!(action_for_id("tab.switch:"), None);
        assert_eq!(action_for_id("tab.switch:abc"), None);
    }

    #[test]
    fn layout_mode_ids_parse() {
        assert_eq!(
            action_for_id("layout.mode:terminal"),
            Some(Action::LayoutTerminal)
        );
        assert_eq!(action_for_id("layout.mode:ide"), Some(Action::LayoutIde));
        assert_eq!(action_for_id("layout.mode:codex"), None);
    }

    #[test]
    fn agent_action_ids_parse() {
        assert_eq!(action_for_id("agent.approve"), Some(Action::AgentApprove));
        assert_eq!(action_for_id("agent.start"), Some(Action::AgentStart));
    }

    #[test]
    fn editor_new_id_maps_to_action() {
        assert_eq!(action_for_id("editor.new"), Some(Action::NewEditorPane));
    }

    #[test]
    fn editor_nvim_id_maps_to_action() {
        assert_eq!(action_for_id("editor.nvim"), Some(Action::OpenInNvim));
    }

    // ── Item 2: file:open: prefix is NOT resolved by action_for_id (handled
    // in the AppShell Invoke path directly so it doesn't need an Action copy).
    #[test]
    fn file_open_prefix_not_in_action_catalog() {
        assert_eq!(action_for_id("file:open:/some/path/buffer.rs"), None);
        assert_eq!(action_for_id("file:open:"), None);
    }

    #[test]
    fn summon_after_ready_shows_immediately() {
        let mut p = Palette {
            ready: true,
            ..Default::default()
        };
        assert!(p.summon());
        assert!(p.visible);
        assert!(!p.pending_show);
    }

    #[test]
    fn summon_before_ready_defers_show() {
        let mut p = Palette::default();
        assert!(!p.summon());
        assert!(p.visible);
        assert!(p.pending_show);
    }

    #[test]
    fn on_ready_flushes_deferred_show_exactly_once() {
        let mut p = Palette::default();
        p.summon();
        assert!(p.on_ready());
        assert!(p.ready);
        assert!(!p.pending_show);
    }

    #[test]
    fn on_ready_with_no_pending_show_returns_false() {
        let mut p = Palette::default();
        assert!(!p.on_ready());
    }

    #[test]
    fn dismiss_clears_visibility_and_pending_state() {
        let mut p = Palette::default();
        p.summon();
        p.dismiss();
        assert!(!p.visible);
        assert!(!p.pending_show);
    }
}
