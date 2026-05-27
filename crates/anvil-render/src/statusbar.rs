//! The always-visible status bar — a fixed-height pixel strip at the bottom of
//! the window. Draws into the BGRA raster like the tab bar and search bar.

use anvil_agent::{Connection, Snapshot};
use anvil_theme::Theme;

use crate::agent_panel::{LocalContext, RunState, format_cwd};
use crate::raster::{FontMetrics, GlyphPainter, Raster, UiTextPainter, UiWeight};
use crate::ui_text_sizes::STATUS_PT;

/// Current editor mode, shown as the leftmost chip in the status bar (O3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StatusMode {
    /// Normal editing state (default).
    #[default]
    Editing,
    /// Search bar is open.
    Searching,
    /// LSP rename overlay is open.
    Renaming,
    /// Any picker overlay is open (Cmd+P / Cmd+T / Cmd+R / Cmd+Shift+O).
    Picking,
}

/// Linear per-channel lerp between two RGB colors: `a*(1-t) + b*t`, clamped to 0..255.
fn mix_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    [
        (a[0] as f32 * (1.0 - t) + b[0] as f32 * t).clamp(0.0, 255.0) as u8,
        (a[1] as f32 * (1.0 - t) + b[1] as f32 * t).clamp(0.0, 255.0) as u8,
        (a[2] as f32 * (1.0 - t) + b[2] as f32 * t).clamp(0.0, 255.0) as u8,
    ]
}

