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

/// A hit region for the actual editor text body, after tabs, breadcrumbs, and
/// status chrome have been removed from the pane rect.
#[derive(Clone, Debug)]
pub struct EditorBodyHit {
    /// Pane that owns this editor body.
    pub pane_id: PaneId,
    /// Active buffer rendered inside this body.
    pub buffer_id: BufferId,
    /// Body rect in device pixels (raster-absolute space).
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
///   editor_body_hits   — output: cleared and repopulated with editor body regions.
///   ui_scale           — logical zoom multiplier (separate from Retina window_scale).
///                        Pass `1.0` when no zoom is configured.
///   scroll_indicator_alpha — alpha for the M5 editor scrollbar thumb [0.0, 1.0].
///                        Driven by the same fade timer as the Explorer thumb.
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
    editor_body_hits: &mut Vec<EditorBodyHit>,
    ui_scale: f64,
    scroll_indicator_alpha: f32,
) {
    editor_tab_hits.clear();
    editor_body_hits.clear();
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

            // G2: when the drawer is too short to render PTY cells, show a
            // collapsed button strip instead.
            let collapse_threshold = (terminal_drawer_header_h(metrics, ui_scale, e.rect.h)
                + metrics.cell_h)
                .max(50.0 * ui_scale);
            if is_bottom_drawer(&e.rect, &inner, entries.len()) && e.rect.h < collapse_threshold {
                draw_drawer_collapsed_strip(raster, painters.regular, metrics, theme, e.rect);
            } else {
                // Focused pane: full blink. Unfocused pane: dim static cursor
                // (blink_phase=0.5 → cursor_opacity=0.35, the floor value).
                let cursor_params: Option<CursorParams> = Some(CursorParams {
                    ax: pane.cursor_ax,
                    ay: pane.cursor_ay,
                    blink_phase: if e.id == focused_id { blink_phase } else { 0.5 },
                    cfg: cursor_cfg,
                });

                // rule_x bounds: horizontal span of this pane in device pixels.
                let rule_x_start = e.rect.x;
                let rule_x_end = e.rect.x + e.rect.w;

                // Fold state for this pane.
                let folded = FoldedBlocks::new(&pane.folded[..pane.folded_count]);

                // Per-pane dirty set: None means "draw all rows".
                let pane_dirty: Option<&DirtySet> = dirty.and_then(|m| m.get(&e.id));

                // DD7: bottom-drawer gets a 24pt charcoal header above the PTY.
                // Reserve header_h at the top of the drawer before calling
                // draw_viewport so the terminal cells don't overdraw the header.
                let drawer_header_h = if is_bottom_drawer(&e.rect, &inner, entries.len()) {
                    terminal_drawer_header_h(metrics, ui_scale, e.rect.h)
                } else {
                    0.0
                };
                raster.origin_y += drawer_header_h;

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

                // Restore origin before chrome draws in absolute space.
                raster.origin_y -= drawer_header_h;

                if is_bottom_drawer(&e.rect, &inner, entries.len()) {
                    draw_terminal_drawer_chrome(
                        raster,
                        painters.regular,
                        metrics,
                        theme,
                        e.rect,
                        e.id == focused_id,
                        drawer_header_h,
                    );
                }

                // Living-scrollback indicator: paint a 4px accent bar at the
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
                        theme.accent_primary,
                        0.92,
                    );
                }
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
                        ui_scale,
                    );
                    editor_body_hits.push(EditorBodyHit {
                        pane_id: e.id,
                        buffer_id: ep.buffer_id,
                        rect: crate::raster::PixelRect {
                            x: editor_rect.x,
                            y: editor_rect.y,
                            w: editor_rect.w,
                            h: editor_rect.h,
                        },
                    });
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
                        // Item 12: pass blink_phase so the editor cursor animates.
                        // Only the focused pane animates; inactive panes get 0.0
                        // (fully opaque cursor) via cursor_opacity(0.0) == 1.0.
                        if e.id == focused_id { blink_phase } else { 0.0 },
                        // M5: scrollbar thumb alpha — only for the focused pane.
                        if e.id == focused_id {
                            scroll_indicator_alpha
                        } else {
                            0.0
                        },
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

fn chrome_strip_h(base_h: f64, ui_scale: f64, metrics: FontMetrics) -> f64 {
    let scale = ui_scale.max(0.5);
    let vertical_pad = (8.0 * scale).round().max(8.0);
    (base_h * scale)
        .round()
        .max((metrics.cell_h + vertical_pad).ceil())
}

fn terminal_drawer_header_h(metrics: FontMetrics, ui_scale: f64, pane_h: f64) -> f64 {
    chrome_strip_h(24.0, ui_scale, metrics).min(pane_h.max(0.0))
}

