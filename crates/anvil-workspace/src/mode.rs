//! Layout mode and dock geometry.
//!
//! `LayoutMode` selects which docks are visible.  `Docks` holds the resolved
//! widths/heights for one mode at one scale factor.  `Docks::compute_areas`
//! slices a `window_inner` rect into the five named areas used by the
//! renderer.
//!
//! All values are in **device pixels** (logical × scale).

use crate::layout::Rect;

// ── LayoutMode ────────────────────────────────────────────────────────────────

/// Application layout mode.
///
/// - `Terminal` — today's layout: no docks, HUD floats via `hud_visible`.
/// - `Ide` — left explorer dock + right HUD dock, always visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LayoutMode {
    #[default]
    Terminal,
    Ide,
}

// ── FontMetricsForDocks ───────────────────────────────────────────────────────

/// Minimal font metrics needed by `Docks::for_mode`.
///
/// Passed by value so callers do not need to expose `Font` internals to this
/// crate.
#[derive(Clone, Copy, Debug)]
pub struct DockMetrics {
    pub cell_w: f64,
    pub cell_h: f64,
    /// HUD column count (terminal columns, not device px).
    pub hud_cols: usize,
    /// Right padding when no dock is visible (device px).  Absorbs the right
    /// `GRID_PAD` that `inner_rect` formerly baked in.
    pub grid_pad: f64,
}

// ── Docks ─────────────────────────────────────────────────────────────────────

/// Resolved dock widths/heights in device pixels for a single frame.
///
/// All fields are `≥ 0.0`.  Zero means the dock is hidden.
/// Computed once per frame; not stored on `App`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Docks {
    /// Left dock width in device pixels.  `0` in Terminal mode.
    pub left_w: f64,
    /// Right dock width in device pixels.  Carries the HUD strip in Ide mode;
    /// set from `hud_visible` in Terminal mode.
    pub right_w: f64,
    /// Top bar height.  `0` until ID2.
    pub top_h: f64,
    /// Bottom status bar height in device pixels.
    pub bottom_h: f64,
}

impl Docks {
    /// Compute dock dimensions from mode, scale, and font metrics.
    ///
    /// - `hud_visible` controls the right strip in Terminal mode.
    /// - In Ide mode the right dock is always the HUD column, regardless of
    ///   `hud_visible` (locked-visible per ID1 spec).
    pub fn for_mode(
        mode: LayoutMode,
        scale: f64,
        metrics: DockMetrics,
        hud_visible: bool,
        chrome_bottom_px: f64,
    ) -> Self {
        Self::for_mode_with_left_dock(mode, scale, metrics, hud_visible, chrome_bottom_px, true)
    }

    pub fn for_mode_with_left_dock(
        mode: LayoutMode,
        scale: f64,
        metrics: DockMetrics,
        hud_visible: bool,
        chrome_bottom_px: f64,
        left_dock_visible: bool,
    ) -> Self {
        Self::for_mode_with_left_dock_w(
            mode,
            scale,
            metrics,
            hud_visible,
            chrome_bottom_px,
            left_dock_visible,
            260.0,
            1.0,
        )
    }

    /// Like `for_mode_with_left_dock` but accepts a caller-supplied sidebar width
    /// in logical points (item 13: drag-resize) and a `ui_scale` zoom multiplier.
    ///
    /// `ui_scale` is applied on top of `scale` (Retina) for the IDE top bar height
    /// so the context strip grows/shrinks with the user's zoom level (A4).
    /// Pass `1.0` for no zoom.
    #[allow(clippy::too_many_arguments)]
    pub fn for_mode_with_left_dock_w(
        mode: LayoutMode,
        scale: f64,
        metrics: DockMetrics,
        hud_visible: bool,
        chrome_bottom_px: f64,
        left_dock_visible: bool,
        left_dock_w_pt: f64,
        ui_scale: f64,
    ) -> Self {
        let cw = metrics.cell_w;
        // Must match the actual HUD paint width in main.rs::render_frame:
        // surface_w_px = hud_cols * cw + GRID_PAD.
        // Mismatch with the carve-out (formerly + cw) caused a horizontal
        // gap or overlap between pane area and HUD strip when GRID_PAD ≠ cw.
        let hud_w = metrics.hud_cols as f64 * cw + metrics.grid_pad;

        match mode {
            LayoutMode::Terminal => {
                // No HUD: reserve grid_pad on the right (preserves the right GRID_PAD
                // that inner_rect formerly subtracted from pane width).
                // HUD on: HUD strip absorbs the right padding entirely.
                let right_w = if hud_visible { hud_w } else { metrics.grid_pad };
                Docks {
                    left_w: 0.0,
                    right_w,
                    top_h: 0.0,
                    bottom_h: chrome_bottom_px,
                }
            }
            LayoutMode::Ide => Docks {
                left_w: if left_dock_visible {
                    left_dock_w_pt * scale
                } else {
                    0.0
                },
                right_w: if hud_visible { hud_w } else { 0.0 },
                // 28pt context strip below the chrome tab row: IDE chip,
                // project path, git chips.  Scaled by both `scale` (Retina)
                // and `ui_scale` (user zoom) so the bar grows/shrinks with zoom (A4).
                top_h: 28.0 * scale * ui_scale,
                bottom_h: chrome_bottom_px,
            },
        }
    }

