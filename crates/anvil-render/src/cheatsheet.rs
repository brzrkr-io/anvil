//! Keyboard-shortcut cheatsheet overlay.
//!
//! Draws a centered modal card listing every shortcut grouped by category.
//! Palette: chrome constants (CHARCOAL panel, CHROME_BORDER edges, MIST title,
//! TEXT_MUTED group headers, TEXT_SUBTLE chords, TEXT_MUTED descriptions).
//!
//! Call `draw` from renderFrame *last* (on top of grid, HUD, tree, tab bar).

use crate::raster::{FontMetrics, GlyphPainter, Raster};

// --- Chrome palette constants (match tabbar.rs / statusbar.rs) ---------------

/// Deep fill: card background.
const CHARCOAL: [u8; 3] = [0x1d, 0x21, 0x29];
/// 1px hairline border color.
const CHROME_BORDER: [u8; 3] = [0x23, 0x26, 0x2b];
/// Dimmed text: group section headers and descriptions.
const TEXT_MUTED: [u8; 3] = [0xa1, 0xa4, 0xa9];
/// Lightest label: card title.
const MIST: [u8; 3] = [0xd2, 0xd8, 0xdb];
/// Mid-tone: key chord labels.
const TEXT_SUBTLE: [u8; 3] = [0x6c, 0x6f, 0x74];

// --- Shortcut data ----------------------------------------------------------