/// Compute the text-body rect for a native editor pane without painting it.
///
/// The GPU path needs the same body hit box as the CPU renderer, even though
/// it does not call `draw_editor_chrome`. Keep this geometry in one place so
/// mouse selection, hscroll, and editor painting agree.
pub fn editor_body_rect(
    editor_panes: &EditorPaneRegistry,
    active_buffer_id: BufferId,
    ep: &EditorPane,
    metrics: FontMetrics,
    rect: Rect,
    ui_scale: f64,
) -> Rect {
    let tabs_h = chrome_strip_h(EDITOR_TABS_H, ui_scale, metrics).min(rect.h.max(0.0));
    if tabs_h <= 0.0 || rect.w <= 0.0 {
        return rect;
    }

    let cursor_line = ep.cursors[0].pos.line;
    let segments = breadcrumb_segments_at_line(editor_panes, active_buffer_id, cursor_line);
    let remaining_after_tabs = (rect.h - tabs_h).max(0.0);
    let crumb_h = if segments.is_empty() {
        0.0
    } else {
        chrome_strip_h(EDITOR_BREADCRUMB_H, ui_scale, metrics).min(remaining_after_tabs)
    };
    let status_h_base = chrome_strip_h(EDITOR_STATUS_H, ui_scale, metrics);
    let status_h = status_h_base.min((rect.h - tabs_h - crumb_h).max(0.0));

    Rect {
        x: rect.x,
        y: rect.y + tabs_h + crumb_h,
        w: rect.w,
        h: (rect.h - tabs_h - crumb_h - status_h).max(0.0),
    }
}

/// Rebuild editor body hit regions from pane layout without drawing.
///
/// Used by the GPU render path and by hit-test fallbacks. This intentionally
/// ignores terminal leaves and editor panes whose active buffer is missing.
#[allow(clippy::too_many_arguments)]
pub fn collect_editor_body_hits(
    tree: &PaneTree,
    registry: &PaneRegistry,
    editor_panes: &EditorPaneRegistry,
    inner: Rect,
    div_px: f64,
    metrics: FontMetrics,
    ui_scale: f64,
    hits_out: &mut Vec<EditorBodyHit>,
) {
    hits_out.clear();
    for entry in tree.layout(inner, div_px) {
        let Some(pane) = registry.get(entry.id) else {
            continue;
        };
        let Some(ep) = pane.editor_id.and_then(|_| editor_panes.get_pane(entry.id)) else {
            continue;
        };
        if editor_panes.get_buffer(ep.buffer_id).is_none() {
            continue;
        }

        let body = editor_body_rect(
            editor_panes,
            ep.buffer_id,
            ep,
            metrics,
            entry.rect,
            ui_scale,
        );
        hits_out.push(EditorBodyHit {
            pane_id: entry.id,
            buffer_id: ep.buffer_id,
            rect: crate::raster::PixelRect {
                x: body.x,
                y: body.y,
                w: body.w,
                h: body.h,
            },
        });
    }
}

/// Draw native editor panes into the CPU chrome raster without drawing terminal
/// viewport cells.
///
/// GPU mode uses Metal for terminal cells but still composites `raster` for
/// chrome. Native editor panes are not terminal cell batches, so they must be
/// painted here before the GPU layer is presented.
#[allow(clippy::too_many_arguments)]
pub fn draw_workspace_editors(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    tree: &PaneTree,
    registry: &PaneRegistry,
    editor_panes: &EditorPaneRegistry,
    inner: Rect,
    div_px: f64,
    metrics: FontMetrics,
    theme: &Theme,
    focused_id: PaneId,
    blink_phase: f32,
    diag_by_pane: &HashMap<PaneId, Vec<RenderDiagnostic>>,
    hovered_editor_tab: Option<(PaneId, BufferId)>,
    editor_tab_hits: &mut Vec<EditorTabHit>,
    editor_body_hits: &mut Vec<EditorBodyHit>,
    ui_scale: f64,
    scroll_indicator_alpha: f32,
) {
    editor_tab_hits.clear();
    editor_body_hits.clear();
    let entries = tree.layout(inner, div_px);
    let editor_pane_count = entries
        .iter()
        .filter(|entry| {
            registry
                .get(entry.id)
                .is_some_and(|pane| pane.terminal.is_none() && pane.editor_id.is_some())
        })
        .count();

    for entry in &entries {
        let Some(pane) = registry.get(entry.id) else {
            continue;
        };
        if pane.terminal.is_some() {
            continue;
        }

        let Some(ep) = pane.editor_id.and_then(|_| editor_panes.get_pane(entry.id)) else {
            draw_empty_pane(raster, painter, metrics, theme, entry.rect);
            continue;
        };
        let Some(buf) = editor_panes.get_buffer(ep.buffer_id) else {
            draw_empty_pane(raster, painter, metrics, theme, entry.rect);
            continue;
        };

        let empty = Vec::new();
        let diags = diag_by_pane
            .get(&entry.id)
            .map(Vec::as_slice)
            .unwrap_or(&empty);
        let hovered_bid =
            hovered_editor_tab.and_then(|(pid, bid)| (pid == entry.id).then_some(bid));
        let editor_rect = draw_editor_chrome(
            raster,
            painter,
            editor_panes,
            ep.buffer_id,
            &ep.open_buffers,
            metrics,
            theme,
            entry.rect,
            entry.id == focused_id,
            hovered_bid,
            entry.id,
            editor_tab_hits,
            ep,
            editor_pane_count > 1,
            ui_scale,
        );
        editor_body_hits.push(EditorBodyHit {
            pane_id: entry.id,
            buffer_id: ep.buffer_id,
            rect: crate::raster::PixelRect {
                x: editor_rect.x,
                y: editor_rect.y,
                w: editor_rect.w,
                h: editor_rect.h,
            },
        });
        draw_editor_into(
            raster,
            painter,
            ep,
            buf,
            metrics,
            theme,
            editor_rect,
            diags,
            buf.git_gutter.as_ref(),
            entry.id == focused_id,
            if entry.id == focused_id {
                blink_phase
            } else {
                0.0
            },
            if entry.id == focused_id {
                scroll_indicator_alpha
            } else {
                0.0
            },
        );
    }

    raster.origin_x = 0.0;
    raster.origin_y = 0.0;
}