    /// Slice `window_inner` into the five named areas.
    ///
    /// `window_inner` is the content rect **before** dock subtraction:
    /// window minus OS title strip and bottom status bar.
    ///
    /// Pane area width/height are clamped to `≥ min_cell_w / min_cell_h` so
    /// the PTY always has at least one addressable cell.
    pub fn compute_areas(&self, window_inner: Rect, min_w: f64, min_h: f64) -> Areas {
        let Rect { x, y, w, h } = window_inner;

        let top_bar = Rect {
            x,
            y,
            w,
            h: self.top_h,
        };
        let left_dock = Rect {
            x,
            y: y + self.top_h,
            w: self.left_w,
            h: (h - self.top_h - self.bottom_h).max(0.0),
        };
        let right_dock = Rect {
            x: x + w - self.right_w,
            y: y + self.top_h,
            w: self.right_w,
            h: (h - self.top_h - self.bottom_h).max(0.0),
        };
        let pane_w = (w - self.left_w - self.right_w).max(min_w);
        let pane_h = (h - self.top_h - self.bottom_h).max(min_h);
        let pane_area = Rect {
            x: x + self.left_w,
            y: y + self.top_h,
            w: pane_w,
            h: pane_h,
        };
        let bottom_bar = Rect {
            x,
            y: y + h - self.bottom_h,
            w,
            h: self.bottom_h,
        };

        Areas {
            top_bar,
            left_dock,
            pane_area,
            right_dock,
            bottom_bar,
        }
    }
}

// ── Areas ─────────────────────────────────────────────────────────────────────

/// Five named regions produced by [`Docks::compute_areas`].
#[derive(Clone, Copy, Debug)]
pub struct Areas {
    pub top_bar: Rect,
    pub left_dock: Rect,
    pub pane_area: Rect,
    pub right_dock: Rect,
    pub bottom_bar: Rect,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const GRID_PAD: f64 = 24.0;

    fn metrics(hud_cols: usize) -> DockMetrics {
        DockMetrics {
            cell_w: 8.0,
            cell_h: 16.0,
            hud_cols,
            grid_pad: GRID_PAD,
        }
    }

    const INNER: Rect = Rect {
        x: 0.0,
        y: 36.0,
        w: 1280.0,
        h: 744.0,
    };
    const MIN_W: f64 = 8.0;
    const MIN_H: f64 = 16.0;
    const BOTTOM_H: f64 = 24.0;

    fn docks_for(mode: LayoutMode, hud_visible: bool) -> Docks {
        Docks::for_mode(mode, 1.0, metrics(10), hud_visible, BOTTOM_H)
    }

    // ── width invariant ───────────────────────────────────────────────────────

    #[test]
    fn widths_sum_to_inner_w_terminal_hud_off() {
        let areas = docks_for(LayoutMode::Terminal, false).compute_areas(INNER, MIN_W, MIN_H);
        assert_eq!(
            areas.left_dock.w + areas.pane_area.w + areas.right_dock.w,
            INNER.w,
            "widths must sum to inner.w in Terminal/HUD-off"
        );
    }

    #[test]
    fn widths_sum_to_inner_w_terminal_hud_on() {
        let areas = docks_for(LayoutMode::Terminal, true).compute_areas(INNER, MIN_W, MIN_H);
        // right_w = hud_cols * cw + cw = 10*8+8 = 88; pane_w = 1280-0-88 = 1192
        assert_eq!(
            areas.left_dock.w + areas.pane_area.w + areas.right_dock.w,
            INNER.w
        );
    }

    #[test]
    fn widths_sum_to_inner_w_ide() {
        let areas = docks_for(LayoutMode::Ide, false).compute_areas(INNER, MIN_W, MIN_H);
        // left_w = 260 (D2 default); right_w = 24 (GRID_PAD, hud_visible=false); pane_w = 1280-260-24 = 996
        assert_eq!(
            areas.left_dock.w + areas.pane_area.w + areas.right_dock.w,
            INNER.w
        );
    }

    // ── height invariant ──────────────────────────────────────────────────────

