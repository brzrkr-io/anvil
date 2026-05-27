//! Custom overlay impls for the 4 overlays deferred from overlay Phase 3.
//!
//! These implement `CustomOverlay` from `anvil_render::overlay` and carry
//! snapshots of the relevant app state. They are created and pushed onto
//! `App::overlays` (replacing the legacy inline draws).
//!
//! Overlays:
//! - `ScmPanelOverlay` — Z1/Z2/Z3/Z5/Z12: SCM panel (centered card).
//! - `CompletionOverlay` — item 16: completion popup anchored to cursor cell.
//! - `CodeActionsOverlay` — item 25: code-actions picker anchored to cursor.
//! - `HoverOverlay` — NE10: hover popup anchored to cursor cell.

use std::path::PathBuf;

use anvil_render::overlay::{
    CardSize, CustomOverlay, OverlayClickResult, OverlayId, OverlayKey, OverlayKeyResult,
    OverlayMeasureCtx, OverlayRenderCtx,
};
use anvil_workspace::editor_pane::{CodeActionEntry, CompletionEntry};

// ── Shared row-draw helper ────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_row_text(
    ctx: &mut OverlayRenderCtx<'_>,
    label: &str,
    color: [u8; 3],
    panel_x: f64,
    panel_w: f64,
    pad_x: f64,
    y: f64,
    row_h: f64,
) -> f64 {
    let cw = ctx.metrics.cell_w;
    let glyph_y = y + 2.0;
    let mut rx = panel_x + pad_x;
    for ch in label.chars() {
        if rx + cw > panel_x + panel_w - pad_x {
            break;
        }
        ctx.raster
            .glyph_at(ctx.painter, ctx.metrics, rx, glyph_y, ch as u32, color);
        rx += cw;
    }
    y + row_h
}

// ── ScmPanelOverlay ───────────────────────────────────────────────────────────

/// Snapshot of one SCM file entry.
#[derive(Clone)]
pub struct ScmFileSnap {
    pub path: PathBuf,
    pub mark: char,
}

/// Snapshot of one stash entry.
#[derive(Clone)]
pub struct StashSnap {
    pub message: String,
}

/// Snapshot of one PR entry.
#[derive(Clone)]
pub struct PrSnap {
    pub number: u32,
    pub title: String,
}

/// SCM panel as a `CustomOverlay`.
///
/// Owns a snapshot of the scm_panel state so the overlay is self-contained.
/// Interaction (key handling) remains in main.rs for the existing scm_panel
/// state machine; this overlay only handles rendering.
pub struct ScmPanelOverlay {
    pub staged: Vec<ScmFileSnap>,
    pub unstaged: Vec<ScmFileSnap>,
    pub selected: usize,
    pub commit_msg: String,
    pub commit_input_active: bool,
    pub stashes: Vec<StashSnap>,
    pub stashes_expanded: bool,
    pub prs: Vec<PrSnap>,
    pub prs_expanded: bool,
}

impl CustomOverlay for ScmPanelOverlay {
    fn id(&self) -> OverlayId {
        OverlayId::ScmPanel
    }

    fn measure(&self, ctx: &OverlayMeasureCtx) -> CardSize {
        let cw = ctx.metrics.cell_w;
        let ch = ctx.metrics.cell_h;
        let row_h = ch + 4.0;
        let dh = ctx.dh;
        let dw = ctx.dw;

        let panel_w = (55.0 * cw).min(dw * 0.72);
        let header_rows = 2usize;
        let file_rows = self.staged.len() + self.unstaged.len();
        let stash_rows = if self.stashes_expanded {
            self.stashes.len()
        } else {
            0
        };
        let pr_rows = if self.prs_expanded {
            self.prs.len().min(8)
        } else {
            0
        };
        let commit_input_h = row_h + 4.0;
        let total_rows = header_rows + file_rows + 2 + stash_rows + pr_rows;
        let panel_h = (total_rows as f64 * row_h + commit_input_h + 8.0).min(dh * 0.8);

        CardSize {
            w: panel_w,
            h: panel_h,
        }
    }