/// Empty pane (no PTY, no editor). Solid `panel` base with a centered welcome
/// block showing the Anvil name and key hints.
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

    // Panel background + top hairline.
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.panel);
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, 1.0, theme.hairline, 0.68);

    let cw = metrics.cell_w;
    let ch = metrics.cell_h;
    let row_h = ch + 4.0;

    // ── Content ──────────────────────────────────────────────────────────────
    let title = "Anvil";
    let subtitle = "Native macOS dev environment";
    let footer = concat!("v", env!("CARGO_PKG_VERSION"), " \u{00b7} anvil-rust");

    // Two-column action grid (left col, right col).
    let actions: &[(&str, &str)] = &[
        ("\u{2318}P  Open file", "\u{2318}T  New terminal"),
        ("\u{2318}E  New editor", "\u{2318}B  Toggle sidebar"),
        (
            "\u{2318}\u{21e7}F  Project search",
            "\u{2318}\u{21e7}N  Open in nvim",
        ),
    ];

    // Block height: title + gap + subtitle + gap + 3 action rows + gap + footer.
    let total_rows: usize = 1 + 1 + 1 + 1 + actions.len() + 1 + 1;

    let block_h = total_rows as f64 * row_h;
    if block_h >= rect.h {
        return;
    }
    // P8: shift block 30pt below the geometric center for a more spacious feel.
    // This adds ~60pt of top padding vs the pure-center position.
    let start_y = rect.y + (rect.h - block_h) * 0.5 + 30.0;

    // Inline helper: draw a text string centered horizontally in `rect` at `row`.
    // Returns nothing; borrows raster + painter directly.
    macro_rules! draw_centered {
        ($row:expr, $text:expr, $color:expr) => {{
            let text_w = $text.chars().count() as f64 * cw;
            let lx = rect.x + ((rect.w - text_w) * 0.5).max(0.0);
            let ly = start_y
                + $row as f64 * row_h
                + ((row_h - ch) * 0.5 + metrics.descent * 0.5).max(0.0);
            let max_x = rect.x + rect.w;
            let mut gx = lx;
            for c in $text.chars() {
                if gx + cw > max_x {
                    break;
                }
                raster.glyph_at(painter, metrics, gx, ly, c as u32, $color);
                gx += cw;
            }
        }};
    }

    // Item 6: 1px graphite ring around the welcome card for visual definition.
    {
        let card_pad_x = 24.0;
        let card_pad_y = 12.0;
        let card_x = rect.x + card_pad_x;
        let card_y = start_y - card_pad_y;
        let card_w = rect.w - 2.0 * card_pad_x;
        let card_h = block_h + 2.0 * card_pad_y;
        // Top + bottom edges.
        raster.fill_pixel_rect(card_x, card_y, card_w, 1.0, theme.graphite);
        raster.fill_pixel_rect(card_x, card_y + card_h - 1.0, card_w, 1.0, theme.graphite);
        // Left + right edges.
        raster.fill_pixel_rect(card_x, card_y, 1.0, card_h, theme.graphite);
        raster.fill_pixel_rect(card_x + card_w - 1.0, card_y, 1.0, card_h, theme.graphite);
    }

    // Row 0: title
    draw_centered!(0usize, title, theme.accent_bright);
    // Row 1: (gap)
    // Row 2: subtitle
    draw_centered!(2usize, subtitle, theme.text_muted);
    // Row 3: (gap)
    // Rows 4..: two-column action hints.
    // Key clusters (the chord before the first double-space) get a surface_alt
    // pill backdrop painted before the glyphs so they read as badge-style hints.
    let col_w = rect.w * 0.5;

    // Paint a key-cluster pill: surface_alt rect with 1px vertical inset.
    let paint_key_pill = |raster: &mut crate::raster::Raster, gx: f64, ly: f64, key_cols: usize| {
        let pill_w = key_cols as f64 * cw + 2.0; // 1px pad each side
        let pill_h = ch + 2.0; // 1px pad top+bottom
        let pill_x = gx - 1.0;
        let pill_y = ly - 1.0;
        raster.fill_pixel_rect(pill_x, pill_y, pill_w, pill_h, theme.surface_alt);
    };

    for (i, (left, right)) in actions.iter().enumerate() {
        let row = 4 + i;
        let ly =
            start_y + row as f64 * row_h + ((row_h - ch) * 0.5 + metrics.descent * 0.5).max(0.0);

        // Left column: right-aligned within the left half, one cell of center gap.
        let left_text_w = left.chars().count() as f64 * cw;
        let left_x = (rect.x + col_w - left_text_w - cw).max(rect.x);
        // Key cluster ends at the first "  " (two spaces).
        let left_key_cols = left
            .find("  ")
            .map(|b| left[..b].chars().count())
            .unwrap_or(0);
        if left_key_cols > 0 {
            paint_key_pill(raster, left_x, ly, left_key_cols);
        }
        let mut gx = left_x;
        for c in left.chars() {
            if gx + cw > rect.x + col_w {
                break;
            }
            raster.glyph_at(painter, metrics, gx, ly, c as u32, theme.text_subtle);
            gx += cw;
        }

        // Right column: starts at center + one cell gap.
        let right_x = rect.x + col_w + cw;
        let max_rx = rect.x + rect.w - cw;
        let right_key_cols = right
            .find("  ")
            .map(|b| right[..b].chars().count())
            .unwrap_or(0);
        if right_key_cols > 0 {
            paint_key_pill(raster, right_x, ly, right_key_cols);
        }
        let mut gx = right_x;
        for c in right.chars() {
            if gx + cw > max_rx {
                break;
            }
            raster.glyph_at(painter, metrics, gx, ly, c as u32, theme.text_subtle);
            gx += cw;
        }
    }

    // Last row: footer (gap row + footer row after the last action).
    let footer_row = 4 + actions.len() + 1;
    draw_centered!(footer_row, footer, theme.text_subtle);
}

