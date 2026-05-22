//! The low-profile terminal tab bar — one text-row tall, drawn into the raster.
//!
//! Ported from `src/render/tabbar.zig`.

use anvil_theme::Theme;
use anvil_workspace::tab::TabManager;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

/// Draw the tab bar across raster row 0. Each tab gets an equal-width segment;
/// the active segment is filled with theme.surface (clearly raised); inactive
/// segments use theme.background (flat canvas). A thin theme.border line runs
/// along the bottom of the whole bar. Labels have 2-col inner left padding.
///
/// No-op when the manager has fewer than 2 tabs (low-profile rule).
pub fn draw_tab_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    tabs: &TabManager,
) {
    let n = tabs.count();
    if n < 2 {
        return;
    }

    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;
    // Match the padded grid width.
    let usable_w = raster.width as f64 - 2.0 * raster.pad_x;
    let total_cols = ((usable_w.max(0.0)) / cell_w) as usize;
    if total_cols == 0 {
        return;
    }
    let seg_cols = (total_cols / n).max(1);

    for t in 0..n {
        let start_col = t * seg_cols;
        let is_active = t == tabs.active;
        // Active tab: raised surface. Inactive: recessed between canvas and surface.
        let bg = if is_active {
            theme.surface
        } else {
            anvil_theme::mix(theme.background, theme.surface, 0.4)
        };
        // Fill the segment background across row 0.
        let end_col = if t == n - 1 {
            total_cols
        } else {
            start_col + seg_cols
        };
        for col in start_col..end_col {
            raster.cell_bg(metrics, col, 0, bg);
        }
        // 2px accent bar along the top edge of the active tab segment.
        if is_active {
            let start_px = raster.pad_x + start_col as f64 * cell_w;
            let tab_top_px = raster.pad_y;
            let seg_w_px = (end_col - start_col) as f64 * cell_w;
            raster.fill_pixel_rect(start_px, tab_top_px, seg_w_px, 2.0, theme.accent);
        }
        // Draw the tab label, truncated to the segment width minus a 2-col pad.
        let fg = if is_active {
            theme.foreground
        } else {
            theme.ansi[8]
        };
        let seg_w = end_col - start_col;
        let label = tab_label(tabs, t);
        for (i, cp) in label.chars().enumerate() {
            if i + 3 >= seg_w {
                break;
            }
            raster.cell_glyph(painter, metrics, start_col + 2 + i, 0, cp as u32, fg);
        }
    }

    // Thin border line along the bottom of the tab bar (bottom of row 0).
    let bar_bottom_px = raster.pad_y + cell_h - 1.0;
    let bar_left_px = raster.pad_x;
    let bar_w_px = total_cols as f64 * cell_w;
    raster.fill_pixel_rect(bar_left_px, bar_bottom_px, bar_w_px, 2.0, theme.border);
}

/// Derive a display label for tab `t`:
///   - shell title if set,
///   - basename of the focused pane's cwd,
///   - fallback to "shell".
fn tab_label(tabs: &TabManager, t: usize) -> String {
    let tab = match tabs.tabs.get(t) {
        Some(tab) => tab,
        None => return "shell".to_string(),
    };
    let focused_id = tab.tree.focused;
    if let Some(pane) = tab.registry.get(focused_id) {
        let title = pane.terminal().title();
        if !title.is_empty() {
            return title.to_string();
        }
        let cwd = pane.terminal().cwd_path();
        if !cwd.is_empty() {
            return anvil_workspace::tab::basename(cwd).to_string();
        }
    }
    "shell".to_string()
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{PixelRect, pixel_at};

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

    /// Port of "drawTabBar is a no-op below 2 tabs" (0-tab case).
    #[test]
    fn draw_tab_bar_noop_below_2_tabs() {
        let m = metrics();
        let mut r = Raster::new(200, 80);
        let mut painter = StubPainter::default();
        r.clear([1, 2, 3]);

        let mgr = TabManager::default(); // 0 tabs
        let theme = anvil_theme::MINERAL_DARK;

        draw_tab_bar(&mut r, &mut painter, m, &theme, &mgr);
        // No changes: pixel (5,5) still the sentinel.
        // The raster uses BGRA, so checking via pixel_at (which returns RGB).
        let px = pixel_at(&r, 5, 5);
        assert_eq!(px, [1, 2, 3], "expected sentinel [1,2,3], got {px:?}");
        assert!(
            painter.calls.is_empty(),
            "expected no glyph calls for 0 tabs"
        );
    }

    /// draw_tab_bar is a no-op for 1 tab.
    #[test]
    fn draw_tab_bar_noop_for_one_tab() {
        use anvil_workspace::tab::{Tab, TabManager};
        let m = metrics();
        let mut r = Raster::new(200, 80);
        let mut painter = StubPainter::default();
        r.clear([1, 2, 3]);

        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(20, 4, 0));
        let theme = anvil_theme::MINERAL_DARK;

        draw_tab_bar(&mut r, &mut painter, m, &theme, &mgr);
        let px = pixel_at(&r, 5, 5);
        assert_eq!(px, [1, 2, 3]);
        assert!(painter.calls.is_empty());
    }

    /// draw_tab_bar paints the bar with 2 tabs.
    #[test]
    fn draw_tab_bar_paints_with_two_tabs() {
        use anvil_workspace::tab::{Tab, TabManager};
        let m = metrics();
        let mut r = Raster::new(400, 80);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let mut mgr = TabManager::default();
        mgr.push(Tab::new_single_pane(20, 4, 0));
        mgr.push(Tab::new_single_pane(20, 4, 0));
        mgr.active = 0;
        let theme = anvil_theme::MINERAL_DARK;

        draw_tab_bar(&mut r, &mut painter, m, &theme, &mgr);

        // Active tab (left half, row 0) should be painted with theme.surface.
        // cell_h=20, so row 0 bitmap is y=[pad_y, pad_y+20). With pad_y=0:
        // Check a pixel at (5, 10) — inside the left tab segment, row 0 center.
        let px = pixel_at(&r, 5, 10);
        assert_eq!(
            px, theme.surface,
            "expected surface for active tab, got {px:?}"
        );
    }
}