/// Draw the status bar at a FIXED pixel strip — `chrome_bottom_px` tall —
/// anchored to the window's bottom edge. Glyphs are pixel-positioned and
/// vertically centred in the strip; nothing here uses cell-row indices.
///
/// Left section (2-col inner pad):
///   - Branch glyph + name in INFO_TEAL, if git is Ok.
///   - Modified count in ATTENTION amber, if dirty > 0.
///   - Added count in VERIFIED green, if git_added (here: no separate field, so
///     we only show dirty count via the `git_dirty` field).
///   - "clean" in TEXT_MUTED when on a branch with no dirty files.
///   - `·` separator in TEXT_MUTED between sections.
///
/// Right section (2-col inner right pad):
///   - `◆ N running` when agent connection is Live and running_count > 0.
///   - Nothing otherwise (honesty rule).
#[allow(clippy::too_many_arguments)]
pub fn draw_status_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    ui_painter: &mut dyn UiTextPainter,
    metrics: FontMetrics,
    theme: &Theme,
    local_ctx: &LocalContext,
    agent_snap: &Snapshot,
    clock: &str,
    chrome_bottom_px: f64,
    window_scale: f64,
    pulse_phase: f32,
    mode: StatusMode,
) {
    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;
    let total_w = raster.width as f64;
    let total_h = raster.height as f64;
    if total_w <= 0.0 || chrome_bottom_px <= 0.0 {
        return;
    }

    let strip_top = total_h - chrome_bottom_px;
    // Charcoal fill across the bottom strip — reaches the window's bottom
    // edge with no canvas peeking through.
    raster.fill_pixel_rect(0.0, strip_top, total_w, chrome_bottom_px, theme.charcoal);
    // 1px hairline at the top of the strip.
    raster.fill_pixel_rect(0.0, strip_top, total_w, 1.0, theme.hairline);

    // Cell top and baseline for vertically centred content in the strip.
    // glyph_at expects the cell-top (icon_top); ui_line expects the baseline (glyph_y).
    let icon_top = strip_top + ((chrome_bottom_px - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    let glyph_y = icon_top + (cell_h - metrics.descent);
    let pad_x = 14.0 * window_scale; // D: .bottom-bar { padding: 0 14px }

    // ── Left: mode chip ───────────────────────────────────────────────────
    let mut x = pad_x;
    {
        let (label, color) = match mode {
            StatusMode::Editing => ("EDITING", theme.text_subtle),
            StatusMode::Searching => ("SEARCHING", theme.accent_primary),
            StatusMode::Renaming => ("RENAMING", theme.accent_bright),
            StatusMode::Picking => ("PICKING", theme.accent_primary),
        };
        // EDITING mode: subtle ember dot before the label as a brand signal.
        if matches!(mode, StatusMode::Editing) {
            let dot_size = 3.0;
            let dot_y = icon_top + (cell_h - dot_size) * 0.5;
            raster.fill_pixel_rect_alpha(x, dot_y, dot_size, dot_size, theme.accent_ember, 0.7);
            x += dot_size + 4.0;
        }
        raster.ui_line(
            ui_painter,
            label,
            x,
            glyph_y,
            STATUS_PT,
            UiWeight::Regular,
            color,
        );
        // P5: 8pt inter-segment gap (was 2 cell-widths).
        x +=
            raster.ui_measure(ui_painter, label, STATUS_PT, UiWeight::Regular) + 8.0 * window_scale;
    }

    // ── Left: cwd  ✓/✗ last 0.1s ─────────────────────────────────────────
    if !local_ctx.cwd.is_empty() {
        let cwd = format_cwd(&local_ctx.cwd);
        raster.ui_line(
            ui_painter,
            &cwd,
            x,
            glyph_y,
            STATUS_PT,
            UiWeight::Regular,
            theme.text_muted,
        );
        // P5: 8pt inter-segment gap (was 2 cell-widths).
        x += raster.ui_measure(ui_painter, &cwd, STATUS_PT, UiWeight::Regular) + 8.0 * window_scale;

        // ✓/✗ symbols stay on mono path (single special chars).
        let (sym_cp, sym_color) = match local_ctx.run {
            RunState::Ok => (Some('\u{2713}'), theme.verified),
            RunState::Failed => (Some('\u{2717}'), theme.failure),
            _ => (None, theme.text_muted),
        };
        if let Some(cp) = sym_cp {
            raster.glyph_at(painter, metrics, x, icon_top, cp as u32, sym_color);
            x += cell_w;
            if local_ctx.run_duration_ms > 0 {
                let dur = format!(" last {}", format_duration_ms(local_ctx.run_duration_ms));
                raster.ui_line(
                    ui_painter,
                    &dur,
                    x,
                    glyph_y,
                    STATUS_PT,
                    UiWeight::Regular,
                    theme.text_muted,
                );
                x += raster.ui_measure(ui_painter, &dur, STATUS_PT, UiWeight::Regular);
            }
        }
    }

    // ── Right: agent · clock ─────────────────────────────────────────────
    let agent_active = agent_snap.connection == Connection::Live;
    // Build text after the dot so we can measure it separately.
    let agent_tail = if agent_active && agent_snap.running_count > 0 {
        format!(" {} running", agent_snap.running_count)
    } else {
        " idle".to_string()
    };
    let sep = if !clock.is_empty() { "   " } else { "" };
    // Width: 1 cell for dot + tail + sep + clock (all via ui_measure for accuracy).
    let tail_w = raster.ui_measure(ui_painter, &agent_tail, STATUS_PT, UiWeight::Regular);
    let sep_w = raster.ui_measure(ui_painter, sep, STATUS_PT, UiWeight::Regular);
    let clock_w = raster.ui_measure(ui_painter, clock, STATUS_PT, UiWeight::Regular);
    let right_w = cell_w + tail_w + sep_w + clock_w; // dot is 1 cell
    let right_start = (total_w - pad_x - right_w).max(x);
    let mut rx = right_start;

    // Agent dot: single special glyph on mono path.
    let dot_color = if agent_active {
        mix_rgb(
            theme.charcoal,
            theme.agent,
            0.5 + 0.5 * (std::f32::consts::TAU * pulse_phase).sin(),
        )
    } else {
        theme.text_subtle
    };
    if rx + cell_w <= total_w {
        raster.glyph_at(painter, metrics, rx, icon_top, '\u{25cf}' as u32, dot_color);
        rx += cell_w;
    }
    // Agent tail text.
    if rx + tail_w <= total_w {
        raster.ui_line(
            ui_painter,
            &agent_tail,
            rx,
            glyph_y,
            STATUS_PT,
            UiWeight::Regular,
            theme.text_muted,
        );
        rx += tail_w;
    }
    if rx + sep_w <= total_w {
        raster.ui_line(
            ui_painter,
            sep,
            rx,
            glyph_y,
            STATUS_PT,
            UiWeight::Regular,
            theme.text_muted,
        );
        rx += sep_w;
    }
    if rx + clock_w <= total_w && !clock.is_empty() {
        raster.ui_line(
            ui_painter,
            clock,
            rx,
            glyph_y,
            STATUS_PT,
            UiWeight::Regular,
            theme.text_muted,
        );
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
    use crate::raster::{PixelRect, UiWeight, pixel_at};
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

    #[derive(Default)]
    struct StubUiPainter {
        pub draws: Vec<(String, [u8; 3])>,
    }

    impl UiTextPainter for StubUiPainter {
        fn measure(&mut self, text: &str, _size_pt: f64, _weight: UiWeight) -> f64 {
            text.chars().count() as f64 * 8.0
        }

        #[allow(clippy::too_many_arguments)]
        fn draw_line(
            &mut self,
            text: &str,
            _x_px: f64,
            _baseline_y_px: f64,
            _size_pt: f64,
            _weight: UiWeight,
            fg: [u8; 3],
            _pixels: &mut [u8],
            _bitmap_w: usize,
            _bitmap_h: usize,
        ) {
            if !text.is_empty() {
                self.draws.push((text.to_string(), fg));
            }
        }
    }

    fn metrics() -> FontMetrics {
        FontMetrics {
            cell_w: 10.0,
            cell_h: 20.0,
            descent: 4.0,
        }
    }

    fn theme() -> anvil_theme::Theme {
        anvil_theme::EMBER_DARK
    }

    // --- draw_status_bar_smoke -----------------------------------------------

    /// Smoke test: no panic, and the bar leaves the background untouched
    /// (transparent — no surface fill, no border).
    #[test]
    fn draw_status_bar_smoke() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let mut up = StubUiPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default();
        let agent_snap = Snapshot::default();

        let chrome_bottom_px = m.cell_h * 2.0;
        draw_status_bar(
            &mut r,
            &mut painter,
            &mut up,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            chrome_bottom_px,
            1.0,
            0.0,
            StatusMode::default(),
        );

        // The strip runs from (total_h - chrome_bottom_px) to total_h.
        // Probe a pixel near the vertical center of the strip.
        let strip_top = r.height as f64 - chrome_bottom_px;
        let px_y = (strip_top + chrome_bottom_px * 0.5) as usize;
        let px = pixel_at(&r, 4, px_y);
        assert_ne!(
            px, th.background,
            "expected status bar painted distinct from canvas, got {px:?}"
        );
    }

    // --- cwd_shown_when_set --------------------------------------------------

    /// CWD characters are drawn in text_muted.
    #[test]
    fn cwd_shown_when_set() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let mut up = StubUiPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext {
            cwd: "/Users/test/work/caldera/anvil".to_string(),
            ..LocalContext::default()
        };
        let agent_snap = Snapshot::default();

        draw_status_bar(
            &mut r,
            &mut painter,
            &mut up,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
            StatusMode::default(),
        );

        // CWD now rendered via UiTextPainter.
        let cwd_draws: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text.contains("anvil") && *fg == th.text_muted)
            .collect();
        assert!(
            !cwd_draws.is_empty(),
            "expected cwd text in text_muted via ui_painter, got no calls"
        );
    }

    /// Last-run exit ✓ rendered in verified green on Ok.
    #[test]
    fn exit_check_in_verified_green_on_ok_run() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let mut up = StubUiPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext {
            cwd: "/tmp".to_string(),
            run: RunState::Ok,
            ..LocalContext::default()
        };
        let agent_snap = Snapshot::default();

        draw_status_bar(
            &mut r,
            &mut painter,
            &mut up,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
            StatusMode::default(),
        );

        let verified_chars: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.verified)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            verified_chars.contains(&'\u{2713}'),
            "expected ✓ in verified, got {verified_chars:?}"
        );
    }

    /// Last-run exit ✗ rendered in failure red on Failed.
    #[test]
    fn exit_cross_in_failure_red_on_failed_run() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let mut up = StubUiPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext {
            cwd: "/tmp".to_string(),
            run: RunState::Failed,
            ..LocalContext::default()
        };
        let agent_snap = Snapshot::default();

        draw_status_bar(
            &mut r,
            &mut painter,
            &mut up,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
            StatusMode::default(),
        );

        let failure_chars: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.failure)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            failure_chars.contains(&'\u{2717}'),
            "expected ✗ in failure, got {failure_chars:?}"
        );
    }

    // --- dot_always_present_when_disconnected --------------------------------

    /// The agent dot is ALWAYS drawn, even when agent_snap is default
    /// (NotInstalled). The dot must use text_subtle (not agent) and
    /// the remaining "idle" chars must use text_muted.
    #[test]
    fn dot_always_present_when_disconnected() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let mut up = StubUiPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default(); // no cwd
        let agent_snap = Snapshot::default(); // NotInstalled

        draw_status_bar(
            &mut r,
            &mut painter,
            &mut up,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
            StatusMode::default(),
        );

        // The dot (●, U+25CF) must appear in text_subtle via mono path.
        let dot_subtle = painter
            .calls
            .iter()
            .any(|(cp, fg)| *cp == 0x25CF && *fg == th.text_subtle);
        assert!(
            dot_subtle,
            "expected ● in text_subtle when disconnected, got {:?}",
            painter.calls
        );

        // "idle" text now rendered via UiTextPainter in text_muted.
        let idle_draws: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text.contains("idle") && *fg == th.text_muted)
            .collect();
        assert!(
            !idle_draws.is_empty(),
            "expected 'idle' in text_muted via ui_painter when disconnected"
        );

        // The dot must NOT appear in agent color when disconnected.
        let dot_agent = painter
            .calls
            .iter()
            .any(|(cp, fg)| *cp == 0x25CF && *fg == th.agent);
        assert!(!dot_agent, "dot must not be agent color when disconnected");
    }

    // --- running_count_shown_when_live ---------------------------------------

    /// "running" chars are recorded in text_muted when connection == Live; the
    /// leading diamond is in agent color.
    #[test]
    fn running_count_shown_when_live() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let mut up = StubUiPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default();
        let agent_snap = Snapshot {
            connection: Connection::Live,
            running_count: 2,
            ..Snapshot::default()
        };
        draw_status_bar(
            &mut r,
            &mut painter,
            &mut up,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
            StatusMode::default(),
        );

        // "running" text now rendered via UiTextPainter in text_muted.
        let running_draws: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| text.contains("running") && *fg == th.text_muted)
            .collect();
        assert!(
            !running_draws.is_empty(),
            "expected 'running' in text_muted via ui_painter"
        );
        // With pulsing, the dot color is a blend of charcoal→agent; it will not
        // equal th.agent exactly. Assert it is NOT the idle text_subtle color.
        let dot_subtle = painter
            .calls
            .iter()
            .any(|(cp, fg)| *cp == 0x25CF && *fg == th.text_subtle);
        assert!(
            !dot_subtle,
            "expected dot NOT in text_subtle when Live, got {:?}",
            painter.calls
        );
    }

    // --- mode_chip_editing_shows_in_text_subtle ------------------------------

    /// In default Editing mode, "EDITING" chars appear in text_subtle.
    #[test]
    fn mode_chip_editing_shows_in_text_subtle() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let mut up = StubUiPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default();
        let agent_snap = Snapshot::default();

        draw_status_bar(
            &mut r,
            &mut painter,
            &mut up,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
            StatusMode::Editing,
        );

        // "EDITING" now rendered via UiTextPainter in text_subtle.
        let editing_draws: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| *text == "EDITING" && *fg == th.text_subtle)
            .collect();
        assert!(
            !editing_draws.is_empty(),
            "expected EDITING in text_subtle via ui_painter"
        );
    }

    // --- mode_chip_picking_shows_in_accent_primary ---------------------------

    /// In Picking mode, "PICKING" chars appear in accent_primary.
    #[test]
    fn mode_chip_picking_shows_in_accent_primary() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let mut up = StubUiPainter::default();
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default();
        let agent_snap = Snapshot::default();

        draw_status_bar(
            &mut r,
            &mut painter,
            &mut up,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
            StatusMode::Picking,
        );

        // "PICKING" now rendered via UiTextPainter in accent_primary.
        let picking_draws: Vec<_> = up
            .draws
            .iter()
            .filter(|(text, fg)| *text == "PICKING" && *fg == th.accent_primary)
            .collect();
        assert!(
            !picking_draws.is_empty(),
            "expected PICKING in accent_primary via ui_painter"
        );
    }
}
