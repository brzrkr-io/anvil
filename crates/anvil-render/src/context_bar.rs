//! Top context bar — drawn only in Ide mode.
//!
//! A fixed-height pixel strip immediately below the OS title strip.
//! Layout: left-anchored project/git info, right-anchored kube/head info,
//! with an optional editor segment right of kube when a native editor pane
//! is focused.

use anvil_theme::Theme;
use anvil_workspace::layout::Rect;

use crate::agent_panel::{GitState, LocalContext};
use crate::raster::{FontMetrics, GlyphPainter, PixelRect, Raster, UiTextPainter, UiWeight};
use crate::ui_text_sizes::CONTEXT_BAR_PT;

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
#[allow(clippy::too_many_arguments)]
pub fn draw_context_bar(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    ui_painter: &mut dyn UiTextPainter,
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

    // DD6: 4pt graphite gap between the OS tab-strip chrome and the context bar.
    // Paint the top 4 device pixels as a dark graphite strip, then the charcoal
    // bar body below it — without touching the rect geometry so pane layout is
    // unchanged.
    const GAP_PT: f64 = 4.0;
    raster.fill_pixel_rect(bx, by, bar_w, GAP_PT, theme.graphite);
    raster.fill_pixel_rect(
        bx,
        by + GAP_PT,
        bar_w,
        (bar_h - GAP_PT).max(0.0),
        theme.charcoal,
    );
    raster.fill_pixel_rect(bx, by + bar_h - 1.0, bar_w, 1.0, theme.hairline);

    // Baseline for ui_line calls in this bar.
    let baseline_y =
        by + ((bar_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0) + (cell_h - metrics.descent);
    let mut x = bx + 12.0 + 80.0;

    x = draw_chip(
        raster, painter, ui_painter, metrics, theme, "IDE", x, by, bar_h, true,
    ) + 10.0;

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
        // Hide the path tail when the focused buffer is the placeholder
        // [scratch] — visually the chip reads "anvil" alone, matching the
        // Option A topbar's quiet identity.
        Some("[scratch]") => cwd_base,
        Some(name) => format!("{cwd_base}  ›  {name}"),
        None => cwd_base,
    };
    draw_run_ellipsized(
        raster,
        ui_painter,
        &path,
        theme.text_muted,
        x,
        baseline_y,
        bx + bar_w - 220.0,
    );

    // U1: show "Parsing…" chip while a large-file syntax parse is in progress.
    if editor.as_ref().map(|e| e.syntax_pending).unwrap_or(false) {
        let rx = bx + bar_w - 12.0;
        draw_chip_right(
            raster,
            painter,
            ui_painter,
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
        rx = draw_chip_right(
            raster, painter, ui_painter, metrics, theme, tok, rx, by, bar_h, dirty,
        );
        rx -= 8.0;
        if !local.head_short.is_empty() {
            rx = draw_chip_right(
                raster,
                painter,
                ui_painter,
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
                raster, painter, ui_painter, metrics, theme, &ab_label, rx, by, bar_h, false,
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
            ui_painter,
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
    ui_painter: &mut dyn UiTextPainter,
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
    let ide_w = ui_painter.measure("IDE", CONTEXT_BAR_PT, UiWeight::Regular) + 14.0;
    let mut x = bx + 12.0 + 80.0 + ide_w + 10.0;

    let push_label = "\u{2191} push";
    let push_w = ui_painter.measure(push_label, CONTEXT_BAR_PT, UiWeight::Regular) + 14.0;
    let push_x = x;
    draw_chip(
        raster, painter, ui_painter, metrics, theme, push_label, x, by, bar_h, false,
    );
    x += push_w + 6.0;

    let pull_label = "\u{2193} pull";
    let pull_w = ui_painter.measure(pull_label, CONTEXT_BAR_PT, UiWeight::Regular) + 14.0;
    let pull_x = x;
    draw_chip(
        raster, painter, ui_painter, metrics, theme, pull_label, x, by, bar_h, false,
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
    ui_painter: &mut dyn UiTextPainter,
    metrics: FontMetrics,
    theme: &Theme,
    label: &str,
    x: f64,
    by: f64,
    bar_h: f64,
    accent: bool,
) -> f64 {
    let label_w = ui_painter.measure(label, CONTEXT_BAR_PT, UiWeight::Regular);
    let w = label_w + 14.0;
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
    let color = if accent {
        theme.accent
    } else {
        theme.text_subtle
    };
    let text_y = by
        + ((bar_h - metrics.cell_h) * 0.5 + metrics.descent * 0.5).max(0.0)
        + (metrics.cell_h - metrics.descent);
    // Clip to chip interior: don't draw outside [x+7, x+w-7].
    let max_x = x + w - 7.0;
    if x + 7.0 < max_x {
        raster.ui_line(
            ui_painter,
            label,
            x + 7.0,
            text_y,
            CONTEXT_BAR_PT,
            UiWeight::Regular,
            color,
        );
    }
    // painter kept alive to satisfy borrow but not used for chip text
    let _ = painter;
    x + w
}

#[allow(clippy::too_many_arguments)]
fn draw_chip_right(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    ui_painter: &mut dyn UiTextPainter,
    metrics: FontMetrics,
    theme: &Theme,
    label: &str,
    right: f64,
    by: f64,
    bar_h: f64,
    accent: bool,
) -> f64 {
    let label_w = ui_painter.measure(label, CONTEXT_BAR_PT, UiWeight::Regular);
    let w = label_w + 14.0;
    let x = right - w;
    draw_chip(
        raster, painter, ui_painter, metrics, theme, label, x, by, bar_h, accent,
    );
    x
}

/// Draw `s` at (`x`, `y`) using UI text, clipping at `max_x`.
/// Appends `…` when overflow detected via `ui_measure`.
#[allow(clippy::too_many_arguments)]
fn draw_run_ellipsized(
    raster: &mut Raster,
    ui_painter: &mut dyn UiTextPainter,
    s: &str,
    color: [u8; 3],
    x: f64,
    y: f64,
    max_x: f64,
) {
    if x >= max_x || s.is_empty() {
        return;
    }
    let total_w = ui_painter.measure(s, CONTEXT_BAR_PT, UiWeight::Regular);
    if x + total_w <= max_x {
        raster.ui_line(
            ui_painter,
            s,
            x,
            y,
            CONTEXT_BAR_PT,
            UiWeight::Regular,
            color,
        );
        return;
    }
    // Overflow: fit as many chars as possible, then append '…'.
    let ellipsis_w = ui_painter.measure("\u{2026}", CONTEXT_BAR_PT, UiWeight::Regular);
    let budget = (max_x - x - ellipsis_w).max(0.0);
    let mut acc = 0.0;
    let mut cut = 0;
    for ch in s.chars() {
        let cw = ui_painter.measure(&ch.to_string(), CONTEXT_BAR_PT, UiWeight::Regular);
        if acc + cw > budget {
            break;
        }
        acc += cw;
        cut += ch.len_utf8();
    }
    let clipped = format!("{}\u{2026}", &s[..cut]);
    raster.ui_line(
        ui_painter,
        &clipped,
        x,
        y,
        CONTEXT_BAR_PT,
        UiWeight::Regular,
        color,
    );
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

    /// Records ui_line draws as (text, color).
    #[derive(Default)]
    struct StubUiPainter {
        pub draws: Vec<(String, [u8; 3])>,
    }

    impl UiTextPainter for StubUiPainter {
        fn measure(&mut self, text: &str, _size_pt: f64, _weight: UiWeight) -> f64 {
            text.chars().count() as f64 * 8.0
        }

        fn draw_line(
            &mut self,
            text: &str,
            _x: f64,
            _y: f64,
            _size_pt: f64,
            _weight: UiWeight,
            fg: [u8; 3],
            _pixels: &mut [u8],
            _bw: usize,
            _bh: usize,
        ) {
            self.draws.push((text.to_string(), fg));
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
        let mut up = StubUiPainter::default();
        r.clear([0, 0, 0]);

        draw_context_bar(
            &mut r,
            &mut p,
            &mut up,
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
        let mut up = StubUiPainter::default();
        let zero = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        draw_context_bar(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            &LocalContext::default(),
            None,
            zero,
        );
    }

    // CWD basename appears in text_muted.
    #[test]
    fn cwd_shown_in_text_muted() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let local = LocalContext {
            cwd: "/Users/test/anvil".to_string(),
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, &mut up, m, &th, &local, None, bar_rect());

        let has_muted = up
            .draws
            .iter()
            .any(|(text, fg)| *fg == th.text_muted && text.contains("anvil"));
        assert!(
            has_muted,
            "expected cwd 'anvil' in text_muted, got {:?}",
            up.draws
        );
    }

    // Dirty branch uses accent color.
    #[test]
    fn dirty_branch_uses_accent() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let local = LocalContext {
            cwd: "/anvil".to_string(),
            git: GitState::Ok,
            branch: "main".to_string(),
            git_dirty: 2,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, &mut up, m, &th, &local, None, bar_rect());

        // "git main" chip drawn in accent (dirty=true)
        let has_accent = up
            .draws
            .iter()
            .any(|(text, fg)| *fg == th.accent && text.contains("main"));
        assert!(
            has_accent,
            "expected branch 'main' in accent color, got {:?}",
            up.draws
        );
    }

    // Clean branch uses text_subtle.
    #[test]
    fn clean_branch_uses_text_subtle() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let local = LocalContext {
            cwd: "/anvil".to_string(),
            git: GitState::Ok,
            branch: "main".to_string(),
            git_dirty: 0,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, &mut up, m, &th, &local, None, bar_rect());

        let has_subtle = up
            .draws
            .iter()
            .any(|(text, fg)| *fg == th.text_subtle && text.contains("main"));
        assert!(
            has_subtle,
            "expected branch 'main' in text_subtle for clean branch, got {:?}",
            up.draws
        );
    }

    // No git data → no separator or branch rendered.
    #[test]
    fn no_git_no_branch_rendered() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let local = LocalContext {
            cwd: "/anvil".to_string(),
            git: GitState::NoRepo,
            branch: String::new(),
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, &mut up, m, &th, &local, None, bar_rect());

        let has_git = up.draws.iter().any(|(text, _)| text.contains("git "));
        assert!(!has_git, "no repo → no git branch chip, got {:?}", up.draws);
    }

    // head_short appears on the right when set.
    #[test]
    fn head_short_shown_when_present() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let local = LocalContext {
            head_short: "abc1234".to_string(),
            git: GitState::Ok,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, &mut up, m, &th, &local, None, bar_rect());

        let has_head = up
            .draws
            .iter()
            .any(|(text, fg)| *fg == th.text_subtle && text.contains("abc1234"));
        assert!(
            has_head,
            "expected head_short 'abc1234' in text_subtle, got {:?}",
            up.draws
        );
    }

    // Editor segment rendered in text_muted when present.
    #[test]
    fn editor_segment_shown_when_present() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let ed = ContextBarEditor {
            name: "foo.rs",
            modified: false,
            syntax_pending: false,
        };
        draw_context_bar(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            &LocalContext::default(),
            Some(ed),
            bar_rect(),
        );

        let has_name = up
            .draws
            .iter()
            .any(|(text, fg)| *fg == th.text_muted && text.contains("foo.rs"));
        assert!(
            has_name,
            "expected 'foo.rs' in text_muted, got {:?}",
            up.draws
        );
    }

    // F4: path text longer than available width gets an ellipsis char appended.
    #[test]
    fn long_path_gets_ellipsis() {
        let m = font_metrics(); // cell_w = 8.0 / measure = 8.0 * chars
        let th = theme();
        let bar = Rect {
            x: 0.0,
            y: 36.0,
            w: 400.0,
            h: 24.0,
        };
        let mut r = Raster::new(400, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();
        let long_cwd = format!("/home/user/{}", "a".repeat(80));
        let local = LocalContext {
            cwd: long_cwd,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, &mut up, m, &th, &local, None, bar);

        let has_ellipsis = up.draws.iter().any(|(text, _)| text.contains('\u{2026}'));
        assert!(
            has_ellipsis,
            "expected ellipsis in overflowing path, got {:?}",
            up.draws
        );
    }

    // S3: ahead/behind chip rendered when ahead > 0.
    #[test]
    fn ahead_behind_chip_rendered() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let local = LocalContext {
            git: GitState::Ok,
            branch: "main".to_string(),
            git_ahead: 3,
            git_behind: 1,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, &mut up, m, &th, &local, None, bar_rect());

        let all_text: String = up.draws.iter().map(|(t, _)| t.as_str()).collect();
        assert!(
            all_text.contains('\u{2191}'),
            "expected ↑ in ahead/behind chip, got {:?}",
            up.draws
        );
        assert!(
            all_text.contains('\u{2193}'),
            "expected ↓ in ahead/behind chip, got {:?}",
            up.draws
        );
        assert!(
            all_text.contains('3'),
            "expected ahead count '3', got {:?}",
            up.draws
        );
    }

    // S3: no ahead/behind chip when both are zero.
    #[test]
    fn no_ahead_behind_chip_when_zero() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        let local = LocalContext {
            git: GitState::Ok,
            branch: "main".to_string(),
            git_ahead: 0,
            git_behind: 0,
            ..LocalContext::default()
        };
        draw_context_bar(&mut r, &mut p, &mut up, m, &th, &local, None, bar_rect());

        let all_text: String = up.draws.iter().map(|(t, _)| t.as_str()).collect();
        assert!(
            !all_text.contains('\u{2191}'),
            "no ↑ chip when ahead==0, got {:?}",
            up.draws
        );
        assert!(
            !all_text.contains('\u{2193}'),
            "no ↓ chip when behind==0, got {:?}",
            up.draws
        );
    }

    // Editor segment falls back to a native-editor label when None.
    #[test]
    fn editor_segment_falls_back_when_none() {
        let m = font_metrics();
        let th = theme();
        let mut r = Raster::new(800, 100);
        let mut p = StubPainter::default();
        let mut up = StubUiPainter::default();

        draw_context_bar(
            &mut r,
            &mut p,
            &mut up,
            m,
            &th,
            &LocalContext::default(),
            None,
            bar_rect(),
        );

        // When editor is None, cwd falls back to "anvil" — check text_muted contains it.
        let has_fallback = up
            .draws
            .iter()
            .any(|(text, fg)| *fg == th.text_muted && text.contains("anvil"));
        assert!(
            has_fallback,
            "expected fallback 'anvil' label in text_muted, got {:?}",
            up.draws
        );
    }
}
