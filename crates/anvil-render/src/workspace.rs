//! Multi-pane render coordinator.
//!
//! `draw_workspace` lays out a PaneTree onto an inner content rect, then calls
//! `draw_viewport` once per leaf with that leaf's pixel origin set on the Raster.
//!
//! Bleed guard: the smooth-scroll path draws row y=0..rows (inclusive — one
//! extra partially visible row). With vertical splits, any bleed into the
//! divider gutter is overdrawn by the divider fill, which is drawn LAST over
//! all panes.

use std::collections::HashMap;

use anvil_editor::Buffer;
use anvil_term::{DirtySet, Search};
use anvil_theme::Theme;
use anvil_workspace::{
    editor_pane::EditorPaneRegistry,
    layout::{LayoutEntry, PaneId, PaneTree, Rect},
    pane::PaneRegistry,
};

use crate::{
    draw::{CursorConfig, CursorParams, FoldedBlocks, GridPainters, draw_viewport},
    editor::{RenderDiagnostic, draw_editor_into},
    raster::{FontMetrics, Raster},
};

/// Pane-divider width in device pixels. 2 device px (= 1 logical pt at 2×
/// Retina) is the minimum that reads as a deliberate divider on a busy
/// terminal screen. 1px hairlines disappeared into surrounding content;
/// the previous 8px read as a structural wall — 2px is the sweet spot.
pub const DIVIDER_PX: f64 = 2.0;

