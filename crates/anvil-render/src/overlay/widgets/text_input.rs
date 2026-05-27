//! Text-input overlay: prompt + single-line editor.
//!
//! Skeleton for Phase 3. Handles printable chars, backspace, enter, home/end,
//! left/right cursor movement.

use crate::overlay::input::{OverlayClickResult, OverlayKey, OverlayKeyResult};
use crate::overlay::{OverlayId, Submission};

/// Text-input overlay state.
pub struct TextInputOverlay {
    pub id: OverlayId,
    pub prompt: String,
    pub value: String,
    pub cursor: usize, // byte offset into `value`
}

impl TextInputOverlay {
    pub fn new(id: OverlayId, prompt: impl Into<String>) -> Self {
        Self {
            id,
            prompt: prompt.into(),
            value: String::new(),
            cursor: 0,
        }
    }

    pub fn handle_key(&mut self, key: OverlayKey) -> OverlayKeyResult {
        match key {
            OverlayKey::Enter => OverlayKeyResult::Submit(Submission::TextValue {
                id: self.id,
                value: self.value.clone(),
            }),
            OverlayKey::Backspace => {
                if self.cursor > 0 {
                    // Remove the char before cursor (UTF-8 safe pop).
                    let mut chars: Vec<char> = self.value.chars().collect();
                    let char_idx = self.value[..self.cursor].chars().count();
                    if char_idx > 0 {
                        chars.remove(char_idx - 1);
                        self.value = chars.into_iter().collect();
                        // Move cursor back by one char's byte length.
                        self.cursor = self
                            .value
                            .chars()
                            .take(char_idx - 1)
                            .map(|c| c.len_utf8())
                            .sum();
                    }
                }
                OverlayKeyResult::Consumed
            }
            OverlayKey::Left => {
                if self.cursor > 0 {
                    // Step back one UTF-8 char.
                    while self.cursor > 0 && !self.value.is_char_boundary(self.cursor - 1) {
                        self.cursor -= 1;
                    }
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                }
                OverlayKeyResult::Consumed
            }
            OverlayKey::Right => {
                if self.cursor < self.value.len() {
                    self.cursor += 1;
                    while self.cursor < self.value.len()
                        && !self.value.is_char_boundary(self.cursor)
                    {
                        self.cursor += 1;
                    }
                }
                OverlayKeyResult::Consumed
            }
            OverlayKey::Home => {
                self.cursor = 0;
                OverlayKeyResult::Consumed
            }
            OverlayKey::End => {
                self.cursor = self.value.len();
                OverlayKeyResult::Consumed
            }
            OverlayKey::Char(c) => {
                self.value.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                OverlayKeyResult::Consumed
            }
            _ => OverlayKeyResult::Consumed,
        }
    }

    pub fn handle_click(&mut self, _x: f64, _y: f64) -> OverlayClickResult {
        OverlayClickResult::Consumed
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::OverlayId;

    fn make_ti() -> TextInputOverlay {
        TextInputOverlay::new(OverlayId::GotoLine, "goto: ")
    }

    #[test]
    fn text_input_enter_emits_submission() {
        let mut ti = make_ti();
        ti.handle_key(OverlayKey::Char('4'));
        ti.handle_key(OverlayKey::Char('2'));
        let result = ti.handle_key(OverlayKey::Enter);
        assert!(
            matches!(
                result,
                OverlayKeyResult::Submit(Submission::TextValue {
                    id: OverlayId::GotoLine,
                    value: ref v
                }) if v == "42"
            ),
            "enter should emit TextValue with '42', got {:?}",
            result
        );
    }

    #[test]
    fn text_input_backspace_deletes_last_char() {
        let mut ti = make_ti();
        ti.handle_key(OverlayKey::Char('a'));
        ti.handle_key(OverlayKey::Char('b'));
        ti.handle_key(OverlayKey::Backspace);
        assert_eq!(ti.value, "a");
    }
}