/// A row in the cheatsheet: either a group header or a chord+description pair.
pub enum Row {
    /// A group header label.
    Header(&'static str),
    /// A shortcut: chord string + description.
    Shortcut {
        chord: &'static str,
        desc: &'static str,
    },
}

/// Static shortcut list. Pure data — no allocations, testable.
pub const ROWS: &[Row] = &[
    Row::Header("Tabs"),
    Row::Shortcut {
        chord: "Cmd T",
        desc: "new tab",
    },
    Row::Shortcut {
        chord: "Cmd W",
        desc: "close tab",
    },
    Row::Shortcut {
        chord: "Ctrl Tab",
        desc: "next tab",
    },
    Row::Shortcut {
        chord: "Ctrl Shift Tab",
        desc: "previous tab",
    },
    Row::Shortcut {
        chord: "Cmd 1-9",
        desc: "jump to tab",
    },
    Row::Header("Panels"),
    Row::Shortcut {
        chord: "Cmd K",
        desc: "command palette",
    },
    Row::Shortcut {
        chord: "Cmd J",
        desc: "toggle HUD",
    },
    Row::Shortcut {
        chord: "Cmd /",
        desc: "this cheatsheet",
    },
    Row::Shortcut {
        chord: "Cmd + / Cmd -",
        desc: "zoom in / out",
    },
    Row::Shortcut {
        chord: "Cmd 0",
        desc: "reset font size",
    },
    Row::Header("Search"),
    Row::Shortcut {
        chord: "Cmd F",
        desc: "search",
    },
    Row::Shortcut {
        chord: "Cmd G",
        desc: "next match",
    },
    Row::Shortcut {
        chord: "Cmd Shift G",
        desc: "previous match",
    },
    Row::Shortcut {
        chord: "Cmd Opt R",
        desc: "regex mode",
    },
    Row::Header("Navigation"),
    Row::Shortcut {
        chord: "Cmd Up",
        desc: "previous command",
    },
    Row::Shortcut {
        chord: "Cmd Down",
        desc: "next command",
    },
    Row::Header("Selection"),
    Row::Shortcut {
        chord: "drag",
        desc: "select text",
    },
    Row::Shortcut {
        chord: "Opt drag",
        desc: "rectangular select",
    },
    Row::Shortcut {
        chord: "Cmd C",
        desc: "copy",
    },
    Row::Shortcut {
        chord: "Cmd V",
        desc: "paste",
    },
    Row::Header("HUD"),
    Row::Shortcut {
        chord: "click row",
        desc: "copy value",
    },
    Row::Shortcut {
        chord: "Cmd-click row",
        desc: "open in Finder",
    },
    Row::Shortcut {
        chord: "drag left edge",
        desc: "resize HUD",
    },
    Row::Header("Agent"),
    Row::Shortcut {
        chord: "Cmd Shift A",
        desc: "send selection to agent",
    },
    Row::Header("Open"),
    Row::Shortcut {
        chord: "Cmd-click",
        desc: "open path / URL",
    },
    Row::Shortcut {
        chord: "Cmd-click f.rs:42",
        desc: "open at line in $EDITOR",
    },
];

/// Card width in columns for the single-column layout.
pub const CARD_COLS_1: usize = 42;
/// Each half-column width in two-column mode (includes inner margins).
pub const HALF_COLS: usize = 35;
/// Card width in columns for the two-column layout (two halves + 1 divider col).
pub const CARD_COLS_2: usize = HALF_COLS * 2 + 1;

/// Card height in terminal rows (title + hint + separator + all rows + 1 padding).
pub const CARD_ROWS: usize = ROWS.len() + 4;

// --- Draw -------------------------------------------------------------------

/// Draw the cheatsheet as a centered modal card. Must be called last in
/// renderFrame so it renders on top of all other UI elements.
///
/// When the safe vertical area is too short for a single column of shortcuts,
/// the content is split into two side-by-side columns so every shortcut is visible.
///
/// `chrome_top_px` / `chrome_bottom_px`: pixel heights of the chrome strips.
/// `total_cols` / `total_rows`: cell grid dimensions of the safe terminal area.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    total_cols: usize,
    total_rows: usize,
    chrome_top_px: f64,
    chrome_bottom_px: f64,
) {
    if total_rows < 6 || total_cols < 20 {
        return;
    }

    let cw = metrics.cell_w;
    let ch = metrics.cell_h;

    // Pixel height of the content area for a slice of ROWS (includes half-row
    // gaps before non-first headers).
    let content_h_px = |rows: &[Row]| -> f64 {
        let mut h = 0.0_f64;
        let mut first = true;
        for row in rows {
            if matches!(row, Row::Header(_)) && !first {
                h += ch * 0.5; // inter-section gap
            }
            if matches!(row, Row::Header(_)) {
                first = false;
            }
            h += ch;
        }
        h
    };

    // Header area (title + hint + separator rule) + 1-row bottom padding.
    let header_px = 3.0 * ch;
    let padding_px = ch;

    // Single-column card pixel height.
    let single_h_px = header_px + content_h_px(ROWS) + padding_px;

    // Safe pixel height.
    let safe_h_px = raster.height as f64 - chrome_top_px - chrome_bottom_px;

    // Choose layout: two-column when single column is taller than the safe area
    // and the window is wide enough.
    let two_col = single_h_px > safe_h_px && total_cols >= CARD_COLS_2 + 2;

    let card_cols = if two_col {
        CARD_COLS_2.min(total_cols.saturating_sub(2))
    } else {
        CARD_COLS_1.min(total_cols.saturating_sub(2)).max(20)
    };

    // In two-col mode each half is HALF_COLS (both halves equal width).
    let half_cols = if two_col { HALF_COLS } else { card_cols };

    // Card pixel height: in two-col mode use the taller of the two halves.
    let card_h_px = if two_col {
        let split = split_index();
        let left_h = header_px + content_h_px(&ROWS[..split]) + padding_px;
        let right_h = header_px + content_h_px(&ROWS[split..]) + padding_px;
        left_h.max(right_h).min(safe_h_px)
    } else {
        single_h_px.min(safe_h_px).max(6.0 * ch)
    };

    let card_w_px = card_cols as f64 * cw;

    // Center horizontally within the terminal grid area.
    let safe_left_px = raster.pad_x;
    let safe_w_px = total_cols as f64 * cw;
    let left_px = (safe_left_px + (safe_w_px - card_w_px) / 2.0).max(safe_left_px);

    // Center vertically within the safe area between chrome strips.
    let top_px = (chrome_top_px + (safe_h_px - card_h_px) / 2.0).max(chrome_top_px);

    // Fully opaque card surface.
    raster.fill_pixel_rect(left_px, top_px, card_w_px, card_h_px, CHARCOAL);

    // 1px hairline border on all four edges.
    let b = 1.0_f64;
    raster.fill_pixel_rect(left_px, top_px, card_w_px, b, CHROME_BORDER);
    raster.fill_pixel_rect(left_px, top_px + card_h_px - b, card_w_px, b, CHROME_BORDER);
    raster.fill_pixel_rect(left_px, top_px, b, card_h_px, CHROME_BORDER);
    raster.fill_pixel_rect(left_px + card_w_px - b, top_px, b, card_h_px, CHROME_BORDER);

    // In two-column mode: 1px vertical divider down the centre.
    if two_col {
        let mid_x = left_px + half_cols as f64 * cw;
        raster.fill_pixel_rect(mid_x, top_px + ch, b, card_h_px - 2.0 * ch, CHROME_BORDER);
    }

    // --- Row 0: title in MIST.
    draw_text_px(
        raster,
        painter,
        metrics,
        left_px + 3.0 * cw,
        top_px,
        "Keyboard Shortcuts",
        MIST,
        left_px + card_w_px - 2.0 * cw,
    );

    // --- Row 1: dim hint in TEXT_MUTED.
    draw_text_px(
        raster,
        painter,
        metrics,
        left_px + 3.0 * cw,
        top_px + ch,
        "Cmd+/ or Esc to close",
        TEXT_MUTED,
        left_px + card_w_px - 2.0 * cw,
    );

    // --- Row 2: 1px separator rule below the header.
    raster.fill_pixel_rect(
        left_px + 2.0 * cw,
        top_px + 2.0 * ch,
        card_w_px - 4.0 * cw,
        1.0,
        CHROME_BORDER,
    );

    // --- Content rows (single- or two-column).
    let content_bottom_y = top_px + card_h_px - ch; // 1 row bottom padding

    if two_col {
        let split = split_index();
        // Left column.
        draw_column(
            raster,
            painter,
            metrics,
            left_px,
            top_px + 3.0 * ch,
            content_bottom_y,
            cw,
            ch,
            half_cols,
            &ROWS[..split],
        );
        // Right column: starts immediately after the divider col.
        let right_x = left_px + (half_cols + 1) as f64 * cw;
        draw_column(
            raster,
            painter,
            metrics,
            right_x,
            top_px + 3.0 * ch,
            content_bottom_y,
            cw,
            ch,
            half_cols,
            &ROWS[split..],
        );
    } else {
        draw_column(
            raster,
            painter,
            metrics,
            left_px,
            top_px + 3.0 * ch,
            content_bottom_y,
            cw,
            ch,
            card_cols,
            ROWS,
        );
    }
}

