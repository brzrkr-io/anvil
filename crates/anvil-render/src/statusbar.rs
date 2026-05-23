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

/// alloy: muted labels / metadata (#86919a) — the default tone for the bar
const ALLOY: [u8; 3] = [0x86, 0x91, 0x9a];
/// status.attention: reviewable warning / pending action (#b07a14)
const ATTENTION: [u8; 3] = [0xb0, 0x7a, 0x14];
/// status.agent: agent / automation / model activity — violet (#6a5fa3)
const AGENT_VIOLET: [u8; 3] = [0x6a, 0x5f, 0xa3];

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

    // Transparent bar: no surface fill, no top border. Status reads as a
    // quiet caption floating on the terminal background. Colours pull
    // attention only when they need to.
    let _ = theme;

    // --- Left: git branch + dirty count, condensed ---------------------------
    //
    // Layout: ` main` in ALLOY, then ` *3` in ATTENTION when dirty.
    // No "modified" / "clean" words — the symbol carries the meaning and the
    // bar stays narrow.
    let mut col = 2usize; // 2-col inner left pad

    if local_ctx.git != GitState::NoRepo && !local_ctx.branch.is_empty() {
        let branch_label = format!("\u{e0a0} {}", local_ctx.branch); // U+E0A0 branch nerd-font glyph
        for ch in branch_label.chars() {
            if col + 2 >= total_cols {
                break;
            }
            raster.cell_glyph(painter, metrics, col, bottom_row, ch as u32, ALLOY);
            col += 1;
        }

        if local_ctx.git_dirty > 0 {
            // " *N" in attention amber
            let dirty = format!(" *{}", local_ctx.git_dirty);
            for ch in dirty.chars() {
                if col + 2 >= total_cols {
                    break;
                }
                raster.cell_glyph(painter, metrics, col, bottom_row, ch as u32, ATTENTION);
                col += 1;
            }
        }
    }

    // --- Right: agent runs ---------------------------------------------------

    if agent_snap.connection == Connection::Live && agent_snap.running_count > 0 {
        let label = format!("\u{25c6} {} running", agent_snap.running_count); // ◆
        let right_text_len = label.chars().count();
        if right_text_len + 2 < total_cols && col + right_text_len + 2 < total_cols {
            let start = total_cols - 2 - right_text_len;
            for (i, (c, ch)) in (start..).zip(label.chars()).enumerate() {
                // First char is the diamond — colour it AGENT_VIOLET; the rest
                // are quiet ALLOY so the right side reads as one unit.
                let fg = if i == 0 { AGENT_VIOLET } else { ALLOY };
                raster.cell_glyph(painter, metrics, c, bottom_row, ch as u32, fg);
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

    /// Smoke test: no panic, and the bar leaves the background untouched
    /// (transparent — no surface fill, no border).
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

        // With no git + no agent runs, the bar emits zero pixels.
        let cell_h = m.cell_h as usize;
        let px_y = row * cell_h + cell_h / 2;
        let px = pixel_at(&r, 4, px_y);
        assert_eq!(px, [0, 0, 0], "expected background untouched, got {px:?}");
    }

    // --- branch_shown_when_git_ok --------------------------------------------

    /// Branch name chars are drawn in ALLOY (quiet) when git state is Ok.
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

        let alloy_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == ALLOY)
            .collect();
        let alloy_chars: Vec<char> = alloy_calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            alloy_chars.contains(&'m'),
            "expected 'm' from 'main' in ALLOY, got {alloy_chars:?}"
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
        assert!(
            painter.calls.is_empty(),
            "expected no glyph calls for default state, got {:?}",
            painter.calls
        );
    }

    // --- running_count_shown_when_live ---------------------------------------

    /// "running" chars are recorded in ALLOY when connection == Live; the
    /// leading diamond is in AGENT_VIOLET.
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

        let alloy_chars: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == ALLOY)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            alloy_chars.contains(&'r'),
            "expected 'r' from 'running' in ALLOY, got {alloy_chars:?}"
        );
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
