//! Top context bar — drawn only in Ide mode.
//!
//! A fixed-height pixel strip immediately below the OS title strip.
//! Layout: left-anchored project/git info, right-anchored kube/head info,
//! with an optional editor segment right of kube when a native editor pane
//! is focused.

use anvil_theme::Theme;
use anvil_workspace::layout::Rect;

use crate::agent_panel::{LocalContext, format_cwd};
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
    let cell_w = metrics.cell_w;
    let cell_h = metrics.cell_h;

    // Background fill.
    raster.fill_pixel_rect(bx, by, bar_w, bar_h, theme.charcoal);
    // 1px hairline at the bottom edge.
    raster.fill_pixel_rect(bx, by + bar_h - 1.0, bar_w, 1.0, theme.hairline);

    // Vertical centre for glyph baseline.
    let glyph_y = by + ((bar_h - cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);

    let pad_x = 14.0; // matches status bar padding

    // Helper: draw a string run, advancing `x`. Clips at bar right edge.
    let draw_run = |raster: &mut Raster,
                    painter: &mut dyn GlyphPainter,
                    s: &str,
                    color: [u8; 3],
                    x: &mut f64| {
        for ch in s.chars() {
            if *x + cell_w > bx + bar_w {
                break;
            }
            raster.glyph_at(painter, metrics, *x, glyph_y, ch as u32, color);
            *x += cell_w;
        }
    };

    // ── Left section ─────────────────────────────────────────────────────────

    let mut lx = bx + pad_x;

    // Project kind icon glyph.
    let project_icon: Option<&str> = match local.project_kind.as_deref() {
        Some("rust") => Some("\u{25b6}"), // ▶  (compact triangle)
        Some("node") => Some("\u{2022}"), // •
        Some("make") => Some("\u{25a0}"), // ■
        Some(_) => Some("\u{25cb}"),      // ○  generic
        None => None,
    };

    if let Some(icon) = project_icon {
        draw_run(raster, painter, icon, theme.text_muted, &mut lx);
        lx += cell_w * 0.5; // small gap after icon (half-cell)
    }

    // cwd basename.
    if !local.cwd.is_empty() {
        let cwd = format_cwd(&local.cwd);
        draw_run(raster, painter, &cwd, theme.text_muted, &mut lx);
    }

    // " · " separator + git branch (only when branch name is known).
    if !local.branch.is_empty() && !local.cwd.is_empty() {
        draw_run(raster, painter, " \u{00b7} ", theme.text_muted, &mut lx);

        let branch_color = if local.git_dirty > 0 {
            theme.accent
        } else {
            theme.text_subtle
        };
        draw_run(raster, painter, &local.branch, branch_color, &mut lx);
    }

    // ── Right section ─────────────────────────────────────────────────────────

    // Editor segment: "edit: <name>" or "edit: <name>•" when modified.
    let editor_owned: Option<String> = editor.as_ref().map(|e| {
        let suffix = if e.modified { "\u{2022}" } else { "" }; // •
        format!("edit: {}{}", e.name, suffix)
    });

    // Build the right string segments.
    let kube_str: Option<String> = local
        .kube_context
        .as_ref()
        .map(|k| format!("\u{2388} {}", k.cluster)); // ⎈ cluster

    let head_str: Option<&str> = if local.head_short.is_empty() {
        None
    } else {
        Some(&local.head_short)
    };

    // Assemble right text width for right-alignment.
    let mut right_parts: Vec<(&str, [u8; 3])> = Vec::new();
    let editor_str_ref: Option<&str> = editor_owned.as_deref();
    if let Some(es) = editor_str_ref {
        right_parts.push((es, theme.text_muted));
        if kube_str.is_some() || head_str.is_some() {
            right_parts.push((" \u{00b7} ", theme.text_muted));
        }
    }
    let kube_owned;
    if let Some(ref ks) = kube_str {
        kube_owned = ks.clone();
        right_parts.push((&kube_owned, theme.text_muted));
        if head_str.is_some() {
            right_parts.push((" \u{00b7} ", theme.text_muted));
        }
    }
    if let Some(hs) = head_str {
        right_parts.push((hs, theme.text_subtle));
    }

    if !right_parts.is_empty() {
        let total_chars: usize = right_parts.iter().map(|(s, _)| s.chars().count()).sum();
        let right_text_w = total_chars as f64 * cell_w;
        let mut rx = (bx + bar_w - pad_x - right_text_w).max(lx);
        for (s, color) in &right_parts {
            draw_run(raster, painter, s, *color, &mut rx);
        }
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

        // No accent or text_subtle calls (branch is the only user of those on left).
        let accent_or_subtle = p
            .calls
            .iter()
            .any(|(_, fg)| *fg == th.accent || *fg == th.text_subtle);
        assert!(
            !accent_or_subtle,
            "no branch data → no accent/subtle glyphs"
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

    // Editor segment omitted when None.
    #[test]
    fn editor_segment_omitted_when_none() {
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

        // No glyphs should appear (LocalContext is empty; editor is None).
        assert!(p.calls.is_empty(), "no glyphs expected when editor is None");
    }
}