/// Returns the index in ROWS at which to split for two-column layout.
/// Splits at the Header boundary nearest to the midpoint.
fn split_index() -> usize {
    let mid = ROWS.len() / 2;
    // Walk forward from mid to find the next Header boundary.
    for (offset, row) in ROWS[mid..].iter().enumerate() {
        if matches!(row, Row::Header(_)) {
            return mid + offset;
        }
    }
    mid
}

/// Draw a column of shortcut rows starting at pixel `(col_left_px, start_y)`.
/// `col_width_cols` is the number of cell-columns available (incl. inner margins).
#[allow(clippy::too_many_arguments)]
fn draw_column(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    col_left_px: f64,
    start_y: f64,
    bottom_y: f64,
    cw: f64,
    ch: f64,
    col_width_cols: usize,
    rows: &[Row],
) {
    let chord_x = col_left_px + 3.0 * cw;
    // Description at chord_x + 15 cols; clears "Ctrl Shift Tab".
    let desc_x = col_left_px + 18.0 * cw;
    let max_x = col_left_px + col_width_cols.saturating_sub(2) as f64 * cw;

    let mut row_y = start_y;
    let mut first_header = true;

    for row in rows {
        if row_y + ch > bottom_y {
            break;
        }
        match row {
            Row::Header(label) => {
                if !first_header {
                    // Half-row gap + hairline before each section after the first.
                    let gap = ch * 0.5;
                    if row_y + gap + ch > bottom_y {
                        break;
                    }
                    raster.fill_pixel_rect(
                        col_left_px + 2.0 * cw,
                        row_y + gap * 0.5,
                        col_width_cols.saturating_sub(4) as f64 * cw,
                        1.0,
                        CHROME_BORDER,
                    );
                    row_y += gap;
                }
                first_header = false;
                draw_text_px(
                    raster, painter, metrics, chord_x, row_y, label, TEXT_MUTED, max_x,
                );
                row_y += ch;
            }
            Row::Shortcut { chord, desc } => {
                draw_text_px(
                    raster,
                    painter,
                    metrics,
                    chord_x,
                    row_y,
                    chord,
                    TEXT_SUBTLE,
                    desc_x - cw,
                );
                if desc_x < max_x {
                    draw_text_px(
                        raster, painter, metrics, desc_x, row_y, desc, TEXT_MUTED, max_x,
                    );
                }
                row_y += ch;
            }
        }
    }
}