/// Draw all panes in `tree` into `raster`, then draw divider hairlines over them.
///
/// Parameters:
///   raster           — full-window raster bitmap.
///   tree             — the current tab's pane tree (layout and focused id).
///   registry         — the pane registry for the current tab.
///   editor_panes     — registry of native editor panes + buffers.
///   inner            — device-pixel content area (window minus top-bar and panels).
///                      y=0 is the top of the raster. Layout is done in this space.
///   div_px           — divider gutter width in device pixels (use `DIVIDER_PX`).
///   metrics          — font metrics shared by all panes.
///   theme            — shared theme for all panes.
///   search           — active search state, or None.
///   focused_id       — the pane that receives cursor rendering.
///   blink_phase      — cursor blink phase [0, 1).
///   cursor_cfg       — cursor style + blink preference from config.
///   dirty            — per-pane dirty sets from `Terminal::take_dirty_rows`. When
///                      `None`, every row of every pane is redrawn (full frame).
///   diag_by_pane     — per-pane render diagnostics (NE10); translated from
///                      `LspManager::diagnostics_for` by `main.rs`. Empty map is fine.
///
/// After this function returns, raster.origin_x and raster.origin_y are both 0.
#[allow(clippy::too_many_arguments)]
pub fn draw_workspace(
    raster: &mut Raster,
    painters: &mut GridPainters<'_>,
    tree: &PaneTree,
    registry: &mut PaneRegistry,
    editor_panes: &EditorPaneRegistry,
    inner: Rect,
    div_px: f64,
    metrics: FontMetrics,
    theme: &Theme,
    search: Option<&Search>,
    focused_id: PaneId,
    blink_phase: f32,
    cursor_cfg: CursorConfig,
    dirty: Option<&HashMap<PaneId, DirtySet>>,
    running_pulse_phase: f32,
    diag_by_pane: &HashMap<PaneId, Vec<RenderDiagnostic>>,
) {
    let entries = tree.layout(inner, div_px);

    // Draw each leaf.
    for e in &entries {
        let pane = match registry.get_mut(e.id) {
            Some(p) => p,
            None => continue,
        };

        // Set the pane's pixel origin on the raster.
        raster.origin_x = e.rect.x;
        raster.origin_y = e.rect.y;

        if let Some(ref mut terminal) = pane.terminal {
            // ── Terminal pane path ────────────────────────────────────────

            let cursor_params: Option<CursorParams> = if e.id == focused_id {
                Some(CursorParams {
                    ax: pane.cursor_ax,
                    ay: pane.cursor_ay,
                    blink_phase,
                    cfg: cursor_cfg,
                })
            } else {
                None
            };

            // rule_x bounds: horizontal span of this pane in device pixels.
            let rule_x_start = e.rect.x;
            let rule_x_end = e.rect.x + e.rect.w;

            // Fold state for this pane.
            let folded = FoldedBlocks::new(&pane.folded[..pane.folded_count]);

            // Per-pane dirty set: None means "draw all rows".
            let pane_dirty: Option<&DirtySet> = dirty.and_then(|m| m.get(&e.id));

            draw_viewport(
                raster,
                painters,
                terminal,
                metrics,
                theme,
                pane.scroll_pos,
                pane.selection,
                search,
                cursor_params,
                rule_x_start,
                rule_x_end,
                folded,
                pane_dirty,
                running_pulse_phase,
            );

            if is_bottom_drawer(&e.rect, &inner, entries.len()) {
                draw_terminal_drawer_chrome(
                    raster,
                    painters.regular,
                    metrics,
                    theme,
                    e.rect,
                    e.id == focused_id,
                );
            }

            // Living-scrollback indicator: paint a 4px ember bar at the
            // bottom edge of the pane when the user is scrolled up and new
            // output has arrived below.
            let unseen = pane.unseen_rows();
            if unseen > 0 {
                let bar_h = 4.0_f64;
                let bar_y = e.rect.y + e.rect.h - bar_h;
                raster.fill_pixel_rect_alpha(
                    e.rect.x,
                    bar_y,
                    e.rect.w,
                    bar_h,
                    theme.accent_ember,
                    0.92,
                );
            }
        } else {
            // ── Native editor pane (NE5) ──────────────────────────────────
            if let Some(ep) = pane.editor_id.and_then(|_| editor_panes.get_pane(e.id)) {
                if let Some(buf) = editor_panes.get_buffer(ep.buffer_id) {
                    let empty: Vec<RenderDiagnostic> = Vec::new();
                    let diags = diag_by_pane.get(&e.id).map(Vec::as_slice).unwrap_or(&empty);
                    let editor_rect = draw_editor_chrome(
                        raster,
                        painters.regular,
                        buf,
                        metrics,
                        theme,
                        e.rect,
                        e.id == focused_id,
                    );
                    draw_editor_into(
                        raster,
                        painters.regular,
                        ep,
                        buf,
                        metrics,
                        theme,
                        editor_rect,
                        diags,
                        buf.git_gutter.as_ref(),
                    );
                } else {
                    // Buffer missing — fall back to blank fill.
                    raster.fill_pixel_rect(
                        e.rect.x,
                        e.rect.y,
                        e.rect.w,
                        e.rect.h,
                        theme.background,
                    );
                }
            } else {
                // EditorPane not registered yet — blank fill.
                raster.fill_pixel_rect(e.rect.x, e.rect.y, e.rect.w, e.rect.h, theme.background);
            }
        }
    }

    // Reset origin before chrome draws in absolute space.
    raster.origin_x = 0.0;
    raster.origin_y = 0.0;

    // Draw divider hairlines over all pane content (bleed guard).
    draw_dividers(raster, &entries, div_px, theme, focused_id, registry);
}

fn is_bottom_drawer(rect: &Rect, inner: &Rect, leaf_count: usize) -> bool {
    leaf_count > 1 && rect.h <= inner.h * 0.40 && rect.y > inner.y + inner.h * 0.45
}

fn draw_terminal_drawer_chrome(
    raster: &mut Raster,
    _painter: &mut dyn crate::raster::GlyphPainter,
    _metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
    active: bool,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }

    // The preserved PTY is a secondary IDE drawer, not a second primary
    // terminal canvas. Wash it toward the Mineral panel stack while keeping
    // output readable underneath; reserve Ember for actual execution/unseen
    // output indicators elsewhere.
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, rect.h, theme.charcoal, 0.34);

    let cap_h = 18.0_f64.min(rect.h.max(0.0));
    if cap_h > 0.0 {
        raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, cap_h, theme.surface, 0.18);
    }

    let top_rule = if active {
        theme.accent_primary
    } else {
        theme.hairline
    };
    let top_alpha = if active { 0.48 } else { 0.68 };
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, 1.0, top_rule, top_alpha);
}