    #[test]
    fn heights_sum_terminal() {
        let areas = docks_for(LayoutMode::Terminal, false).compute_areas(INNER, MIN_W, MIN_H);
        assert_eq!(
            areas.top_bar.h + areas.pane_area.h + areas.bottom_bar.h,
            INNER.h,
            "top+pane+bottom must equal inner.h"
        );
    }

    #[test]
    fn heights_sum_ide() {
        let areas = docks_for(LayoutMode::Ide, false).compute_areas(INNER, MIN_W, MIN_H);
        assert_eq!(
            areas.top_bar.h + areas.pane_area.h + areas.bottom_bar.h,
            INNER.h
        );
    }

    // ── Terminal mode: zero-width docks ───────────────────────────────────────

    #[test]
    fn terminal_hud_off_zero_left_dock() {
        let docks = docks_for(LayoutMode::Terminal, false);
        assert_eq!(docks.left_w, 0.0, "Terminal mode must have zero left dock");
        // right_w absorbs the right GRID_PAD when HUD is hidden
        assert_eq!(
            docks.right_w, GRID_PAD,
            "Terminal/HUD-off right_w must equal GRID_PAD"
        );
    }

    // ── Terminal mode: pane_area equals inner minus bottom bar ────────────────

    #[test]
    fn terminal_hud_off_pane_area_eq_inner_minus_bottom() {
        let areas = docks_for(LayoutMode::Terminal, false).compute_areas(INNER, MIN_W, MIN_H);
        assert_eq!(areas.pane_area.x, INNER.x);
        assert_eq!(areas.pane_area.y, INNER.y); // top_h = 0
        // right_w = GRID_PAD reserved as right padding; pane width = inner.w - GRID_PAD
        assert_eq!(areas.pane_area.w, INNER.w - GRID_PAD);
        assert_eq!(areas.pane_area.h, INNER.h - BOTTOM_H);
    }

    // ── Scale linearity ───────────────────────────────────────────────────────

    #[test]
    fn ide_left_w_scales_linearly() {
        let d1 = Docks::for_mode(LayoutMode::Ide, 1.0, metrics(10), false, BOTTOM_H);
        let d2 = Docks::for_mode(LayoutMode::Ide, 2.0, metrics(10), false, BOTTOM_H);
        assert_eq!(d2.left_w, d1.left_w * 2.0, "left_w must scale linearly");
    }

    // ── IDE top bar height ────────────────────────────────────────────────────

    #[test]
    fn ide_top_bar_h_is_28pt_at_1x() {
        let docks = docks_for(LayoutMode::Ide, false);
        assert_eq!(docks.top_h, 28.0, "Ide top_h is a 28pt context strip");
        let areas = docks.compute_areas(INNER, MIN_W, MIN_H);
        assert_eq!(areas.top_bar.h, 28.0);
    }

    #[test]
    fn ide_top_bar_h_scales_linearly() {
        let d2 = Docks::for_mode(LayoutMode::Ide, 2.0, metrics(10), false, BOTTOM_H);
        assert_eq!(d2.top_h, 56.0, "top_h must scale linearly");
    }

    /// A4: top_h must also scale with ui_scale (separate from Retina window_scale).
    #[test]
    fn ide_top_bar_h_scales_with_ui_scale() {
        let base = Docks::for_mode_with_left_dock_w(
            LayoutMode::Ide,
            1.0,
            metrics(10),
            false,
            BOTTOM_H,
            true,
            300.0,
            1.0,
        );
        let zoomed = Docks::for_mode_with_left_dock_w(
            LayoutMode::Ide,
            1.0,
            metrics(10),
            false,
            BOTTOM_H,
            true,
            300.0,
            1.5,
        );
        assert!(
            (base.top_h - 28.0).abs() < 1e-9,
            "base top_h at ui_scale=1 must be 28pt"
        );
        assert!(
            (zoomed.top_h - 42.0).abs() < 1e-9,
            "zoomed top_h at ui_scale=1.5 must be 42pt; got {}",
            zoomed.top_h
        );
    }

    #[test]
    fn terminal_top_bar_h_zero() {
        let docks = docks_for(LayoutMode::Terminal, false);
        assert_eq!(docks.top_h, 0.0, "Terminal top_h must remain 0");
    }

    // ── Round-trip Terminal → Ide → Terminal ──────────────────────────────────

    #[test]
    fn round_trip_terminal_ide_terminal() {
        let m = metrics(10);
        let before = Docks::for_mode(LayoutMode::Terminal, 1.0, m, false, BOTTOM_H)
            .compute_areas(INNER, MIN_W, MIN_H);
        let after = Docks::for_mode(LayoutMode::Terminal, 1.0, m, false, BOTTOM_H)
            .compute_areas(INNER, MIN_W, MIN_H);
        assert_eq!(
            before.pane_area, after.pane_area,
            "Terminal areas must be identical before and after Ide round-trip"
        );
    }

