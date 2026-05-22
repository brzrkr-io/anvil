//! File-tree panel renderer.
//!
//! Ported from `src/render/filetree.zig`.
//!
//! Draws the left-edge panel in absolute pixel space (raster.origin_x and
//! raster.origin_y must be 0 when this is called).
//!
//! Brand: Mineral palette — theme.surface panel bg, theme.border separator,
//! alloy-grey file names, foreground for dir names, info-teal icons.

use anvil_theme::{Theme, mix};
use anvil_workspace::filetree::FileTree;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

/// Number of terminal columns the tree panel occupies.
pub const TREE_COLS: usize = 30;

// Brand color constants (Mineral palette).
/// alloy: muted text / file names (#86919a)
const ALLOY: [u8; 3] = [0x86, 0x91, 0x9a];
/// info/trace teal for dir icons (#2f7f86)
const INFO_TEAL: [u8; 3] = [0x2f, 0x7f, 0x86];

// Nerd Font icon codepoints.
const ICON_FOLDER_CLOSED: u32 = 0xf07b;
const ICON_FOLDER_OPEN: u32 = 0xf07c;
const ICON_FILE: u32 = 0xf15b;

/// Draw the file-tree panel into the left `TREE_COLS` columns of the raster.
/// `total_rows` is the full visible row count (including top bar).
/// `top_offset` is the number of rows taken by the tab bar (0 or 1).
/// `raster.origin_x` and `raster.origin_y` must be 0 when this is called.
pub fn draw(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    tree: &FileTree,
    total_rows: usize,
    top_offset: usize,
) {
    if total_rows == 0 {
        return;
    }

    let cw = metrics.cell_w;
    let ch = metrics.cell_h;

    // Panel pixel extents (raster device-pixel space, y=0 at top).
    let pad_x = raster.pad_x;
    let pad_y = raster.pad_y;
    let panel_w_px = TREE_COLS as f64 * cw;
    let panel_top_px = pad_y + top_offset as f64 * ch;
    let panel_h_px = total_rows.saturating_sub(top_offset) as f64 * ch;

    // Panel background: solid surface tone.
    raster.fill_pixel_rect(pad_x, panel_top_px, panel_w_px, panel_h_px, theme.surface);

    // 1px right-edge border.
    raster.fill_pixel_rect(
        pad_x + panel_w_px - 1.0,
        panel_top_px,
        1.0,
        panel_h_px,
        theme.border,
    );

    // Header row: "FILES" label in info_teal, 2-col left pad; border below.
    let header_raster_row = top_offset;
    if total_rows > top_offset {
        draw_text(
            raster,
            painter,
            metrics,
            2,
            header_raster_row,
            "FILES",
            INFO_TEAL,
            TREE_COLS - 1,
        );
        let header_rule_y = pad_y + (header_raster_row + 1) as f64 * ch;
        raster.fill_pixel_rect(pad_x, header_rule_y, panel_w_px - 1.0, 1.0, theme.border);
    }

    // Draw entries (start one row below the header).
    let content_rows = total_rows.saturating_sub(top_offset);
    for (row_idx, (entry_idx, e)) in (1_usize..).zip(tree.entries.iter().enumerate()) {
        if row_idx >= content_rows {
            break;
        }
        let raster_row = top_offset + row_idx;

        // Selected row: fill full panel width with a subtle accent tint.
        if let Some(sel) = tree.selected_idx {
            if entry_idx == sel {
                let row_top_px = pad_y + raster_row as f64 * ch;
                let tinted = mix(theme.background, theme.accent, 0.18);
                raster.fill_pixel_rect(pad_x, row_top_px, panel_w_px, ch, tinted);
            }
        }

        // Indent: 1 col inner left padding, then depth, then icon.
        let indent_cols = 1 + e.depth as usize * 2;

        // Icon codepoint and color.
        let (icon_cp, icon_color) = if e.is_dir {
            let cp = if e.expanded {
                ICON_FOLDER_OPEN
            } else {
                ICON_FOLDER_CLOSED
            };
            (cp, INFO_TEAL)
        } else {
            (ICON_FILE, ALLOY)
        };

        // Draw icon if it fits within the panel.
        let icon_col = indent_cols;
        if icon_col < TREE_COLS {
            raster.cell_glyph(painter, metrics, icon_col, raster_row, icon_cp, icon_color);
        }

        // Draw name starting one col after the icon.
        let name_start_col = indent_cols + 2;
        let name_max_col = TREE_COLS - 1; // leave 1-col right margin
        if name_start_col < name_max_col {
            let name_color: [u8; 3] = if e.is_dir { theme.foreground } else { ALLOY };
            draw_text(
                raster,
                painter,
                metrics,
                name_start_col,
                raster_row,
                &e.name,
                name_color,
                name_max_col,
            );
        }
    }
}