    fn render(&self, ctx: &mut OverlayRenderCtx<'_>) {
        let cw = ctx.metrics.cell_w;
        let ch = ctx.metrics.cell_h;
        let theme = ctx.theme;
        let g = ctx.geom;
        let panel_x = g.x;
        let panel_y = g.y;
        let panel_w = g.w;
        let row_h = ch + 4.0;
        let pad_x = 1.5 * cw;

        let mut y = panel_y + 2.0;

        // STAGED section header.
        y = draw_row_text(
            ctx,
            "STAGED",
            theme.text_subtle,
            panel_x,
            panel_w,
            pad_x,
            y,
            row_h,
        );
        for (i, f) in self.staged.iter().enumerate() {
            let is_sel = i == self.selected;
            if is_sel {
                ctx.raster
                    .fill_pixel_rect_alpha(panel_x, y, panel_w, row_h, theme.accent, 0.14);
            }
            let name = f.path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            let label = format!("{} {name}", f.mark);
            let color = if is_sel {
                theme.foreground
            } else {
                theme.verified
            };
            y = draw_row_text(ctx, &label, color, panel_x, panel_w, pad_x, y, row_h);
        }

        // UNSTAGED section.
        ctx.raster.fill_pixel_rect(
            panel_x + pad_x,
            y,
            panel_w - pad_x * 2.0,
            1.0,
            theme.hairline,
        );
        y = draw_row_text(
            ctx,
            "UNSTAGED",
            theme.text_subtle,
            panel_x,
            panel_w,
            pad_x,
            y,
            row_h,
        );
        for (i, f) in self.unstaged.iter().enumerate() {
            let abs_i = self.staged.len() + i;
            let is_sel = abs_i == self.selected;
            if is_sel {
                ctx.raster
                    .fill_pixel_rect_alpha(panel_x, y, panel_w, row_h, theme.accent, 0.14);
            }
            let name = f.path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            let label = format!("{} {name}", f.mark);
            let color = if is_sel {
                theme.foreground
            } else {
                theme.attention
            };
            y = draw_row_text(ctx, &label, color, panel_x, panel_w, pad_x, y, row_h);
        }

        // STASHES section (Z5).
        ctx.raster.fill_pixel_rect(
            panel_x + pad_x,
            y,
            panel_w - pad_x * 2.0,
            1.0,
            theme.hairline,
        );
        let stash_label = if self.stashes_expanded {
            format!("v STASHES ({})", self.stashes.len())
        } else {
            format!("> STASHES ({})", self.stashes.len())
        };
        y = draw_row_text(
            ctx,
            &stash_label,
            theme.text_subtle,
            panel_x,
            panel_w,
            pad_x,
            y,
            row_h,
        );
        if self.stashes_expanded {
            for st in &self.stashes {
                let label = format!("  {}", &st.message);
                y = draw_row_text(
                    ctx,
                    &label,
                    theme.text_muted,
                    panel_x,
                    panel_w,
                    pad_x,
                    y,
                    row_h,
                );
            }
        }

        // PULL REQUESTS section (Z12).
        ctx.raster.fill_pixel_rect(
            panel_x + pad_x,
            y,
            panel_w - pad_x * 2.0,
            1.0,
            theme.hairline,
        );
        let pr_label = if self.prs_expanded {
            format!("v PULL REQUESTS ({})", self.prs.len())
        } else {
            format!("> PULL REQUESTS ({})", self.prs.len())
        };
        y = draw_row_text(
            ctx,
            &pr_label,
            theme.text_subtle,
            panel_x,
            panel_w,
            pad_x,
            y,
            row_h,
        );
        if self.prs_expanded {
            for pr in self.prs.iter().take(8) {
                let label = format!("  #{} {}", pr.number, pr.title);
                y = draw_row_text(ctx, &label, theme.info, panel_x, panel_w, pad_x, y, row_h);
            }
        }

        // Commit message input (Z3).
        ctx.raster.fill_pixel_rect(
            panel_x + pad_x,
            y,
            panel_w - pad_x * 2.0,
            1.0,
            theme.hairline,
        );
        let commit_input_h = row_h + 4.0;
        let commit_bg = if self.commit_input_active {
            theme.panel
        } else {
            theme.surface
        };
        ctx.raster
            .fill_pixel_rect(panel_x, y + 2.0, panel_w, commit_input_h - 2.0, commit_bg);
        let commit_text: &str = if !self.commit_msg.is_empty() {
            &self.commit_msg
        } else if !self.commit_input_active {
            "Commit message (Tab to focus, Cmd+Enter to commit)"
        } else {
            ""
        };
        let commit_color = if self.commit_msg.is_empty() {
            theme.text_subtle
        } else {
            theme.foreground
        };
        let mut rx = panel_x + pad_x;
        let glyph_y = y + 4.0;
        let cw = ctx.metrics.cell_w;
        for ch_c in commit_text.chars() {
            if rx + cw > panel_x + panel_w - pad_x {
                break;
            }
            ctx.raster.glyph_at(
                ctx.painter,
                ctx.metrics,
                rx,
                glyph_y,
                ch_c as u32,
                commit_color,
            );
            rx += cw;
        }
        if self.commit_input_active {
            let cursor_x = panel_x + pad_x + self.commit_msg.chars().count() as f64 * cw;
            ctx.raster
                .fill_pixel_rect(cursor_x, y + 3.0, 2.0, ch, theme.accent);
        }
    }

