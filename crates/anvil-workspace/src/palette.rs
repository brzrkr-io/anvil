//! Command-palette controller: the static command catalog, id→action mapping,
//! and the summon/dismiss + ready-handshake state machine.  Pure — no platform
//! I/O.

/// A native action a palette command runs.  The host maps each to real work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    ThemeDark,
    ThemeLight,
    ConfigReload,
    ClearScreen,
    ScrollTop,
    ScrollBottom,
    AppQuit,
    HudToggle,
    TreeToggle,
    CheatsheetShow,
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
        title: "Switch to Dark Theme",
        subtitle: None,
        action: Action::ThemeDark,
    },
    Entry {
        id: "theme.light",
        title: "Switch to Light Theme",
        subtitle: None,
        action: Action::ThemeLight,
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
        id: "tree.toggle",
        title: "Toggle File Tree",
        subtitle: Some("Show or hide the file explorer panel"),
        action: Action::TreeToggle,
    },
    Entry {
        id: "cheatsheet.show",
        title: "Keyboard Shortcuts",
        subtitle: Some("Show the keyboard shortcut cheatsheet"),
        action: Action::CheatsheetShow,
    },
];

/// Look up the action for a command id, or `None` if unknown.
pub fn action_for_id(id: &str) -> Option<Action> {
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
        assert_eq!(action_for_id("app.quit"), Some(Action::AppQuit));
        assert_eq!(action_for_id("scroll.top"), Some(Action::ScrollTop));
    }

    #[test]
    fn unknown_id_has_no_action() {
        assert_eq!(action_for_id("nope.nope"), None);
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