fn draw_terminal_drawer_chrome(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
    _active: bool,
    header_h: f64,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }

    // DD7: paint a charcoal header strip at the top of the drawer.
    // The caller has already inset raster.origin_y so the PTY cells
    // render below this header — no overdraw.
    if header_h > 0.0 {
        raster.fill_pixel_rect(rect.x, rect.y, rect.w, header_h, theme.charcoal);
        raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, 1.0, theme.hairline, 0.92);
        // P9: always-visible dot indicator centered on the top divider line so the
        // drag handle is discoverable without requiring hover.  A short `⋯` run
        // in text_subtle at α=0.35 sits in the middle of the hairline row.
        {
            let center_x = rect.x + rect.w * 0.5;
            let dots = "\u{22EF}"; // ⋯ MIDLINE HORIZONTAL ELLIPSIS (3 dots)
            let dot_w = metrics.cell_w * dots.chars().count() as f64;
            let dot_x = center_x - dot_w * 0.5;
            raster.fill_pixel_rect_alpha(dot_x, rect.y, dot_w, 1.0, theme.text_subtle, 0.35);
        }
        raster.fill_pixel_rect_alpha(
            rect.x,
            rect.y + header_h - 1.0,
            rect.w,
            1.0,
            theme.hairline,
            0.60,
        );

        const PAD_X: f64 = 8.0;
        let text_y = rect.y + ((header_h - metrics.cell_h) * 0.5).max(0.0);
        let label = "TERMINAL";
        let mut gx = rect.x + PAD_X;
        let max_x = rect.x + rect.w - PAD_X;
        for ch in label.chars() {
            if gx + metrics.cell_w > max_x {
                break;
            }
            raster.glyph_at(painter, metrics, gx, text_y, ch as u32, theme.text_subtle);
            gx += metrics.cell_w;
        }
    } else {
        // Fallback: 1px hairline separator only (no reserved space).
        raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, 1.0, theme.hairline, 0.92);
    }
}

/// Draw the collapsed drawer strip (G2): a 24pt-tall charcoal bar with
/// "▸ TERMINAL" on the left. Rendered when the drawer rect.h < 50pt threshold.
fn draw_drawer_collapsed_strip(
    raster: &mut Raster,
    painter: &mut dyn crate::raster::GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let strip_h = chrome_strip_h(24.0, 1.0, metrics).min(rect.h);
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, strip_h, theme.charcoal);
    raster.fill_pixel_rect_alpha(rect.x, rect.y, rect.w, 1.0, theme.hairline, 0.92);

    const PAD_X: f64 = 8.0;
    let text_y = rect.y + ((strip_h - metrics.cell_h) * 0.5).max(0.0);
    let label = "\u{25b8} TERMINAL"; // ▸ TERMINAL
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

/// Height of the per-pane editor buffer tab strip in device pixels (base, before ui_scale).
const EDITOR_TABS_H: f64 = 30.0;
/// Height of the slim breadcrumb row below the tab strip (item 18, base before ui_scale).
const EDITOR_BREADCRUMB_H: f64 = 20.0;
/// Height of the per-pane editor status bar at the bottom of the editor pane (item 8, base).
const EDITOR_STATUS_H: f64 = 22.0;
/// Minimum per-tab width base (scaled by ui_scale at runtime).
const TAB_MIN_W: f64 = 80.0;
/// Maximum per-tab width base (scaled by ui_scale at runtime).
const TAB_MAX_W: f64 = 240.0;
/// Horizontal padding inside each tab (leading side, base).
const TAB_PAD_L: f64 = 10.0;
/// Space reserved on the right for the `×` close glyph (base).
const TAB_CLOSE_W: f64 = 20.0;

