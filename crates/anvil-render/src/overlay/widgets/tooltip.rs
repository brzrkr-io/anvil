//! Tooltip overlay: anchored, non-modal, click-through.
//!
//! Skeleton for Phase 3. Positions a small card near an anchor point.

use crate::overlay::OverlayId;
use crate::overlay::input::{OverlayClickResult, OverlayKey, OverlayKeyResult};

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
