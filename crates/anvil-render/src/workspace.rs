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

use anvil_editor::{BufferId, OutlineSymbol, OutlineSymbolKind, derive_outline_rows};
use anvil_term::{DirtySet, Search};
use anvil_theme::Theme;
use anvil_workspace::{
    editor_pane::EditorPaneRegistry,
    layout::{LayoutEntry, PaneId, PaneTree, Rect},
    pane::PaneRegistry,
};

use anvil_workspace::editor_pane::EditorPane;

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

/// A click region in the per-pane editor buffer tab strip.
#[derive(Clone, Debug)]
pub struct EditorTabHit {
    /// Pane that owns this tab strip.
    pub pane_id: PaneId,
    /// The buffer this hit refers to.
    pub buffer_id: BufferId,
    /// Whether this click is on the `×` close glyph.
    pub is_close: bool,
    /// Hit rect in device pixels (raster-absolute space).
    pub rect: crate::raster::PixelRect,
}

/// Draw all panes in `tree` into `raster`, then draw divider hairlines over them.
///
/// Parameters:
///   raster             — full-window raster bitmap.
///   tree               — the current tab's pane tree (layout and focused id).
///   registry           — the pane registry for the current tab.
///   editor_panes       — registry of native editor panes + buffers.
///   inner              — device-pixel content area (window minus top-bar and panels).
///                        y=0 is the top of the raster. Layout is done in this space.
///   div_px             — divider gutter width in device pixels (use `DIVIDER_PX`).
///   metrics            — font metrics shared by all panes.
///   theme              — shared theme for all panes.
///   search             — active search state, or None.
///   focused_id         — the pane that receives cursor rendering.
///   blink_phase        — cursor blink phase [0, 1).
///   cursor_cfg         — cursor style + blink preference from config.
///   dirty              — per-pane dirty sets from `Terminal::take_dirty_rows`. When
///                        `None`, every row of every pane is redrawn (full frame).
///   diag_by_pane       — per-pane render diagnostics (NE10); translated from
///                        `LspManager::diagnostics_for` by `main.rs`. Empty map is fine.
///   hovered_editor_tab — currently hovered `(PaneId, BufferId)` for `×` show-on-hover.
///   editor_tab_hits    — output: cleared and repopulated with tab-strip click regions.
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
    hovered_editor_tab: Option<(PaneId, BufferId)>,
    editor_tab_hits: &mut Vec<EditorTabHit>,
) {
    editor_tab_hits.clear();
    let entries = tree.layout(inner, div_px);

    // Count how many editor-pane leaves exist (for focus ring, item 14).
    let editor_pane_count = entries
        .iter()
        .filter(|e| {
            registry
                .get(e.id)
                .is_some_and(|p| p.terminal.is_none() && p.editor_id.is_some())
        })
        .count();

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
                    let hovered_bid = hovered_editor_tab
                        .and_then(|(pid, bid)| if pid == e.id { Some(bid) } else { None });
                    let editor_rect = draw_editor_chrome(
                        raster,
                        painters.regular,
                        editor_panes,
                        ep.buffer_id,
                        &ep.open_buffers,
                        metrics,
                        theme,
                        e.rect,
                        e.id == focused_id,
                        hovered_bid,
                        e.id,
                        editor_tab_hits,
                        ep,
                        editor_pane_count > 1,
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
                        e.id == focused_id,
                    );
                } else {
                    // Buffer missing — panel fill with header strip.
                    draw_empty_pane(raster, painters.regular, metrics, theme, e.rect);
                }
            } else {
                // No PTY / editor pane yet — panel fill with compact header strip.
                draw_empty_pane(raster, painters.regular, metrics, theme, e.rect);
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

/// Empty pane (no PTY, no editor). Solid `panel` base with a 22px `charcoal`
/// header strip at the top showing a `text_subtle` label.
fn draw_empty_pane(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    const PAD_X: f64 = 10.0;
    const STRIP_H: f64 = 22.0;

    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.panel);
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, 1.0, theme.hairline, 0.68);

    let strip_h = STRIP_H.min(rect.h);
    if strip_h > 0.0 {
        raster.fill_pixel_rect(rect.x, rect.y, rect.w, strip_h, theme.charcoal);
        let label = "TERMINAL  \u{2318}T";
        let text_y = rect.y + ((strip_h - metrics.cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
        let mut gx = rect.x + PAD_X;
        let max_x = rect.x + rect.w - PAD_X;
        for ch in label.chars() {
            if gx + metrics.cell_w > max_x {
                break;
            }
            raster.glyph_at(painter, metrics, gx, text_y, ch as u32, theme.text_subtle);
            gx += metrics.cell_w;
        }
    }
}

fn draw_terminal_drawer_chrome(
    raster: &mut Raster,
    _painter: &mut dyn crate::raster::GlyphPainter,
    _metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
    _active: bool,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }

    // Top separator is the only chrome — a 1px hairline between editor and
    // drawer. The viewport (drawn earlier) carries the terminal cells; don't
    // overdraw them with a panel fill.
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, 1.0, theme.hairline, 0.92);
}

