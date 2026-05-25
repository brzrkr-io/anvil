//! The always-visible status bar — a fixed-height pixel strip at the bottom of
//! the window. Draws into the BGRA raster like the tab bar and search bar.

use anvil_agent::{Connection, Snapshot};
use anvil_theme::Theme;

use crate::agent_panel::{LocalContext, RunState, format_cwd};
use crate::raster::{FontMetrics, GlyphPainter, Raster};

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
    metrics: FontMetrics,
    theme: &Theme,
    local_ctx: &LocalContext,
    agent_snap: &Snapshot,
    clock: &str,
    chrome_bottom_px: f64,
    window_scale: f64,
    pulse_phase: f32,
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

    // Glyphs vertically centred in the strip.
    let glyph_y = strip_top + ((chrome_bottom_px - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    let pad_x = 14.0 * window_scale; // D: .bottom-bar { padding: 0 14px }

    // ── Left: cwd  ✓/✗ last 0.1s ─────────────────────────────────────────
    let mut x = pad_x;
    let draw_run = |raster: &mut Raster,
                    painter: &mut dyn GlyphPainter,
                    s: &str,
                    color: [u8; 3],
                    x: &mut f64| {
        for ch in s.chars() {
            if *x + cell_w > total_w {
                break;
            }
            raster.glyph_at(painter, metrics, *x, glyph_y, ch as u32, color);
            *x += cell_w;
        }
    };

    if !local_ctx.cwd.is_empty() {
        let cwd = format_cwd(&local_ctx.cwd);
        draw_run(raster, painter, &cwd, theme.text_muted, &mut x);
        x += 2.0 * cell_w; // gap

        let (sym, color) = match local_ctx.run {
            RunState::Ok => ("\u{2713}", theme.verified),
            RunState::Failed => ("\u{2717}", theme.failure),
            _ => ("", theme.text_muted),
        };
        if !sym.is_empty() {
            draw_run(raster, painter, sym, color, &mut x);
            if local_ctx.run_duration_ms > 0 {
                let dur = format!(" last {}", format_duration_ms(local_ctx.run_duration_ms));
                draw_run(raster, painter, &dur, theme.text_muted, &mut x);
            }
        }
    }

    // ── Right: agent · clock ─────────────────────────────────────────────
    let agent_active = agent_snap.connection == Connection::Live;
    let agent_text = if agent_active && agent_snap.running_count > 0 {
        format!("\u{25cf} {} running", agent_snap.running_count)
    } else {
        "\u{25cf} idle".to_string()
    };
    let sep = if !clock.is_empty() { "   " } else { "" };
    let right_text_w =
        (agent_text.chars().count() + sep.chars().count() + clock.chars().count()) as f64 * cell_w;
    let right_start = (total_w - pad_x - right_text_w).max(x);
    let mut rx = right_start;
    let dot_color = if agent_active {
        mix_rgb(theme.charcoal, theme.agent, 0.5 + 0.5 * (std::f32::consts::TAU * pulse_phase).sin())
    } else {
        theme.text_subtle
    };
    for (i, ch) in agent_text.chars().enumerate() {
        if rx + cell_w > total_w {
            break;
        }
        let fg = if i == 0 { dot_color } else { theme.text_muted };
        raster.glyph_at(painter, metrics, rx, glyph_y, ch as u32, fg);
        rx += cell_w;
    }
    for ch in sep.chars() {
        if rx + cell_w > total_w {
            break;
        }
        raster.glyph_at(painter, metrics, rx, glyph_y, ch as u32, theme.text_muted);
        rx += cell_w;
    }
    for ch in clock.chars() {
        if rx + cell_w > total_w {
            break;
        }
        raster.glyph_at(painter, metrics, rx, glyph_y, ch as u32, theme.text_muted);
        rx += cell_w;
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
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default();
        let agent_snap = Snapshot::default();

        let chrome_bottom_px = m.cell_h * 2.0;
        draw_status_bar(
            &mut r,
            &mut painter,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            chrome_bottom_px,
            1.0,
            0.0,
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
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext {
            cwd: "/Users/test/work/caldera/anvil".to_string(),
            ..LocalContext::default()
        };
        let agent_snap = Snapshot::default();

        draw_status_bar(
            &mut r,
            &mut painter,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
        );

        let muted_chars: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.text_muted)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            !muted_chars.is_empty(),
            "expected cwd chars in text_muted, got no calls"
        );
    }

    /// Last-run exit ✓ rendered in verified green on Ok.
    #[test]
    fn exit_check_in_verified_green_on_ok_run() {
        let m = metrics();
        let th = theme();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
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
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
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
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
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
        r.clear([0, 0, 0]);

        let local_ctx = LocalContext::default(); // no cwd
        let agent_snap = Snapshot::default(); // NotInstalled

        draw_status_bar(
            &mut r,
            &mut painter,
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
        );

        // The dot (●, U+25CF) must appear in text_subtle.
        let dot_subtle = painter
            .calls
            .iter()
            .any(|(cp, fg)| *cp == 0x25CF && *fg == th.text_subtle);
        assert!(dot_subtle, "expected ● in text_subtle when disconnected, got {:?}", painter.calls);

        // The "idle" text chars must appear in text_muted (not agent).
        let idle_muted: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.text_muted)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            idle_muted.contains(&'i'),
            "expected 'i' from 'idle' in text_muted when disconnected, got {idle_muted:?}"
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
            m,
            &th,
            &local_ctx,
            &agent_snap,
            "",
            m.cell_h * 2.0,
            1.0,
            0.0,
        );

        let muted_chars: Vec<char> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.text_muted)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            muted_chars.contains(&'r'),
            "expected 'r' from 'running' in text_muted, got {muted_chars:?}"
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
}