/// Map a click's y-coordinate (measured from the top of the view, in points)
/// to a zero-based entry index into the file-tree list.
///
/// Returns `None` when the click lands on the header row or above/below the
/// visible tree content. Returns the entry index otherwise — the caller is
/// responsible for bounds-checking against the actual entry count.
///
/// Parameters:
///   `click_y_from_top` — y in points measured from the top of the view.
///   `tree_top`         — y in points where the tree content area begins.
///   `cell_h`           — row height in points.
///   `header_rows`      — number of non-entry header rows at the top of the
///                         panel (always 1: the "FILES" label row).
pub fn tree_row_at_click(
    click_y_from_top: f64,
    tree_top: f64,
    cell_h: f64,
    header_rows: usize,
) -> Option<usize> {
    if cell_h <= 0.0 {
        return None;
    }
    // Must be past the header rows.
    let header_h = header_rows as f64 * cell_h;
    if click_y_from_top < tree_top + header_h {
        return None;
    }
    let raw_row = ((click_y_from_top - tree_top) / cell_h) as usize;
    if raw_row < header_rows {
        return None;
    }
    Some(raw_row - header_rows)
}

/// Draw a UTF-8 string from cell `col`, one codepoint per cell, stopping at `max_col`.
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

    /// Port of "treeRowAtClick maps click-y to entry index"
    #[test]
    fn tree_row_at_click_maps_y_to_entry_index() {
        let tree_top = 30.0_f64; // e.g. 1 tab row * 20px + 10px pad
        let cell_h = 20.0_f64;
        let header_rows = 1_usize;

        // Click inside the header row → null.
        assert_eq!(
            tree_row_at_click(tree_top + 5.0, tree_top, cell_h, header_rows),
            None
        );
        // Click exactly at the header top boundary → null.
        assert_eq!(
            tree_row_at_click(tree_top, tree_top, cell_h, header_rows),
            None
        );
        // Click in the first entry row (just past header) → index 0.
        assert_eq!(
            tree_row_at_click(tree_top + cell_h + 1.0, tree_top, cell_h, header_rows),
            Some(0)
        );
        // Click near the bottom of the first entry row → index 0.
        assert_eq!(
            tree_row_at_click(tree_top + cell_h * 2.0 - 1.0, tree_top, cell_h, header_rows),
            Some(0)
        );
        // Click in the second entry row → index 1.
        assert_eq!(
            tree_row_at_click(tree_top + cell_h * 2.0 + 1.0, tree_top, cell_h, header_rows),
            Some(1)
        );
        // Click above the tree_top → null.
        assert_eq!(
            tree_row_at_click(tree_top - 1.0, tree_top, cell_h, header_rows),
            None
        );
        // Zero cell_h → null (guard against division by zero).
        assert_eq!(tree_row_at_click(100.0, tree_top, 0.0, header_rows), None);
    }

    /// draw: no panic on empty FileTree.
    #[test]
    fn draw_empty_tree_no_panic() {
        let m = metrics();
        let mut r = Raster::new(400, 400);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);
        let tree = FileTree::default();
        let theme = anvil_theme::MINERAL_DARK;
        draw(&mut r, &mut painter, m, &theme, &tree, 20, 0);
        // Should have drawn "FILES" header glyphs.
        assert!(
            !painter.calls.is_empty(),
            "expected glyph calls for FILES header"
        );
    }

    /// draw: no panic when total_rows == 0.
    #[test]
    fn draw_zero_rows_is_noop() {
        let m = metrics();
        let mut r = Raster::new(400, 400);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);
        let tree = FileTree::default();
        let theme = anvil_theme::MINERAL_DARK;
        draw(&mut r, &mut painter, m, &theme, &tree, 0, 0);
        // No glyph calls (early return).
        assert!(painter.calls.is_empty());
    }
}
