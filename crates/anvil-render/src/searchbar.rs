//! The in-terminal search bar — one text row at the bottom of the window.
//!
//! Ported from `src/render/searchbar.zig`.

use anvil_term::Search;
use anvil_theme::Theme;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

/// Draw the search bar across the bottom raster row. `bottom_row` is the cell
/// row index of the last row. Shows a "find:" prefix, the query, and a
/// `current/total` match counter.
pub fn draw_search_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    search: &Search,
    bottom_row: usize,
) {
    let cell_w = metrics.cell_w;
    // Match the padded grid width: the bar spans the inset region.
    let usable_w = raster.width as f64 - 2.0 * raster.pad_x;
    let total_cols = ((usable_w.max(0.0)) / cell_w) as usize;
    if total_cols == 0 {
        return;
    }

    // Bar background across the whole bottom row (opaque surface tone).
    for c in 0..total_cols {
        raster.cell_bg(metrics, c, bottom_row, theme.surface);
    }

    // 1px border rule above the bar (same pattern as the tab bar bottom rule).
    let bar_top_px = raster.pad_y + bottom_row as f64 * metrics.cell_h;
    let bar_left_px = raster.pad_x;
    let bar_w_px = total_cols as f64 * cell_w;
    raster.fill_pixel_rect(bar_left_px, bar_top_px, bar_w_px, 1.0, theme.border);

    // Compose the bar text: "find: <query>" left-aligned, "<cur>/<total>" right.
    let count = search.count();
    let cur = if count == 0 { 0 } else { search.current + 1 };
    let counter = format!("{cur}/{count}");

    let text = format!("find: {}", search.query());

    // Left text must not reach the counter; leave at least a 1-column gap.
    // Prefix chars 0–5 ("find: ") are drawn muted; query chars 6+ use foreground.
    const PREFIX_LEN: usize = 6; // "find: "
    if counter.len() + 1 < total_cols {
        let left_limit = total_cols - counter.len() - 1 - 2;
        for (i, ch) in text.chars().enumerate() {
            if i >= left_limit {
                break;
            }
            let color = if i < PREFIX_LEN {
                theme.ansi[8]
            } else {
                theme.foreground
            };
            raster.cell_glyph(painter, metrics, 2 + i, bottom_row, ch as u32, color);
        }
    }

    // Right-aligned counter — metadata, drawn muted (the current-match highlight
    // in the grid itself uses theme.accent — the counter doesn't need to compete).
    if counter.len() <= total_cols {
        let start = total_cols - counter.len();
        for (j, ch) in counter.chars().enumerate() {
            raster.cell_glyph(
                painter,
                metrics,
                start + j,
                bottom_row,
                ch as u32,
                theme.ansi[8],
            );
        }
    }
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

    /// Port of "drawSearchBar fills the bottom row background"
    ///
    /// Uses cell_bg to paint the surface tone — verifies by checking a pixel
    /// inside the bottom row carries theme.surface after the call.
    #[test]
    fn draw_search_bar_fills_bottom_row_background() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let search = Search::new();
        let theme = anvil_theme::MINERAL_DARK;

        let cell_h = m.cell_h as usize;
        // 200 / 20 = 10 rows; bottom_row = 9
        let bottom_row = (200 / cell_h) - 1;
        draw_search_bar(&mut r, &mut painter, m, &theme, &search, bottom_row);

        // A pixel at the center of the bottom row should carry theme.surface.
        let px_y = bottom_row * cell_h + cell_h / 2;
        let px = pixel_at(&r, 4, px_y); // x=4 is inside a cell column
        assert_eq!(
            px, theme.surface,
            "expected surface color at bottom row, got {px:?}"
        );
    }

    /// draw_search_bar: no-op when total_cols == 0 (zero-width raster).
    #[test]
    fn draw_search_bar_noop_on_zero_cols() {
        let m = metrics();
        let mut r = Raster::new(1, 40);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);
        let search = Search::new();
        let theme = anvil_theme::MINERAL_DARK;
        // pad_x = 0 by default; width=1, cell_w=10 → total_cols=0
        draw_search_bar(&mut r, &mut painter, m, &theme, &search, 0);
        // No glyph calls (early return).
        assert!(painter.calls.is_empty());
    }
}
