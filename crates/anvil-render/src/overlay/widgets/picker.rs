//! Picker overlay: query input + scrollable list.
//!
//! Layout:
//!   chrome → row 0 (search prefix + query + cursor) → separator →
//!   result rows (1.4 × ch each) → padding.
//!
//! Each row is two-tone: selected rows get a subtle accent tint; the rest use
//! `text_muted`. Text is rendered via `raster.glyph_at` (MonoPainter seam).

use crate::overlay::chrome::{CardGeom, draw_card_chrome};
use crate::overlay::input::{OverlayClickResult, OverlayKey, OverlayKeyResult};
use crate::overlay::{OverlayId, Submission};
use crate::raster::{FontMetrics, GlyphPainter, Raster};
use anvil_theme::Theme;

/// An optional badge shown on the right side of a picker row.
#[derive(Clone, Debug)]
pub struct Badge {
    pub text: String,
    pub color: [u8; 3],
}

/// A single row in the picker.
#[derive(Clone, Debug)]
pub struct PickerRow {
    pub primary: String,
    pub secondary: Option<String>,
    pub badge: Option<Badge>,
}

/// Picker overlay state.
pub struct PickerOverlay {
    pub id: OverlayId,
    pub title: Option<String>,
    pub query: String,
    pub rows: Vec<PickerRow>,
    pub selected: usize,
    pub max_visible: usize,
}

