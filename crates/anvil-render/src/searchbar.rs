//! The in-terminal search bar — a fixed-pixel-height strip at the bottom of
//! the window. Mirrors the status-bar pixel-strip pattern.

use anvil_term::{Search, SearchScope};
use anvil_theme::Theme;
use anvil_workspace::editor_search::EditorSearch;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

// ── Hit regions for the nav arrows (N4) ──────────────────────────────────────

/// Pixel rects for the ◀ prev and ▶ next buttons painted in the search bar.
///
/// Cleared by `draw_search_bar_with_replace` before each frame; populated when
/// a search is active and there is at least one match.  Consumed by
/// `mouse_down` in `main.rs` to fire `SearchPrev` / `SearchNext`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SearchBarArrowHits {
    /// Hit rect for the ◀ (prev) arrow. Zero-sized when not present.
    pub prev: [f64; 4], // x, y, w, h
    /// Hit rect for the ▶ (next) arrow. Zero-sized when not present.
    pub next: [f64; 4],
}

/// Draw the search bar as a fixed-pixel strip at the bottom of the raster.
///
/// `chrome_bottom_px` is the strip height (same constant used by the status
/// bar so the two bars swap in/out of the same slot without a layout jump).
/// Glyphs are pixel-positioned and vertically centred using the same
/// `descent * 0.5` formula as `statusbar.rs`.
///
/// Left: "find: " prefix (muted) + query (foreground) + cursor block
///       (accent_bright) at the insertion point.
/// Right: `N of M` counter + ◀ ▶ nav arrows, all in `text_muted`.
///
/// When `editor_search.replace_input` is `Some`, a second row is painted
/// beneath the first with "replace: " prefix + replace text + two buttons
/// `[replace]` and `[replace all]` on the right (item 9).
///
/// Pass `editor_search` when the focused pane is a native editor pane (NE11);
/// pass `None` for terminal panes (which use `search` instead).
///
/// `replace_active` controls which row the text cursor appears in: `false` =
/// find row, `true` = replace row.
#[allow(clippy::too_many_arguments)]
pub fn draw_search_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    search: &Search,
    chrome_bottom_px: f64,
    window_scale: f64,
    editor_search: Option<&EditorSearch>,
) {
    draw_search_bar_with_replace(
        raster,
        painter,
        metrics,
        theme,
        search,
        chrome_bottom_px,
        window_scale,
        editor_search,
        false,
        &mut SearchBarArrowHits::default(),
    );
}

