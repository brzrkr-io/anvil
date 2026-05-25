//! Multi-pane render coordinator.
//!
//! `draw_workspace` lays out a PaneTree onto an inner content rect, then calls
//! `draw_viewport` once per leaf with that leaf's pixel origin set on the Raster.
//!
//! Bleed guard: the smooth-scroll path draws row y=0..rows (inclusive — one
//! extra partially visible row). With vertical splits, any bleed into the
//! divider gutter is overdrawn by the divider fill, which is drawn LAST over
//! all panes.

use std::collections::HashMap;

use anvil_term::{DirtySet, Search};
use anvil_theme::Theme;
use anvil_workspace::{
    layout::{LayoutEntry, PaneId, PaneTree, Rect},
    pane::PaneRegistry,
};

use crate::{
    draw::{CursorConfig, CursorParams, FoldedBlocks, draw_viewport},
    raster::{FontMetrics, GlyphPainter, Raster},
};

/// Pane-divider width in device pixels. 2 device px (= 1 logical pt at 2×
/// Retina) is the minimum that reads as a deliberate divider on a busy
/// terminal screen. 1px hairlines disappeared into surrounding content;
/// the previous 8px read as a structural wall — 2px is the sweet spot.
pub const DIVIDER_PX: f64 = 2.0;

/// Draw all panes in `tree` into `raster`, then draw divider hairlines over them.
///
/// Parameters:
///   raster       — full-window raster bitmap.
///   tree         — the current tab's pane tree (layout and focused id).
///   registry     — the pane registry for the current tab.
///   inner        — device-pixel content area (window minus top-bar and panels).
///                  y=0 is the top of the raster. Layout is done in this space.
///   div_px       — divider gutter width in device pixels (use `DIVIDER_PX`).
///   metrics      — font metrics shared by all panes.
///   theme        — shared theme for all panes.
///   search       — active search state, or None.
///   focused_id   — the pane that receives cursor rendering.
///   blink_phase  — cursor blink phase [0, 1).
///   cursor_cfg   — cursor style + blink preference from config.
///   dirty        — per-pane dirty sets from `Terminal::take_dirty_rows`. When
///                  `None`, every row of every pane is redrawn (full frame).
///
/// After this function returns, raster.origin_x and raster.origin_y are both 0.
#[allow(clippy::too_many_arguments)]
pub fn draw_workspace(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    tree: &PaneTree,
    registry: &mut PaneRegistry,
    inner: Rect,
    div_px: f64,
    metrics: FontMetrics,
    theme: &Theme,
    search: Option<&Search>,
    focused_id: PaneId,
    blink_phase: f32,
    cursor_cfg: CursorConfig,
    dirty: Option<&HashMap<PaneId, DirtySet>>,
) {
    let entries = tree.layout(inner, div_px);

    // Draw each leaf.
    for e in &entries {
        let pane = match registry.get_mut(e.id) {
            Some(p) => p,
            None => continue,
        };

        // Set the pane's pixel origin on the raster.
        raster.origin_x = e.rect.x;
        raster.origin_y = e.rect.y;

        let cursor_params: Option<CursorParams> = if e.id == focused_id {
            Some(CursorParams {
                ax: pane.cursor_ax,
                ay: pane.cursor_ay,
                blink_phase,
                cfg: cursor_cfg,
            })
        } else {
            None
        };

        // rule_x bounds: horizontal span of this pane in device pixels.
        let rule_x_start = e.rect.x;
        let rule_x_end = e.rect.x + e.rect.w;

        // Fold state for this pane.
        let folded = FoldedBlocks::new(&pane.folded[..pane.folded_count]);

        // Per-pane dirty set: None means "draw all rows".
        let pane_dirty: Option<&DirtySet> = dirty.and_then(|m| m.get(&e.id));

        draw_viewport(
            raster,
            painter,
            &mut pane.terminal,
            metrics,
            theme,
            pane.scroll_pos,
            pane.selection,
            search,
            cursor_params,
            rule_x_start,
            rule_x_end,
            folded,
            pane_dirty,
        );
    }

    // Reset origin before chrome draws in absolute space.
    raster.origin_x = 0.0;
    raster.origin_y = 0.0;

    // Draw divider hairlines over all pane content (bleed guard).
    draw_dividers(raster, &entries, div_px, theme, focused_id);
}