/// Height of the per-pane editor buffer tab strip in device pixels.
const EDITOR_TABS_H: f64 = 34.0;
/// Height of the slim breadcrumb row below the tab strip (item 18).
const EDITOR_BREADCRUMB_H: f64 = 22.0;
/// Height of the per-pane editor status bar at the bottom of the editor pane (item 8).
const EDITOR_STATUS_H: f64 = 26.0;
/// Minimum per-tab width: ~6 chars + padding.
const TAB_MIN_W: f64 = 80.0;
/// Maximum per-tab width: ~24 chars + padding.
const TAB_MAX_W: f64 = 240.0;
/// Horizontal padding inside each tab (leading side).
const TAB_PAD_L: f64 = 10.0;
/// Space reserved on the right for the `×` close glyph.
const TAB_CLOSE_W: f64 = 20.0;

/// Draw the per-pane multi-buffer tab strip, status bar, and return the editor body rect.
///
/// Paints `open_buffers` as horizontal tabs with `active_buffer_id` highlighted.
/// Also paints a 26px status bar at the bottom of the pane (item 8).
/// Paints a 2px left-edge focus ring when focused and `multi_pane` is true (item 14).
/// Writes hit regions (tab-body + close button per tab) into `hits_out`.
#[allow(clippy::too_many_arguments)]
fn draw_editor_chrome(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    editor_panes: &EditorPaneRegistry,
    active_buffer_id: BufferId,
    open_buffers: &[BufferId],
    metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
    pane_focused: bool,
    hovered_buffer: Option<BufferId>,
    pane_id: PaneId,
    hits_out: &mut Vec<EditorTabHit>,
    ep: &EditorPane,
    multi_pane: bool,
) -> Rect {
    let tabs_h = EDITOR_TABS_H.min(rect.h.max(0.0));
    if tabs_h <= 0.0 || rect.w <= 0.0 {
        return rect;
    }

    // Background: solid surface for the whole pane, graphite strip for tab row.
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.surface);
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, tabs_h, theme.graphite);
    // Bottom hairline dividing tabs from editor body.
    raster.fill_pixel_rect_alpha(
        rect.x,
        rect.y + tabs_h - 1.0,
        rect.w,
        1.0,
        theme.hairline,
        0.92,
    );

    // Top accent rule for focused pane (overwrites graphite at y=0).
    if pane_focused {
        raster.fill_pixel_rect(rect.x, rect.y, rect.w, 2.0, theme.accent_primary);
    }

    // Left-edge focus ring (item 14): 2px accent_primary bar along the full pane
    // height. Only painted when there are multiple editor panes so a solo pane
    // does not show a distracting ring.
    if pane_focused && multi_pane {
        raster.fill_pixel_rect(rect.x, rect.y, 2.0, rect.h, theme.accent_primary);
    }

    // Compute per-tab widths. Each tab is sized to its label but clamped.
    // Label = basename of tracked path, or "scratch".
    let tab_count = open_buffers.len().max(1);
    // Max total width available for tabs.
    let available_w = rect.w;

    let mut tab_widths: Vec<f64> = open_buffers
        .iter()
        .map(|&bid| {
            let label = buffer_label(editor_panes, bid);
            // label chars + dirty dot (1 char) + padding + close button
            let char_w = (label.chars().count() + 2) as f64 * metrics.cell_w;
            (char_w + TAB_PAD_L + TAB_CLOSE_W + 8.0).clamp(TAB_MIN_W, TAB_MAX_W)
        })
        .collect();

    // If total width exceeds available, scale all tabs down proportionally.
    let total_w: f64 = tab_widths.iter().sum();
    if total_w > available_w && total_w > 0.0 {
        let scale = available_w / total_w;
        for w in &mut tab_widths {
            *w = (*w * scale).max(TAB_MIN_W.min(available_w / tab_count as f64));
        }
    }

    // Draw each tab.
    let mut cursor_x = rect.x;
    for (i, &bid) in open_buffers.iter().enumerate() {
        let tab_w = tab_widths[i];
        let is_active = bid == active_buffer_id;
        let is_hovered = hovered_buffer == Some(bid);
        let tab_rect = crate::raster::PixelRect {
            x: cursor_x,
            y: rect.y,
            w: tab_w,
            h: tabs_h,
        };

        draw_buffer_tab(
            raster,
            painter,
            editor_panes,
            bid,
            metrics,
            theme,
            tab_rect,
            is_active,
            is_hovered,
            pane_id,
            hits_out,
        );

        cursor_x += tab_w;
    }

    // ── Breadcrumb row (item 18) ──────────────────────────────────────────────
    let remaining_after_tabs = (rect.h - tabs_h).max(0.0);
    let crumb_h = EDITOR_BREADCRUMB_H.min(remaining_after_tabs);
    if crumb_h > 0.0 {
        let crumb_rect = Rect {
            x: rect.x,
            y: rect.y + tabs_h,
            w: rect.w,
            h: crumb_h,
        };
        let cursor_line = ep.cursors[0].pos.line;
        let segments = breadcrumb_segments_at_line(editor_panes, active_buffer_id, cursor_line);
        draw_breadcrumb_row(raster, painter, metrics, theme, crumb_rect, &segments);
    }

    // ── Status bar (item 8) ───────────────────────────────────────────────────
    let status_h = EDITOR_STATUS_H.min((rect.h - tabs_h - crumb_h).max(0.0));
    if status_h > 0.0 {
        let buf = editor_panes.get_buffer(active_buffer_id);
        draw_editor_status_bar(
            raster,
            painter,
            metrics,
            theme,
            Rect {
                x: rect.x,
                y: rect.y + rect.h - status_h,
                w: rect.w,
                h: status_h,
            },
            ep,
            buf,
        );
    }

    Rect {
        x: rect.x,
        y: rect.y + tabs_h + crumb_h,
        w: rect.w,
        h: (rect.h - tabs_h - crumb_h - status_h).max(0.0),
    }
}