    fn handle_key(&mut self, _key: OverlayKey) -> OverlayKeyResult {
        // Key handling for the SCM panel lives in AppShell::key_down so it can
        // mutate App::scm_panel. The overlay just delegates PassThrough here.
        OverlayKeyResult::PassThrough
    }

    fn handle_click(&mut self, _x: f64, _y: f64) -> OverlayClickResult {
        OverlayClickResult::Consumed
    }
}

// ── CompletionOverlay ─────────────────────────────────────────────────────────

/// Completion popup as a `CustomOverlay`.
///
/// Anchored to the cursor cell; positioned below the trigger line.
pub struct CompletionOverlay {
    /// Visible items (pre-filtered snapshot).
    pub items: Vec<CompletionEntry>,
    pub selected: usize,
    /// Pixel x of the left edge of the popup (computed by the caller).
    pub pixel_x: f64,
    /// Pixel y of the top of the popup (computed by the caller — one row below anchor).
    pub pixel_y: f64,
}

impl CompletionOverlay {
    const MAX_ROWS: usize = 12;
    const LABEL_COLS: usize = 24;
    const DETAIL_COLS: usize = 20;
}

impl CustomOverlay for CompletionOverlay {
    fn id(&self) -> OverlayId {
        OverlayId::Completion
    }

    fn measure(&self, ctx: &OverlayMeasureCtx) -> CardSize {
        let cw = ctx.metrics.cell_w;
        let ch = ctx.metrics.cell_h;
        let show_count = self.items.len().min(Self::MAX_ROWS);
        let popup_cols = Self::LABEL_COLS + 1 + Self::DETAIL_COLS;
        CardSize {
            w: popup_cols as f64 * cw,
            h: show_count as f64 * ch,
        }
    }

    fn card_origin(
        &self,
        size: CardSize,
        dw: f64,
        dh: f64,
        _chrome_top: f64,
    ) -> Option<(f64, f64)> {
        let x = self.pixel_x.min(dw - size.w).max(0.0);
        let y = self.pixel_y.min(dh - size.h).max(0.0);
        Some((x, y))
    }

    fn close_on_blur(&self) -> bool {
        false // anchored / non-modal; dismissed by editor key events
    }

