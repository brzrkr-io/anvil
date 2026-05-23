//! The always-visible status bar — one text row at the bottom of the window.
//!
//! Draws into the BGRA raster like the tab bar and search bar.

use anvil_agent::{Connection, Snapshot};
use anvil_theme::Theme;

use crate::agent_panel::{GitState, LocalContext};
use crate::raster::{FontMetrics, GlyphPainter, Raster};

/// The status bar is always one cell row tall.
pub const STATUS_BAR_ROWS: usize = 1;

// --- Brand color constants (Mineral palette) --------------------------------

/// alloy: muted labels / metadata (#86919a)
const ALLOY: [u8; 3] = [0x86, 0x91, 0x9a];
/// status.attention: reviewable warning / pending action (#b07a14)
const ATTENTION: [u8; 3] = [0xb0, 0x7a, 0x14];
/// status.agent: agent / automation / model activity — violet (#6a5fa3)
const AGENT_VIOLET: [u8; 3] = [0x6a, 0x5f, 0xa3];
/// status.info: info teal (#2f7f86)
const INFO_TEAL: [u8; 3] = [0x2f, 0x7f, 0x86];

/// Draw the status bar across `bottom_row` (a cell-row index into the raster).
///
/// Left section (2-col inner pad):
///   - Branch glyph + name in INFO_TEAL, if git is Ok.
///   - Modified count in ATTENTION amber, if dirty > 0.
///   - Added count in VERIFIED green, if git_added (here: no separate field, so
///     we only show dirty count via the `git_dirty` field).
///   - "clean" in ALLOY when on a branch with no dirty files.
///   - `·` separator in ALLOY between sections.
///
/// Right section (2-col inner right pad):
///   - `◆ N running` when agent connection is Live and running_count > 0.
///   - Nothing otherwise (honesty rule).
pub fn draw_status_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    local_ctx: &LocalContext,
    agent_snap: &Snapshot,
    bottom_row: usize,
) {
    let cell_w = metrics.cell_w;
    let usable_w = raster.width as f64 - 2.0 * raster.pad_x;
    let total_cols = ((usable_w.max(0.0)) / cell_w) as usize;
    if total_cols == 0 {
        return;
    }

    // Background: surface tone across the whole row.
    for c in 0..total_cols {
        raster.cell_bg(metrics, c, bottom_row, theme.surface);
    }

    // 1px border rule above the bar.
    let bar_top_px = raster.pad_y + bottom_row as f64 * metrics.cell_h;
    let bar_left_px = raster.pad_x;
    let bar_w_px = total_cols as f64 * cell_w;
    raster.fill_pixel_rect(bar_left_px, bar_top_px, bar_w_px, 1.0, theme.border);

    // --- Build left segments ---

    // Each segment is a (text, color) pair. We write them left-to-right with a
    // ALLOY `·` separator between non-empty segments.
    let mut segments: Vec<(&str, [u8; 3], String)> = Vec::new();

    let branch_label;
    let dirty_label;
    let clean_label = "clean".to_string();

    if local_ctx.git != GitState::NoRepo && !local_ctx.branch.is_empty() {
        branch_label = format!(" {}", local_ctx.branch);
        segments.push(("", INFO_TEAL, branch_label));
    }

    if local_ctx.git_dirty > 0 {
        dirty_label = format!("{} modified", local_ctx.git_dirty);
        segments.push(("", ATTENTION, dirty_label));
    } else if local_ctx.git != GitState::NoRepo && !local_ctx.branch.is_empty() {
        segments.push(("", ALLOY, clean_label));
    }

    // Write left segments with `·` separator.
    let mut col = 2usize; // 2-col inner left pad
    for (idx, (_prefix, color, text)) in segments.iter().enumerate() {
        if idx > 0 {
            // separator
            if col + 3 < total_cols {
                raster.cell_glyph(painter, metrics, col, bottom_row, '·' as u32, ALLOY);
                col += 2; // separator + 1 space
            }
        }
        for ch in text.chars() {
            if col + 1 >= total_cols.saturating_sub(2) {
                break;
            }
            raster.cell_glyph(painter, metrics, col, bottom_row, ch as u32, *color);
            col += 1;
        }
    }

    // --- Right section ---

    if agent_snap.connection == Connection::Live && agent_snap.running_count > 0 {
        let count = agent_snap.running_count;
        let label = format!("{} running", count);
        // Diamond glyph + space + label text. We draw right-to-left from the
        // right edge (with 2-col inner right pad), then flip.
        // Simpler: compute start col from the right.
        let diamond = '◆';
        // Total chars: 1 (diamond) + 1 (space) + label.len()
        let right_text_len = 1 + 1 + label.chars().count();
        if right_text_len + 2 < total_cols && col + right_text_len + 2 < total_cols {
            let start = total_cols - 2 - right_text_len;
            raster.cell_glyph(
                painter,
                metrics,
                start,
                bottom_row,
                diamond as u32,
                AGENT_VIOLET,
            );
            let mut c = start + 1;
            // space
            raster.cell_glyph(
                painter,
                metrics,
                c,
                bottom_row,
                ' ' as u32,
                theme.foreground,
            );
            c += 1;
            for ch in label.chars() {
                raster.cell_glyph(painter, metrics, c, bottom_row, ch as u32, theme.foreground);
                c += 1;
            }
        }
    }
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{PixelRect, pixel_at};
    use anvil_agent::{Connection, Snapshot};

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

    fn bottom_row(raster: &Raster, metrics: FontMetrics) -> usize {
        let total_rows = (raster.height as f64 / metrics.cell_h) as usize;
        total_rows.saturating_sub(1)
    }

    // --- draw_status_bar_smoke -----------------------------------------------

    /// Smoke test: no panic, and the row is painted in theme.surface.
    #[test]
    fn draw_status_bar_smoke() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default();
        let agent_snap = Snapshot::default();
        let theme = anvil_theme::MINERAL_DARK;

        let row = bottom_row(&r, m);
        draw_status_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &local_ctx,
            &agent_snap,
            row,
        );

        // A pixel at the center of the bottom row should carry theme.surface.
        let cell_h = m.cell_h as usize;
        let px_y = row * cell_h + cell_h / 2;
        let px = pixel_at(&r, 4, px_y);
        assert_eq!(
            px, theme.surface,
            "expected surface at bottom row, got {px:?}"
        );
    }

    // --- branch_shown_when_git_ok --------------------------------------------

    /// Branch name chars are drawn in INFO_TEAL when git state is Ok.
    #[test]
    fn branch_shown_when_git_ok() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext {
            git: GitState::Ok,
            branch: "main".to_string(),
            git_dirty: 0,
            ..LocalContext::default()
        };
        let agent_snap = Snapshot::default();
        let theme = anvil_theme::MINERAL_DARK;

        let row = bottom_row(&r, m);
        draw_status_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &local_ctx,
            &agent_snap,
            row,
        );

        // At least one glyph call in INFO_TEAL for a branch char.
        let teal_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == INFO_TEAL)
            .collect();
        assert!(
            !teal_calls.is_empty(),
            "expected branch chars in INFO_TEAL, got calls: {:?}",
            painter.calls
        );
        // The branch name 'main' chars should be present.
        let teal_chars: Vec<char> = teal_calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            teal_chars.contains(&'m'),
            "expected 'm' from 'main' in INFO_TEAL, got {teal_chars:?}"
        );
    }

    // --- dirty_count_shown_in_attention_color --------------------------------

    /// "3 modified" is drawn in ATTENTION amber when git_dirty == 3.
    #[test]
    fn dirty_count_shown_in_attention_color() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext {
            git: GitState::Dirty,
            branch: "main".to_string(),
            git_dirty: 3,
            ..LocalContext::default()
        };
        let agent_snap = Snapshot::default();
        let theme = anvil_theme::MINERAL_DARK;

        let row = bottom_row(&r, m);
        draw_status_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &local_ctx,
            &agent_snap,
            row,
        );

        let attention_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == ATTENTION)
            .collect();
        assert!(
            !attention_calls.is_empty(),
            "expected chars in ATTENTION color, got calls: {:?}",
            painter.calls
        );
        // The digit '3' should be among them.
        let attention_chars: Vec<char> = attention_calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            attention_chars.contains(&'3'),
            "expected '3' in ATTENTION color, got {attention_chars:?}"
        );
    }

    // --- right_side_blank_when_no_runs ---------------------------------------

    /// No glyph calls in the right half when agent_snap is default (NotInstalled).
    #[test]
    fn right_side_blank_when_no_runs() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default(); // NoRepo, no dirty
        let agent_snap = Snapshot::default(); // NotInstalled
        let theme = anvil_theme::MINERAL_DARK;

        let row = bottom_row(&r, m);
        draw_status_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &local_ctx,
            &agent_snap,
            row,
        );

        // With NoRepo and no agent runs, expect zero glyph calls.
        // (The surface fill + border use cell_bg / fill_pixel_rect, not cell_glyph.)
        assert!(
            painter.calls.is_empty(),
            "expected no glyph calls for default state, got {:?}",
            painter.calls
        );
    }

    // --- running_count_shown_when_live ---------------------------------------

    /// "running" chars are recorded when connection == Live and running_count > 0.
    #[test]
    fn running_count_shown_when_live() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default();
        let agent_snap = Snapshot {
            connection: Connection::Live,
            running_count: 2,
            ..Snapshot::default()
        };
        let theme = anvil_theme::MINERAL_DARK;

        let row = bottom_row(&r, m);
        draw_status_bar(
            &mut r,
            &mut painter,
            m,
            &theme,
            &local_ctx,
            &agent_snap,
            row,
        );

        // "running" chars should appear in theme.foreground.
        let fg_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.foreground)
            .collect();
        let fg_chars: Vec<char> = fg_calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            fg_chars.contains(&'r'),
            "expected 'r' from 'running' in foreground color, got {fg_chars:?}"
        );
        // Diamond glyph should appear in AGENT_VIOLET.
        let violet_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == AGENT_VIOLET)
            .collect();
        assert!(
            !violet_calls.is_empty(),
            "expected diamond glyph in AGENT_VIOLET, got {:?}",
            painter.calls
        );
    }
}
