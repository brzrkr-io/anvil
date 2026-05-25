//! Native editor pane render path — NE5.
//!
//! `draw_editor_into` paints a `Buffer`'s text into a pixel-raster pane area.
//! It mirrors the structure of `draw_viewport` in `draw.rs` but sources cells
//! from `Buffer` rows instead of a `Terminal` grid.
//!
//! Features:
//! - Solid background fill.
//! - Left gutter with right-aligned line numbers in `theme.text_muted`.
//! - Buffer rows rendered grapheme-by-grapheme in `theme.foreground`.
//! - Long-line clip: lines wider than the content area get a `▸` marker at
//!   the right edge in `theme.text_muted`.
//! - Cursor: 2 px-wide vertical bar at `(cursor.pos.line, cursor.pos.col)` in
//!   `theme.accent`.
//! - Selection wash: `fill_pixel_rect_alpha` over selected cells at α=0.18
//!   using `theme.accent_ember`.
//! - No syntax color (NE8). No soft-wrap (long lines clip).
//! - Scroll is integer-row-aligned: `floor(editor_pane.scroll_pos)`.

use unicode_segmentation::UnicodeSegmentation as _;

use anvil_editor::Buffer;
use anvil_theme::Theme;
use anvil_workspace::{editor_pane::EditorPane, layout::Rect, selection::Selection};

use crate::raster::{FontMetrics, GlyphPainter, Raster};

// ── Public entry point ────────────────────────────────────────────────────────

/// Draw the contents of `editor_pane` / `buffer` into `raster`.
///
/// `rect` is the pane area in device pixels (absolute, not relative to origin).
/// After this call the raster's `origin_x`/`origin_y` are not changed; callers
/// are expected to set them before calling (matching the `draw_workspace` pattern).
#[allow(clippy::too_many_arguments)]
pub fn draw_editor_into(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    editor_pane: &EditorPane,
    buffer: &Buffer,
    metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
) {
    let cw = metrics.cell_w;
    let ch = metrics.cell_h;

    // ── Background ────────────────────────────────────────────────────────────
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.background);

    // ── Geometry ──────────────────────────────────────────────────────────────
    let line_count = buffer.line_count().max(1);
    // Gutter width: digits needed for highest line number + 2 padding cols.
    let digit_cols = line_count.to_string().len();
    let gutter_cols = digit_cols + 2;
    let gutter_w = gutter_cols as f64 * cw;

    // Available content columns to the right of the gutter.
    let content_cols = ((rect.w - gutter_w) / cw).floor() as usize;
    // Number of visible rows that fit in the pane height.
    let visible_rows = (rect.h / ch).ceil() as usize;

    // First visible buffer line (integer snap).
    let scroll_line = editor_pane.scroll_pos.floor() as usize;

    // ── Selection bounds (pre-compute for wash pass) ──────────────────────────
    let sel = &editor_pane.selection;

    // ── Row loop ──────────────────────────────────────────────────────────────
    for vrow in 0..visible_rows {
        let line_idx = scroll_line + vrow;
        if line_idx >= line_count {
            break;
        }

        let row_y = rect.y + vrow as f64 * ch;

        // ── Selection wash for this row ───────────────────────────────────────
        if sel.active {
            paint_selection_row(raster, sel, line_idx, vrow, rect, gutter_w, cw, ch, theme);
        }

        // ── Gutter: right-aligned line number ─────────────────────────────────
        let line_num_str = (line_idx + 1).to_string();
        // Right-align within the gutter: pad = (digit_cols - digits) + 1 space
        let pad_cols = digit_cols.saturating_sub(line_num_str.len()) + 1;
        for (i, ch_g) in line_num_str.chars().enumerate() {
            let gx = rect.x + (pad_cols + i) as f64 * cw;
            raster.glyph_at(painter, metrics, gx, row_y, ch_g as u32, theme.text_muted);
        }

        // ── Buffer content ────────────────────────────────────────────────────
        let line_slice = buffer.line(line_idx);
        let line_str: String = line_slice.chars().collect();
        // Strip trailing newline before grapheme iteration.
        let line_content = line_str.trim_end_matches('\n').trim_end_matches('\r');

        let graphemes: Vec<&str> = line_content.graphemes(true).collect();
        let mut painted = 0usize;
        let overflow = graphemes.len() > content_cols;
        let paint_limit = if overflow {
            content_cols.saturating_sub(1)
        } else {
            content_cols
        };

        for (col, g) in graphemes.iter().enumerate() {
            if col >= paint_limit {
                break;
            }
            // Use the first scalar of the grapheme cluster as the glyph key.
            let cp = g.chars().next().unwrap_or(' ') as u32;
            let gx = rect.x + gutter_w + col as f64 * cw;
            raster.glyph_at(painter, metrics, gx, row_y, cp, theme.foreground);
            painted = col + 1;
        }

        // ── Long-line overflow marker ─────────────────────────────────────────
        if overflow {
            let marker_col = paint_limit;
            let gx = rect.x + gutter_w + marker_col as f64 * cw;
            raster.glyph_at(painter, metrics, gx, row_y, '▸' as u32, theme.text_muted);
        }
        let _ = painted; // suppress dead-code lint
    }

    // ── Cursor bar ────────────────────────────────────────────────────────────
    let cursor_line = editor_pane.cursor.pos.line;
    let cursor_col = editor_pane.cursor.pos.col;
    if cursor_line >= scroll_line {
        let vrow = cursor_line - scroll_line;
        if vrow < visible_rows {
            let cx = rect.x + gutter_w + cursor_col as f64 * cw;
            let cy = rect.y + vrow as f64 * ch;
            // 2 px-wide vertical bar, full cell height.
            raster.fill_pixel_rect(cx, cy, 2.0, ch, theme.accent);
        }
    }
}

