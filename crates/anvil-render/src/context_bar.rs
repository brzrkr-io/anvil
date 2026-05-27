//! Top context bar — drawn only in Ide mode.
//!
//! A fixed-height pixel strip immediately below the OS title strip.
//! Layout: left-anchored project/git info, right-anchored kube/head info,
//! with an optional editor segment right of kube when a native editor pane
//! is focused.

use anvil_theme::Theme;
use anvil_workspace::layout::Rect;

use crate::agent_panel::{GitState, LocalContext};
use crate::raster::{FontMetrics, GlyphPainter, PixelRect, Raster};

/// Editor-segment input for the context bar.
///
/// Built by the caller from the focused native editor pane's buffer:
/// `name` = path basename or `"[scratch]"`; `modified` = dirty flag if
/// available.  `None` when no native editor pane is focused.
#[derive(Clone, Debug)]
pub struct ContextBarEditor<'a> {
    pub name: &'a str,
    pub modified: bool,
    /// U1: true while a large file's syntax tree is being parsed asynchronously.
    /// When true, a "Parsing…" chip is rendered right of the filename.
    pub syntax_pending: bool,
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

    let cwd_base = if local.cwd.is_empty() {
        "anvil".to_string()
    } else {
        local
            .cwd
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("anvil")
            .to_string()
    };
    let path = match editor.as_ref().map(|e| e.name) {
        Some(name) => format!("{cwd_base} · {name}"),
        None => cwd_base,
    };
    draw_run_ellipsized(
        raster,
        painter,
        metrics,
        &path,
        theme.text_muted,
        x,
        glyph_y,
        bx + bar_w - 220.0,
    );

    // U1: show "Parsing…" chip while a large-file syntax parse is in progress.
    if editor.as_ref().map(|e| e.syntax_pending).unwrap_or(false) {
        let rx = bx + bar_w - 12.0;
        draw_chip_right(
            raster,
            painter,
            metrics,
            theme,
            "Parsing\u{2026}",
            rx,
            by,
            bar_h,
            false,
        );
    }

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
        // ↑N ↓M ahead/behind chip (S3).
        if local.git_ahead > 0 || local.git_behind > 0 {
            let ab_label = match (local.git_ahead, local.git_behind) {
                (a, 0) => format!("\u{2191}{a}"),
                (0, b) => format!("\u{2193}{b}"),
                (a, b) => format!("\u{2191}{a} \u{2193}{b}"),
            };
            rx = draw_chip_right(
                raster, painter, metrics, theme, &ab_label, rx, by, bar_h, false,
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

/// Draw `↑ push` and `↓ pull` chips on the left side of the context bar (Z9).
///
/// Chips appear between the IDE chip and the cwd path text.  Returns
/// `(push_rect, pull_rect)` so the caller can route mouse clicks.
/// Returns `None` when the git state is `NoRepo` or the bar is too narrow.
#[allow(clippy::too_many_arguments)]
pub fn draw_push_pull_chips(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    local: &LocalContext,
    rect: Rect,
) -> Option<(PixelRect, PixelRect)> {
    if local.git == GitState::NoRepo || rect.w < 200.0 {
        return None;
    }
    // Left anchor: same as draw_context_bar's `x` after IDE chip.
    let bx = rect.x;
    let by = rect.y;
    let bar_h = rect.h;
    // Start x: 12 (left margin) + 80 (traffic-light area) + IDE chip width + 10 gap.
    let ide_w = "IDE".chars().count() as f64 * metrics.cell_w + 14.0;
    let mut x = bx + 12.0 + 80.0 + ide_w + 10.0;

    let push_w = "\u{2191} push".chars().count() as f64 * metrics.cell_w + 14.0;
    let push_x = x;
    draw_chip(
        raster,
        painter,
        metrics,
        theme,
        "\u{2191} push",
        x,
        by,
        bar_h,
        false,
    );
    x += push_w + 6.0;

    let pull_w = "\u{2193} pull".chars().count() as f64 * metrics.cell_w + 14.0;
    let pull_x = x;
    draw_chip(
        raster,
        painter,
        metrics,
        theme,
        "\u{2193} pull",
        x,
        by,
        bar_h,
        false,
    );

    let chip_y = by + ((bar_h - 20.0) * 0.5).max(0.0);
    Some((
        PixelRect {
            x: push_x,
            y: chip_y,
            w: push_w,
            h: 20.0,
        },
        PixelRect {
            x: pull_x,
            y: chip_y,
            w: pull_w,
            h: 20.0,
        },
    ))
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

/// Like `draw_run_clipped` but appends `…` when the string overflows `max_x`.
#[allow(clippy::too_many_arguments)]
fn draw_run_ellipsized(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    s: &str,
    color: [u8; 3],
    mut x: f64,
    y: f64,
    max_x: f64,
) {
    let cw = metrics.cell_w;
    let total_w = s.chars().count() as f64 * cw;
    if x + total_w <= max_x {
        // No overflow: paint normally.
        draw_run_clipped(raster, painter, metrics, s, color, x, y, max_x);
        return;
    }
    // Reserve one cell for the ellipsis.
    let ellipsis_x = max_x - cw;
    for ch in s.chars() {
        if x + cw > ellipsis_x {
            break;
        }
        raster.glyph_at(painter, metrics, x, y, ch as u32, color);
        x += cw;
    }
    if ellipsis_x >= x {
        raster.glyph_at(painter, metrics, ellipsis_x, y, '…' as u32, color);
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
            syntax_pending: false,
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

    // F4: path text longer than available width gets an ellipsis char appended.
    // Setup: bar_w=400 → max_x = 400 - 220 = 180. IDE chip + margins put the
    // path start at ~138px. A 10-char path = 80px → fits. A 100-char path
    // = 800px → overflows → must produce '…'.
    #[test]
    fn long_path_gets_ellipsis() {
        let m = font_metrics(); // cell_w = 8.0
        let th = theme();
        let bar = Rect {
            x: 0.0,
            y: 36.0,
            w: 400.0,
            h: 24.0,
        };
        let mut r = Raster::new(400, 100);
        let mut p = StubPainter::default();
        // cwd that produces a very long basename after the last '/'.
        let long_cwd = format!("/home/user/{}", "a".repeat(80));
        let local = LocalContext {
            cwd: long_cwd,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, m, &th, &local, None, bar);
        let chars: Vec<char> = p
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            chars.contains(&'…'),
            "expected ellipsis in overflowing path, got {chars:?}"
        );
    }

    // S3: ahead/behind chip rendered when ahead > 0.
    #[test]
    fn ahead_behind_chip_rendered() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();

        let local = LocalContext {
            git: GitState::Ok,
            branch: "main".to_string(),
            git_ahead: 3,
            git_behind: 1,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, m, &th, &local, None, bar_rect());

        let chars: Vec<char> = p
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        // '↑' = U+2191, '↓' = U+2193
        assert!(
            chars.contains(&'\u{2191}'),
            "expected ↑ in ahead/behind chip, got {chars:?}"
        );
        assert!(
            chars.contains(&'\u{2193}'),
            "expected ↓ in ahead/behind chip, got {chars:?}"
        );
        assert!(
            chars.contains(&'3'),
            "expected ahead count '3', got {chars:?}"
        );
    }

    // S3: no ahead/behind chip when both are zero.
    #[test]
    fn no_ahead_behind_chip_when_zero() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();

        let local = LocalContext {
            git: GitState::Ok,
            branch: "main".to_string(),
            git_ahead: 0,
            git_behind: 0,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, m, &th, &local, None, bar_rect());

        let chars: Vec<char> = p
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            !chars.contains(&'\u{2191}'),
            "no ↑ chip when ahead==0, got {chars:?}"
        );
        assert!(
            !chars.contains(&'\u{2193}'),
            "no ↓ chip when behind==0, got {chars:?}"
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
