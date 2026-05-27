//! Tooltip overlay: anchored, non-modal, click-through.
//!
//! Phase 3: full render. Positions a small card near an anchor point.

use crate::overlay::OverlayId;
use crate::overlay::chrome::{CardGeom, draw_card_chrome};
use crate::overlay::input::{OverlayClickResult, OverlayKey, OverlayKeyResult};
use crate::raster::{FontMetrics, GlyphPainter, Raster};
use anvil_theme::Theme;

/// Anchor point for a tooltip.
#[derive(Clone, Debug)]
pub enum Anchor {
    /// Fixed device-pixel position.
    Pixel(f64, f64),
    /// Editor cell position (row, col).
    EditorCell(usize, usize),
}

/// Tooltip content.
#[derive(Clone, Debug)]
pub struct TooltipBody {
    pub lines: Vec<String>,
}

/// Tooltip overlay state.
pub struct TooltipOverlay {
    pub id: OverlayId,
    pub anchor: Anchor,
    pub body: TooltipBody,
    pub follow_cursor: bool,
}

impl TooltipOverlay {
    /// Compute the card's top-left pixel from the anchor and available space.
    ///
    /// Ensures the card stays within `[0, dw) × [0, dh)`.
    pub fn anchor_position(&self, card_w: f64, card_h: f64, dw: f64, dh: f64) -> (f64, f64) {
        let (ax, ay) = match self.anchor {
            Anchor::Pixel(x, y) => (x, y),
            Anchor::EditorCell(row, col) => (col as f64 * 8.0, row as f64 * 16.0),
        };
        // Place card below and to the right of anchor; clamp to viewport.
        let x = (ax + 4.0).min(dw - card_w).max(0.0);
        let y = (ay + 4.0).min(dh - card_h).max(0.0);
        (x, y)
    }

    pub fn handle_key(&mut self, _key: OverlayKey) -> OverlayKeyResult {
        // Tooltips are non-modal; pass through.
        OverlayKeyResult::PassThrough
    }

    pub fn handle_click(&mut self, _x: f64, _y: f64) -> OverlayClickResult {
        OverlayClickResult::PassThrough
    }

    /// Render the tooltip card onto `raster`.
    ///
    /// Non-modal: no scrim. Card is positioned via `anchor_position`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        raster: &mut Raster,
        painter: &mut dyn GlyphPainter,
        metrics: FontMetrics,
        theme: &Theme,
        dw: f64,
        dh: f64,
        anim_alpha: f64,
        anim_scale: f64,
    ) {
        let cw = metrics.cell_w;
        let ch = metrics.cell_h;
        let pad = 6.0; // fixed px padding independent of scale for tight tooltip
        let n_lines = self.body.lines.len().max(1);
        let max_line_len = self
            .body
            .lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(4);
        let card_w = (max_line_len as f64 * cw + pad * 2.0).max(4.0 * cw);
        let card_h = n_lines as f64 * ch + pad * 2.0;

        let (card_x, card_y) = self.anchor_position(card_w, card_h, dw, dh);

        let geom = CardGeom {
            x: card_x,
            y: card_y,
            w: card_w,
            h: card_h,
            radius: 0.0,
            padding: pad,
            anim_scale,
            anim_alpha,
        };
        // Non-modal: no scrim.
        draw_card_chrome(raster, theme, geom, false);

        for (li, line) in self.body.lines.iter().enumerate() {
            let gy = card_y + pad + li as f64 * ch;
            let mut gx = card_x + pad;
            for c in line.chars() {
                if gx + cw > card_x + card_w - pad {
                    break;
                }
                raster.glyph_at(painter, metrics, gx, gy, c as u32, theme.text_subtle);
                gx += cw;
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::OverlayId;

    fn make_tooltip(anchor: Anchor) -> TooltipOverlay {
        TooltipOverlay {
            id: OverlayId::BlameTip,
            anchor,
            body: TooltipBody {
                lines: vec!["hello".into()],
            },
            follow_cursor: false,
        }
    }

    #[test]
    fn tooltip_anchor_positions_card() {
        let tt = make_tooltip(Anchor::Pixel(100.0, 200.0));
        let (x, y) = tt.anchor_position(120.0, 60.0, 800.0, 600.0);
        // Should be at anchor + 4px offset.
        assert_eq!(x, 104.0, "x should be anchor.x + 4");
        assert_eq!(y, 204.0, "y should be anchor.y + 4");
    }

    #[test]
    fn tooltip_anchor_clamps_to_viewport() {
        // Anchor near the right edge: card should be pushed left.
        let tt = make_tooltip(Anchor::Pixel(700.0, 500.0));
        let (x, y) = tt.anchor_position(120.0, 60.0, 800.0, 600.0);
        assert!(
            x + 120.0 <= 800.0,
            "card right edge should be within viewport: x={x}"
        );
        assert!(
            y + 60.0 <= 600.0,
            "card bottom edge should be within viewport: y={y}"
        );
    }
}