// ── Selection wash helper ─────────────────────────────────────────────────────

/// Paint the selection wash for a single buffer row.
///
/// Computes the column range that is selected on `line_idx` and fills those
/// cell rects with `theme.accent_ember` at α=0.18.  First and last selected
/// rows use column-precise bounds; middle rows fill the full content width.
#[allow(clippy::too_many_arguments)]
fn paint_selection_row(
    raster: &mut Raster,
    sel: &Selection,
    line_idx: usize,
    vrow: usize,
    rect: Rect,
    gutter_w: f64,
    cw: f64,
    ch: f64,
    theme: &Theme,
) {
    use anvil_workspace::selection::SelectionMode;

    if !sel.active {
        return;
    }
    let (start, end) = sel.ordered();
    if line_idx < start.row || line_idx > end.row {
        return;
    }

    let row_y = rect.y + vrow as f64 * ch;

    let (col_start, col_end) = match sel.mode {
        SelectionMode::Rect => {
            let lo = start.col.min(end.col);
            let hi = start.col.max(end.col);
            (lo, hi)
        }
        SelectionMode::Linear => {
            if start.row == end.row {
                (start.col, end.col)
            } else if line_idx == start.row {
                // First row: from start.col to far right (use a large sentinel).
                (start.col, usize::MAX)
            } else if line_idx == end.row {
                // Last row: from col 0 to end.col.
                (0, end.col)
            } else {
                // Middle rows: full width.
                (0, usize::MAX)
            }
        }
    };

    if col_start >= col_end {
        return;
    }

    let content_w = rect.w - gutter_w;
    let x_start = rect.x + gutter_w + col_start as f64 * cw;
    let x_end = if col_end == usize::MAX {
        rect.x + rect.w
    } else {
        (rect.x + gutter_w + col_end as f64 * cw).min(rect.x + rect.w)
    };

    let wash_w = (x_end - x_start).max(0.0).min(content_w);
    if wash_w > 0.0 {
        raster.fill_pixel_rect_alpha(x_start, row_y, wash_w, ch, theme.accent_ember, 0.18);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_editor::{Buffer, Cursor, Position};
    use anvil_theme::MINERAL_DARK;
    use anvil_workspace::{
        editor_pane::EditorPane,
        layout::Rect,
        selection::{Selection, SelectionMode},
    };

    use crate::raster::{FontMetrics, GlyphPainter, PixelRect, Raster};

    // ── Stub painter that captures every glyph call ───────────────────────────

    #[derive(Default)]
    struct CapturePainter {
        /// (codepoint, fg_color)
        pub calls: Vec<(u32, [u8; 3])>,
    }

    impl GlyphPainter for CapturePainter {
        #[allow(clippy::too_many_arguments)]
        fn draw_glyph(
            &mut self,
            codepoint: u32,
            _dest: PixelRect,
            fg: [u8; 3],
            _metrics: FontMetrics,
            _pixels: &mut [u8],
            _bw: usize,
            _bh: usize,
        ) {
            self.calls.push((codepoint, fg));
        }
    }

    fn metrics() -> FontMetrics {
        FontMetrics {
            cell_w: 8.0,
            cell_h: 16.0,
            descent: 3.0,
        }
    }

    fn rect() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        }
    }

    fn make_pane(buffer_id: u64) -> EditorPane {
        let origin = Position { line: 0, col: 0 };
        EditorPane {
            buffer_id,
            cursor: Cursor {
                pos: origin,
                anchor: origin,
            },
            selection: Selection::default(),
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
        }
    }

    // ── draw_editor_empty_buffer_paints_only_gutter_line_one ─────────────────

    /// An empty buffer renders the gutter line-number "1" and nothing else in
    /// the content area (no buffer grapheme calls).
    #[test]
    fn draw_editor_empty_buffer_paints_only_gutter_line_one() {
        let buf = Buffer::new();
        let pane = make_pane(1);
        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
        );

        // The gutter should paint the digit '1' in text_muted.
        let gutter_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.text_muted)
            .collect();
        assert!(
            gutter_calls.iter().any(|(cp, _)| *cp == '1' as u32),
            "gutter must paint '1' for empty buffer, got: {gutter_calls:?}"
        );

        // No foreground calls (empty line has no graphemes).
        let fg_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.foreground)
            .collect();
        assert!(
            fg_calls.is_empty(),
            "empty buffer must produce no foreground glyph calls, got: {fg_calls:?}"
        );
    }

    // ── draw_editor_hello_world_paints_each_grapheme ──────────────────────────

    /// A buffer with "hello" on line 0 must produce a foreground glyph call for
    /// each of the 5 characters.
    #[test]
    fn draw_editor_hello_world_paints_each_grapheme() {
        let buf = Buffer::from_text("hello\n");
        let pane = make_pane(1);
        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
        );

        let fg_cps: Vec<u32> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.foreground)
            .map(|(cp, _)| *cp)
            .collect();

        // Expect exactly h, e, l, l, o.
        let expected: Vec<u32> = "hello".chars().map(|c| c as u32).collect();
        assert_eq!(
            fg_cps, expected,
            "must paint each grapheme of 'hello' in foreground order"
        );
    }

    // ── draw_editor_cursor_at_row_5_col_3_paints_cursor_rect ─────────────────

    /// When the cursor is at (line=5, col=3), the pixel at the expected cursor
    /// x position carries `theme.accent`.
    #[test]
    fn draw_editor_cursor_at_row_5_col_3_paints_cursor_rect() {
        use crate::raster::pixel_at;

        // Build a buffer with 10 lines so line 5 exists.
        let text: String = (0..10).map(|i| format!("line{i}\n")).collect();
        let buf = Buffer::from_text(&text);
        let mut pane = make_pane(1);
        pane.cursor = Cursor {
            pos: Position { line: 5, col: 3 },
            anchor: Position { line: 5, col: 3 },
        };

        let m = metrics();
        let r = rect();
        let line_count = buf.line_count().max(1);
        let digit_cols = line_count.to_string().len();
        let gutter_cols = digit_cols + 2;
        let gutter_w = gutter_cols as f64 * m.cell_w;

        let mut raster = Raster::new(400, 200);
        raster.clear(MINERAL_DARK.background);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(&mut raster, &mut painter, &pane, &buf, m, &theme, r);

        // Cursor pixel: x = gutter_w + col * cw, y = row * ch (row 5, col 3).
        let cx = (r.x + gutter_w + 3.0 * m.cell_w) as usize;
        let cy = (r.y + 5.0 * m.cell_h) as usize;
        let px = pixel_at(&raster, cx, cy);
        assert_eq!(
            px, theme.accent,
            "cursor pixel at ({cx},{cy}) should be accent, got {px:?}"
        );
    }

    // ── draw_editor_long_line_paints_overflow_marker ──────────────────────────

    /// A line longer than the available content columns must paint the `▸`
    /// overflow marker in `theme.text_muted`.
    #[test]
    fn draw_editor_long_line_paints_overflow_marker() {
        // content area = 400px, gutter ~3 cols * 8px = 24px, leaving 47 cols.
        // Build a line with 60 characters — well past 47.
        let long_line: String = "a".repeat(60) + "\n";
        let buf = Buffer::from_text(&long_line);
        let pane = make_pane(1);
        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
        );

        let has_overflow_marker = painter
            .calls
            .iter()
            .any(|(cp, fg)| *cp == '▸' as u32 && *fg == theme.text_muted);
        assert!(
            has_overflow_marker,
            "overflow marker '▸' in text_muted must be painted for a 60-char line"
        );
    }

    // ── draw_editor_scroll_skips_top_rows ─────────────────────────────────────

    /// When `scroll_pos = 3.0`, lines 0-2 must not appear and line 3 must be
    /// the first painted row.
    #[test]
    fn draw_editor_scroll_skips_top_rows() {
        let text: String = (0..10).map(|i| format!("L{i}\n")).collect();
        let buf = Buffer::from_text(&text);
        let mut pane = make_pane(1);
        pane.scroll_pos = 3.0;

        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
        );

        // Line 3 starts with 'L' followed by '3'. The foreground calls should
        // contain '4' (gutter line number for logical line 4 = 1-indexed 4).
        // More directly: there must be no foreground call for '0', '1', or '2'
        // — those are the content chars from lines 0-2.
        let fg_cps: Vec<u32> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.foreground)
            .map(|(cp, _)| *cp)
            .collect();

        // Lines 0–2 contain '0', '1', '2' as second char of "L0", "L1", "L2".
        // With scroll=3 those must be absent.
        assert!(
            !fg_cps.contains(&('0' as u32)),
            "scroll should skip line 0; '0' must not appear in fg glyphs"
        );
        assert!(
            !fg_cps.contains(&('1' as u32)),
            "scroll should skip line 1; '1' must not appear in fg glyphs"
        );
        assert!(
            !fg_cps.contains(&('2' as u32)),
            "scroll should skip line 2; '2' must not appear in fg glyphs"
        );
        // Line 3 ("L3") must be present.
        assert!(
            fg_cps.contains(&('L' as u32)),
            "'L' from visible lines must appear in fg glyphs"
        );
        assert!(
            fg_cps.contains(&('3' as u32)),
            "'3' from line 3 must appear in fg glyphs"
        );
    }
}