/// Variant that accepts `replace_active` to show the cursor in the replace row
/// and outputs nav-arrow hit regions into `hits_out` (N4).
#[allow(clippy::too_many_arguments)]
pub fn draw_search_bar_with_replace(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    search: &Search,
    chrome_bottom_px: f64,
    window_scale: f64,
    editor_search: Option<&EditorSearch>,
    replace_active: bool,
    hits_out: &mut SearchBarArrowHits,
) {
    *hits_out = SearchBarArrowHits::default();
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

    // ── Resolve query + counter from either source ────────────────────────────
    let (prefix, query, cur, count) = if let Some(es) = editor_search {
        let count = es.count();
        let cur = if count == 0 { 0 } else { es.current + 1 };
        ("find: ", es.query.as_str(), cur, count)
    } else {
        let prefix = if search.scope() == SearchScope::Block {
            "block find: "
        } else {
            "find: "
        };
        let count = search.count();
        let cur = if count == 0 { 0 } else { search.current + 1 };
        (prefix, search.query(), cur, count)
    };

    // N4: "N of M" label + ◀ ▶ arrows on the right.
    let counter = if count == 0 {
        format!("{cur}/{count}")
    } else {
        format!("{cur} of {count}")
    };

    // Right-side reservation: ▶ + space + ◀ + space + counter + 1-column gap.
    // Arrows are each 1 cell wide; gap between arrow and counter is 1 cell.
    let arrow_cols = 2usize; // ◀ + ▶
    let arrow_gaps = 3usize; // spaces between/around arrows and counter
    let reserved_right = (counter.chars().count() + arrow_cols + arrow_gaps + 1) as f64 * cell_w;

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

    // Cursor block after the last query character (find row, when not replace_active).
    if !replace_active && x + cell_w <= right_edge {
        raster.fill_pixel_rect(
            x,
            strip_top + 2.0,
            cell_w,
            cell_h - 4.0,
            theme.accent_bright,
        );
    }

    // ── Right: nav arrows (N4) + match counter ───────────────────────────────
    // Layout (right-to-left): pad_x | ▶ | space | ◀ | space | counter | space
    // ▶ is at the far right (before pad_x), ◀ is one cell to its left.
    let next_x = total_w - pad_x - cell_w;
    let prev_x = next_x - cell_w * 2.0; // 1 space gap between arrows
    let counter_end_x = prev_x - cell_w; // 1 space gap between ◀ and counter
    let counter_x = (counter_end_x - counter.chars().count() as f64 * cell_w).max(0.0);

    // Counter.
    let mut rx = counter_x;
    for ch in counter.chars() {
        if rx + cell_w > counter_end_x {
            break;
        }
        raster.glyph_at(painter, metrics, rx, glyph_y, ch as u32, theme.text_muted);
        rx += cell_w;
    }

    // ◀ arrow.
    if count > 0 && prev_x >= 0.0 {
        raster.glyph_at(
            painter,
            metrics,
            prev_x,
            glyph_y,
            '\u{25C0}' as u32,
            theme.text_muted,
        );
        hits_out.prev = [prev_x, strip_top, cell_w, chrome_bottom_px];
    }
    // ▶ arrow.
    if count > 0 && next_x >= 0.0 {
        raster.glyph_at(
            painter,
            metrics,
            next_x,
            glyph_y,
            '\u{25B6}' as u32,
            theme.text_muted,
        );
        hits_out.next = [next_x, strip_top, cell_w, chrome_bottom_px];
    }

    // ── Replace row (item 9) ────────────────────────────────────────────
    if let Some(es) = editor_search {
        if let Some(replace_text) = &es.replace_input {
            let row2_y = glyph_y + cell_h;
            let replace_prefix = "replace: ";
            // Hairline separator between the two rows.
            raster.fill_pixel_rect(0.0, strip_top + cell_h, total_w, 1.0, theme.hairline);

            // Buttons on the right: "[replace]" and "[all]"
            let btn_all = "[all]";
            let btn_one = "[replace]";
            let btn_all_w = btn_all.chars().count() as f64 * cell_w;
            let btn_one_w = btn_one.chars().count() as f64 * cell_w;
            let btn_gap = cell_w;
            let replace_right =
                (total_w - pad_x - btn_all_w - btn_gap - btn_one_w - btn_gap).max(pad_x);

            let mut rx2 = pad_x;
            for ch in replace_prefix.chars() {
                if rx2 + cell_w > replace_right {
                    break;
                }
                raster.glyph_at(painter, metrics, rx2, row2_y, ch as u32, theme.text_muted);
                rx2 += cell_w;
            }
            for ch in replace_text.chars() {
                if rx2 + cell_w > replace_right {
                    break;
                }
                raster.glyph_at(painter, metrics, rx2, row2_y, ch as u32, theme.foreground);
                rx2 += cell_w;
            }
            // Cursor block in replace row when replace_active.
            if replace_active && rx2 + cell_w <= replace_right {
                raster.fill_pixel_rect(
                    rx2,
                    strip_top + cell_h + 2.0,
                    cell_w,
                    cell_h - 4.0,
                    theme.accent_bright,
                );
            }
            // Draw [replace] button.
            let mut bx = total_w - pad_x - btn_all_w - btn_gap - btn_one_w;
            for ch in btn_one.chars() {
                if bx + cell_w > total_w {
                    break;
                }
                raster.glyph_at(painter, metrics, bx, row2_y, ch as u32, theme.text_muted);
                bx += cell_w;
            }
            // Draw [all] button.
            let mut bx2 = total_w - pad_x - btn_all_w;
            for ch in btn_all.chars() {
                if bx2 + cell_w > total_w {
                    break;
                }
                raster.glyph_at(painter, metrics, bx2, row2_y, ch as u32, theme.text_muted);
                bx2 += cell_w;
            }
        }
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
            None,
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
        draw_search_bar(&mut r, &mut painter, m, &theme, &search, 40.0, 1.0, None);
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
            None,
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

    // ── N4: search bar nav arrows ─────────────────────────────────────────────

    /// When there are matches, `draw_search_bar_with_replace` must populate
    /// non-zero hit rects for both ◀ and ▶ arrows.
    #[test]
    fn nav_arrows_hit_rects_populated_when_matches_exist() {
        use anvil_workspace::editor_search::EditorSearch;
        use anvil_editor::Buffer;

        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let search = Search::new();
        let theme = anvil_theme::MINERAL_DARK;

        // Build an EditorSearch with one match so count > 0.
        let buf = Buffer::from_text("hello hello\n");
        let mut es = EditorSearch::new();
        es.rescan(&buf);
        es.query = "hello".into();
        es.rescan(&buf);

        let mut hits = SearchBarArrowHits::default();
        draw_search_bar_with_replace(
            &mut r,
            &mut painter,
            m,
            &theme,
            &search,
            m.cell_h * 2.0,
            1.0,
            Some(&es),
            false,
            &mut hits,
        );

        assert!(
            hits.prev[2] > 0.0,
            "prev arrow hit rect must have non-zero width when matches exist"
        );
        assert!(
            hits.next[2] > 0.0,
            "next arrow hit rect must have non-zero width when matches exist"
        );
    }

    /// When there are no matches, arrow hit rects must be zero-sized.
    #[test]
    fn nav_arrows_hit_rects_absent_when_no_matches() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let search = Search::new();
        let theme = anvil_theme::MINERAL_DARK;
        // No editor_search → terminal search with count=0.

        let mut hits = SearchBarArrowHits::default();
        draw_search_bar_with_replace(
            &mut r,
            &mut painter,
            m,
            &theme,
            &search,
            m.cell_h * 2.0,
            1.0,
            None,
            false,
            &mut hits,
        );

        assert_eq!(
            hits.prev[2], 0.0,
            "prev arrow hit rect must be zero-sized when count=0"
        );
        assert_eq!(
            hits.next[2], 0.0,
            "next arrow hit rect must be zero-sized when count=0"
        );
    }
}