fn draw_editor_chrome(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    buffer: &Buffer,
    metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
    active: bool,
) -> Rect {
    let tabs_h = 30.0_f64.min(rect.h.max(0.0));
    // One status bar per editor group looked like repeated terminal chrome in
    // the failed smoke. Keep editor state in the active tab/chrome until real
    // file status data exists.
    let status_h = 0.0_f64;
    if tabs_h <= 0.0 || rect.w <= 0.0 {
        return rect;
    }

    // Native editor chrome should read as IDE structure, not terminal output or
    // decorative ember. Keep warmth for active state only and use Mineral
    // graphite/charcoal/hairline tokens for the shell.
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.surface);
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, tabs_h, theme.graphite);
    raster.fill_pixel_rect_alpha(
        rect.x,
        rect.y + tabs_h - 1.0,
        rect.w,
        1.0,
        theme.hairline,
        0.92,
    );

    let filename = buffer
        .tracked_path()
        .and_then(|p| p.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("scratch");
    let dirty_dot = if buffer.revisions > 0 { "●" } else { "" };
    let lang = buffer.language_id().unwrap_or("plain");
    let active_title = if dirty_dot.is_empty() {
        format!("◇ {filename}")
    } else {
        format!("{dirty_dot} ◇ {filename}")
    };

    let tab_w = (active_title.chars().count() as f64 * metrics.cell_w + 42.0).clamp(96.0, 188.0);
    let _tab_x = draw_editor_tab(
        raster,
        painter,
        metrics,
        theme,
        &active_title,
        Rect {
            x: rect.x,
            y: rect.y,
            w: tab_w,
            h: tabs_h,
        },
        true,
    );

    let meta = format!("{lang} · native");
    let meta_w = meta.chars().count() as f64 * metrics.cell_w;
    if rect.w > tab_w + meta_w + 40.0 {
        draw_chrome_text(
            raster,
            painter,
            metrics,
            &meta,
            theme.text_subtle,
            Rect {
                x: rect.x + rect.w - meta_w - 12.0,
                y: rect.y + ((tabs_h - metrics.cell_h) * 0.5).max(0.0),
                w: meta_w,
                h: metrics.cell_h,
            },
        );
    }

    if status_h > 0.0 {
        let status_y = rect.y + rect.h - status_h;
        raster.fill_pixel_rect_alpha(rect.x, status_y, rect.w, status_h, [0x0b, 0x0a, 0x09], 0.86);
        raster.fill_pixel_rect_alpha(rect.x, status_y, rect.w, 1.0, theme.accent_bright, 0.20);
        let left = "main · rust-analyzer live · native";
        draw_chrome_text(
            raster,
            painter,
            metrics,
            left,
            theme.text_subtle,
            Rect {
                x: rect.x + 12.0,
                y: status_y + ((status_h - metrics.cell_h) * 0.5).max(0.0),
                w: (rect.w * 0.55).max(0.0),
                h: metrics.cell_h,
            },
        );
        let right = format!("UTF-8 · {lang}");
        let right_w = right.chars().count() as f64 * metrics.cell_w;
        if rect.w > right_w + 28.0 {
            draw_chrome_text(
                raster,
                painter,
                metrics,
                &right,
                theme.text_subtle,
                Rect {
                    x: rect.x + rect.w - right_w - 12.0,
                    y: status_y + ((status_h - metrics.cell_h) * 0.5).max(0.0),
                    w: right_w,
                    h: metrics.cell_h,
                },
            );
        }
    }

    if active {
        raster.fill_pixel_rect(rect.x, rect.y, rect.w, 1.0, theme.accent_primary);
    }

    Rect {
        x: rect.x,
        y: rect.y + tabs_h,
        w: rect.w,
        h: (rect.h - tabs_h - status_h).max(0.0),
    }
}