impl PickerOverlay {
    /// Filter `rows` to those whose `primary` contains `query` (case-insensitive).
    /// Returns the filtered rows and adjusts selection.
    ///
    /// NOTE: This is a stateless helper. The caller (open_project_search) owns
    /// the full row list and passes filtered rows to push(Overlay::Picker(...)).
    /// In Phase 2, the ProjectSearch state machine drives filtering; we use this
    /// for the unit-test surface.
    pub fn filter_rows<'a>(rows: &'a [PickerRow], query: &str) -> Vec<&'a PickerRow> {
        if query.is_empty() {
            return rows.iter().collect();
        }
        let q = query.to_lowercase();
        rows.iter()
            .filter(|r| r.primary.to_lowercase().contains(&q))
            .collect()
    }

    /// Handle a key event. Called by `OverlayInputRouter`.
    pub fn handle_key(&mut self, key: OverlayKey) -> OverlayKeyResult {
        match key {
            OverlayKey::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                OverlayKeyResult::Consumed
            }
            OverlayKey::Down => {
                if !self.rows.is_empty() && self.selected + 1 < self.rows.len() {
                    self.selected += 1;
                }
                OverlayKeyResult::Consumed
            }
            OverlayKey::Enter => {
                if self.rows.is_empty() {
                    OverlayKeyResult::Consumed
                } else {
                    let idx = self.selected.min(self.rows.len().saturating_sub(1));
                    OverlayKeyResult::Submit(Submission::PickerRow {
                        id: self.id,
                        index: idx,
                    })
                }
            }
            OverlayKey::Backspace => {
                self.query.pop();
                OverlayKeyResult::Consumed
            }
            OverlayKey::Char(c) => {
                self.query.push(c);
                OverlayKeyResult::Consumed
            }
            _ => OverlayKeyResult::Consumed,
        }
    }

    /// Handle a mouse click. Returns `Consumed` for clicks inside the card,
    /// `Close` for clicks outside (blur).
    pub fn handle_click(&mut self, _x: f64, _y: f64) -> OverlayClickResult {
        // TODO(Phase 3): hit-test rows.
        OverlayClickResult::Consumed
    }

    /// Render the picker onto `raster`.
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
        chrome_bot: f64,
        anim_alpha: f64,
        anim_scale: f64,
    ) {
        let cw = metrics.cell_w;
        let ch = metrics.cell_h;
        let max_results = self.max_visible;
        let panel_rows = 1 + self.rows.len().min(max_results) + 1;
        let panel_h = panel_rows as f64 * ch + ch;
        let panel_w = (dw * 0.6).min(dw - 4.0 * cw).max(20.0 * cw);
        let panel_x = ((dw - panel_w) * 0.5).max(0.0);
        let panel_y = (chrome_top + (dh - chrome_top - chrome_bot - panel_h) * 0.2).max(chrome_top);

        let geom = CardGeom {
            x: panel_x,
            y: panel_y,
            w: panel_w,
            h: panel_h,
            radius: 0.0,
            padding: 2.0 * cw,
            anim_scale,
            anim_alpha,
        };

        draw_card_chrome(raster, theme, geom, true);

        let pad_x = 2.0 * cw;
        let row0_y = panel_y + 0.5 * ch;

        // Row 0: "search: " + query + cursor.
        let prefix = "search: ";
        let mut x = panel_x + pad_x;
        for c in prefix.chars() {
            if x + cw > panel_x + panel_w - pad_x {
                break;
            }
            raster.glyph_at(painter, metrics, x, row0_y, c as u32, theme.text_muted);
            x += cw;
        }
        for c in self.query.chars() {
            if x + cw > panel_x + panel_w - pad_x {
                break;
            }
            raster.glyph_at(painter, metrics, x, row0_y, c as u32, theme.foreground);
            x += cw;
        }
        // Cursor block.
        raster.fill_pixel_rect(x, panel_y + 2.0, cw, ch - 4.0, theme.accent_bright);

        // Separator below input row.
        raster.fill_pixel_rect(panel_x, panel_y + ch, panel_w, 1.0, theme.hairline);

        // Result rows.
        for (i, row) in self.rows.iter().take(max_results).enumerate() {
            let row_y = panel_y + (i + 1) as f64 * ch + 0.5 * ch;
            let is_selected = i == self.selected;
            if is_selected {
                raster.fill_pixel_rect_alpha(
                    panel_x,
                    panel_y + (i + 1) as f64 * ch,
                    panel_w,
                    ch,
                    theme.accent,
                    0.12,
                );
            }
            let label = if let Some(sec) = &row.secondary {
                format!("{} {}", row.primary, sec)
            } else {
                row.primary.clone()
            };
            let color = if is_selected {
                theme.foreground
            } else {
                theme.text_muted
            };
            let mut rx = panel_x + pad_x;
            for c in label.chars() {
                if rx + cw > panel_x + panel_w - pad_x {
                    break;
                }
                raster.glyph_at(painter, metrics, rx, row_y, c as u32, color);
                rx += cw;
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::OverlayId;

    fn make_rows(names: &[&str]) -> Vec<PickerRow> {
        names
            .iter()
            .map(|n| PickerRow {
                primary: n.to_string(),
                secondary: None,
                badge: None,
            })
            .collect()
    }

    fn make_picker(rows: Vec<PickerRow>) -> PickerOverlay {
        PickerOverlay {
            id: OverlayId::ProjectSearch,
            title: None,
            query: String::new(),
            rows,
            selected: 0,
            max_visible: 10,
        }
    }

    #[test]
    fn picker_filters_rows_on_query() {
        let rows = make_rows(&["alpha.rs", "beta.rs", "gamma_alpha.rs", "delta.rs"]);
        let filtered = PickerOverlay::filter_rows(&rows, "alpha");
        assert_eq!(
            filtered.len(),
            2,
            "should match alpha.rs and gamma_alpha.rs"
        );
        assert!(filtered.iter().any(|r| r.primary == "alpha.rs"));
        assert!(filtered.iter().any(|r| r.primary == "gamma_alpha.rs"));
    }

    #[test]
    fn picker_filter_empty_query_returns_all() {
        let rows = make_rows(&["a.rs", "b.rs"]);
        let filtered = PickerOverlay::filter_rows(&rows, "");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn picker_arrows_clamp() {
        let rows = make_rows(&["a.rs", "b.rs", "c.rs"]);
        let mut picker = make_picker(rows);
        // Down past end.
        picker.handle_key(OverlayKey::Down);
        picker.handle_key(OverlayKey::Down);
        picker.handle_key(OverlayKey::Down); // should clamp at 2
        assert_eq!(picker.selected, 2, "should clamp at last row");
        // Up past start.
        picker.handle_key(OverlayKey::Up);
        picker.handle_key(OverlayKey::Up);
        picker.handle_key(OverlayKey::Up); // should clamp at 0
        assert_eq!(picker.selected, 0, "should clamp at first row");
    }

    #[test]
    fn picker_enter_submits_selected_index() {
        let rows = make_rows(&["a.rs", "b.rs"]);
        let mut picker = make_picker(rows);
        picker.handle_key(OverlayKey::Down); // select index 1
        let result = picker.handle_key(OverlayKey::Enter);
        assert!(
            matches!(
                result,
                OverlayKeyResult::Submit(Submission::PickerRow {
                    id: OverlayId::ProjectSearch,
                    index: 1
                })
            ),
            "enter should submit index 1, got {:?}",
            result
        );
    }
}
