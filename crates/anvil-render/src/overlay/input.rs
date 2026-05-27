//! Overlay input routing.
//!
//! `OverlayInputRouter::dispatch_key` routes a key event to the top overlay
//! on the stack. The caller (main.rs `AppShell::key_down`) checks the
//! stack first; only `PassThrough` means the event should continue to
//! the normal key-down path.

use super::{AnimState, Overlay, OverlayStack, Submission};

// ── Key and mouse types ────────────────────────────────────────────────────────
// Lightweight mirror of `anvil-platform` types so `anvil-render` stays
// platform-free. `main.rs` translates `KeyInput` → `OverlayKey` before dispatch.

/// A key event delivered to the overlay system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKey {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Other,
}

/// Result returned from `dispatch_key`.
#[derive(Debug, PartialEq)]
pub enum OverlayKeyResult {
    /// The event was handled; do not propagate.
    Consumed,
    /// The overlay requests close (e.g. Esc). Caller should call `stack.close_top()`.
    Close,
    /// The overlay submitted a value (Enter with selection). Contains the submission.
    Submit(Submission),
    /// The stack was empty or the event was not handled; propagate to normal handlers.
    PassThrough,
}

/// Result returned from `dispatch_click`.
#[derive(Debug, PartialEq)]
pub enum OverlayClickResult {
    /// The click was handled by the overlay.
    Consumed,
    /// The click was outside the overlay (when `close_on_blur`).
    Close,
    /// Let normal handlers process the click.
    PassThrough,
}

/// Routes input events to the top entry of an `OverlayStack`.
pub struct OverlayInputRouter;

impl OverlayInputRouter {
    /// Route a key event to the top overlay.
    ///
    /// Spec § 5:
    /// 1. Empty stack → PassThrough.
    /// 2. Top not Visible → consume but ignore.
    /// 3. Esc → signal Close.
    /// 4. Else delegate to overlay.
    pub fn dispatch_key(stack: &mut OverlayStack, key: OverlayKey) -> OverlayKeyResult {
        let entry = match stack.top_entry_mut() {
            Some(e) => e,
            None => return OverlayKeyResult::PassThrough,
        };

        // Rule 2: not yet visible → consume silently.
        if !matches!(entry.anim.state, AnimState::Visible) {
            return OverlayKeyResult::Consumed;
        }

        // Rule 3: Esc always closes.
        if key == OverlayKey::Escape {
            return OverlayKeyResult::Close;
        }

        // Rule 4: delegate.
        match &mut entry.overlay {
            Overlay::Picker(p) => p.handle_key(key),
            Overlay::TextInput(ti) => ti.handle_key(key),
            Overlay::Tooltip(_) => {
                // Tooltips are non-modal; pass through.
                OverlayKeyResult::PassThrough
            }
            Overlay::Custom(c) => c.handle_key(key),
        }
    }

    /// Route a click to the top overlay.
    pub fn dispatch_click(stack: &mut OverlayStack, x: f64, y: f64) -> OverlayClickResult {
        let entry = match stack.top_entry_mut() {
            Some(e) => e,
            None => return OverlayClickResult::PassThrough,
        };

        match &mut entry.overlay {
            Overlay::Picker(p) => p.handle_click(x, y),
            Overlay::TextInput(ti) => ti.handle_click(x, y),
            Overlay::Tooltip(_) => OverlayClickResult::PassThrough,
            Overlay::Custom(c) => c.handle_click(x, y),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{Overlay, OverlayId, OverlayStack, PickerOverlay};

    fn make_picker_stack() -> OverlayStack {
        let mut stack = OverlayStack::new();
        let picker = PickerOverlay {
            id: OverlayId::ProjectSearch,
            title: None,
            query: String::new(),
            rows: vec![
                crate::overlay::widgets::picker::PickerRow {
                    primary: "alpha.rs".into(),
                    secondary: None,
                    badge: None,
                },
                crate::overlay::widgets::picker::PickerRow {
                    primary: "beta.rs".into(),
                    secondary: None,
                    badge: None,
                },
            ],
            selected: 0,
            max_visible: 10,
        };
        stack.push(Overlay::Picker(picker));
        // Tick to Visible.
        stack.tick(1000.0);
        stack
    }

    #[test]
    fn empty_stack_passthrough() {
        let mut stack = OverlayStack::new();
        let result = OverlayInputRouter::dispatch_key(&mut stack, OverlayKey::Enter);
        assert_eq!(result, OverlayKeyResult::PassThrough);
    }

    #[test]
    fn esc_returns_close() {
        let mut stack = make_picker_stack();
        let result = OverlayInputRouter::dispatch_key(&mut stack, OverlayKey::Escape);
        assert_eq!(result, OverlayKeyResult::Close);
    }

    #[test]
    fn picker_arrow_down_consumed() {
        let mut stack = make_picker_stack();
        let result = OverlayInputRouter::dispatch_key(&mut stack, OverlayKey::Down);
        assert_eq!(result, OverlayKeyResult::Consumed);
    }
}