    fn render(&self, ctx: &mut OverlayRenderCtx<'_>) {
        let cw = ctx.metrics.cell_w;
        let ch = ctx.metrics.cell_h;
        let theme = ctx.theme;
        let g = ctx.geom;
        let list_x = g.x;
        let list_y = g.y;
        let popup_w = g.w;

        let show_count = self.items.len().min(Self::MAX_ROWS);
        let visible_selected = self.selected.min(show_count.saturating_sub(1));

        for (ri, entry) in self.items.iter().enumerate().take(show_count) {
            let row_y = list_y + ri as f64 * ch;
            if ri == visible_selected {
                ctx.raster
                    .fill_pixel_rect_alpha(list_x, row_y, popup_w, ch, theme.accent, 0.18);
            }
            // Label.
            let label_chars: Vec<char> = entry.label.chars().take(Self::LABEL_COLS).collect();
            for (ci, &c) in label_chars.iter().enumerate() {
                let tx = list_x + (ci + 1) as f64 * cw;
                ctx.raster.glyph_at(
                    ctx.painter,
                    ctx.metrics,
                    tx,
                    row_y,
                    c as u32,
                    theme.foreground,
                );
            }
            // Detail.
            if let Some(detail) = &entry.detail {
                let detail_chars: Vec<char> = detail.chars().take(Self::DETAIL_COLS).collect();
                let detail_start_col = Self::LABEL_COLS + 2;
                for (ci, &c) in detail_chars.iter().enumerate() {
                    let tx = list_x + (detail_start_col + ci) as f64 * cw;
                    ctx.raster.glyph_at(
                        ctx.painter,
                        ctx.metrics,
                        tx,
                        row_y,
                        c as u32,
                        theme.text_subtle,
                    );
                }
            }
        }
    }

    fn handle_key(&mut self, _key: OverlayKey) -> OverlayKeyResult {
        // Navigation handled by AppShell before overlay dispatch.
        OverlayKeyResult::PassThrough
    }

    fn handle_click(&mut self, _x: f64, _y: f64) -> OverlayClickResult {
        OverlayClickResult::Consumed
    }
}

// ── CodeActionsOverlay ────────────────────────────────────────────────────────

/// Code-actions popup as a `CustomOverlay`.
///
/// Anchored to the cursor cell; positioned below the trigger line.
pub struct CodeActionsOverlay {
    pub items: Vec<CodeActionEntry>,
    pub selected: usize,
    /// Pixel x of the popup left edge.
    pub pixel_x: f64,
    /// Pixel y of the popup top edge.
    pub pixel_y: f64,
}

impl CodeActionsOverlay {
    const MAX_ROWS: usize = 12;
    const LABEL_COLS: usize = 40;
}

impl CustomOverlay for CodeActionsOverlay {
    fn id(&self) -> OverlayId {
        OverlayId::CodeActions
    }

    fn measure(&self, ctx: &OverlayMeasureCtx) -> CardSize {
        let cw = ctx.metrics.cell_w;
        let ch = ctx.metrics.cell_h;
        let show_count = self.items.len().min(Self::MAX_ROWS);
        CardSize {
            w: (Self::LABEL_COLS + 2) as f64 * cw,
            h: show_count as f64 * ch,
        }
    }

    fn card_origin(
        &self,
        size: CardSize,
        dw: f64,
        dh: f64,
        _chrome_top: f64,
    ) -> Option<(f64, f64)> {
        let x = self.pixel_x.min(dw - size.w).max(0.0);
        let y = self.pixel_y.min(dh - size.h).max(0.0);
        Some((x, y))
    }

    fn close_on_blur(&self) -> bool {
        false
    }