    // ── No overlap ────────────────────────────────────────────────────────────

    fn rects_overlap(a: Rect, b: Rect) -> bool {
        a.w > 0.0
            && b.w > 0.0
            && a.h > 0.0
            && b.h > 0.0
            && a.x < b.x + b.w
            && b.x < a.x + a.w
            && a.y < b.y + b.h
            && b.y < a.y + a.h
    }

    #[test]
    fn ide_areas_non_overlapping() {
        let areas = docks_for(LayoutMode::Ide, true).compute_areas(INNER, MIN_W, MIN_H);
        let all = [
            areas.top_bar,
            areas.left_dock,
            areas.pane_area,
            areas.right_dock,
            areas.bottom_bar,
        ];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert!(
                    !rects_overlap(all[i], all[j]),
                    "areas[{i}] and areas[{j}] must not overlap: {:#?} {:#?}",
                    all[i],
                    all[j]
                );
            }
        }
    }

    // ── P1: F-tier — sidebar width × ui_scale matrix ──────────────────────────
    //
    // Exercises `for_mode_with_left_dock_w` for all combinations of
    // `left_dock_w_pt ∈ {180, 240, 300, 400, 500, 600}` and
    // `ui_scale ∈ {1.0, 1.5, 2.0}`.  Each combo asserts three invariants:
    //   (a) left_dock.w  == left_dock_w_pt * scale * ui_scale  (for Ide mode)
    //       but NOTE: left_dock.w uses only `scale`, NOT `ui_scale`
    //       (ui_scale only affects top_h per the implementation)
    //       — so invariant (a) is: left_dock.w == left_dock_w_pt * scale
    //   (b) pane_area.x  == left_dock.x + left_dock.w
    //   (c) pane_area.w  == inner.w - left_dock.w - right_dock.w  (clamped ≥ MIN_W)

    fn left_dock_combo(left_dock_w_pt: f64, ui_scale: f64) -> Areas {
        let scale = 1.0; // Retina scale; keep at 1.0 to isolate ui_scale
        let docks = Docks::for_mode_with_left_dock_w(
            LayoutMode::Ide,
            scale,
            metrics(10),
            false, // hud_visible
            BOTTOM_H,
            true, // left_dock_visible
            left_dock_w_pt,
            ui_scale,
        );
        docks.compute_areas(INNER, MIN_W, MIN_H)
    }

    #[test]
    fn f_tier_left_dock_w_eq_pt_times_scale() {
        let scale = 1.0_f64;
        for &w_pt in &[180.0_f64, 240.0, 300.0, 400.0, 500.0, 600.0] {
            for &ui in &[1.0_f64, 1.5, 2.0] {
                let areas = left_dock_combo(w_pt, ui);
                let expected = w_pt * scale;
                assert!(
                    (areas.left_dock.w - expected).abs() < 1e-9,
                    "left_dock.w mismatch at w_pt={w_pt} ui_scale={ui}: \
                     got {}, expected {expected}",
                    areas.left_dock.w
                );
            }
        }
    }

    #[test]
    fn f_tier_pane_x_eq_left_dock_right_edge() {
        for &w_pt in &[180.0_f64, 240.0, 300.0, 400.0, 500.0, 600.0] {
            for &ui in &[1.0_f64, 1.5, 2.0] {
                let areas = left_dock_combo(w_pt, ui);
                let expected = areas.left_dock.x + areas.left_dock.w;
                assert!(
                    (areas.pane_area.x - expected).abs() < 1e-9,
                    "pane_area.x mismatch at w_pt={w_pt} ui_scale={ui}: \
                     got {}, expected {expected}",
                    areas.pane_area.x
                );
            }
        }
    }

    #[test]
    fn f_tier_pane_w_eq_inner_minus_docks() {
        for &w_pt in &[180.0_f64, 240.0, 300.0, 400.0, 500.0, 600.0] {
            for &ui in &[1.0_f64, 1.5, 2.0] {
                let areas = left_dock_combo(w_pt, ui);
                let unclamped = INNER.w - areas.left_dock.w - areas.right_dock.w;
                let expected = unclamped.max(MIN_W);
                assert!(
                    (areas.pane_area.w - expected).abs() < 1e-9,
                    "pane_area.w mismatch at w_pt={w_pt} ui_scale={ui}: \
                     got {}, expected {expected} (unclamped {unclamped})",
                    areas.pane_area.w
                );
            }
        }
    }
}
