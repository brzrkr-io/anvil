//! Text-input overlay: prompt + single-line editor.
//!
//! Phase 3: full render + key handling. Handles printable chars, backspace,
//! enter, home/end, left/right cursor movement.

use crate::overlay::chrome::{CardGeom, draw_card_chrome};
use crate::overlay::input::{OverlayClickResult, OverlayKey, OverlayKeyResult};
use crate::overlay::{OverlayId, Submission};
use crate::raster::{FontMetrics, GlyphPainter, Raster};
use anvil_theme::Theme;

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

    /// Render the text-input overlay onto `raster`.
    ///
    /// Layout: chrome → prompt (text_muted) → value (foreground) → cursor block.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        raster: &mut Raster,
        painter: &mut dyn GlyphPainter,
        metrics: FontMetrics,
        theme: &Theme,
        dw: f64,
        dh: f64,
        chrome_top: f64,
        anim_alpha: f64,
        anim_scale: f64,
    ) {
        let cw = metrics.cell_w;
        let ch = metrics.cell_h;
        let pad_x = 1.5 * cw;
        // Width: prompt + value + cursor, minimum 24 cols.
        let prompt_len = self.prompt.chars().count();
        let value_len = self.value.chars().count();
        let cols = (prompt_len + value_len + 2).max(24) as f64;
        let panel_w = (cols * cw + pad_x * 2.0).min(dw - 4.0 * cw);
        let panel_h = ch + 8.0;
        let panel_x = ((dw - panel_w) * 0.5).max(0.0);
        let panel_y = chrome_top + 4.0 * ch;
        let _ = dh; // reserved for future clamping

        let geom = CardGeom {
            x: panel_x,
            y: panel_y,
            w: panel_w,
            h: panel_h,
            radius: 0.0,
            padding: pad_x,
            anim_scale,
            anim_alpha,
        };
        draw_card_chrome(raster, theme, geom, true);

        let glyph_y = panel_y + 4.0;
        let mut x = panel_x + pad_x;

        // Prompt.
        for c in self.prompt.chars() {
            if x + cw > panel_x + panel_w - pad_x {
                break;
            }
            raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.text_muted);
            x += cw;
        }

        // Value — show tail so cursor end is visible for long strings.
        let max_value_cols = ((panel_w - pad_x * 2.0 - self.prompt.chars().count() as f64 * cw)
            / cw)
            .floor() as usize;
        let chars: Vec<char> = self.value.chars().collect();
        let start = chars.len().saturating_sub(max_value_cols);
        for &c in &chars[start..] {
            if x + cw > panel_x + panel_w - pad_x {
                break;
            }
            raster.glyph_at(painter, metrics, x, glyph_y, c as u32, theme.foreground);
            x += cw;
        }

        // Cursor block.
        raster.fill_pixel_rect(x, panel_y + 2.0, cw, panel_h - 4.0, theme.accent_bright);
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
