//! The in-terminal search bar — a fixed-pixel-height strip at the bottom of
//! the window. Mirrors the status-bar pixel-strip pattern.

use anvil_term::{Search, SearchScope};
use anvil_theme::Theme;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

/// Draw the search bar as a fixed-pixel strip at the bottom of the raster.
///
/// `chrome_bottom_px` is the strip height (same constant used by the status
/// bar so the two bars swap in/out of the same slot without a layout jump).
/// Glyphs are pixel-positioned and vertically centred using the same
/// `descent * 0.5` formula as `statusbar.rs`.
///
/// Left: "find: " prefix (muted) + query (foreground) + cursor block
///       (accent_bright) at the insertion point.
/// Right: match counter `cur/total` in `text_muted`.
#[allow(clippy::too_many_arguments)]
pub fn draw_search_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    search: &Search,
    chrome_bottom_px: f64,
    window_scale: f64,
) {
    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;
    let total_w = raster.width as f64;
    let total_h = raster.height as f64;
    if total_w <= 0.0 || chrome_bottom_px <= 0.0 {
        return;
    }

    let strip_top = total_h - chrome_bottom_px;

    // Background fill + 1px hairline at top of strip.
    raster.fill_pixel_rect(0.0, strip_top, total_w, chrome_bottom_px, theme.charcoal);
    raster.fill_pixel_rect(0.0, strip_top, total_w, 1.0, theme.hairline);

    // Vertical glyph baseline — same formula as statusbar.rs.
    let glyph_y = strip_top + ((chrome_bottom_px - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    let pad_x = 14.0 * window_scale;

    // ── Left: scope tag (when Block) + "find: " prefix + query + cursor block
    let prefix = if search.scope() == SearchScope::Block {
        "block find: "
    } else {
        "find: "
    };

    let query = search.query();

    let count = search.count();
    let cur = if count == 0 { 0 } else { search.current + 1 };
    let counter = format!("{cur}/{count}");

    // Right-side reservation: counter + 1-column gap.
    let reserved_right = (counter.chars().count() + 1) as f64 * cell_w;

    let right_edge = (total_w - pad_x - reserved_right).max(pad_x);

    let mut x = pad_x;
    let draw_char = |raster: &mut Raster,
                     painter: &mut dyn GlyphPainter,
                     ch: char,
                     color: [u8; 3],
                     x: &mut f64| {
        if *x + cell_w > right_edge {
            return;
        }
        raster.glyph_at(painter, metrics, *x, glyph_y, ch as u32, color);
        *x += cell_w;
    };

    for ch in prefix.chars() {
        draw_char(raster, painter, ch, theme.text_muted, &mut x);
    }

    for ch in query.chars() {
        draw_char(raster, painter, ch, theme.foreground, &mut x);
    }

    // Cursor block after the last query character.
    if x + cell_w <= right_edge {
        raster.fill_pixel_rect(
            x,
            strip_top + 2.0,
            cell_w,
            chrome_bottom_px - 4.0,
            theme.accent_bright,
        );
    }

    // ── Right: match counter ─────────────────────────────────────────────
    let counter_x = total_w - pad_x - counter.chars().count() as f64 * cell_w;
    let mut rx = counter_x.max(0.0);
    for ch in counter.chars() {
        if rx + cell_w > total_w {
            break;
        }
        raster.glyph_at(painter, metrics, rx, glyph_y, ch as u32, theme.text_muted);
        rx += cell_w;
    }
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{PixelRect, pixel_at};

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

    /// draw_search_bar fills the strip background in theme.charcoal.
    #[test]
    fn draw_search_bar_fills_strip_background() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let search = Search::new();
        let theme = anvil_theme::MINERAL_DARK;
        let chrome_bottom_px = m.cell_h * 2.0; // 40px

        draw_search_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &search,
            chrome_bottom_px,
            1.0,
        );

        // A pixel near the vertical center of the strip should be theme.charcoal.
        let strip_top = (200.0 - chrome_bottom_px) as usize;
        let px_y = strip_top + (chrome_bottom_px * 0.5) as usize;
        let px = pixel_at(&r, 4, px_y);
        assert_eq!(
            px, theme.charcoal,
            "expected charcoal fill in strip, got {px:?}"
        );
    }

    /// draw_search_bar: no-op when zero width.
    #[test]
    fn draw_search_bar_noop_on_zero_width() {
        let m = metrics();
        let mut r = Raster::new(1, 40);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);
        let search = Search::new();
        let theme = anvil_theme::MINERAL_DARK;
        // width=1, chrome_bottom_px=40 — strip_top=0, but total_w=1 is > 0,
        // so the function runs without panic and the counter space calculation
        // pushes x past right_edge — no glyphs drawn.
        draw_search_bar(&mut r, &mut painter, m, &theme, &search, 40.0, 1.0);
        // No assertion on glyph calls — just verify no panic.
    }

    /// Prefix characters are drawn in text_muted.
    #[test]
    fn prefix_chars_drawn_in_text_muted() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);
        let search = Search::new();
        let theme = anvil_theme::MINERAL_DARK;
        draw_search_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &search,
            m.cell_h * 2.0,
            1.0,
        );

        let muted: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.text_muted)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            muted.contains(&'f'),
            "expected 'f' from 'find: ' prefix in text_muted, got {muted:?}"
        );
    }
}
