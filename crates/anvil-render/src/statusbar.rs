//! The always-visible status bar — one text row at the bottom of the window.
//!
//! Draws into the BGRA raster like the tab bar and search bar.

use anvil_agent::{Connection, Snapshot};
use anvil_theme::Theme;

use crate::agent_panel::{LocalContext, RunState, format_cwd};
use crate::raster::{FontMetrics, GlyphPainter, Raster};

/// The status bar is always one cell row tall.
pub const STATUS_BAR_ROWS: usize = 1;

// --- Brand color constants (Mineral palette) --------------------------------

/// alloy: muted labels / metadata (#86919a) — the default tone for the bar
const ALLOY: [u8; 3] = [0x86, 0x91, 0x9a];
/// status.verified: success / clean state — green (#3f8a5b)
const VERIFIED: [u8; 3] = [0x3f, 0x8a, 0x5b];
/// status.failure: failure / error state — red (#b13a30)
const FAILURE: [u8; 3] = [0xb1, 0x3a, 0x30];
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
#[allow(clippy::too_many_arguments)]
pub fn draw_status_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    local_ctx: &LocalContext,
    agent_snap: &Snapshot,
    clock: &str,
    bottom_row: usize,
) {
    let cell_w = metrics.cell_w;
    let usable_w = raster.width as f64 - 2.0 * raster.pad_x;
    let total_cols = ((usable_w.max(0.0)) / cell_w) as usize;
    if total_cols == 0 {
        return;
    }
    let _ = theme;

    // Status bar is text-only on the canvas — no chrome strip. Just clear
    // the row's cells to theme.background so older chars don't bleed
    // through under shorter content. No bottom-strip fill; the bar reads
    // as a quiet caption, not a heavy ledge.
    for col in 0..total_cols {
        raster.cell_bg(metrics, col, bottom_row, theme.background);
    }

    // ── Left: cwd  ✓/✗ last 0.1s ────────────────────────────────────────────
    let mut col = 2usize;
    if !local_ctx.cwd.is_empty() {
        let cwd = format_cwd(&local_ctx.cwd);
        for ch in cwd.chars() {
            if col + 2 >= total_cols {
                break;
            }
            raster.cell_glyph(painter, metrics, col, bottom_row, ch as u32, ALLOY);
            col += 1;
        }
        col += 2; // gap

        // Exit symbol + " last <dur>" when we have a last-run result.
        let (sym, color) = match local_ctx.run {
            RunState::Ok => ("\u{2713}", VERIFIED), // ✓
            RunState::Failed => ("\u{2717}", FAILURE), // ✗
            _ => ("", ALLOY),
        };
        if !sym.is_empty() {
            for ch in sym.chars() {
                if col + 2 >= total_cols {
                    break;
                }
                raster.cell_glyph(painter, metrics, col, bottom_row, ch as u32, color);
                col += 1;
            }
            if local_ctx.run_duration_ms > 0 {
                let dur = format!(" last {}", format_duration_ms(local_ctx.run_duration_ms));
                for ch in dur.chars() {
                    if col + 2 >= total_cols {
                        break;
                    }
                    raster.cell_glyph(painter, metrics, col, bottom_row, ch as u32, ALLOY);
                    col += 1;
                }
            }
        }
    }

    // ── Right (anchored from the right edge): agent · clock ──────────────────
    // Build the right segment then position it.
    let agent_text = if agent_snap.connection == Connection::Live {
        if agent_snap.running_count > 0 {
            format!("\u{25cf} {} running", agent_snap.running_count)
        } else {
            "\u{25cf} idle".to_string()
        }
    } else {
        String::new()
    };
    let sep = if !agent_text.is_empty() && !clock.is_empty() {
        "   "
    } else {
        ""
    };
    let right_len = agent_text.chars().count() + sep.len() + clock.chars().count();
    if right_len + 2 < total_cols && col + right_len + 2 < total_cols {
        let mut c = total_cols - 2 - right_len;
        for (i, ch) in agent_text.chars().enumerate() {
            let fg = if i == 0 { AGENT_VIOLET } else { ALLOY };
            raster.cell_glyph(painter, metrics, c, bottom_row, ch as u32, fg);
            c += 1;
        }
        for ch in sep.chars() {
            raster.cell_glyph(painter, metrics, c, bottom_row, ch as u32, ALLOY);
            c += 1;
        }
        for ch in clock.chars() {
            raster.cell_glyph(painter, metrics, c, bottom_row, ch as u32, ALLOY);
            c += 1;
        }
    }
}

/// Format a non-zero ms duration as "0.1s" / "12.3s" — drops to ".Xs" for
/// sub-second so the bar stays narrow.
fn format_duration_ms(ms: i64) -> String {
    let secs = ms as f64 / 1000.0;
    format!("{:.1}s", secs)
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
            "",
            row,
        );

        // Status bar paints its row to theme.background — text-only caption.
        let cell_h = m.cell_h as usize;
        let px_y = row * cell_h + cell_h / 2;
        let px = pixel_at(&r, 4, px_y);
        assert_eq!(
            px, theme.background,
            "expected status bar row painted to theme.background, got {px:?}"
        );
    }

    // --- branch_shown_when_git_ok --------------------------------------------

    /// Branch name chars are drawn in ALLOY (quiet) when git state is Ok.
    #[test]
    fn cwd_shown_when_set() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext {
            cwd: "/Users/test/work/caldera/anvil".to_string(),
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
            "",
            row,
        );

        // ALLOY chars should include cwd content ('a' from "anvil" or similar).
        let alloy_chars: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == ALLOY)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            !alloy_chars.is_empty(),
            "expected cwd chars in ALLOY, got no calls"
        );
    }

    /// Last-run exit ✓ rendered in VERIFIED green on Ok.
    #[test]
    fn exit_check_in_verified_green_on_ok_run() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext {
            cwd: "/tmp".to_string(),
            run: RunState::Ok,
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
            "",
            row,
        );

        let verified_chars: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == VERIFIED)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            verified_chars.contains(&'\u{2713}'),
            "expected ✓ in VERIFIED green, got {verified_chars:?}"
        );
    }

    /// Last-run exit ✗ rendered in FAILURE red on Failed.
    #[test]
    fn exit_cross_in_failure_red_on_failed_run() {
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext {
            cwd: "/tmp".to_string(),
            run: RunState::Failed,
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
            "",
            row,
        );

        let failure_chars: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == FAILURE)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            failure_chars.contains(&'\u{2717}'),
            "expected ✗ in FAILURE red, got {failure_chars:?}"
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
            "",
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
            "",
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