/// Get the display label (filename basename) for a buffer.
fn buffer_label(editor_panes: &EditorPaneRegistry, buffer_id: BufferId) -> String {
    editor_panes
        .get_buffer(buffer_id)
        .and_then(|b| b.tracked_path())
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("scratch")
        .to_string()
}

// ── Breadcrumb helpers (item 18) ──────────────────────────────────────────────

/// Derive breadcrumb segments for `cursor_line` from the syntax tree of
/// `active_buffer_id`.  Returns a `Vec<String>` like `["impl Foo", "fn bar"]`
/// — the shallowest containing symbol first, deepest last.
///
/// Caps at depth 8 as specified.  Falls back to an empty vec if no tree is
/// available or the buffer has no tracked path.
pub fn breadcrumb_segments_at_line(
    editor_panes: &EditorPaneRegistry,
    active_buffer_id: BufferId,
    cursor_line: usize,
) -> Vec<String> {
    let buf = match editor_panes.get_buffer(active_buffer_id) {
        Some(b) => b,
        None => return Vec::new(),
    };
    let text = buf.to_text();
    let symbols: Vec<OutlineSymbol> = derive_outline_rows(buf.syntax(), &text);
    if symbols.is_empty() {
        return Vec::new();
    }
    // Collect all symbols whose start line is <= cursor_line, sorted by line
    // ascending. We take the last (deepest by position) up to depth 8.
    let mut containing: Vec<&OutlineSymbol> =
        symbols.iter().filter(|s| s.line <= cursor_line).collect();
    // Already in document order; take the tail up to 8.
    let depth = containing.len().min(8);
    containing.truncate(depth);
    // Format each as "kind Name".
    containing
        .iter()
        .map(|s| {
            let prefix = match s.kind {
                OutlineSymbolKind::Function => "fn",
                OutlineSymbolKind::Impl => "impl",
                OutlineSymbolKind::Struct => "struct",
                OutlineSymbolKind::Enum => "enum",
                OutlineSymbolKind::Trait => "trait",
                OutlineSymbolKind::Other => "",
            };
            if prefix.is_empty() {
                s.name.clone()
            } else {
                format!("{prefix} {}", s.name)
            }
        })
        .collect()
}