/// Draw only the chrome portion of the workspace (divider hairlines, focused
/// pane accent border) without drawing any terminal viewport content.
///
/// Used by the GPU rendering path (`ANVIL_RENDER=gpu`) where viewport cells
/// are drawn by the GPU cell pipeline instead of the CPU raster.  The caller
/// is responsible for calling `draw_viewport_gpu` per pane separately.
///
/// After this function returns, raster.origin_x and raster.origin_y are both 0.
#[allow(clippy::too_many_arguments)]
pub fn draw_workspace_chrome(
    raster: &mut Raster,
    tree: &PaneTree,
    registry: &PaneRegistry,
    inner: Rect,
    div_px: f64,
    theme: &Theme,
    focused_id: PaneId,
) {
    let entries = tree.layout(inner, div_px);
    let _ = registry; // registry not needed for chrome-only draw
    // Reset origin (no pane origins needed — we skip viewport drawing).
    raster.origin_x = 0.0;
    raster.origin_y = 0.0;
    // Draw divider hairlines.
    draw_dividers(raster, &entries, div_px, theme, focused_id);
}

/// Fill divider gutters between all adjacent leaf pairs. Called after all pane
/// content is drawn so the dividers overdraw any scroll bleed.
fn draw_dividers(
    raster: &mut Raster,
    entries: &[LayoutEntry],
    div_px: f64,
    theme: &Theme,
    focused_id: PaneId,
) {
    // For each pair of leaves, if they share a boundary (with a gutter between
    // them), fill the gutter rectangle.
    for (ai, a) in entries.iter().enumerate() {
        for b in &entries[ai + 1..] {
            // Horizontal split: b is to the right of a.
            {
                let gap_x = a.rect.x + a.rect.w;
                let gap_end = b.rect.x;
                if gap_end > gap_x && gap_end - gap_x <= div_px + 1.0 {
                    let oy = f64::max(a.rect.y, b.rect.y);
                    let oy_end = f64::min(a.rect.y + a.rect.h, b.rect.y + b.rect.h);
                    if oy_end > oy {
                        raster.fill_pixel_rect(
                            gap_x,
                            oy,
                            gap_end - gap_x,
                            oy_end - oy,
                            theme.border,
                        );
                    }
                }
            }
            // Vertical split: b is below a.
            {
                let gap_y = a.rect.y + a.rect.h;
                let gap_end = b.rect.y;
                if gap_end > gap_y && gap_end - gap_y <= div_px + 1.0 {
                    let ox = f64::max(a.rect.x, b.rect.x);
                    let ox_end = f64::min(a.rect.x + a.rect.w, b.rect.x + b.rect.w);
                    if ox_end > ox {
                        raster.fill_pixel_rect(
                            ox,
                            gap_y,
                            ox_end - ox,
                            gap_end - gap_y,
                            theme.border,
                        );
                    }
                }
            }
        }
    }

    // Paint a 1px inset accent border on the focused pane (only when there are
    // 2+ panes — single-pane layout needs no focus ring).
    if entries.len() >= 2 {
        if let Some(e) = entries.iter().find(|e| e.id == focused_id) {
            let r = &e.rect;
            let c = theme.accent;
            // Top edge (1px tall, full width).
            raster.fill_pixel_rect(r.x, r.y, r.w, 1.0, c);
            // Bottom edge.
            raster.fill_pixel_rect(r.x, r.y + r.h - 1.0, r.w, 1.0, c);
            // Left edge (inner height, avoids double-painting corners).
            raster.fill_pixel_rect(r.x, r.y + 1.0, 1.0, r.h - 2.0, c);
            // Right edge.
            raster.fill_pixel_rect(r.x + r.w - 1.0, r.y + 1.0, 1.0, r.h - 2.0, c);
        }
    }
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{PixelRect, pixel_at};
    use anvil_workspace::{layout::SplitDir, pane::PaneRegistry};

    // Stub painter.
    #[derive(Default)]
    struct StubPainter {
        pub calls: Vec<(u32, [u8; 3])>,
    }

    impl GlyphPainter for StubPainter {
        #[allow(clippy::too_many_arguments)]
        fn draw_glyph(
            &mut self,
            glyph_id: u32,
            _dest: PixelRect,
            fg: [u8; 3],
            _metrics: FontMetrics,
            _pixels: &mut [u8],
            _bw: usize,
            _bh: usize,
        ) {
            self.calls.push((glyph_id, fg));
        }
    }

    fn metrics() -> FontMetrics {
        FontMetrics {
            cell_w: 10.0,
            cell_h: 20.0,
            descent: 4.0,
        }
    }

    fn make_registry_single(cols: usize, rows: usize) -> (PaneRegistry, PaneId) {
        let mut reg = PaneRegistry::default();
        let id = reg.create_and_register(cols, rows, 0);
        (reg, id)
    }

    /// drawWorkspace single-leaf: leaf rect equals inner rect
    ///
    /// Verifies that a single-leaf tree gives the full inner rect, and that
    /// raster.origin_x / origin_y are reset to 0 after the call.
    #[test]
    fn single_leaf_rect_equals_inner() {
        let m = metrics();
        let pad = 24.0_f64;
        let w = 400_usize;
        let h = 300_usize;
        let inner = Rect {
            x: pad,
            y: pad,
            w: w as f64 - 2.0 * pad,
            h: h as f64 - 2.0 * pad,
        };

        let tree = PaneTree::init_single(1);
        let entries = tree.layout(inner, DIVIDER_PX);

        assert_eq!(entries.len(), 1);
        assert!((entries[0].rect.x - inner.x).abs() < 1e-9);
        assert!((entries[0].rect.y - inner.y).abs() < 1e-9);
        assert!((entries[0].rect.w - inner.w).abs() < 1e-9);
        assert!((entries[0].rect.h - inner.h).abs() < 1e-9);

        // Full draw_workspace call must not panic and must reset origin.
        let mut r = Raster::new(w, h);
        let mut painter = StubPainter::default();
        let (mut reg, first_id) = make_registry_single(20, 6);
        let theme = anvil_theme::MINERAL_DARK;
        let cursor_cfg = CursorConfig::default();

        if let Some(pane) = reg.get_mut(first_id) {
            pane.terminal.feed(b"hello\r\n");
        }

        let tree = PaneTree::init_single(first_id);
        draw_workspace(
            &mut r,
            &mut painter,
            &tree,
            &mut reg,
            inner,
            DIVIDER_PX,
            m,
            &theme,
            None,
            first_id,
            0.0,
            cursor_cfg,
            None,
        );

        assert_eq!(r.origin_x, 0.0, "origin_x must be reset to 0");
        assert_eq!(r.origin_y, 0.0, "origin_y must be reset to 0");
    }

    /// drawWorkspace two-pane: divider pixels carry theme.border
    ///
    /// Lay out two horizontally-split panes with a deliberately wide divider
    /// so the gutter pixel is comfortably in the middle of the border band
    /// (the production `DIVIDER_PX = 1.0` hairline is sandwiched between
    /// adjacent panes' 2px focus accents — fine in production, but the
    /// sampling test needs the divider to be the dominant feature at the
    /// sample point). The drawing logic is identical for any width.
    #[test]
    fn two_pane_divider_pixel_is_border() {
        const TEST_DIV: f64 = 8.0;
        let m = metrics();
        let w = 400_usize;
        let h = 300_usize;
        let pad = 24.0_f64;
        let inner = Rect {
            x: pad,
            y: pad,
            w: w as f64 - 2.0 * pad,
            h: h as f64 - 2.0 * pad,
        };

        let mut reg = PaneRegistry::default();
        let id1 = reg.create_and_register(20, 6, 0);
        let id2 = reg.create_and_register(20, 6, 0);
        if let Some(p) = reg.get_mut(id1) {
            p.terminal.feed(b"pane one");
        }
        if let Some(p) = reg.get_mut(id2) {
            p.terminal.feed(b"pane two");
        }

        let mut tree = PaneTree::init_single(id1);
        tree.split(SplitDir::Horizontal, id2).unwrap();

        let mut r = Raster::new(w, h);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        r.clear(theme.background);

        draw_workspace(
            &mut r,
            &mut painter,
            &tree,
            &mut reg,
            inner,
            TEST_DIV,
            m,
            &theme,
            None,
            id1,
            0.0,
            CursorConfig::default(),
            None,
        );

        // Gutter center: pane1_w = (inner.w - TEST_DIV) * 0.5
        let pane1_w = (inner.w - TEST_DIV) * 0.5;
        let gutter_x = inner.x + pane1_w;
        let gutter_center_x = (gutter_x + TEST_DIV * 0.5) as usize;
        let mid_y = (inner.y + inner.h * 0.5) as usize;

        let px = pixel_at(&r, gutter_center_x, mid_y);
        // With a 1px hairline divider, the gutter pixel may be theme.border or
        // theme.accent (focused-pane accent border) — either way it must not be
        // raw background.
        assert!(
            px == theme.border || px == theme.accent,
            "gutter pixel should be border or accent, got {px:?}"
        );
    }

    /// Two-pane: focused pane has a 1px inset accent border; non-focused does not.
    #[test]
    fn focused_pane_has_accent_border() {
        let m = metrics();
        let w = 800_usize;
        let h = 400_usize;
        let pad = 24.0_f64;
        let inner = Rect {
            x: pad,
            y: pad,
            w: w as f64 - 2.0 * pad,
            h: h as f64 - 2.0 * pad,
        };
        let theme = anvil_theme::MINERAL_DARK;

        let mut reg = PaneRegistry::default();
        let id1 = reg.create_and_register(20, 6, 0);
        let id2 = reg.create_and_register(20, 6, 0);

        let mut tree = PaneTree::init_single(id1);
        tree.split(SplitDir::Horizontal, id2).unwrap();

        let mut r = Raster::new(w, h);
        let mut painter = StubPainter::default();
        r.clear(theme.background);
        draw_workspace(
            &mut r,
            &mut painter,
            &tree,
            &mut reg,
            inner,
            DIVIDER_PX,
            m,
            &theme,
            None,
            id1, // focused
            0.0,
            CursorConfig::default(),
            None,
        );

        // Focused pane (id1) is the left half of inner.
        let pane1_w = (inner.w - DIVIDER_PX) * 0.5;
        let mid_y = (inner.y + inner.h * 0.5) as usize;

        // Left inset edge of focused pane should carry accent.
        let left_x = inner.x as usize;
        let px = pixel_at(&r, left_x, mid_y);
        assert_eq!(
            px, theme.accent,
            "focused pane left inset should be accent (x={left_x}, got {px:?})"
        );

        // Top inset edge of focused pane should carry accent.
        let top_y = inner.y as usize;
        let mid_x = (inner.x + pane1_w * 0.5) as usize;
        let px = pixel_at(&r, mid_x, top_y);
        assert_eq!(
            px, theme.accent,
            "focused pane top inset should be accent (y={top_y}, got {px:?})"
        );

        // Non-focused pane (id2, right half): its left inset must NOT be accent.
        let pane2_left_x = (inner.x + pane1_w + DIVIDER_PX) as usize;
        let px = pixel_at(&r, pane2_left_x, mid_y);
        assert_ne!(
            px, theme.accent,
            "non-focused pane must not have accent border (x={pane2_left_x}, got {px:?})"
        );
    }

    /// draw_workspace smoke: does not panic on single pane with content.
    #[test]
    fn draw_workspace_smoke_no_panic() {
        let m = metrics();
        let mut r = Raster::new(400, 300);
        let mut painter = StubPainter::default();
        let (mut reg, id) = make_registry_single(20, 6);
        if let Some(pane) = reg.get_mut(id) {
            pane.terminal.feed(b"hello world\r\n");
        }
        let tree = PaneTree::init_single(id);
        let inner = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        let theme = anvil_theme::MINERAL_DARK;
        r.clear(theme.background);
        draw_workspace(
            &mut r,
            &mut painter,
            &tree,
            &mut reg,
            inner,
            DIVIDER_PX,
            m,
            &theme,
            None,
            id,
            0.0,
            CursorConfig::default(),
            None,
        );
        // "hello world" starts with 'h' — expect glyph calls.
        assert!(!painter.calls.is_empty());
    }
}