    fn render(&self, ctx: &mut OverlayRenderCtx<'_>) {
        let cw = ctx.metrics.cell_w;
        let ch = ctx.metrics.cell_h;
        let theme = ctx.theme;
        let g = ctx.geom;
        let list_x = g.x;
        let list_y = g.y;
        let popup_w = g.w;

        let show_count = self.items.len().min(Self::MAX_ROWS);
        let visible_selected = self.selected.min(show_count.saturating_sub(1));

        for (ri, entry) in self.items.iter().enumerate().take(show_count) {
            let row_y = list_y + ri as f64 * ch;
            if ri == visible_selected {
                ctx.raster
                    .fill_pixel_rect_alpha(list_x, row_y, popup_w, ch, theme.accent, 0.18);
            }
            let label_chars: Vec<char> = entry.title.chars().take(Self::LABEL_COLS).collect();
            for (ci, &c) in label_chars.iter().enumerate() {
                let tx = list_x + (ci + 1) as f64 * cw;
                ctx.raster.glyph_at(
                    ctx.painter,
                    ctx.metrics,
                    tx,
                    row_y,
                    c as u32,
                    theme.foreground,
                );
            }
        }
    }

    fn handle_key(&mut self, _key: OverlayKey) -> OverlayKeyResult {
        OverlayKeyResult::PassThrough
    }

    fn handle_click(&mut self, _x: f64, _y: f64) -> OverlayClickResult {
        OverlayClickResult::Consumed
    }
}

// ── HoverOverlay ──────────────────────────────────────────────────────────────

/// Hover popup as a `CustomOverlay`.
///
/// Non-modal, anchored to the cursor cell. Paragraph text only (v1).
pub struct HoverOverlay {
    pub lines: Vec<String>,
    /// Pixel x anchor (left edge of popup).
    pub pixel_x: f64,
    /// Pixel y anchor (top edge of popup — placed one row below the anchor cell).
    pub pixel_y: f64,
}

impl HoverOverlay {
    const MAX_COLS: usize = 60;
}

impl CustomOverlay for HoverOverlay {
    fn id(&self) -> OverlayId {
        OverlayId::Hover
    }

    fn measure(&self, ctx: &OverlayMeasureCtx) -> CardSize {
        let cw = ctx.metrics.cell_w;
        let ch = ctx.metrics.cell_h;
        let text_w = self
            .lines
            .iter()
            .map(|l| l.len().min(Self::MAX_COLS))
            .max()
            .unwrap_or(0);
        CardSize {
            w: (text_w + 2) as f64 * cw,
            h: (self.lines.len() + 1) as f64 * ch,
        }
    }

    fn card_origin(
        &self,
        size: CardSize,
        dw: f64,
        dh: f64,
        _chrome_top: f64,
    ) -> Option<(f64, f64)> {
        let x = self.pixel_x.min(dw - size.w).max(0.0);
        let y = self.pixel_y.min(dh - size.h).max(0.0);
        Some((x, y))
    }

    fn close_on_blur(&self) -> bool {
        false // dismissed by editor key / mouse events
    }

    fn render(&self, ctx: &mut OverlayRenderCtx<'_>) {
        let cw = ctx.metrics.cell_w;
        let ch = ctx.metrics.cell_h;
        let theme = ctx.theme;
        let g = ctx.geom;

        for (li, line) in self.lines.iter().enumerate() {
            let ty = g.y + (li as f64 + 0.5) * ch;
            let chars: Vec<char> = line.chars().take(Self::MAX_COLS).collect();
            for (ci, &c) in chars.iter().enumerate() {
                let tx = g.x + (ci + 1) as f64 * cw;
                ctx.raster
                    .glyph_at(ctx.painter, ctx.metrics, tx, ty, c as u32, theme.foreground);
            }
        }
    }

    fn handle_key(&mut self, _key: OverlayKey) -> OverlayKeyResult {
        OverlayKeyResult::PassThrough
    }

    fn handle_click(&mut self, _x: f64, _y: f64) -> OverlayClickResult {
        OverlayClickResult::PassThrough
    }
}