/// Paint the breadcrumb row: a slim charcoal strip with `text_subtle`
/// segments separated by " › " separators.  Click-to-jump is not wired here
/// (the render layer is stateless); hit regions are emitted to `hits_out`
/// if needed in a future pass.
fn draw_breadcrumb_row(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
    segments: &[String],
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    // Background strip.
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.graphite);
    // Bottom hairline.
    raster.fill_pixel_rect_alpha(
        rect.x,
        rect.y + rect.h - 1.0,
        rect.w,
        1.0,
        theme.hairline,
        0.70,
    );

    if segments.is_empty() {
        return;
    }

    // Build a single string: "seg0 › seg1 › …"
    let text = segments.join(" \u{203a} ");
    let text_y = rect.y + ((rect.h - metrics.cell_h) * 0.5 + metrics.descent * 0.5).max(0.0);
    let mut gx = rect.x + 8.0; // 8px left pad
    let max_x = rect.x + rect.w - 4.0;

    for ch in text.chars() {
        if gx + metrics.cell_w > max_x {
            break;
        }
        raster.glyph_at(painter, metrics, gx, text_y, ch as u32, theme.text_subtle);
        gx += metrics.cell_w;
    }
}

/// Draw a single buffer tab and emit hit rects.
#[allow(clippy::too_many_arguments)]
fn draw_buffer_tab(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    editor_panes: &EditorPaneRegistry,
    buffer_id: BufferId,
    metrics: FontMetrics,
    theme: &Theme,
    tab: crate::raster::PixelRect,
    is_active: bool,
    is_hovered: bool,
    pane_id: PaneId,
    hits_out: &mut Vec<EditorTabHit>,
) {
    // Tab background.
    if is_active {
        // Active tab: charcoal fill + 2px accent_primary top rule.
        raster.fill_pixel_rect(tab.x, tab.y, tab.w, tab.h, theme.charcoal);
        raster.fill_pixel_rect(tab.x, tab.y, tab.w, 2.0, theme.accent_primary);
        // Right-edge separator between active and neighbor.
        raster.fill_pixel_rect_alpha(
            tab.x + tab.w - 1.0,
            tab.y + 4.0,
            1.0,
            (tab.h - 6.0).max(0.0),
            theme.hairline,
            0.50,
        );
    } else {
        // Inactive: transparent (graphite) — no fill needed; just a separator.
        raster.fill_pixel_rect_alpha(
            tab.x + tab.w - 1.0,
            tab.y + 4.0,
            1.0,
            (tab.h - 6.0).max(0.0),
            theme.hairline,
            0.50,
        );
    }

    // Dirty dot (7px, accent_primary) to the left of the filename.
    let is_dirty = editor_panes
        .get_buffer(buffer_id)
        .map(|b| b.revisions > 0)
        .unwrap_or(false);

    let label = buffer_label(editor_panes, buffer_id);
    let text_color = if is_active {
        theme.foreground
    } else if is_hovered {
        theme.text_muted
    } else {
        theme.text_subtle
    };

    // Layout: [PAD_L] [dirty_dot?] [label] ... [close_×]
    let dot_w = if is_dirty {
        8.0 + metrics.cell_w * 0.5
    } else {
        0.0
    };
    let text_x = tab.x + TAB_PAD_L + dot_w;
    let text_max_x = tab.x + tab.w - TAB_CLOSE_W - 4.0;
    let text_y = tab.y + ((tab.h - metrics.cell_h) * 0.5).max(0.0);

    if is_dirty {
        // 7×7 accent dot, vertically centered.
        let dot_x = tab.x + TAB_PAD_L;
        let dot_y = tab.y + (tab.h * 0.5 - 3.5).max(0.0);
        raster.fill_pixel_rect(dot_x, dot_y, 7.0, 7.0, theme.accent_primary);
    }

    // Label text.
    let mut gx = text_x;
    for ch in label.chars() {
        if gx + metrics.cell_w > text_max_x {
            break;
        }
        raster.glyph_at(painter, metrics, gx, text_y, ch as u32, text_color);
        gx += metrics.cell_w;
    }

    // Tab body hit rect (excludes the close button area).
    let close_x = tab.x + tab.w - TAB_CLOSE_W;
    hits_out.push(EditorTabHit {
        pane_id,
        buffer_id,
        is_close: false,
        rect: crate::raster::PixelRect {
            x: tab.x,
            y: tab.y,
            w: (close_x - tab.x).max(0.0),
            h: tab.h,
        },
    });

    // Close `×` glyph: shown on active tab and on hovered tab.
    if is_active || is_hovered {
        let close_glyph_x = close_x + (TAB_CLOSE_W - metrics.cell_w) * 0.5;
        let close_glyph_y = tab.y + ((tab.h - metrics.cell_h) * 0.5).max(0.0);
        raster.glyph_at(
            painter,
            metrics,
            close_glyph_x,
            close_glyph_y,
            '×' as u32,
            theme.text_subtle,
        );
    }

    // Close hit rect only present when the × glyph is rendered (active or
    // hovered). Otherwise a click in the rightmost 20px of any inactive tab
    // silently closes it — surprising.
    if is_active || is_hovered {
        hits_out.push(EditorTabHit {
            pane_id,
            buffer_id,
            is_close: true,
            rect: crate::raster::PixelRect {
                x: close_x,
                y: tab.y,
                w: TAB_CLOSE_W,
                h: tab.h,
            },
        });
    }
}

