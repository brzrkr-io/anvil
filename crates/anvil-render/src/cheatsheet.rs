//! Keyboard-shortcut cheatsheet overlay.
//!
//! Ported from `src/render/cheatsheet.zig`.
//!
//! Draws a centered modal card listing every shortcut grouped by category.
//! Brand: Mineral palette — near-opaque theme.surface card, theme.border edges,
//! alloy group headers, accent (mineral teal) chords, foreground descriptions.
//!
//! Call `draw` from renderFrame *last* (on top of grid, HUD, tree, tab bar).

use anvil_theme::Theme;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

// --- Brand color constants (Mineral palette) --------------------------------

/// alloy: muted labels / group headers (#86919a)
const ALLOY: [u8; 3] = [0x86, 0x91, 0x9a];

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
        chord: "Cmd E",
        desc: "toggle file tree",
    },
    Row::Shortcut {
        chord: "Cmd /",
        desc: "this cheatsheet",
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
        chord: "Cmd C",
        desc: "copy",
    },
    Row::Shortcut {
        chord: "Cmd-click",
        desc: "open path or URL",
    },
];

/// Card width in terminal columns. Wide enough for the longest row.
pub const CARD_COLS: usize = 42;

/// Card height in terminal rows (title + hint + rule + all rows + 1 padding).
pub const CARD_ROWS: usize = ROWS.len() + 4;

// --- Draw -------------------------------------------------------------------

/// Draw the cheatsheet as a centered modal card. Must be called last in
/// renderFrame so it renders on top of all other UI elements.
pub fn draw(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    total_cols: usize,
    total_rows: usize,
) {
    if total_rows < CARD_ROWS + 2 || total_cols < CARD_COLS + 2 {
        return;
    }

    // Center the card.
    let card_col = (total_cols - CARD_COLS) / 2;
    let card_row = (total_rows - CARD_ROWS) / 2;

    let cw = metrics.cell_w;
    let ch = metrics.cell_h;
    let left_px = raster.pad_x + card_col as f64 * cw;
    let top_px = raster.pad_y + card_row as f64 * ch;
    let card_w_px = CARD_COLS as f64 * cw;
    let card_h_px = CARD_ROWS as f64 * ch;

    // Fully opaque surface panel — occludes the terminal beneath it.
    raster.fill_pixel_rect(left_px, top_px, card_w_px, card_h_px, theme.surface);

    // 2px border on all four edges.
    let b = 2.0_f64;
    raster.fill_pixel_rect(left_px, top_px, card_w_px, b, theme.border);
    raster.fill_pixel_rect(left_px, top_px + card_h_px - b, card_w_px, b, theme.border);
    raster.fill_pixel_rect(left_px, top_px, b, card_h_px, theme.border);
    raster.fill_pixel_rect(left_px + card_w_px - b, top_px, b, card_h_px, theme.border);

    // Content rows inside the card.
    // 3-col inner left margin; 2-col right margin.
    let max_col = card_col + CARD_COLS - 2;

    // Row 0: title in accent color.
    draw_text(
        raster,
        painter,
        metrics,
        card_col + 3,
        card_row,
        "Keyboard Shortcuts",
        theme.accent,
        max_col,
    );

    // Row 1: dim hint.
    draw_text(
        raster,
        painter,
        metrics,
        card_col + 3,
        card_row + 1,
        "Cmd / or Esc to close",
        ALLOY,
        max_col,
    );

    // Row 2: full-width 1px border rule below the hint text.
    {
        let rule_px_x = left_px + 2.0 * cw;
        let rule_px_y = raster.pad_y + (card_row + 2) as f64 * ch;
        let rule_w = (CARD_COLS as f64 - 4.0) * cw;
        raster.fill_pixel_rect(rule_px_x, rule_px_y, rule_w, 1.0, theme.border);
    }

    // Rows 3+: content.
    let mut r = card_row + 3;
    let mut first_header = true;
    for row in ROWS {
        match row {
            Row::Header(label) => {
                // Draw a 1px border rule before each header, skipping the first.
                if !first_header {
                    let rule_px_x = left_px + 2.0 * cw;
                    let rule_px_y = raster.pad_y + r as f64 * ch;
                    let rule_w = (CARD_COLS as f64 - 4.0) * cw;
                    raster.fill_pixel_rect(rule_px_x, rule_px_y, rule_w, 1.0, theme.border);
                }
                first_header = false;
                // Section headers recede below content: alloy-muted, not foreground.
                draw_text(
                    raster,
                    painter,
                    metrics,
                    card_col + 3,
                    r,
                    label,
                    ALLOY,
                    max_col,
                );
                r += 1;
            }
            Row::Shortcut { chord, desc } => {
                // Chord: left-aligned in accent color at col+3.
                draw_text(
                    raster,
                    painter,
                    metrics,
                    card_col + 3,
                    r,
                    chord,
                    theme.accent,
                    max_col,
                );
                // Description: starts at a fixed column in foreground, clear
                // of the widest chord ("Ctrl Shift Tab"). Shifted right by 1
                // to match the new inner padding.
                let desc_col = card_col + 18;
                if desc_col < max_col {
                    draw_text(
                        raster,
                        painter,
                        metrics,
                        desc_col,
                        r,
                        desc,
                        theme.foreground,
                        max_col,
                    );
                }
                r += 1;
            }
        }
    }
}

// --- Internal helpers -------------------------------------------------------

/// Draw a UTF-8 string from cell `col`, one codepoint per cell, stopping at
/// `max_col`.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    col: usize,
    row: usize,
    text: &str,
    color: [u8; 3],
    max_col: usize,
) {
    for (i, cp) in text.chars().enumerate() {
        let cx = col + i;
        if cx >= max_col {
            break;
        }
        raster.cell_glyph(painter, metrics, cx, row, cp as u32, color);
    }
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::PixelRect;

    // Stub painter that records calls.
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

    /// Port of "cheatsheet rows list is non-empty"
    #[test]
    fn rows_list_is_non_empty() {
        assert!(!ROWS.is_empty());
    }

    /// Port of "cheatsheet rows contain at least one header"
    #[test]
    fn rows_contain_at_least_one_header() {
        let found = ROWS.iter().any(|r| matches!(r, Row::Header(_)));
        assert!(found);
    }

    /// Port of "cheatsheet rows contain at least one shortcut"
    #[test]
    fn rows_contain_at_least_one_shortcut() {
        let found = ROWS.iter().any(|r| matches!(r, Row::Shortcut { .. }));
        assert!(found);
    }

    /// Port of "all shortcut chords and descs are non-empty"
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

    /// Port of "cheatsheet rows are valid UTF-8"
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

    /// Port of "card_rows covers all content rows"
    #[test]
    fn card_rows_covers_all_content_rows() {
        assert_eq!(ROWS.len() + 4, CARD_ROWS);
    }

    /// Smoke test: draw does not panic on a reasonably-sized raster.
    #[test]
    fn draw_no_panic() {
        let m = metrics();
        let mut r = Raster::new(800, 600);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        // 800/10 = 80 cols, 600/20 = 30 rows — large enough for CARD_COLS+2 / CARD_ROWS+2.
        draw(&mut r, &mut painter, m, &theme, 80, 30);
        // Title "Keyboard Shortcuts" should produce glyph calls.
        assert!(!painter.calls.is_empty());
    }

    /// draw returns early when the raster is too small.
    #[test]
    fn draw_noop_when_too_small() {
        let m = metrics();
        let mut r = Raster::new(100, 80);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        draw(&mut r, &mut painter, m, &theme, 5, 5);
        assert!(painter.calls.is_empty());
    }
}