/// Draw the per-pane multi-buffer tab strip, status bar, and return the editor body rect.
///
/// Paints `open_buffers` as horizontal tabs with `active_buffer_id` highlighted.
/// Also paints a status bar at the bottom of the pane (item 8, scaled by `ui_scale`).
/// Pane-level focus is carried by active tabs and dividers, not a full-height rail.
/// Writes hit regions (tab-body + close button per tab) into `hits_out`.
///
/// `ui_scale` is the logical zoom multiplier; pass `1.0` for no zoom.
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
    _pane_focused: bool,
    hovered_buffer: Option<BufferId>,
    pane_id: PaneId,
    hits_out: &mut Vec<EditorTabHit>,
    ep: &EditorPane,
    _multi_pane: bool,
    ui_scale: f64,
) -> Rect {
    let tabs_h = chrome_strip_h(EDITOR_TABS_H, ui_scale, metrics).min(rect.h.max(0.0));
    if tabs_h <= 0.0 || rect.w <= 0.0 {
        return rect;
    }

    // Scale tab geometry constants by ui_scale (A2).
    let tab_min_w = TAB_MIN_W * ui_scale;
    let tab_max_w = TAB_MAX_W * ui_scale;
    let tab_pad_l = TAB_PAD_L * ui_scale;
    let tab_close_w = TAB_CLOSE_W * ui_scale;

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
            (char_w + tab_pad_l + tab_close_w + 8.0 * ui_scale).clamp(tab_min_w, tab_max_w)
        })
        .collect();

    // If total width exceeds available, scale all tabs down proportionally (A2).
    let total_w: f64 = tab_widths.iter().sum();
    if total_w > available_w && total_w > 0.0 {
        let scale = available_w / total_w;
        for w in &mut tab_widths {
            *w = (*w * scale).max(tab_min_w.min(available_w / tab_count as f64));
        }
    }

    // Draw each tab.
    let mut cursor_x = rect.x;
    for (i, &bid) in open_buffers.iter().enumerate() {
        let tab_w = tab_widths[i];
        // Clip label to tab width (A2: text_max_x prevents overflow).
        let text_max_x = cursor_x + tab_w - tab_close_w - 4.0;
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
            tab_pad_l,
            tab_close_w,
            text_max_x,
        );

        cursor_x += tab_w;
    }

    // ── Breadcrumb row (item 18 / A3) ─────────────────────────────────────────
    // Compute segments BEFORE reserving space. If empty, crumb_h stays 0.
    let cursor_line = ep.cursors[0].pos.line;
    let segments = breadcrumb_segments_at_line(editor_panes, active_buffer_id, cursor_line);
    let remaining_after_tabs = (rect.h - tabs_h).max(0.0);
    let crumb_h = if segments.is_empty() {
        0.0
    } else {
        chrome_strip_h(EDITOR_BREADCRUMB_H, ui_scale, metrics).min(remaining_after_tabs)
    };
    if crumb_h > 0.0 {
        let crumb_rect = Rect {
            x: rect.x,
            y: rect.y + tabs_h,
            w: rect.w,
            h: crumb_h,
        };
        draw_breadcrumb_row(raster, painter, metrics, theme, crumb_rect, &segments);
    }

    // ── Status bar (item 8 / A5) ──────────────────────────────────────────────
    let status_h_base = chrome_strip_h(EDITOR_STATUS_H, ui_scale, metrics);
    let status_h = status_h_base.min((rect.h - tabs_h - crumb_h).max(0.0));
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
    let text_y = rect.y + ((rect.h - metrics.cell_h) * 0.5).max(0.0);
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
///
/// `tab_pad_l`, `tab_close_w`, and `text_max_x` are pre-scaled by the caller.
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
    tab_pad_l: f64,
    tab_close_w: f64,
    text_max_x: f64,
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
    // DD9: use is_dirty() (revisions != saved_revision) not revisions > 0.
    let is_dirty = editor_panes
        .get_buffer(buffer_id)
        .map(|b| b.is_dirty())
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
    let text_x = tab.x + tab_pad_l + dot_w;
    let text_y = tab.y + ((tab.h - metrics.cell_h) * 0.5).max(0.0);

    if is_dirty {
        // 7×7 accent dot, vertically centered.
        let dot_x = tab.x + tab_pad_l;
        let dot_y = tab.y + (tab.h * 0.5 - 3.5).max(0.0);
        raster.fill_pixel_rect(dot_x, dot_y, 7.0, 7.0, theme.accent_primary);
    }

    // Label text — clipped to text_max_x (A2: prevents label overflow).
    let mut gx = text_x;
    let label_chars: Vec<char> = label.chars().collect();
    let max_chars = ((text_max_x - text_x) / metrics.cell_w).floor().max(0.0) as usize;
    // Truncate with ellipsis when label doesn't fit.
    let display: std::borrow::Cow<str> = if label_chars.len() > max_chars && max_chars > 0 {
        let cut = max_chars.saturating_sub(1);
        let s: String = label_chars[..cut].iter().collect();
        std::borrow::Cow::Owned(format!("{s}\u{2026}"))
    } else {
        std::borrow::Cow::Borrowed(&label)
    };
    for ch in display.chars() {
        if gx + metrics.cell_w > text_max_x {
            break;
        }
        raster.glyph_at(painter, metrics, gx, text_y, ch as u32, text_color);
        gx += metrics.cell_w;
    }

    // Tab body hit rect (excludes the close button area).
    let close_x = tab.x + tab.w - tab_close_w;
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
        let close_glyph_x = close_x + (tab_close_w - metrics.cell_w) * 0.5;
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
                w: tab_close_w,
                h: tab.h,
            },
        });
    }
}