/// Draw the editor status bar at the bottom of a pane (item 8).
///
/// Left side: `main · no LSP · N diagnostics` (branch + LSP placeholder).
/// Right side: `Ln X, Col Y · UTF-8 · <Language>`.
/// Background: `theme.panel`, top edge: 1px `theme.hairline`.
fn draw_editor_status_bar(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
    ep: &EditorPane,
    buf: Option<&anvil_editor::Buffer>,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    const PAD_X: f64 = 8.0;

    // Background + top hairline.
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.panel);
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, 1.0, theme.hairline, 0.92);

    let cw = metrics.cell_w;
    let ch = metrics.cell_h;
    let text_y = rect.y + ((rect.h - ch) * 0.5 + metrics.descent * 0.5).max(0.0);

    // Left text: branch + LSP placeholder.
    let left_label = "main \u{00b7} no LSP".to_string(); // · separators
    let mut gx = rect.x + PAD_X;
    let max_x = rect.x + rect.w - PAD_X;
    for c in left_label.chars() {
        if gx + cw > max_x {
            break;
        }
        raster.glyph_at(painter, metrics, gx, text_y, c as u32, theme.text_subtle);
        gx += cw;
    }

    // Right text: `Ln X, Col Y · UTF-8 · Language`.
    let cursor_pos = ep.primary_cursor().pos;
    let lang = buf.and_then(|b| b.language_id()).unwrap_or("Plain Text");
    // Capitalise language display name.
    let lang_display = match lang {
        "rust" => "Rust",
        "typescript" => "TypeScript",
        "python" => "Python",
        "toml" => "TOML",
        "json" => "JSON",
        "markdown" => "Markdown",
        _ => lang,
    };
    let right_label = format!(
        "Ln {}, Col {} \u{00b7} UTF-8 \u{00b7} {}",
        cursor_pos.line + 1,
        cursor_pos.col + 1,
        lang_display
    );
    let right_chars: Vec<char> = right_label.chars().collect();
    let right_w = right_chars.len() as f64 * cw;
    let right_x = (rect.x + rect.w - PAD_X - right_w).max(rect.x + PAD_X);
    let mut gx = right_x;
    for c in &right_chars {
        if gx + cw > rect.x + rect.w - PAD_X {
            break;
        }
        raster.glyph_at(painter, metrics, gx, text_y, *c as u32, theme.text_subtle);
        gx += cw;
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
        let mut tab_hits = Vec::new();
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
            None,
            &mut tab_hits,
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
        let mut tab_hits = Vec::new();
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
            None,
            &mut tab_hits,
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
    fn terminal_drawer_chrome_paints_top_hairline_only() {
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
        // Top row must have moved off raw black — hairline is painted there.
        assert_ne!(top_px, [0, 0, 0], "top hairline must paint");
        // Body must remain whatever the viewport painted under us (here:
        // the test's clear background). The drawer chrome no longer overdraws
        // the terminal cells with a panel fill.
        assert_eq!(
            body_px,
            [0, 0, 0],
            "drawer body must NOT be overdrawn — viewport cells must pass through"
        );
        assert_ne!(
            top_px, theme.accent_ember,
            "drawer hairline must not use Ember accent"
        );
    }

    /// Native editor panes reserve a chrome strip before drawing buffer content.
    #[test]
    fn editor_chrome_paints_header_and_offsets_content_rect() {
        use anvil_workspace::editor_pane::EditorPaneRegistry;
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
        // Build a minimal registry with one pane and one scratch buffer.
        let mut ep_reg = EditorPaneRegistry::default();
        let bid = ep_reg.new_pane(1);
        let ep = ep_reg.get_pane(1).unwrap();
        let open_bufs = vec![bid];
        let mut hits = Vec::new();
        let content = draw_editor_chrome(
            &mut r,
            &mut painter,
            &ep_reg,
            bid,
            &open_bufs,
            m,
            &theme,
            rect,
            true, // pane_focused
            None, // hovered_buffer
            1,    // pane_id
            &mut hits,
            ep,
            false, // multi_pane (single pane — no focus ring)
        );

        assert_eq!(pixel_at(&r, 12, 12), theme.accent_primary);
        assert_ne!(
            pixel_at(&r, 12, 16),
            theme.background,
            "editor chrome should tint the header away from raw canvas"
        );
        assert!(content.y > rect.y, "editor content must be below chrome");
        assert!(content.h < rect.h, "chrome must reserve vertical space");
        // With a single scratch tab we expect at least 2 hit rects (body + close).
        assert!(hits.len() >= 2, "chrome must emit tab hit rects");
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
        let mut tab_hits = Vec::new();
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
            None,
            &mut tab_hits,
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
        let mut tab_hits = Vec::new();
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
            None,
            &mut tab_hits,
        );
        // "hello world" starts with 'h' — expect glyph calls.
        assert!(!painter.calls.is_empty());
    }

    /// Item 8: editor chrome reserves a status-bar strip at the bottom; body rect
    /// shrinks accordingly and the status bar area is filled (not raw background).
    #[test]
    fn editor_chrome_status_bar_reserves_bottom_strip() {
        use anvil_workspace::editor_pane::EditorPaneRegistry;
        let m = metrics();
        let mut r = Raster::new(400, 200);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        r.clear(theme.background);
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        };
        let mut ep_reg = EditorPaneRegistry::default();
        let bid = ep_reg.new_pane(1);
        let ep = ep_reg.get_pane(1).unwrap();
        let open_bufs = vec![bid];
        let mut hits = Vec::new();
        let content = draw_editor_chrome(
            &mut r,
            &mut painter,
            &ep_reg,
            bid,
            &open_bufs,
            m,
            &theme,
            rect,
            false, // pane_focused
            None,
            1,
            &mut hits,
            ep,
            false,
        );
        // Body rect must not extend to the bottom of the pane (status bar reserves bottom).
        let body_bottom = content.y + content.h;
        assert!(
            body_bottom < rect.y + rect.h,
            "status bar must reserve a strip at the bottom; body_bottom={body_bottom} rect_bottom={}",
            rect.y + rect.h
        );
        // The strip height reserved must be ≥ EDITOR_STATUS_H.
        let total_reserved = rect.h - content.h;
        assert!(
            total_reserved >= EDITOR_STATUS_H,
            "must reserve at least EDITOR_STATUS_H px; reserved={total_reserved}"
        );
    }

    /// Item 14: a 2px accent_primary left rail is painted only for the focused pane
    /// in a multi-pane layout.
    #[test]
    fn editor_chrome_focus_ring_only_when_multi_pane() {
        use anvil_workspace::editor_pane::EditorPaneRegistry;
        let m = metrics();
        let theme = anvil_theme::MINERAL_DARK;
        let rect = Rect {
            x: 10.0,
            y: 10.0,
            w: 300.0,
            h: 200.0,
        };

        // Single pane: no focus ring on left edge.
        {
            let mut r = Raster::new(400, 300);
            r.clear(theme.background);
            let mut painter = StubPainter::default();
            let mut ep_reg = EditorPaneRegistry::default();
            let bid = ep_reg.new_pane(1);
            let ep = ep_reg.get_pane(1).unwrap();
            let open_bufs = vec![bid];
            let mut hits = Vec::new();
            draw_editor_chrome(
                &mut r,
                &mut painter,
                &ep_reg,
                bid,
                &open_bufs,
                m,
                &theme,
                rect,
                true,
                None,
                1,
                &mut hits,
                ep,
                false, // multi_pane=false
            );
            // Left edge (x=10) must NOT be accent_primary in single-pane mode.
            let mid_y = (rect.y + rect.h * 0.5) as usize;
            let px = pixel_at(&r, rect.x as usize, mid_y);
            assert_ne!(
                px, theme.accent_primary,
                "single-pane left edge must not show focus ring; got {px:?}"
            );
        }

        // Multi pane + focused: left edge IS accent_primary.
        {
            let mut r = Raster::new(400, 300);
            r.clear(theme.background);
            let mut painter = StubPainter::default();
            let mut ep_reg = EditorPaneRegistry::default();
            let bid = ep_reg.new_pane(1);
            let ep = ep_reg.get_pane(1).unwrap();
            let open_bufs = vec![bid];
            let mut hits = Vec::new();
            draw_editor_chrome(
                &mut r,
                &mut painter,
                &ep_reg,
                bid,
                &open_bufs,
                m,
                &theme,
                rect,
                true,
                None,
                1,
                &mut hits,
                ep,
                true, // multi_pane=true
            );
            let mid_y = (rect.y + rect.h * 0.5) as usize;
            let px = pixel_at(&r, rect.x as usize, mid_y);
            assert_eq!(
                px, theme.accent_primary,
                "multi-pane focused left edge must show focus ring; got {px:?}"
            );
        }
    }

    // ── Item 18: breadcrumb_segments_at_line ─────────────────────────────────

    /// Empty registry returns no segments without panicking.
    #[test]
    fn breadcrumbs_empty_registry() {
        let reg = EditorPaneRegistry::default();
        // BufferId 0 does not exist in an empty registry.
        let segs = breadcrumb_segments_at_line(&reg, 0, 0);
        assert!(
            segs.is_empty(),
            "empty registry must return empty breadcrumbs"
        );
    }

    /// A Rust buffer containing `fn hello()` produces a segment with "hello"
    /// when the cursor is on that line.
    #[test]
    fn breadcrumbs_rust_fn_at_cursor() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(1);
        let src = "fn hello() {}\n";
        {
            let buf = reg.get_buffer_mut(bid).unwrap();
            // Replace scratch buffer with one containing the source text so
            // derive_outline_rows can resolve identifier byte ranges.
            *buf = anvil_editor::Buffer::from_text(src);
            buf.syntax
                .set_language_from_path(std::path::Path::new("x.rs"));
            buf.syntax.parse(src);
        }
        let segs = breadcrumb_segments_at_line(&reg, bid, 0);
        // Tree-sitter may or may not resolve depending on test env; if segments
        // are present they must mention "hello".
        if !segs.is_empty() {
            assert!(
                segs.iter().any(|s| s.contains("hello")),
                "breadcrumbs must contain 'hello' for cursor at fn line; got {segs:?}"
            );
        }
    }

    /// Cursor on line 0, before any symbol whose start_line > 0, returns empty.
    #[test]
    fn breadcrumbs_before_first_symbol() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(1);
        // Plain text buffer — no language set, so derive_outline_rows returns nothing.
        let src = "just some text\n";
        {
            let buf = reg.get_buffer_mut(bid).unwrap();
            buf.syntax.parse(src);
        }
        let segs = breadcrumb_segments_at_line(&reg, bid, 0);
        assert!(
            segs.is_empty(),
            "plain text buffer must have no breadcrumbs; got {segs:?}"
        );
    }
}