fn draw_editor_tab(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    label: &str,
    tab: Rect,
    active: bool,
) -> f64 {
    let fill = if active {
        theme.charcoal
    } else {
        theme.graphite
    };
    raster.fill_pixel_rect(tab.x, tab.y + 5.0, tab.w, (tab.h - 5.0).max(0.0), fill);
    raster.fill_pixel_rect_alpha(
        tab.x + tab.w - 1.0,
        tab.y + 7.0,
        1.0,
        (tab.h - 9.0).max(0.0),
        theme.hairline,
        0.86,
    );
    if active {
        raster.fill_pixel_rect(tab.x, tab.y, tab.w, 2.0, theme.accent_primary);
        raster.fill_pixel_rect_alpha(
            tab.x,
            tab.y + 5.0,
            tab.w,
            tab.h - 5.0,
            theme.panel_raised,
            0.16,
        );
    }

    let text_x = tab.x + if active { 16.0 } else { 12.0 };
    if active {
        raster.fill_pixel_rect_alpha(
            tab.x + 8.0,
            tab.y + (tab.h * 0.5 - 3.0).max(0.0),
            2.0,
            6.0,
            theme.accent_primary,
            0.88,
        );
    }
    draw_chrome_text(
        raster,
        painter,
        metrics,
        label,
        if active {
            theme.foreground
        } else {
            theme.text_muted
        },
        Rect {
            x: text_x,
            y: tab.y + ((tab.h - metrics.cell_h) * 0.5).max(0.0),
            w: (tab.w - (text_x - tab.x) - 10.0).max(0.0),
            h: metrics.cell_h,
        },
    );
    tab.x + tab.w
}

fn draw_chrome_text(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    metrics: FontMetrics,
    text: &str,
    color: [u8; 3],
    bounds: Rect,
) {
    let mut gx = bounds.x;
    let max_x = bounds.x + bounds.w;
    for ch in text.chars() {
        if gx + metrics.cell_w > max_x {
            break;
        }
        raster.glyph_at(painter, metrics, gx, bounds.y, ch as u32, color);
        gx += metrics.cell_w;
    }
}

/// Draw only the chrome portion of the workspace (divider hairlines, focused
/// pane accent border) without drawing any terminal viewport content.
///
/// Used by the GPU rendering path (`ANVIL_RENDER=gpu`) where viewport cells
/// are drawn by the GPU cell pipeline instead of the CPU raster.  The caller
/// is responsible for calling `draw_viewport_gpu` per pane separately.
///
/// After this function returns, raster.origin_x and raster.origin_y are both 0.
#[allow(clippy::too_many_arguments)]
pub fn draw_workspace_chrome(
    raster: &mut Raster,
    tree: &PaneTree,
    registry: &PaneRegistry,
    inner: Rect,
    div_px: f64,
    theme: &Theme,
    focused_id: PaneId,
) {
    let entries = tree.layout(inner, div_px);
    // Reset origin (no pane origins needed — we skip viewport drawing).
    raster.origin_x = 0.0;
    raster.origin_y = 0.0;
    // Draw divider hairlines.
    draw_dividers(raster, &entries, div_px, theme, focused_id, registry);
}