// --- Internal helpers -------------------------------------------------------

/// Draw a UTF-8 string starting at pixel position `(px, py)` (top-left of the
/// first cell), stopping when the next glyph would start at or past `max_px`.
#[allow(clippy::too_many_arguments)]
fn draw_text_px(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    px: f64,
    py: f64,
    text: &str,
    color: [u8; 3],
    max_px: f64,
) {
    let cw = metrics.cell_w;
    for (i, cp) in text.chars().enumerate() {
        let gx = px + i as f64 * cw;
        if gx + cw > max_px {
            break;
        }
        raster.glyph_at(painter, metrics, gx, py, cp as u32, color);
    }
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::PixelRect;

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

    #[test]
    fn rows_list_is_non_empty() {
        assert!(!ROWS.is_empty());
    }

    #[test]
    fn rows_contain_at_least_one_header() {
        assert!(ROWS.iter().any(|r| matches!(r, Row::Header(_))));
    }

    #[test]
    fn rows_contain_at_least_one_shortcut() {
        assert!(ROWS.iter().any(|r| matches!(r, Row::Shortcut { .. })));
    }

    #[test]
    fn all_shortcut_chords_and_descs_are_non_empty() {
        for row in ROWS {
            match row {
                Row::Header(label) => assert!(!label.is_empty()),
                Row::Shortcut { chord, desc } => {
                    assert!(!chord.is_empty());
                    assert!(!desc.is_empty());
                }
            }
        }
    }

    #[test]
    fn rows_are_valid_utf8() {
        for row in ROWS {
            match row {
                Row::Header(label) => assert!(std::str::from_utf8(label.as_bytes()).is_ok()),
                Row::Shortcut { chord, desc } => {
                    assert!(std::str::from_utf8(chord.as_bytes()).is_ok());
                    assert!(std::str::from_utf8(desc.as_bytes()).is_ok());
                }
            }
        }
    }

    #[test]
    fn card_rows_covers_all_content_rows() {
        assert_eq!(ROWS.len() + 4, CARD_ROWS);
    }

    /// Smoke: draw does not panic on a reasonably-sized raster.
    #[test]
    fn draw_no_panic() {
        let m = metrics();
        let cols = CARD_COLS_1 + 4;
        let rows = CARD_ROWS + 4;
        let mut r = Raster::new(
            (cols as f64 * m.cell_w) as usize,
            (rows as f64 * m.cell_h) as usize,
        );
        r.pad_x = 0.0;
        r.pad_y = 0.0;
        let mut painter = StubPainter::default();
        draw(&mut r, &mut painter, m, cols, rows, 0.0, 0.0);
        assert!(!painter.calls.is_empty());
    }

    /// draw returns early when the raster is too small.
    #[test]
    fn draw_noop_when_too_small() {
        let m = metrics();
        let mut r = Raster::new(100, 80);
        let mut painter = StubPainter::default();
        draw(&mut r, &mut painter, m, 5, 5, 0.0, 0.0);
        assert!(painter.calls.is_empty());
    }
}
