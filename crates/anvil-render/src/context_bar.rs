//! Top context bar — drawn only in Ide mode.
//!
//! A fixed-height pixel strip immediately below the OS title strip.
//! Layout: left-anchored project/git info, right-anchored kube/head info,
//! with an optional editor segment right of kube when a native editor pane
//! is focused.

use anvil_theme::Theme;
use anvil_workspace::layout::Rect;

use crate::agent_panel::{GitState, LocalContext, format_cwd};
use crate::raster::{FontMetrics, GlyphPainter, Raster};

/// Editor-segment input for the context bar.
///
/// Built by the caller from the focused native editor pane's buffer:
/// `name` = path basename or `"[scratch]"`; `modified` = dirty flag if
/// available.  `None` when no native editor pane is focused.
#[derive(Clone, Debug)]
pub struct ContextBarEditor<'a> {
    pub name: &'a str,
    pub modified: bool,
}

/// Draw the context bar into `rect`.
///
/// - Background: `theme.charcoal`.
/// - Bottom edge: 1px hairline (`theme.hairline`).
/// - Left: project_kind icon + cwd basename · git branch (muted/accent).
/// - Right: editor segment (when present) · kube_context · head_short.
/// - Sections omitted when data is absent; no placeholder text.
/// - `editor`: optional native editor descriptor. Rendered as
///   `edit: <name>[•]` when `Some`.
pub fn draw_context_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    local: &LocalContext,
    editor: Option<ContextBarEditor<'_>>,
    rect: Rect,
) {
    let bar_w = rect.w;
    let bar_h = rect.h;
    if bar_w <= 0.0 || bar_h <= 0.0 {
        return;
    }

    let bx = rect.x;
    let by = rect.y;
    let cell_h = metrics.cell_h;

    raster.fill_pixel_rect(bx, by, bar_w, bar_h, theme.graphite);
    raster.fill_pixel_rect(bx, by + bar_h - 1.0, bar_w, 1.0, theme.hairline);

    let glyph_y = by + ((bar_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    let mut x = bx + 12.0 + 80.0;

    x = draw_chip(raster, painter, metrics, theme, "IDE", x, by, bar_h, true) + 10.0;

    let cwd = if local.cwd.is_empty() {
        "anvil".to_string()
    } else {
        format_cwd(&local.cwd)
    };
    let file = editor.as_ref().map(|e| e.name).unwrap_or("native editor");
    let path = format!("caldera/{cwd} · {file}");
    draw_run_clipped(
        raster,
        painter,
        metrics,
        &path,
        theme.text_muted,
        x,
        glyph_y,
        bx + bar_w - 220.0,
    );

    if local.git != GitState::NoRepo {
        let mut rx = bx + bar_w - 12.0;
        let dirty = local.git_dirty > 0 || local.git == GitState::Dirty;
        let tok = if dirty { "dirty" } else { "clean" };
        rx = draw_chip_right(raster, painter, metrics, theme, tok, rx, by, bar_h, dirty);
        rx -= 8.0;
        if !local.head_short.is_empty() {
            rx = draw_chip_right(
                raster,
                painter,
                metrics,
                theme,
                &local.head_short,
                rx,
                by,
                bar_h,
                false,
            );
            rx -= 8.0;
        }
        let branch = if local.branch.is_empty() {
            "main"
        } else {
            &local.branch
        };
        let branch_label = format!("git {branch}");
        let _ = draw_chip_right(
            raster,
            painter,
            metrics,
            theme,
            &branch_label,
            rx,
            by,
            bar_h,
            dirty,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_chip(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    label: &str,
    x: f64,
    by: f64,
    bar_h: f64,
    accent: bool,
) -> f64 {
    let w = label.chars().count() as f64 * metrics.cell_w + 14.0;
    let y = by + ((bar_h - 20.0) * 0.5).max(0.0);
    raster.fill_pixel_rect_alpha(
        x,
        y,
        w,
        20.0,
        theme.surface,
        if accent { 0.34 } else { 0.22 },
    );
    raster.fill_pixel_rect_alpha(
        x,
        y,
        w,
        1.0,
        theme.accent_bright,
        if accent { 0.36 } else { 0.16 },
    );
    raster.fill_pixel_rect_alpha(x, y + 19.0, w, 1.0, theme.hairline, 0.8);
    draw_run_clipped(
        raster,
        painter,
        metrics,
        label,
        if accent {
            theme.accent
        } else {
            theme.text_subtle
        },
        x + 7.0,
        by + ((bar_h - metrics.cell_h) * 0.5 + metrics.descent * 0.5).max(0.0),
        x + w - 7.0,
    );
    x + w
}

#[allow(clippy::too_many_arguments)]
fn draw_chip_right(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    label: &str,
    right: f64,
    by: f64,
    bar_h: f64,
    accent: bool,
) -> f64 {
    let w = label.chars().count() as f64 * metrics.cell_w + 14.0;
    let x = right - w;
    draw_chip(raster, painter, metrics, theme, label, x, by, bar_h, accent);
    x
}

#[allow(clippy::too_many_arguments)]
fn draw_run_clipped(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    s: &str,
    color: [u8; 3],
    mut x: f64,
    y: f64,
    max_x: f64,
) {
    for ch in s.chars() {
        if x + metrics.cell_w > max_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, y, ch as u32, color);
        x += metrics.cell_w;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_panel::GitState;
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

    fn font_metrics() -> FontMetrics {
        FontMetrics {
            cell_w: 8.0,
            cell_h: 16.0,
            descent: 3.0,
        }
    }

    fn theme() -> Theme {
        anvil_theme::EMBER_DARK
    }

    fn bar_rect() -> Rect {
        Rect {
            x: 0.0,
            y: 36.0,
            w: 800.0,
            h: 24.0,
        }
    }

    // Smoke: no panic, background painted.
    #[test]
    fn draw_context_bar_smoke() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        r.clear([0, 0, 0]);

        draw_context_bar(
            &mut r,
            &mut p,
            m,
            &th,
            &LocalContext::default(),
            None,
            bar_rect(),
        );

        let px = pixel_at(&r, 4, 42); // inside the bar
        assert_ne!(px, th.background, "bar background must be painted");
    }

    // Zero-size rect must not panic.
    #[test]
    fn zero_rect_no_panic() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let zero = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        draw_context_bar(&mut r, &mut p, m, &th, &LocalContext::default(), None, zero);
    }

    // CWD basename appears in text_muted.
    #[test]
    fn cwd_shown_in_text_muted() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();

        let local = LocalContext {
            cwd: "/Users/test/anvil".to_string(),
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, m, &th, &local, None, bar_rect());

        let muted: Vec<char> = p
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.text_muted)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(!muted.is_empty(), "expected cwd chars in text_muted");
    }

    // Dirty branch uses accent color.
    #[test]
    fn dirty_branch_uses_accent() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();

        let local = LocalContext {
            cwd: "/anvil".to_string(),
            git: GitState::Ok,
            branch: "main".to_string(),
            git_dirty: 2,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, m, &th, &local, None, bar_rect());

        let accent_chars: Vec<char> = p
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.accent)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            accent_chars.contains(&'m'),
            "expected branch 'm' in accent color, got {accent_chars:?}"
        );
    }

    // Clean branch uses text_subtle.
    #[test]
    fn clean_branch_uses_text_subtle() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();

        let local = LocalContext {
            cwd: "/anvil".to_string(),
            git: GitState::Ok,
            branch: "main".to_string(),
            git_dirty: 0,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, m, &th, &local, None, bar_rect());

        let subtle_chars: Vec<char> = p
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.text_subtle)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            subtle_chars.contains(&'m'),
            "expected branch 'm' in text_subtle for clean branch, got {subtle_chars:?}"
        );
    }

    // No git data → no separator or branch rendered.
    #[test]
    fn no_git_no_branch_rendered() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();

        let local = LocalContext {
            cwd: "/anvil".to_string(),
            git: GitState::NoRepo,
            branch: String::new(),
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, m, &th, &local, None, bar_rect());

        let rendered: String = p
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            !rendered.contains("git "),
            "no repo → no git branch chip, got {rendered:?}"
        );
    }

    // head_short appears on the right when set.
    #[test]
    fn head_short_shown_when_present() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();

        let local = LocalContext {
            head_short: "abc1234".to_string(),
            git: GitState::Ok,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, m, &th, &local, None, bar_rect());

        let subtle_chars: Vec<char> = p
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.text_subtle)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            subtle_chars.contains(&'a'),
            "expected head_short 'a' in text_subtle, got {subtle_chars:?}"
        );
    }

    // Editor segment rendered in text_muted when present.
    #[test]
    fn editor_segment_shown_when_present() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();

        let ed = ContextBarEditor {
            name: "foo.rs",
            modified: false,
        };
        draw_context_bar(
            &mut r,
            &mut p,
            m,
            &th,
            &LocalContext::default(),
            Some(ed),
            bar_rect(),
        );

        let muted_chars: Vec<char> = p
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.text_muted)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            muted_chars.contains(&'f'),
            "expected 'f' from 'foo.rs' in text_muted, got {muted_chars:?}"
        );
    }

    // Editor segment falls back to a native-editor label when None.
    #[test]
    fn editor_segment_falls_back_when_none() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();

        draw_context_bar(
            &mut r,
            &mut p,
            m,
            &th,
            &LocalContext::default(),
            None,
            bar_rect(),
        );

        let muted_chars: Vec<char> = p
            .calls
            .iter()
            .filter(|(_, fg)| *fg == th.text_muted)
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            muted_chars.contains(&'n'),
            "expected fallback native-editor label in text_muted, got {muted_chars:?}"
        );
    }
}