/// Fill divider gutters between all adjacent leaf pairs. Called after all pane
/// content is drawn so the dividers overdraw any scroll bleed.
fn draw_dividers(
    raster: &mut Raster,
    entries: &[LayoutEntry],
    div_px: f64,
    theme: &Theme,
    focused_id: PaneId,
    registry: &PaneRegistry,
) {
    // For each pair of leaves, if they share a boundary (with a gutter between
    // them), fill the gutter rectangle.
    for (ai, a) in entries.iter().enumerate() {
        for b in &entries[ai + 1..] {
            // Horizontal split: b is to the right of a.
            {
                let gap_x = a.rect.x + a.rect.w;
                let gap_end = b.rect.x;
                if gap_end > gap_x && gap_end - gap_x <= div_px + 1.0 {
                    let oy = f64::max(a.rect.y, b.rect.y);
                    let oy_end = f64::min(a.rect.y + a.rect.h, b.rect.y + b.rect.h);
                    if oy_end > oy {
                        raster.fill_pixel_rect(
                            gap_x,
                            oy,
                            gap_end - gap_x,
                            oy_end - oy,
                            theme.border,
                        );
                    }
                }
            }
            // Vertical split: b is below a.
            {
                let gap_y = a.rect.y + a.rect.h;
                let gap_end = b.rect.y;
                if gap_end > gap_y && gap_end - gap_y <= div_px + 1.0 {
                    let ox = f64::max(a.rect.x, b.rect.x);
                    let ox_end = f64::min(a.rect.x + a.rect.w, b.rect.x + b.rect.w);
                    if ox_end > ox {
                        raster.fill_pixel_rect(
                            ox,
                            gap_y,
                            ox_end - ox,
                            gap_end - gap_y,
                            theme.border,
                        );
                    }
                }
            }
        }
    }

    // Paint a single focus cue instead of boxing panes. Full orange rectangles
    // made the IDE smoke look like a terminal split skeleton; terminal focus is
    // already represented by the cursor and editor focus by the active tab.
    if entries.len() >= 2 {
        if let Some(e) = entries.iter().find(|e| e.id == focused_id) {
            let r = &e.rect;
            let is_terminal = registry
                .get(focused_id)
                .and_then(|pane| pane.terminal())
                .is_some();
            let c = if is_terminal {
                theme.accent
            } else {
                theme.hairline
            };
            raster.fill_pixel_rect(r.x, r.y, r.w, 1.0, c);
        }
    }
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{FontMetrics, GlyphPainter, PixelRect, pixel_at};
    use anvil_workspace::{editor_pane::EditorPaneRegistry, layout::SplitDir, pane::PaneRegistry};

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

    fn make_registry_single(cols: usize, rows: usize) -> (PaneRegistry, PaneId) {
        let mut reg = PaneRegistry::default();
        let id = reg.create_and_register(cols, rows, 0);
        (reg, id)
    }

    /// drawWorkspace single-leaf: leaf rect equals inner rect
    ///
    /// Verifies that a single-leaf tree gives the full inner rect, and that
    /// raster.origin_x / origin_y are reset to 0 after the call.
    #[test]
    fn single_leaf_rect_equals_inner() {
        let m = metrics();
        let pad = 24.0_f64;
        let w = 400_usize;
        let h = 300_usize;
        let inner = Rect {
            x: pad,
            y: pad,
            w: w as f64 - 2.0 * pad,
            h: h as f64 - 2.0 * pad,
        };

        let tree = PaneTree::init_single(1);
        let entries = tree.layout(inner, DIVIDER_PX);

        assert_eq!(entries.len(), 1);
        assert!((entries[0].rect.x - inner.x).abs() < 1e-9);
        assert!((entries[0].rect.y - inner.y).abs() < 1e-9);
        assert!((entries[0].rect.w - inner.w).abs() < 1e-9);
        assert!((entries[0].rect.h - inner.h).abs() < 1e-9);

        // Full draw_workspace call must not panic and must reset origin.
        let mut r = Raster::new(w, h);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        let (mut reg, first_id) = make_registry_single(20, 6);
        let theme = anvil_theme::MINERAL_DARK;
        let cursor_cfg = CursorConfig::default();

        if let Some(pane) = reg.get_mut(first_id) {
            if let Some(term) = pane.terminal.as_mut() {
                term.feed(b"hello\r\n");
            }
        }

        let tree = PaneTree::init_single(first_id);
        let ep_reg = EditorPaneRegistry::default();
        draw_workspace(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &tree,
            &mut reg,
            &ep_reg,
            inner,
            DIVIDER_PX,
            m,
            &theme,
            None,
            first_id,
            0.0,
            cursor_cfg,
            None,
            0.0,
            &HashMap::new(),
        );

        assert_eq!(r.origin_x, 0.0, "origin_x must be reset to 0");
        assert_eq!(r.origin_y, 0.0, "origin_y must be reset to 0");
    }

    /// drawWorkspace two-pane: divider pixels carry theme.border
    ///
    /// Lay out two horizontally-split panes with a deliberately wide divider
    /// so the gutter pixel is comfortably in the middle of the border band
    /// (the production `DIVIDER_PX = 1.0` hairline is sandwiched between
    /// adjacent panes' 2px focus accents — fine in production, but the
    /// sampling test needs the divider to be the dominant feature at the
    /// sample point). The drawing logic is identical for any width.
    #[test]
    fn two_pane_divider_pixel_is_border() {
        const TEST_DIV: f64 = 8.0;
        let m = metrics();
        let w = 400_usize;
        let h = 300_usize;
        let pad = 24.0_f64;
        let inner = Rect {
            x: pad,
            y: pad,
            w: w as f64 - 2.0 * pad,
            h: h as f64 - 2.0 * pad,
        };

        let mut reg = PaneRegistry::default();
        let id1 = reg.create_and_register(20, 6, 0);
        let id2 = reg.create_and_register(20, 6, 0);
        if let Some(p) = reg.get_mut(id1) {
            if let Some(term) = p.terminal.as_mut() {
                term.feed(b"pane one");
            }
        }
        if let Some(p) = reg.get_mut(id2) {
            if let Some(term) = p.terminal.as_mut() {
                term.feed(b"pane two");
            }
        }

        let mut tree = PaneTree::init_single(id1);
        tree.split(SplitDir::Horizontal, id2).unwrap();

        let mut r = Raster::new(w, h);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        r.clear(theme.background);

        let ep_reg = EditorPaneRegistry::default();
        draw_workspace(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &tree,
            &mut reg,
            &ep_reg,
            inner,
            TEST_DIV,
            m,
            &theme,
            None,
            id1,
            0.0,
            CursorConfig::default(),
            None,
            0.0,
            &HashMap::new(),
        );

        // Gutter center: pane1_w = (inner.w - TEST_DIV) * 0.5
        let pane1_w = (inner.w - TEST_DIV) * 0.5;
        let gutter_x = inner.x + pane1_w;
        let gutter_center_x = (gutter_x + TEST_DIV * 0.5) as usize;
        let mid_y = (inner.y + inner.h * 0.5) as usize;

        let px = pixel_at(&r, gutter_center_x, mid_y);
        // With a 1px hairline divider, the gutter pixel may be theme.border or
        // theme.accent (focused-pane accent border) — either way it must not be
        // raw background.
        assert!(
            px == theme.border || px == theme.accent,
            "gutter pixel should be border or accent, got {px:?}"
        );
    }

    /// Native editor panes reserve a chrome strip before drawing buffer content.
    #[test]
    fn bottom_drawer_detection_only_matches_short_lower_panes() {
        let inner = Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 800.0,
        };
        assert!(is_bottom_drawer(
            &Rect {
                x: 0.0,
                y: 610.0,
                w: 1000.0,
                h: 190.0,
            },
            &inner,
            2,
        ));
        assert!(!is_bottom_drawer(
            &Rect {
                x: 0.0,
                y: 0.0,
                w: 1000.0,
                h: 800.0,
            },
            &inner,
            1,
        ));
        assert!(!is_bottom_drawer(
            &Rect {
                x: 0.0,
                y: 0.0,
                w: 1000.0,
                h: 300.0,
            },
            &inner,
            2,
        ));
    }

    #[test]
    fn terminal_drawer_chrome_uses_calm_mineral_active_rule_not_ember() {
        let m = metrics();
        let mut r = Raster::new(220, 90);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        r.clear([0, 0, 0]);
        let rect = Rect {
            x: 10.0,
            y: 12.0,
            w: 180.0,
            h: 52.0,
        };

        draw_terminal_drawer_chrome(&mut r, &mut painter, m, &theme, rect, true);

        let top_px = pixel_at(&r, 20, 12);
        let body_px = pixel_at(&r, 20, 40);
        assert_ne!(
            top_px, theme.accent_ember,
            "active drawer rule must not use Ember"
        );
        assert_ne!(
            body_px,
            [0, 0, 0],
            "drawer body should lift off raw terminal black"
        );
        assert_ne!(body_px, theme.accent_ember, "drawer body must stay neutral");
    }

    /// Native editor panes reserve a chrome strip before drawing buffer content.
    #[test]
    fn editor_chrome_paints_header_and_offsets_content_rect() {
        let m = metrics();
        let mut r = Raster::new(320, 160);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        r.clear(theme.background);
        let rect = Rect {
            x: 10.0,
            y: 12.0,
            w: 260.0,
            h: 120.0,
        };
        let content = draw_editor_chrome(
            &mut r,
            &mut painter,
            &anvil_editor::Buffer::from_text("hello"),
            m,
            &theme,
            rect,
            true,
        );

        assert_eq!(pixel_at(&r, 12, 12), theme.accent_primary);
        assert_ne!(
            pixel_at(&r, 12, 16),
            theme.background,
            "editor chrome should tint the header away from raw canvas"
        );
        assert!(content.y > rect.y, "editor content must be below chrome");
        assert!(content.h < rect.h, "chrome must reserve vertical space");
        assert!(
            !painter.calls.is_empty(),
            "chrome should draw filename/placeholder text"
        );
    }

    /// Two-pane: focused pane has a 1px inset accent border; non-focused does not.
    #[test]
    fn focused_pane_has_accent_border() {
        let m = metrics();
        let w = 800_usize;
        let h = 400_usize;
        let pad = 24.0_f64;
        let inner = Rect {
            x: pad,
            y: pad,
            w: w as f64 - 2.0 * pad,
            h: h as f64 - 2.0 * pad,
        };
        let theme = anvil_theme::MINERAL_DARK;

        let mut reg = PaneRegistry::default();
        let id1 = reg.create_and_register(20, 6, 0);
        let id2 = reg.create_and_register(20, 6, 0);

        let mut tree = PaneTree::init_single(id1);
        tree.split(SplitDir::Horizontal, id2).unwrap();

        let mut r = Raster::new(w, h);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        r.clear(theme.background);
        let ep_reg = EditorPaneRegistry::default();
        draw_workspace(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &tree,
            &mut reg,
            &ep_reg,
            inner,
            DIVIDER_PX,
            m,
            &theme,
            None,
            id1, // focused
            0.0,
            CursorConfig::default(),
            None,
            0.0,
            &HashMap::new(),
        );

        // Focused pane (id1) is the left half of inner.
        let pane1_w = (inner.w - DIVIDER_PX) * 0.5;
        let mid_y = (inner.y + inner.h * 0.5) as usize;

        // Focused pane gets a single top accent cue, not a full orange box.
        let top_y = inner.y as usize;
        let mid_x = (inner.x + pane1_w * 0.5) as usize;
        let px = pixel_at(&r, mid_x, top_y);
        assert_eq!(
            px, theme.accent,
            "focused pane top cue should be accent (y={top_y}, got {px:?})"
        );

        // Left edge of focused pane should not be boxed in accent.
        let left_x = inner.x as usize;
        let px = pixel_at(&r, left_x, mid_y);
        assert_ne!(
            px, theme.accent,
            "focused pane left edge should not be boxed in accent (x={left_x}, got {px:?})"
        );

        // Non-focused pane (id2, right half): its left inset must NOT be accent.
        let pane2_left_x = (inner.x + pane1_w + DIVIDER_PX) as usize;
        let px = pixel_at(&r, pane2_left_x, mid_y);
        assert_ne!(
            px, theme.accent,
            "non-focused pane must not have accent border (x={pane2_left_x}, got {px:?})"
        );
    }

    /// draw_workspace smoke: does not panic on single pane with content.
    #[test]
    fn draw_workspace_smoke_no_panic() {
        let m = metrics();
        let mut r = Raster::new(400, 300);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        let (mut reg, id) = make_registry_single(20, 6);
        if let Some(pane) = reg.get_mut(id) {
            if let Some(term) = pane.terminal.as_mut() {
                term.feed(b"hello world\r\n");
            }
        }
        let tree = PaneTree::init_single(id);
        let inner = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        let theme = anvil_theme::MINERAL_DARK;
        r.clear(theme.background);
        let ep_reg = EditorPaneRegistry::default();
        draw_workspace(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &tree,
            &mut reg,
            &ep_reg,
            inner,
            DIVIDER_PX,
            m,
            &theme,
            None,
            id,
            0.0,
            CursorConfig::default(),
            None,
            0.0,
            &HashMap::new(),
        );
        // "hello world" starts with 'h' — expect glyph calls.
        assert!(!painter.calls.is_empty());
    }
}