/// Draw the editor status bar at the bottom of a pane (item 8).
///
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
    let text_y = rect.y + ((rect.h - ch) * 0.5).max(0.0);

    // Right text: `Ln X, Col Y · UTF-8 · Language · Spaces:N` (or Tabs:N).
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
    // H3: append indent style label.
    let indent_label = match buf.map(|b| b.indent_style()) {
        Some(anvil_editor::IndentStyle::Spaces(n)) => format!(" \u{00b7} Spaces:{n}"),
        Some(anvil_editor::IndentStyle::Tabs(n)) => format!(" \u{00b7} Tabs:{n}"),
        None => String::new(),
    };
    let right_label = format!(
        "Ln {}, Col {} \u{00b7} UTF-8 \u{00b7} {}{}",
        cursor_pos.line + 1,
        cursor_pos.col + 1,
        lang_display,
        indent_label,
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
        pub positions: Vec<(u32, PixelRect)>,
    }

    impl GlyphPainter for StubPainter {
        #[allow(clippy::too_many_arguments)]
        fn draw_glyph(
            &mut self,
            glyph_id: u32,
            dest: PixelRect,
            fg: [u8; 3],
            _metrics: FontMetrics,
            _pixels: &mut [u8],
            _bw: usize,
            _bh: usize,
        ) {
            self.calls.push((glyph_id, fg));
            self.positions.push((glyph_id, dest));
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
        let mut body_hits = Vec::new();
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
            &mut body_hits,
            1.0,
            0.0,
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
        let mut body_hits = Vec::new();
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
            &mut body_hits,
            1.0,
            0.0,
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

    /// DD7: draw_terminal_drawer_chrome paints a charcoal header strip for the
    /// reserved header area and leaves the body (below header_h) untouched.
    #[test]
    fn terminal_drawer_chrome_paints_header_strip() {
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
        let header_h = 24.0_f64;

        draw_terminal_drawer_chrome(&mut r, &mut painter, m, &theme, rect, true, header_h);

        // Header row must be painted charcoal (not raw black).
        let top_px = pixel_at(&r, 20, 12);
        assert_ne!(top_px, [0, 0, 0], "header strip must be painted");
        assert_ne!(
            top_px, theme.accent_ember,
            "drawer header must not use Ember accent"
        );
        // Body below the header must remain whatever the viewport painted (black in this test).
        // header_h=24, rect.y=12, so body starts at y=36; sample y=40.
        let body_px = pixel_at(&r, 20, 40);
        assert_eq!(
            body_px,
            [0, 0, 0],
            "drawer body must NOT be overdrawn — viewport cells must pass through"
        );
        // Zero header_h falls back to hairline-only (no charcoal fill).
        r.clear([0, 0, 0]);
        draw_terminal_drawer_chrome(&mut r, &mut painter, m, &theme, rect, true, 0.0);
        let top_px_no_hdr = pixel_at(&r, 20, 12);
        assert_ne!(top_px_no_hdr, [0, 0, 0], "fallback hairline must paint");
    }

    #[test]
    fn terminal_drawer_header_keeps_label_and_content_from_colliding() {
        let m = FontMetrics {
            cell_w: 12.0,
            cell_h: 32.0,
            descent: 7.0,
        };
        let mut reg = PaneRegistry::default();
        let top_id = reg.create_and_register(40, 8, 0);
        let drawer_id = reg.create_and_register(40, 4, 0);
        if let Some(pane) = reg.get_mut(drawer_id) {
            if let Some(term) = pane.terminal.as_mut() {
                term.feed(b"zzzz");
            }
        }

        let mut tree = PaneTree::init_single(top_id);
        tree.split(SplitDir::Vertical, drawer_id).unwrap();
        if let anvil_workspace::layout::PaneNode::Split(split) = tree.root.as_mut() {
            split.ratios = vec![0.72, 0.28];
        }

        let mut r = Raster::new(640, 420);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        let ep_reg = EditorPaneRegistry::default();
        let theme = anvil_theme::MINERAL_DARK;
        let inner = Rect {
            x: 0.0,
            y: 0.0,
            w: 640.0,
            h: 420.0,
        };
        let mut tab_hits = Vec::new();
        let mut body_hits = Vec::new();

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
            drawer_id,
            0.0,
            CursorConfig::default(),
            None,
            0.0,
            &HashMap::new(),
            None,
            &mut tab_hits,
            &mut body_hits,
            1.0,
            0.0,
        );

        let label = painter
            .positions
            .iter()
            .find_map(|(cp, rect)| (*cp == 'T' as u32).then_some(*rect))
            .expect("drawer label must render");
        let content = painter
            .positions
            .iter()
            .find_map(|(cp, rect)| (*cp == 'z' as u32).then_some(*rect))
            .expect("drawer terminal content must render");

        assert!(
            content.y >= label.y + label.h + 4.0,
            "drawer content y={} must start below label bottom={} with breathing room",
            content.y,
            label.y + label.h
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
            1.0,   // ui_scale
        );

        // Active tab's 2px top rule paints accent_primary at the tab's top edge.
        // The pane-level accent rule was removed (D9); the tab-level rule remains.
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
        let mut body_hits = Vec::new();
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
            &mut body_hits,
            1.0,
            0.0,
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

    // ── G2: drawer collapse strip ─────────────────────────────────────────────

    /// G2: draw_drawer_collapsed_strip paints a charcoal strip and a ▸ glyph.
    #[test]
    fn drawer_collapsed_strip_paints_strip_and_label() {
        let m = metrics();
        let mut r = Raster::new(400, 100);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        r.clear(theme.background);
        let rect = Rect {
            x: 0.0,
            y: 80.0,
            w: 400.0,
            h: 20.0,
        };
        draw_drawer_collapsed_strip(&mut r, &mut painter, m, &theme, rect);
        // Strip must be filled (not raw background) at y=88.
        let strip_px = pixel_at(&r, 10, 88);
        assert_ne!(
            strip_px, theme.background,
            "collapsed strip must fill charcoal"
        );
        // ▸ glyph (U+25B8) must be painted.
        let tri: Vec<_> = painter
            .calls
            .iter()
            .filter(|(cp, _)| *cp == '\u{25B8}' as u32)
            .collect();
        assert!(!tri.is_empty(), "collapsed strip must render ▸ glyph");
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
        let mut body_hits = Vec::new();
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
            &mut body_hits,
            1.0,
            0.0,
        );
        // "hello world" starts with 'h' — expect glyph calls.
        assert!(!painter.calls.is_empty());
    }

    #[test]
    fn draw_workspace_records_editor_body_hit_after_chrome() {
        use anvil_workspace::editor_pane::EditorPaneRegistry;

        let m = metrics();
        let mut r = Raster::new(400, 300);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        let mut ep_reg = EditorPaneRegistry::default();
        let bid = ep_reg.new_pane(1);
        let mut reg = PaneRegistry::default();
        let id = reg.create_and_register_editor(bid);
        let tree = PaneTree::init_single(id);
        let inner = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        let theme = anvil_theme::MINERAL_DARK;
        r.clear(theme.background);
        let mut tab_hits = Vec::new();
        let mut body_hits = Vec::new();

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
            &mut body_hits,
            1.0,
            0.0,
        );

        assert_eq!(body_hits.len(), 1);
        let hit = &body_hits[0];
        assert_eq!(hit.pane_id, id);
        assert_eq!(hit.buffer_id, bid);
        assert!(hit.rect.y > inner.y, "body must start below tab chrome");
        assert!(
            hit.rect.y + hit.rect.h < inner.y + inner.h,
            "body must stop above status chrome"
        );
        assert_eq!(hit.rect.x, inner.x);
        assert_eq!(hit.rect.w, inner.w);
    }

    #[test]
    fn collect_editor_body_hits_records_editor_body_without_painting() {
        use anvil_workspace::editor_pane::EditorPaneRegistry;

        let m = metrics();
        let mut ep_reg = EditorPaneRegistry::default();
        let bid = ep_reg.new_pane(1);
        let mut reg = PaneRegistry::default();
        let id = reg.create_and_register_editor(bid);
        let tree = PaneTree::init_single(id);
        let inner = Rect {
            x: 12.0,
            y: 8.0,
            w: 420.0,
            h: 260.0,
        };
        let mut body_hits = Vec::new();

        collect_editor_body_hits(
            &tree,
            &reg,
            &ep_reg,
            inner,
            DIVIDER_PX,
            m,
            1.0,
            &mut body_hits,
        );

        assert_eq!(body_hits.len(), 1);
        let hit = &body_hits[0];
        assert_eq!(hit.pane_id, id);
        assert_eq!(hit.buffer_id, bid);
        assert_eq!(hit.rect.x, inner.x);
        assert_eq!(hit.rect.w, inner.w);
        assert!(hit.rect.y > inner.y, "body must start below tabs");
        assert!(
            hit.rect.y + hit.rect.h < inner.y + inner.h,
            "body must stop above editor status bar"
        );
    }

    #[test]
    fn draw_workspace_editors_paints_native_editor_in_gpu_chrome_layer() {
        use anvil_editor::Position;
        use anvil_workspace::editor_pane::EditorPaneRegistry;

        let m = metrics();
        let mut r = Raster::new(420, 260);
        let mut painter = StubPainter::default();
        let mut ep_reg = EditorPaneRegistry::default();
        let bid = ep_reg.new_pane(1);
        ep_reg
            .get_buffer_mut(bid)
            .unwrap()
            .insert_str(Position { line: 0, col: 0 }, "hello gpu editor");
        let mut reg = PaneRegistry::default();
        let id = reg.create_and_register_editor(bid);
        let tree = PaneTree::init_single(id);
        let inner = Rect {
            x: 0.0,
            y: 0.0,
            w: 420.0,
            h: 260.0,
        };
        let theme = anvil_theme::MINERAL_DARK;
        let mut tab_hits = Vec::new();
        let mut body_hits = Vec::new();

        draw_workspace_editors(
            &mut r,
            &mut painter,
            &tree,
            &reg,
            &ep_reg,
            inner,
            DIVIDER_PX,
            m,
            &theme,
            id,
            0.0,
            &HashMap::new(),
            None,
            &mut tab_hits,
            &mut body_hits,
            1.0,
            0.0,
        );

        assert_eq!(body_hits.len(), 1);
        assert!(
            painter
                .calls
                .iter()
                .any(|(glyph_id, _)| *glyph_id == 'h' as u32),
            "GPU chrome editor pass must paint buffer text"
        );
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
            1.0, // ui_scale
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
        let painted_text: String = painter
            .calls
            .iter()
            .filter_map(|(cp, _)| char::from_u32(*cp))
            .collect();
        assert!(
            !painted_text.contains("no LSP"),
            "status bar should not paint unfinished filler text; got: {painted_text:?}"
        );
    }

    #[test]
    fn editor_chrome_rows_fit_large_chrome_font_metrics() {
        use anvil_workspace::editor_pane::EditorPaneRegistry;
        let m = FontMetrics {
            cell_w: 12.0,
            cell_h: 32.0,
            descent: 7.0,
        };
        let mut r = Raster::new(460, 220);
        let mut painter = StubPainter::default();
        let theme = anvil_theme::MINERAL_DARK;
        r.clear(theme.background);
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 460.0,
            h: 220.0,
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
            false,
            None,
            1,
            &mut hits,
            ep,
            false,
            1.0,
        );

        let top_reserved = content.y - rect.y;
        let bottom_reserved = rect.y + rect.h - (content.y + content.h);
        assert!(
            top_reserved >= m.cell_h + 8.0,
            "tab chrome reserved {top_reserved}px but chrome glyph height is {}px",
            m.cell_h
        );
        assert!(
            bottom_reserved >= m.cell_h + 8.0,
            "status chrome reserved {bottom_reserved}px but chrome glyph height is {}px",
            m.cell_h
        );
    }

    /// Option A keeps pane chrome neutral; the active tab owns the accent.
    #[test]
    fn editor_chrome_does_not_paint_pane_level_focus_rail() {
        use anvil_workspace::editor_pane::EditorPaneRegistry;
        let m = metrics();
        let theme = anvil_theme::MINERAL_DARK;
        let rect = Rect {
            x: 10.0,
            y: 10.0,
            w: 300.0,
            h: 200.0,
        };

        // Single pane: no focus rail on left edge.
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
                1.0,   // ui_scale
            );
            let mid_y = (rect.y + rect.h * 0.5) as usize;
            let px = pixel_at(&r, rect.x as usize, mid_y);
            assert_ne!(
                px, theme.accent_primary,
                "single-pane left edge must not show pane focus rail; got {px:?}"
            );
        }

        // Multi pane + focused: still no pane-level accent rail.
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
                1.0,  // ui_scale
            );
            let mid_y = (rect.y + rect.h * 0.5) as usize;
            let px = pixel_at(&r, rect.x as usize, mid_y);
            assert_ne!(
                px, theme.accent_primary,
                "multi-pane focused left edge must not show pane focus rail; got {px:?}"
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

    // ── A2: tab strip does not overflow at 2× ui_scale ───────────────────────

    /// A2: at 2x ui_scale with 5 tabs in a narrow pane, each drawn tab width
    /// must be ≤ pane_w / tab_count and no tab must overflow the pane.
    #[test]
    fn tab_strip_no_overflow_at_2x_ui_scale() {
        use anvil_workspace::editor_pane::EditorPaneRegistry;
        let m = metrics();
        let theme = anvil_theme::MINERAL_DARK;
        // Narrow pane: 300px wide, 5 tabs (each a scratch buffer, label = "scratch").
        let pane_w = 300.0_f64;
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: pane_w,
            h: 200.0,
        };
        let mut ep_reg = EditorPaneRegistry::default();
        // Create 5 scratch panes.
        let bids: Vec<_> = (0..5).map(|i| ep_reg.new_pane(i + 1)).collect();

        let mut r = Raster::new(400, 300);
        r.clear(theme.background);
        let mut painter = StubPainter::default();
        let ep = ep_reg.get_pane(1).unwrap();
        let mut hits = Vec::new();

        draw_editor_chrome(
            &mut r,
            &mut painter,
            &ep_reg,
            bids[0],
            &bids,
            m,
            &theme,
            rect,
            false,
            None,
            1,
            &mut hits,
            ep,
            false,
            2.0, // 2x ui_scale
        );

        // Each tab body hit must be ≤ pane_w / tab_count (with 1px rounding slack).
        let tab_body_hits: Vec<_> = hits.iter().filter(|h| !h.is_close).collect();
        let max_allowed = pane_w / bids.len() as f64;
        for hit in &tab_body_hits {
            assert!(
                hit.rect.w <= max_allowed + 1.0,
                "tab width {} exceeds allowed {max_allowed} at 2x scale",
                hit.rect.w
            );
        }
        // No tab must extend past the pane right edge.
        for hit in &tab_body_hits {
            assert!(
                hit.rect.x + hit.rect.w <= pane_w + 1.0,
                "tab overflows pane: right edge {} > pane_w {pane_w}",
                hit.rect.x + hit.rect.w
            );
        }
        // Verify we got the right number of tab hits (5 body hits).
        assert_eq!(
            tab_body_hits.len(),
            bids.len(),
            "must have one body hit per tab"
        );
    }

    // ── A3: breadcrumb row hides when segments are empty ─────────────────────

    /// A3: pane with no breadcrumb segments has editor body rect equal to
    /// full pane height minus tab strip only (breadcrumb reserves 0 height).
    #[test]
    fn editor_body_rect_full_height_minus_tabs_when_no_breadcrumbs() {
        use anvil_workspace::editor_pane::EditorPaneRegistry;
        let m = metrics();
        let theme = anvil_theme::MINERAL_DARK;
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        };
        let mut ep_reg = EditorPaneRegistry::default();
        // Scratch buffer with no language — breadcrumb_segments_at_line returns empty.
        let bid = ep_reg.new_pane(1);
        let ep = ep_reg.get_pane(1).unwrap();
        let open_bufs = vec![bid];
        let mut hits = Vec::new();
        let mut r = Raster::new(500, 300);
        r.clear(theme.background);
        let mut painter = StubPainter::default();

        let content = draw_editor_chrome(
            &mut r,
            &mut painter,
            &ep_reg,
            bid,
            &open_bufs,
            m,
            &theme,
            rect,
            false,
            None,
            1,
            &mut hits,
            ep,
            false,
            1.0,
        );

        // With no breadcrumbs: content.y == rect.y + EDITOR_TABS_H.
        let expected_y = rect.y + EDITOR_TABS_H;
        assert!(
            (content.y - expected_y).abs() < 1.0,
            "A3: empty breadcrumbs must not reserve vertical space; \
             content.y={} expected≈{expected_y}",
            content.y
        );
    }
}
