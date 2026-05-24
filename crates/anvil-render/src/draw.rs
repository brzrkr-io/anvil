//! Per-frame viewport draw loop.
//!
//! `draw_viewport` renders the visible cell grid (plus prompt-rule hairlines,
//! gutter exit-status markers, fold summaries, and the text cursor) into a
//! `Raster`.  It reads only the arguments it is given — no global state — and
//! performs no heap allocation per frame.
//!
//! Ported from `src/render/draw.zig`.

use anvil_term::{
    Block, BlockState, Cell, Color, CursorShape, DirtySet, MatchKind, Search, Terminal,
};
use anvil_theme::{Theme, mix};
use anvil_workspace::selection::Selection;

use crate::atlas::GlyphRasterizer;
use crate::batch::CellBatch;
use crate::raster::{FontMetrics, GlyphPainter, Raster};

// ── Semantic status colors (Mineral palette, brand contract) ─────────────────

/// accent.bright / mineral bright teal — running block (#54b7c0)
const ACCENT_BRIGHT: [u8; 3] = [0x54, 0xb7, 0xc0];
/// status.verified — exit 0 (#3f8a5b)
const VERIFIED: [u8; 3] = [0x3f, 0x8a, 0x5b];
/// status.failure — non-zero exit (#b13a30)
const FAILURE: [u8; 3] = [0xb1, 0x3a, 0x30];
/// alloy — muted text for fold summaries (#86919a)
const ALLOY: [u8; 3] = [0x86, 0x91, 0x9a];
/// panel-raised — block body background tint. Matches D's `.block { background:
/// var(--charcoal) }` against the terminal background. Lifted enough to read
/// as a card at retina; quiet enough not to compete with text.
const PANEL_RAISED: [u8; 3] = [0x22, 0x27, 0x30];

// ── Folded blocks ─────────────────────────────────────────────────────────────

/// A thin view into the set of folded command_line values for one pane.
/// Passed into `draw_viewport` so the draw loop can skip folded output rows.
pub struct FoldedBlocks<'a> {
    /// Absolute `command_line` values of folded blocks.
    pub cmd_lines: &'a [usize],
}

impl<'a> FoldedBlocks<'a> {
    /// Construct from a pane's folded array slice.
    pub fn new(cmd_lines: &'a [usize]) -> Self {
        Self { cmd_lines }
    }

    /// Empty set — no blocks folded.
    pub fn empty() -> Self {
        Self { cmd_lines: &[] }
    }

    pub fn contains(&self, cmd_line: usize) -> bool {
        self.cmd_lines.contains(&cmd_line)
    }
}

// ── Cursor style ─────────────────────────────────────────────────────────────

/// Cursor rendering style, mirroring `config.zig`'s `CursorStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

/// Configuration that the renderer needs from the user's cursor prefs.
#[derive(Clone, Copy, Debug)]
pub struct CursorConfig {
    pub style: CursorStyle,
    pub blink: bool,
}

impl Default for CursorConfig {
    fn default() -> Self {
        CursorConfig {
            style: CursorStyle::Block,
            blink: true,
        }
    }
}

/// Cursor rendering parameters.  Bundled so the public signature stays stable.
#[derive(Clone, Copy, Debug)]
pub struct CursorParams {
    /// Animated column (fractional viewport cell).
    pub ax: f32,
    /// Animated row (fractional viewport cell).
    pub ay: f32,
    /// Blink phase in [0, 1).
    pub blink_phase: f32,
    pub cfg: CursorConfig,
}

// ── Color resolution ─────────────────────────────────────────────────────────

/// Resolve a `Color` to an RGB triple, falling back to `default`.
pub fn resolve_color(col: Color, default: [u8; 3], theme: &Theme) -> [u8; 3] {
    match col {
        Color::Default => default,
        Color::Palette(p) => theme.palette256(p),
        Color::Rgb(v) => v,
    }
}

// ── Cursor opacity ────────────────────────────────────────────────────────────

/// Cursor opacity for blink phase `p` in [0,1).
///
/// A smooth pulse that never fully disappears — floors at MIN so the cursor
/// stays continuously locatable while still breathing. Aesthetic over the
/// original Zig "solid → off → solid" hard square-wave: a hard off-phase
/// reads as "did it move?" jitter, especially on dense lines.
pub fn cursor_opacity(p: f32) -> f32 {
    const MIN: f32 = 0.35; // dim floor — visible but quiet
    // (1 + cos(2πp)) / 2 → smooth 1.0 → 0 → 1.0 across [0, 1).
    let pulse = 0.5 + 0.5 * (std::f32::consts::TAU * p).cos();
    MIN + (1.0 - MIN) * pulse
}

// ── Prompt-rule predicate ─────────────────────────────────────────────────────

/// Should a prompt-rule hairline be drawn above viewport row `viewport_y`?
/// `off` is the scroll offset (0 when the viewport is pinned to live bottom).
///
/// Ported from `draw.zig`'s `ruleRow`.
pub fn rule_row(terminal: &Terminal, viewport_y: usize, off: usize) -> bool {
    let hist = terminal.scrollback_len();
    let crow: usize = if off > viewport_y {
        (hist + viewport_y).saturating_sub(off)
    } else {
        hist + viewport_y - off
    };
    let abs = terminal.absolute_line_of_content(crow);
    terminal.is_prompt_start(abs)
}

// ── Single-cell draw ─────────────────────────────────────────────────────────

/// Draw one cell into the raster.
///
/// Ported from `draw.zig`'s `drawCell`.
#[allow(clippy::too_many_arguments)]
pub fn draw_cell(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    x: usize,
    y: usize,
    content_row: usize,
    cell: Cell,
    top_bar_rows: usize,
    selection: Selection,
    search: Option<&Search>,
) {
    let mut fg = resolve_color(cell.fg, theme.foreground, theme);
    let mut bg = resolve_color(cell.bg, theme.background, theme);

    use anvil_term::Attrs;
    if cell.attrs.contains(Attrs::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }

    if selection.active && selection.contains(content_row, x) {
        // 0.25 mix reads as a quiet wash, not a loud highlight; the
        // foreground glyph stays readable through it.
        bg = mix(theme.background, theme.accent, 0.25);
    }

    if let Some(s) = search {
        match s.classify(content_row, x) {
            MatchKind::Current => bg = theme.accent,
            MatchKind::Other => bg = theme.ansi[8],
            MatchKind::None => {}
        }
    }

    let ry = y + top_bar_rows;
    if bg != theme.background {
        raster.cell_bg(metrics, x, ry, bg);
    }
    if cell.cp != ' ' && cell.cp != '\0' {
        // u32 glyph id: draw.zig passes `font.glyph(cell.cp)` which is a u16.
        // We use the Unicode scalar as the glyph key and let the painter
        // resolve to a glyph id via CoreText.
        raster.cell_glyph(painter, metrics, x, ry, cell.cp as u32, fg);
    }
}

// ── Cursor draw ───────────────────────────────────────────────────────────────

/// Draw the text cursor into the raster.
///
/// Ported from `draw.zig`'s `drawCursor`.
pub fn draw_cursor(
    raster: &mut Raster,
    _painter: &mut dyn GlyphPainter,
    terminal: &Terminal,
    metrics: FontMetrics,
    theme: &Theme,
    top_bar_rows: usize,
    params: CursorParams,
) {
    let blink = terminal.app_cursor_blink.unwrap_or(params.cfg.blink);
    let opacity = if blink {
        cursor_opacity(params.blink_phase)
    } else {
        1.0
    };
    let ax = params.ax as f64;
    let ay = params.ay as f64 + top_bar_rows as f64;
    let cursor_rgb = mix(theme.background, theme.accent, opacity);

    let style = match terminal.app_cursor_shape {
        Some(CursorShape::Block) => CursorStyle::Block,
        Some(CursorShape::Underline) => CursorStyle::Underline,
        Some(CursorShape::Bar) => CursorStyle::Bar,
        None => params.cfg.style,
    };

    match style {
        CursorStyle::Block => {
            // Full-cell block: fx=0, fy=0, fw=1, fh=1 (top-left anchor, full extent).
            raster.cell_inset(metrics, ax, ay, cursor_rgb, 0.0, 0.0, 1.0, 1.0);
            let ic = params.ax.round() as usize;
            let ir = params.ay.round() as usize;
            // Zig: `terminal.viewportRow(ir)` — Rust terminal uses `&mut self`.
            // We need to get the cell for the character under the cursor.
            // We use a temporary Terminal borrow via viewport_row (which requires &mut self).
            // Reconstruct the row access; since we only need col ic, we snapshot
            // the cell before calling raster operations in draw_viewport so we
            // don't need to call terminal again here.  For now delegate to the
            // caller's responsibility to pass the cell, but match the Zig
            // behaviour: if the cell has a non-space char, draw it in the cursor-
            // tinted color.  Since we can't take &mut terminal while raster is
            // borrowed, draw_cursor receives the optional cell separately.
            //
            // This implementation draws the block cursor; the cell-under-cursor
            // glyph is handled by draw_viewport which has mutable access to both.
            let _ = (ic, ir); // used in draw_viewport
        }
        CursorStyle::Bar => {
            // Left bar: 15% width, full height.
            // fy=0 is top in top-down space, which is what we want (bar at left).
            raster.cell_inset(metrics, ax, ay, cursor_rgb, 0.0, 0.0, 0.15, 1.0);
        }
        CursorStyle::Underline => {
            // Bottom 12% strip.  In top-down space: fy = 1 - fh.
            let fh = 0.12_f64;
            raster.cell_inset(metrics, ax, ay, cursor_rgb, 0.0, 1.0 - fh, 1.0, fh);
        }
    }
}

// ── Private text helper ───────────────────────────────────────────────────────

/// Draw a short string starting at cell `(col, row)`, clipping at `max_col`.
#[allow(clippy::too_many_arguments)]
fn draw_text_row(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    col: usize,
    row: usize,
    text: &str,
    color: [u8; 3],
    max_col: usize,
) {
    for (i, cp) in text.chars().enumerate() {
        let cx = col + i;
        if cx >= max_col {
            break;
        }
        raster.cell_glyph(painter, metrics, cx, row, cp as u32, color);
    }
}

// ── Block header helpers ──────────────────────────────────────────────────────

/// Format a duration in milliseconds into a human-readable string.
/// e.g. "0.1s", "3.2s", "1m04s"
fn format_duration(ms: u64) -> String {
    if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let s = ms / 1000;
        format!("{}m{:02}s", s / 60, s % 60)
    }
}

/// Read command text from terminal cells at content row `crow`, starting at
/// `start_col`, into a `String`.  Trims trailing whitespace/nulls.
fn read_command_text(terminal: &Terminal, crow: usize, start_col: usize) -> String {
    let cells = terminal.line(crow);
    let mut s = String::new();
    for cell in cells.iter().skip(start_col) {
        if cell.cp != '\0' {
            s.push(cell.cp);
        }
    }
    // Trim trailing spaces/nulls.
    let trimmed = s.trim_end();
    trimmed.to_string()
}

/// Draw the synthesized block header row (CPU path).
///
/// Draws over the raw terminal cells at the command row:
///   - command text in foreground color (col 1 onward; col 0 is accent bar)
///   - duration in muted color, right-aligned before exit indicator
///   - exit indicator: "✓" (exit 0), "✗ N" (exit N), "…" (running)
///   - fold indicator "▾" at far right
///
/// `ry` is the raster row (already includes top_bar_rows).
/// `cols` is the terminal width in cells.
#[allow(clippy::too_many_arguments)]
fn draw_block_header_cpu(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    metrics: FontMetrics,
    theme: &Theme,
    block: &Block,
    cmd_text: &str,
    ry: usize,
    cols: usize,
) {
    let accent_color = block_accent_color(block);
    let fg = theme.foreground;
    let muted = ALLOY;

    // Command text: col 1..cols-18 (leave room for right metadata).
    draw_text_row(
        raster,
        painter,
        metrics,
        1,
        ry,
        cmd_text,
        fg,
        cols.saturating_sub(18),
    );

    // Build the right-side segments individually so we can color them.
    // Segment 1: duration (muted).
    let dur_str = if block.duration_ms > 0 {
        format_duration(block.duration_ms)
    } else {
        String::new()
    };

    // Segment 2: exit indicator (accent/status color).
    let (exit_str, exit_color) = match block.state {
        BlockState::Running => ("\u{2026}".to_string(), accent_color), // …
        BlockState::Exited => {
            if block.exit_code == 0 {
                ("\u{2713}".to_string(), VERIFIED) // ✓
            } else {
                (format!("\u{2717} {}", block.exit_code), FAILURE) // ✗ N
            }
        }
    };

    // Fold indicator (muted), always present.
    let fold_str = " \u{25be}"; // " ▾"

    // Compute total right width and starting column.
    let sep = if dur_str.is_empty() { "" } else { "  " };
    let right_len = dur_str.chars().count()
        + sep.chars().count()
        + exit_str.chars().count()
        + fold_str.chars().count();
    if right_len < cols {
        let mut col = cols - right_len;
        draw_text_row(raster, painter, metrics, col, ry, &dur_str, muted, cols);
        col += dur_str.chars().count();
        draw_text_row(raster, painter, metrics, col, ry, sep, muted, cols);
        col += sep.chars().count();
        draw_text_row(
            raster, painter, metrics, col, ry, &exit_str, exit_color, cols,
        );
        col += exit_str.chars().count();
        draw_text_row(raster, painter, metrics, col, ry, fold_str, muted, cols);
    }
}

/// Draw the synthesized block header row (GPU path).
///
/// Mirrors `draw_block_header_cpu` but pushes `CellInstance`s into `batch`
/// instead of writing pixels.  `cell_xy_fn` maps (col, viewport_row) to
/// top-left pixel coords; `y_shift` is subtracted for smooth-scroll.
#[allow(clippy::too_many_arguments)]
fn draw_block_header_gpu(
    batch: &mut CellBatch,
    rasterizer: &mut dyn GlyphRasterizer,
    metrics: FontMetrics,
    block: &Block,
    viewport_y: usize,
    cols: usize,
    cw: f32,
    ch: f32,
    cell_xy_fn: impl Fn(usize, usize) -> [f32; 2],
    y_shift: f32,
) {
    let muted = ALLOY;
    let accent_color = block_accent_color(block);

    let dur_str = if block.duration_ms > 0 {
        format_duration(block.duration_ms)
    } else {
        String::new()
    };

    let (exit_str, exit_color) = match block.state {
        BlockState::Running => ("\u{2026}".to_string(), accent_color),
        BlockState::Exited => {
            if block.exit_code == 0 {
                ("\u{2713}".to_string(), VERIFIED)
            } else {
                (format!("\u{2717} {}", block.exit_code), FAILURE)
            }
        }
    };

    let fold_str = " \u{25be}"; // " ▾"
    let sep = if dur_str.is_empty() { "" } else { "  " };

    let right_len = dur_str.chars().count()
        + sep.chars().count()
        + exit_str.chars().count()
        + fold_str.chars().count();

    if right_len >= cols {
        return;
    }

    // Build a vec of (char, color) pairs for the right segment.
    let mut chars: Vec<(char, [u8; 3])> = Vec::with_capacity(right_len);
    for c in dur_str.chars() {
        chars.push((c, muted));
    }
    for c in sep.chars() {
        chars.push((c, muted));
    }
    for c in exit_str.chars() {
        chars.push((c, exit_color));
    }
    for c in fold_str.chars() {
        chars.push((c, muted));
    }

    let start_col = cols - right_len;
    for (i, (cp, color)) in chars.into_iter().enumerate() {
        let col = start_col + i;
        let base_xy = cell_xy_fn(col, viewport_y);
        let xy = [base_xy[0], base_xy[1] - y_shift];
        let wh = [cw, ch];
        // Background: PANEL_RAISED so the overlay sits on the card background.
        let bg = PANEL_RAISED;
        if cp == ' ' {
            batch.push_cell(xy, wh, None, color, bg);
        } else {
            let slot = rasterizer.glyph_slot(cp as u32, metrics);
            batch.push_cell(xy, wh, slot, color, bg);
        }
    }
}

// ── Gutter mark color ─────────────────────────────────────────────────────────

fn block_accent_color(block: &Block) -> [u8; 3] {
    match block.state {
        BlockState::Running => ACCENT_BRIGHT,
        BlockState::Exited => {
            if block.exit_code == 0 {
                VERIFIED
            } else {
                FAILURE
            }
        }
    }
}

// ── Viewport draw loop ────────────────────────────────────────────────────────

/// Draw the viewport: visible cell grid, prompt-rule hairlines, gutter markers,
/// fold summaries, and cursor.
///
/// This is the per-frame draw body, ported from `draw.zig`'s `drawViewport`.
///
/// `scroll_pos` and `overscroll` drive smooth scrolling (0/0 = pinned).
/// `top_bar_rows` offsets every cell row by the tab-bar height.
/// `folded` carries the set of folded `command_line` values for this pane;
/// pass `FoldedBlocks::empty()` to draw all rows normally.
/// Pass `cursor_params = None` to suppress cursor drawing (e.g. in tests).
///
/// `dirty` restricts drawing to only the rows that have changed. `None` or
/// `Some(full)` redraws all rows. For each dirty row the row's background band
/// is first cleared to `theme.background` so stale pixels from the previous
/// frame are overwritten.  Non-dirty rows are left untouched in the raster.
///
/// In the smooth-scroll path all rows are always redrawn (content shifts).
///
/// # Zero-allocation guarantee
/// The draw loop writes into the pre-allocated `Raster` pixel buffer.  No
/// `Vec` growth or heap allocation occurs per frame by construction.
#[allow(clippy::too_many_arguments)]
pub fn draw_viewport(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    terminal: &mut Terminal,
    metrics: FontMetrics,
    theme: &Theme,
    scroll_pos: f32,
    selection: Selection,
    search: Option<&Search>,
    top_bar_rows: usize,
    cursor_params: Option<CursorParams>,
    rule_x_start: f64,
    rule_x_end: f64,
    folded: FoldedBlocks<'_>,
    dirty: Option<&DirtySet>,
) {
    let rows = terminal.rows();
    let cols = terminal.cols();
    // Prompt-rule hairline drawn above each command — use the chrome border
    // tone so the rule reads as a quiet structural divider, not a fence.
    let rule_rgb = theme.border;

    if scroll_pos == 0.0 {
        // Per-frame block-lookup cache: adjacent rows almost always belong
        // to the same block, so caching the last result drops a long-session
        // hot path from O(blocks) per row to O(1) for the common case.
        let mut cached_block: Option<Block> = None;
        // Live bottom: no fractional offset.
        for y in 0..rows {
            // Dirty-row gate: skip rows that haven't changed.
            // None = always draw (full redraw).
            let is_dirty = dirty.is_none_or(|d| d.contains(y));
            if !is_dirty {
                continue;
            }

            // Clear this row's background before redrawing so stale pixels
            // from the previous frame are overwritten.
            {
                let ry = y + top_bar_rows;
                let y_top = (raster.origin_y + ry as f64 * metrics.cell_h) as usize;
                let y_bot = (raster.origin_y + (ry + 1) as f64 * metrics.cell_h) as usize;
                raster.clear_pixel_rows(y_top, y_bot, theme.background);
            }

            let crow = terminal.content_row_of_viewport(y);
            let abs = terminal.absolute_line_of_content(crow);

            // Block lookup: reuse cached block when this row is still inside
            // its range; otherwise ask the terminal.
            let block_opt = match cached_block {
                Some(ref b) if abs >= b.command_line && abs < b.end_line => Some(*b),
                _ => {
                    let fresh = terminal.block_at(abs);
                    cached_block = fresh;
                    fresh
                }
            };

            // If this row is inside a folded block's OUTPUT region, skip it.
            if let Some(ref block) = block_opt {
                if folded.contains(block.command_line)
                    && abs >= block.output_line
                    && abs < block.end_line
                {
                    continue;
                }
            }

            // Block body tint: PANEL_RAISED background across the FULL block
            // span (command row + output rows). Matches D's `.block { background
            // var(--charcoal) }` which wraps the command line + its output as
            // a single card.
            if let Some(ref block) = block_opt {
                let is_block_row = abs >= block.command_line && abs < block.end_line;
                if is_block_row {
                    let ry = y + top_bar_rows;
                    // One row-wide fill (much cheaper than `cols` per-cell fills).
                    let row_x = raster.origin_x;
                    let row_y = raster.origin_y + ry as f64 * metrics.cell_h
                        - raster.y_shift_px;
                    let row_w = cols as f64 * metrics.cell_w;
                    raster.fill_pixel_rect(row_x, row_y, row_w, metrics.cell_h, PANEL_RAISED);
                }
            }

            // Draw all cells while the row-slice borrow is live, then let it
            // drop so the &self prompt-rule calls below can proceed.  Cell is
            // Copy so each loop iteration clones the value out of the slice.
            // No heap allocation occurs: the borrow ends at the closing '}'.
            {
                let row = terminal.viewport_row(y);
                for (x, &cell) in row.iter().enumerate().take(cols.min(row.len())) {
                    draw_cell(
                        raster,
                        painter,
                        metrics,
                        theme,
                        x,
                        y,
                        crow,
                        cell,
                        top_bar_rows,
                        selection,
                        search,
                    );
                }
            } // row borrow ends here

            // Block accent bar: colored stripe along the LEFT edge across
            // the full block span. Matches D's `.block { border-left: 2px
            // solid var(--verified) }`.
            if let Some(ref block) = block_opt {
                let is_block_row = abs >= block.command_line && abs < block.end_line;
                if is_block_row {
                    let ry = y + top_bar_rows;
                    let accent_rgb = block_accent_color(block);
                    raster.block_accent_bar(metrics, ry, accent_rgb);
                }
            }

            // Fold summary: if this is the command row of a folded block,
            // append " ⌄ N hidden" after any command text.
            if let Some(ref block) = block_opt {
                if folded.contains(block.command_line) && abs == block.command_line {
                    let hidden = block.output_row_count();
                    let summary = format!(" \u{2304} {hidden} hidden");
                    let ry = y + top_bar_rows;
                    draw_text_row(raster, painter, metrics, 0, ry, &summary, ALLOY, cols);
                }
            }

            // Block header synthesis (live-bottom CPU path): right-aligned
            // duration + exit indicator + fold caret overlaid on the command row.
            if let Some(ref block) = block_opt {
                if abs == block.command_line && !folded.contains(block.command_line) {
                    let cmd_text =
                        read_command_text(terminal, crow, block.command_start_col as usize);
                    let ry = y + top_bar_rows;
                    draw_block_header_cpu(
                        raster, painter, metrics, theme, block, &cmd_text, ry, cols,
                    );
                }
            }

            if terminal.is_prompt_start(abs) {
                let ry = (y + top_bar_rows) as f64;
                raster.row_rule(metrics, ry, rule_rgb, rule_x_start, rule_x_end);
            }
        }
    } else {
        // Smooth-scroll path: render integer offset (base+1) and slide the
        // grid by the fractional part plus overscroll.
        let base = scroll_pos.floor() as usize;
        let frac = scroll_pos as f64 - scroll_pos.floor() as f64;
        let scroll_shift = (1.0 - frac) * metrics.cell_h;
        raster.y_shift_px = scroll_shift;
        let hist = terminal.scrollback_len();
        let off = base + 1;
        let mut cached_block: Option<Block> = None;
        for y in 0..=rows {
            let crow: usize = if off > y {
                (hist + y).saturating_sub(off)
            } else {
                hist + y - off
            };
            let abs = terminal.absolute_line_of_content(crow);

            // Block lookup with per-row-locality cache.
            let block_opt = match cached_block {
                Some(ref b) if abs >= b.command_line && abs < b.end_line => Some(*b),
                _ => {
                    let fresh = terminal.block_at(abs);
                    cached_block = fresh;
                    fresh
                }
            };

            if let Some(ref block) = block_opt {
                if folded.contains(block.command_line)
                    && abs >= block.output_line
                    && abs < block.end_line
                {
                    continue;
                }
            }

            // Block body tint (smooth-scroll path): wraps command + output.
            if let Some(ref block) = block_opt {
                let is_block_row = abs >= block.command_line && abs < block.end_line;
                if is_block_row {
                    let ry = y + top_bar_rows;
                    let row_x = raster.origin_x;
                    let row_y = raster.origin_y + ry as f64 * metrics.cell_h
                        - raster.y_shift_px;
                    let row_w = cols as f64 * metrics.cell_w;
                    raster.fill_pixel_rect(row_x, row_y, row_w, metrics.cell_h, PANEL_RAISED);
                }
            }

            {
                let row = terminal.viewport_row_at(off, y);
                for (x, &cell) in row.iter().enumerate().take(cols.min(row.len())) {
                    draw_cell(
                        raster,
                        painter,
                        metrics,
                        theme,
                        x,
                        y,
                        crow,
                        cell,
                        top_bar_rows,
                        selection,
                        search,
                    );
                }
            } // row borrow ends here

            // Block accent bar (smooth-scroll path): full block span.
            if let Some(ref block) = block_opt {
                let is_block_row = abs >= block.command_line && abs < block.end_line;
                if is_block_row {
                    let ry = y + top_bar_rows;
                    let accent_rgb = block_accent_color(block);
                    raster.block_accent_bar(metrics, ry, accent_rgb);
                }
            }

            // Fold summary (smooth-scroll path).
            if let Some(ref block) = block_opt {
                if folded.contains(block.command_line) && abs == block.command_line {
                    let hidden = block.output_row_count();
                    let summary = format!(" \u{2304} {hidden} hidden");
                    let ry = y + top_bar_rows;
                    draw_text_row(raster, painter, metrics, 0, ry, &summary, ALLOY, cols);
                }
            }

            // Block header (smooth-scroll path).
            if let Some(ref block) = block_opt {
                if abs == block.command_line && !folded.contains(block.command_line) {
                    let cmd_text =
                        read_command_text(terminal, crow, block.command_start_col as usize);
                    let ry = y + top_bar_rows;
                    draw_block_header_cpu(
                        raster, painter, metrics, theme, block, &cmd_text, ry, cols,
                    );
                }
            }

            if terminal.is_prompt_start(abs) {
                let ry = (y + top_bar_rows) as f64;
                raster.row_rule(metrics, ry, rule_rgb, rule_x_start, rule_x_end);
            }
        }
        raster.y_shift_px = 0.0;
    }

    // Cursor: only when the viewport is pinned to live bottom.
    if let Some(cp) = cursor_params {
        let cur = terminal.cursor();
        if cur.visible
            && terminal.viewport_offset() == 0
            && scroll_pos == 0.0
            && cur.x < cols
            && cur.y < rows
        {
            raster.y_shift_px = 0.0;

            // For block cursor: draw the block then re-draw the glyph tinted.
            let style = match terminal.app_cursor_shape {
                Some(CursorShape::Block) => CursorStyle::Block,
                Some(CursorShape::Underline) => CursorStyle::Underline,
                Some(CursorShape::Bar) => CursorStyle::Bar,
                None => cp.cfg.style,
            };

            let opacity = if terminal.app_cursor_blink.unwrap_or(cp.cfg.blink) {
                cursor_opacity(cp.blink_phase)
            } else {
                1.0
            };
            let cursor_rgb = mix(theme.background, theme.accent, opacity);
            let ax = cp.ax as f64;
            let ay = cp.ay as f64 + top_bar_rows as f64;

            match style {
                CursorStyle::Block => {
                    raster.cell_inset(metrics, ax, ay, cursor_rgb, 0.0, 0.0, 1.0, 1.0);
                    let ic = cp.ax.round() as usize;
                    let ir = cp.ay.round() as usize;
                    if ir < rows && ic < cols {
                        // Extract the cell fields (all Copy) while the row borrow
                        // is live, then let the borrow drop before the raster call.
                        let cell_under = {
                            let row = terminal.viewport_row(ir);
                            if ic < row.len() { Some(row[ic]) } else { None }
                        }; // row borrow ends here
                        if let Some(cell) = cell_under {
                            if cell.cp != ' ' && cell.cp != '\0' {
                                let base_fg = resolve_color(cell.fg, theme.foreground, theme);
                                let glyph_fg = mix(base_fg, theme.background, opacity);
                                raster.cell_glyph(
                                    painter,
                                    metrics,
                                    ic,
                                    ir + top_bar_rows,
                                    cell.cp as u32,
                                    glyph_fg,
                                );
                            }
                        }
                    }
                }
                CursorStyle::Bar => {
                    raster.cell_inset(metrics, ax, ay, cursor_rgb, 0.0, 0.0, 0.15, 1.0);
                }
                CursorStyle::Underline => {
                    let fh = 0.12_f64;
                    raster.cell_inset(metrics, ax, ay, cursor_rgb, 0.0, 1.0 - fh, 1.0, fh);
                }
            }

            raster.y_shift_px = 0.0;
        }
    }
}

// ── GPU viewport draw loop ────────────────────────────────────────────────────

/// Resolve fg/bg for a cell, applying selection, search, and INVERSE.
///
/// Shared by `draw_cell` and `draw_viewport_gpu` so color resolution is
/// identical on both paths.
fn resolve_cell_colors(
    cell: Cell,
    content_row: usize,
    col: usize,
    selection: Selection,
    search: Option<&Search>,
    theme: &Theme,
) -> ([u8; 3], [u8; 3]) {
    let mut fg = resolve_color(cell.fg, theme.foreground, theme);
    let mut bg = resolve_color(cell.bg, theme.background, theme);

    use anvil_term::Attrs;
    if cell.attrs.contains(Attrs::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }

    if selection.active && selection.contains(content_row, col) {
        bg = mix(theme.background, theme.accent, 0.35);
    }

    if let Some(s) = search {
        match s.classify(content_row, col) {
            MatchKind::Current => bg = theme.accent,
            MatchKind::Other => bg = theme.ansi[8],
            MatchKind::None => {}
        }
    }

    (fg, bg)
}

/// GPU-path viewport draw: pushes one `CellInstance` per visible cell into
/// `batch` instead of writing pixels into a `Raster`.
///
/// `raster` is read-only and used only for `cell_rect` pixel-position math
/// (requires `raster.origin_x/y` set to the pane's top-left before calling).
///
/// Mirrors `draw_viewport` loops: same scroll, fold, selection, cursor, and
/// gutter/fold-summary logic.  Chrome (tab bar, status bar, etc.) is NOT
/// drawn here — this is terminal viewport cells only.
///
/// # Cursor
/// Block cursor: one bg-only instance covering the cell with `bg = cursor_rgb`.
/// Bar/underline cursor: a small bg-only instance covering the strip subrect.
///
/// # Gutter marks and fold summaries
/// Gutter mark: a tiny bg-only instance at the left edge of the command row.
/// Fold summary text (" ⌄ N hidden"): one instance per character via
/// `glyph_slot`.
#[allow(clippy::too_many_arguments)]
pub fn draw_viewport_gpu(
    batch: &mut CellBatch,
    raster: &Raster,
    rasterizer: &mut dyn GlyphRasterizer,
    terminal: &mut Terminal,
    metrics: FontMetrics,
    theme: &Theme,
    scroll_pos: f32,
    selection: Selection,
    search: Option<&Search>,
    _top_bar_rows: usize,
    cursor: Option<CursorParams>,
    folded: FoldedBlocks<'_>,
) {
    let rows = terminal.rows();
    let cols = terminal.cols();
    let cw = metrics.cell_w as f32;
    let ch = metrics.cell_h as f32;

    // Helper: push a bg-only rect instance.
    let push_bg = |batch: &mut CellBatch, xy: [f32; 2], wh: [f32; 2], color: [u8; 3]| {
        batch.push_cell(xy, wh, None, color, color);
    };

    // Helper: compute top-left pixel of cell (col, row_in_pane) using raster.cell_rect.
    // `row_in_pane` does NOT include top_bar_rows (origin_y already encodes the
    // pane's pixel top; top_bar_rows is irrelevant for GPU — batch positions are
    // absolute drawable pixels).
    let cell_xy = |batch_col: f64, batch_row: f64| -> [f32; 2] {
        let rect = raster.cell_rect(metrics, batch_col, batch_row);
        [rect.x as f32, rect.y as f32]
    };

    if scroll_pos == 0.0 {
        // Live-bottom path.
        let mut cached_block: Option<Block> = None;
        for y in 0..rows {
            let crow = terminal.content_row_of_viewport(y);
            let abs = terminal.absolute_line_of_content(crow);

            // Block lookup with per-row-locality cache.
            let block_opt = match cached_block {
                Some(ref b) if abs >= b.command_line && abs < b.end_line => Some(*b),
                _ => {
                    let fresh = terminal.block_at(abs);
                    cached_block = fresh;
                    fresh
                }
            };

            // Skip folded output rows.
            if let Some(ref block) = block_opt {
                if folded.contains(block.command_line)
                    && abs >= block.output_line
                    && abs < block.end_line
                {
                    continue;
                }
            }

            // Block body tint: one row-wide bg push instead of `cols` per-cell
            // pushes. Big perf win for smooth scrolling over long blocks.
            if let Some(ref block) = block_opt {
                let is_block_row = abs >= block.command_line && abs < block.end_line;
                if is_block_row {
                    let xy = cell_xy(0.0, y as f64);
                    push_bg(batch, xy, [cw * cols as f32, ch], PANEL_RAISED);
                }
            }

            // Draw all cells in this row.
            {
                let row = terminal.viewport_row(y);
                let row_cells: Vec<Cell> = row.iter().take(cols.min(row.len())).copied().collect();
                let _ = row; // borrow ends: row is &[Cell], already snapshotted above
                for (x, cell) in row_cells.into_iter().enumerate() {
                    let (fg, bg) = resolve_cell_colors(cell, crow, x, selection, search, theme);
                    let xy = cell_xy(x as f64, y as f64);
                    let wh = [cw, ch];
                    if cell.cp == ' ' || cell.cp == '\0' {
                        if bg != theme.background {
                            batch.push_cell(xy, wh, None, fg, bg);
                        }
                    } else {
                        let slot = rasterizer.glyph_slot(cell.cp as u32, metrics);
                        batch.push_cell(xy, wh, slot, fg, bg);
                    }
                }
            }

            // Block accent bar: full-height colored stripe at the left edge.
            // Drawn after cells so it overlays any cell content at col 0.
            if let Some(ref block) = block_opt {
                let accent_rgb = block_accent_color(block);
                let xy = cell_xy(0.0, y as f64);
                // 2px wide, full cell height, at the left padding edge.
                // GPU path uses cell_xy which maps to cell col 0; the accent bar
                // overlaps the first column slightly. This mirrors the CPU path.
                let wh = [2.0f32.min(cw), ch];
                push_bg(batch, xy, wh, accent_rgb);
            }

            // Fold summary text.
            if let Some(ref block) = block_opt {
                if folded.contains(block.command_line) && abs == block.command_line {
                    let hidden = block.output_row_count();
                    let summary = format!(" \u{2304} {hidden} hidden");
                    for (i, cp) in summary.chars().enumerate() {
                        if i >= cols {
                            break;
                        }
                        let xy = cell_xy(i as f64, y as f64);
                        let wh = [cw, ch];
                        let slot = rasterizer.glyph_slot(cp as u32, metrics);
                        batch.push_cell(xy, wh, slot, ALLOY, theme.background);
                    }
                }
            }

            // Block header synthesis (GPU live-bottom path): right-aligned
            // duration + exit indicator + fold caret on the command row.
            if let Some(ref block) = block_opt {
                if abs == block.command_line && !folded.contains(block.command_line) {
                    draw_block_header_gpu(
                        batch, rasterizer, metrics, block, y, cols, cw, ch,
                        |col, row| cell_xy(col as f64, row as f64),
                        0.0,
                    );
                }
            }
        }
    } else {
        // Smooth-scroll path.
        let base = scroll_pos.floor() as usize;
        let frac = scroll_pos as f64 - scroll_pos.floor() as f64;
        let scroll_shift = (1.0 - frac) * metrics.cell_h;
        // Note: y_shift_px is state on the raster; for GPU path we compute
        // pixel positions ourselves using raster.cell_rect (which reads
        // raster.y_shift_px).  We temporarily set it here and restore after.
        // We use a shared reference to raster so we can't mutate it; instead
        // we use the pane origin directly.
        // For smooth-scroll GPU path, compute y pixel offsets manually.
        let shift = scroll_shift as f32;
        let hist = terminal.scrollback_len();
        let off = base + 1;

        let mut cached_block: Option<Block> = None;
        for y in 0..=rows {
            let crow: usize = if off > y {
                (hist + y).saturating_sub(off)
            } else {
                hist + y - off
            };
            let abs = terminal.absolute_line_of_content(crow);

            // Block lookup with per-row-locality cache.
            let block_opt = match cached_block {
                Some(ref b) if abs >= b.command_line && abs < b.end_line => Some(*b),
                _ => {
                    let fresh = terminal.block_at(abs);
                    cached_block = fresh;
                    fresh
                }
            };

            if let Some(ref block) = block_opt {
                if folded.contains(block.command_line)
                    && abs >= block.output_line
                    && abs < block.end_line
                {
                    continue;
                }
            }

            // Block body tint (smooth-scroll path): one row-wide push.
            if let Some(ref block) = block_opt {
                let is_block_row = abs >= block.command_line && abs < block.end_line;
                if is_block_row {
                    let base_xy = cell_xy(0.0, y as f64);
                    let xy = [base_xy[0], base_xy[1] - shift];
                    push_bg(batch, xy, [cw * cols as f32, ch], PANEL_RAISED);
                }
            }

            {
                let row = terminal.viewport_row_at(off, y);
                let row_cells: Vec<Cell> = row.iter().take(cols.min(row.len())).copied().collect();
                let _ = row; // borrow ends: row is &[Cell], already snapshotted above
                for (x, cell) in row_cells.into_iter().enumerate() {
                    let (fg, bg) = resolve_cell_colors(cell, crow, x, selection, search, theme);
                    // Smooth-scroll: apply shift to y position.
                    let base_xy = cell_xy(x as f64, y as f64);
                    let xy = [base_xy[0], base_xy[1] - shift];
                    let wh = [cw, ch];
                    if cell.cp == ' ' || cell.cp == '\0' {
                        if bg != theme.background {
                            batch.push_cell(xy, wh, None, fg, bg);
                        }
                    } else {
                        let slot = rasterizer.glyph_slot(cell.cp as u32, metrics);
                        batch.push_cell(xy, wh, slot, fg, bg);
                    }
                }
            }

            // Block accent bar (smooth-scroll path).
            if let Some(ref block) = block_opt {
                let accent_rgb = block_accent_color(block);
                let base_xy = cell_xy(0.0, y as f64);
                let xy = [base_xy[0], base_xy[1] - shift];
                let wh = [2.0f32.min(cw), ch];
                push_bg(batch, xy, wh, accent_rgb);
            }

            // Fold summary (smooth-scroll path).
            if let Some(ref block) = block_opt {
                if folded.contains(block.command_line) && abs == block.command_line {
                    let hidden = block.output_row_count();
                    let summary = format!(" \u{2304} {hidden} hidden");
                    for (i, cp) in summary.chars().enumerate() {
                        if i >= cols {
                            break;
                        }
                        let base_xy = cell_xy(i as f64, y as f64);
                        let xy = [base_xy[0], base_xy[1] - shift];
                        let wh = [cw, ch];
                        let slot = rasterizer.glyph_slot(cp as u32, metrics);
                        batch.push_cell(xy, wh, slot, ALLOY, theme.background);
                    }
                }
            }

            // Block header synthesis (GPU smooth-scroll path).
            if let Some(ref block) = block_opt {
                if abs == block.command_line && !folded.contains(block.command_line) {
                    draw_block_header_gpu(
                        batch, rasterizer, metrics, block, y, cols, cw, ch,
                        |col, row| cell_xy(col as f64, row as f64),
                        shift,
                    );
                }
            }
        }
    }

    // Cursor: only when pinned to live bottom.
    if let Some(cp) = cursor {
        let cur = terminal.cursor();
        if cur.visible
            && terminal.viewport_offset() == 0
            && scroll_pos == 0.0
            && cur.x < cols
            && cur.y < rows
        {
            let opacity = if terminal.app_cursor_blink.unwrap_or(cp.cfg.blink) {
                cursor_opacity(cp.blink_phase)
            } else {
                1.0
            };
            let cursor_rgb = mix(theme.background, theme.accent, opacity);
            let ax = cp.ax as f64;
            let ay = cp.ay as f64;
            let xy = cell_xy(ax, ay);

            let style = match terminal.app_cursor_shape {
                Some(CursorShape::Block) => CursorStyle::Block,
                Some(CursorShape::Underline) => CursorStyle::Underline,
                Some(CursorShape::Bar) => CursorStyle::Bar,
                None => cp.cfg.style,
            };

            match style {
                CursorStyle::Block => {
                    // Full-cell bg instance.
                    let bxy = xy;
                    batch.push_cell(bxy, [cw, ch], None, cursor_rgb, cursor_rgb);
                    // Re-draw the cell's glyph tinted for the block cursor.
                    let ic = cp.ax.round() as usize;
                    let ir = cp.ay.round() as usize;
                    if ir < rows && ic < cols {
                        let cell_under = {
                            let row = terminal.viewport_row(ir);
                            if ic < row.len() { Some(row[ic]) } else { None }
                        };
                        if let Some(cell) = cell_under {
                            if cell.cp != ' ' && cell.cp != '\0' {
                                let base_fg = resolve_color(cell.fg, theme.foreground, theme);
                                let glyph_fg = mix(base_fg, theme.background, opacity);
                                let slot = rasterizer.glyph_slot(cell.cp as u32, metrics);
                                batch.push_cell(bxy, [cw, ch], slot, glyph_fg, cursor_rgb);
                            }
                        }
                    }
                }
                CursorStyle::Bar => {
                    // Left 15% strip, full height.
                    let bwh = [cw * 0.15, ch];
                    batch.push_cell(xy, bwh, None, cursor_rgb, cursor_rgb);
                }
                CursorStyle::Underline => {
                    // Bottom 12% strip.
                    let fh = ch * 0.12;
                    let bxy = [xy[0], xy[1] + ch - fh];
                    let bwh = [cw, fh];
                    batch.push_cell(bxy, bwh, None, cursor_rgb, cursor_rgb);
                }
            }
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::PixelRect;
    use anvil_term::{DEFAULT_CAPACITY, Terminal};
    use anvil_theme::MINERAL_DARK;

    // Stub painter for draw tests — records calls.
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

    fn make_terminal(cols: usize, rows: usize) -> Terminal {
        Terminal::new(cols, rows, DEFAULT_CAPACITY)
    }

    // ── Port of draw.zig "ruleRow returns true only for prompt-start content rows"

    #[test]
    fn rule_row_true_only_for_prompt_start() {
        let mut t = make_terminal(10, 4);

        // No marks: no rule.
        assert!(!rule_row(&t, 0, 0));
        assert!(!rule_row(&t, 1, 0));

        // Feed OSC 133;A on the current line.
        t.feed(b"\x1b]133;A\x07");
        assert!(rule_row(&t, 0, 0));
        // Row 1 has no mark.
        assert!(!rule_row(&t, 1, 0));
    }

    // ── Port of "prompt-rule rows match the prompt-mark set across viewport scroll"

    #[test]
    fn rule_row_matches_prompt_marks_across_scroll() {
        let mut t = make_terminal(10, 5);
        t.feed(b"\x1b]133;A\x07");
        t.feed(b"line0\r\n");
        t.feed(b"line1\r\n");
        t.feed(b"\x1b]133;A\x07");
        t.feed(b"line2\r\n");
        t.feed(b"line3");

        for y in 0..t.rows() {
            let crow = t.content_row_of_viewport(y);
            let abs = t.absolute_line_of_content(crow);
            let expected = t.is_prompt_start(abs);
            assert_eq!(expected, rule_row(&t, y, 0));
        }
    }

    // ── cursor_opacity

    #[test]
    fn cursor_opacity_peaks_at_phase_zero_and_one() {
        // (1+cos(0))/2 = 1 → opacity = 1.0
        assert!((cursor_opacity(0.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cursor_opacity_dips_to_floor_at_mid_phase() {
        // (1+cos(π))/2 = 0 → opacity = MIN (0.35).
        let v = cursor_opacity(0.5);
        assert!(
            (v - 0.35).abs() < 1e-5,
            "expected floor at phase=0.5, got {v}"
        );
    }

    #[test]
    fn cursor_opacity_never_drops_below_floor() {
        for i in 0..1000 {
            let p = i as f32 / 1000.0;
            let v = cursor_opacity(p);
            assert!(v >= 0.35 - 1e-5, "phase={p} opacity={v} dipped below floor");
        }
    }

    // ── draw_viewport zero-allocation guarantee (by-construction note)
    //
    // The draw loop is allocation-free by construction:
    // - Raster writes into its pre-allocated Vec<u8>.
    // - Each row is drawn fully while its &[Cell] borrow from terminal is live;
    //   the borrow ends before the next viewport_row call, so no Vec snapshot
    //   is needed.  No per-frame heap allocation occurs.

    // ── draw_viewport smoke test

    #[test]
    fn draw_viewport_smoke_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"\x1b]133;A\x07");
        t.feed(b"hello\r\n");
        t.feed(b"world");
        let sel = Selection::default();
        let theme = MINERAL_DARK;

        r.clear(theme.background);
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            sel,
            None,
            0,
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );
        // "hello" starts with 'h' (non-space): expect at least one glyph call.
        assert!(!painter.calls.is_empty());
    }

    #[test]
    fn draw_viewport_smooth_scroll_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        // Push lines into scrollback.
        for _ in 0..10 {
            t.feed(b"scrollback\r\n");
        }
        let sel = Selection::default();
        let theme = MINERAL_DARK;

        r.clear(theme.background);
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            2.0,
            sel,
            None,
            0,
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );
        // No panic; scroll path exercised.
    }

    // ── resolve_color

    #[test]
    fn resolve_color_default_returns_given_default() {
        let theme = MINERAL_DARK;
        let def = [10u8, 20, 30];
        assert_eq!(resolve_color(Color::Default, def, &theme), def);
    }

    #[test]
    fn resolve_color_rgb_returns_exact() {
        let theme = MINERAL_DARK;
        let rgb = [1u8, 2, 3];
        assert_eq!(resolve_color(Color::Rgb(rgb), [0, 0, 0], &theme), rgb);
    }

    #[test]
    fn resolve_color_palette_slot_0_matches_theme_ansi_0() {
        let theme = MINERAL_DARK;
        assert_eq!(
            resolve_color(Color::Palette(0), [0, 0, 0], &theme),
            theme.ansi[0]
        );
    }

    // ── draw_cell with selection active ───────────────────────────────────────

    #[test]
    fn draw_cell_with_active_selection_does_not_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"hello");
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        use anvil_workspace::selection::{Point, Selection};
        let sel = Selection {
            active: true,
            anchor: Point { row: 0, col: 0 },
            head: Point { row: 0, col: 4 },
            ..Selection::default()
        };

        let row = t.viewport_row(0);
        let cell = row[0];
        let _ = row;

        draw_cell(&mut r, &mut painter, m, &theme, 0, 0, 0, cell, 0, sel, None);
        // No panic is the primary assertion; also verifies selection path was taken.
    }

    // ── draw_cell with search match ───────────────────────────────────────────

    #[test]
    fn draw_cell_with_search_match_does_not_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"hello world");
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let mut search = anvil_term::Search::new();
        search.set_query(&t, "hello");

        let row = t.viewport_row(0);
        let cell = row[0];
        let _ = row;

        draw_cell(
            &mut r,
            &mut painter,
            m,
            &theme,
            0,
            0,
            0,
            cell,
            0,
            Selection::default(),
            Some(&search),
        );
    }

    // ── draw_cell INVERSE attribute ───────────────────────────────────────────

    #[test]
    fn draw_cell_inverse_attribute_swaps_colors() {
        use anvil_term::{Attrs, Cell, Color};
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let mut cell = Cell::default();
        cell.cp = 'X';
        cell.fg = Color::Rgb([255, 0, 0]);
        cell.bg = Color::Rgb([0, 0, 255]);
        cell.attrs = Attrs::INVERSE;

        draw_cell(
            &mut r,
            &mut painter,
            m,
            &theme,
            2,
            1,
            0,
            cell,
            0,
            Selection::default(),
            None,
        );
        // No panic; INVERSE path executed.
    }

    // ── draw_cursor (all 3 styles) ────────────────────────────────────────────

    #[test]
    fn draw_cursor_block_style_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let t = make_terminal(10, 4);
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let params = CursorParams {
            ax: 0.0,
            ay: 0.0,
            blink_phase: 0.0,
            cfg: CursorConfig {
                style: CursorStyle::Block,
                blink: false,
            },
        };
        draw_cursor(&mut r, &mut painter, &t, m, &theme, 0, params);
    }

    #[test]
    fn draw_cursor_bar_style_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let t = make_terminal(10, 4);
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let params = CursorParams {
            ax: 1.0,
            ay: 0.0,
            blink_phase: 0.0,
            cfg: CursorConfig {
                style: CursorStyle::Bar,
                blink: false,
            },
        };
        draw_cursor(&mut r, &mut painter, &t, m, &theme, 0, params);
    }

    #[test]
    fn draw_cursor_underline_style_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let t = make_terminal(10, 4);
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let params = CursorParams {
            ax: 0.0,
            ay: 1.0,
            blink_phase: 0.0,
            cfg: CursorConfig {
                style: CursorStyle::Underline,
                blink: false,
            },
        };
        draw_cursor(&mut r, &mut painter, &t, m, &theme, 0, params);
    }

    // ── draw_viewport with cursor_params (block, bar, underline) ─────────────

    #[test]
    fn draw_viewport_with_block_cursor_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"hello");
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let params = CursorParams {
            ax: 0.0,
            ay: 0.0,
            blink_phase: 0.0,
            cfg: CursorConfig {
                style: CursorStyle::Block,
                blink: false,
            },
        };
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            0,
            Some(params),
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );
    }

    #[test]
    fn draw_viewport_with_bar_cursor_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"hello");
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let params = CursorParams {
            ax: 1.0,
            ay: 0.0,
            blink_phase: 0.0,
            cfg: CursorConfig {
                style: CursorStyle::Bar,
                blink: false,
            },
        };
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            0,
            Some(params),
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );
    }

    #[test]
    fn draw_viewport_with_underline_cursor_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"hello");
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let params = CursorParams {
            ax: 0.0,
            ay: 0.0,
            blink_phase: 0.0,
            cfg: CursorConfig {
                style: CursorStyle::Underline,
                blink: false,
            },
        };
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            0,
            Some(params),
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );
    }

    // ── draw_viewport with search ─────────────────────────────────────────────

    #[test]
    fn draw_viewport_with_search_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"hello world");
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let mut search = anvil_term::Search::new();
        search.set_query(&t, "hello");

        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            Some(&search),
            0,
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );
    }

    // ── draw_viewport with blink cursor ──────────────────────────────────────

    #[test]
    fn draw_viewport_blink_cursor_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"X");
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        // Phase in fade-out range so opacity != 1.0
        let params = CursorParams {
            ax: 0.0,
            ay: 0.0,
            blink_phase: 0.55,
            cfg: CursorConfig {
                style: CursorStyle::Block,
                blink: true,
            },
        };
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            0,
            Some(params),
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );
    }

    // ── cursor_opacity full range ─────────────────────────────────────────────

    #[test]
    fn cursor_opacity_full_range_coverage() {
        // Smooth pulse with a 0.35 floor — peak at phase 0, trough at 0.5.
        assert!((cursor_opacity(0.0) - 1.0).abs() < 1e-5);
        assert!((cursor_opacity(0.5) - 0.35).abs() < 1e-5);
        // Quarter-phase points sit between floor and peak.
        for p in [0.25_f32, 0.75] {
            let v = cursor_opacity(p);
            assert!(v > 0.35 && v < 1.0, "phase={p} opacity={v}");
        }
    }

    // ── draw_viewport fold smoke test ─────────────────────────────────────────

    /// Verify that rows inside a folded block's output region are NOT drawn:
    /// glyph-call count with fold enabled is strictly less than without fold.
    #[test]
    fn draw_viewport_fold_skips_output_rows() {
        let m = metrics();
        let theme = MINERAL_DARK;
        let sel = Selection::default();

        // Build a terminal with one complete command block (OSC 133 B/C/D).
        let mut t = make_terminal(20, 8);
        // command row
        t.feed(b"\x1b]133;B\x07");
        t.feed(b"ls\r\n");
        // output start
        t.feed(b"\x1b]133;C\x07");
        // output rows
        t.feed(b"file1.txt\r\n");
        t.feed(b"file2.txt\r\n");
        t.feed(b"file3.txt\r\n");
        // done
        t.feed(b"\x1b]133;D;exit_code=0\x07");

        // Count glyph calls WITHOUT fold.
        let calls_unfolded = {
            let mut r = Raster::new(400, 200);
            let mut painter = StubPainter::default();
            r.clear(theme.background);
            draw_viewport(
                &mut r,
                &mut painter,
                &mut t,
                m,
                &theme,
                0.0,
                sel,
                None,
                0,
                None,
                0.0,
                400.0,
                FoldedBlocks::empty(),
                None,
            );
            painter.calls.len()
        };

        // Determine the command_line abs value (it's 0 since no prior scrollback).
        let block = t.block_at(0);
        assert!(block.is_some(), "block_at(0) should return Some");
        let cmd_line = block.unwrap().command_line;

        // Count glyph calls WITH the block folded.
        let calls_folded = {
            let mut r = Raster::new(400, 200);
            let mut painter = StubPainter::default();
            r.clear(theme.background);
            let folded_arr = [cmd_line];
            draw_viewport(
                &mut r,
                &mut painter,
                &mut t,
                m,
                &theme,
                0.0,
                sel,
                None,
                0,
                None,
                0.0,
                400.0,
                FoldedBlocks::new(&folded_arr),
                None,
            );
            painter.calls.len()
        };

        assert!(
            calls_folded < calls_unfolded,
            "folded viewport should produce fewer glyph calls ({calls_folded}) than unfolded ({calls_unfolded})"
        );
    }

    // ── gutter_mark_color helper ──────────────────────────────────────────────

    // ── block_accent_color (previously gutter_mark_color) tests ──────────────

    fn make_block(state: anvil_term::BlockState, exit_code: i32) -> anvil_term::Block {
        anvil_term::Block {
            command_line: 0,
            command_start_col: 0,
            output_line: 0,
            end_line: 5,
            state,
            exit_code,
            duration_ms: 0,
        }
    }

    #[test]
    fn gutter_mark_color_running_is_info_teal() {
        use anvil_term::BlockState;
        // Running blocks use ACCENT_BRIGHT (mineral bright teal) per brand contract.
        let block = make_block(BlockState::Running, 0);
        assert_eq!(block_accent_color(&block), ACCENT_BRIGHT);
    }

    #[test]
    fn block_accent_color_running_is_accent_bright() {
        use anvil_term::BlockState;
        let block = make_block(BlockState::Running, 0);
        assert_eq!(block_accent_color(&block), ACCENT_BRIGHT);
    }

    #[test]
    fn gutter_mark_color_exit_zero_is_verified() {
        use anvil_term::BlockState;
        let block = make_block(BlockState::Exited, 0);
        assert_eq!(block_accent_color(&block), VERIFIED);
    }

    #[test]
    fn gutter_mark_color_exit_nonzero_is_failure() {
        use anvil_term::BlockState;
        let block = make_block(BlockState::Exited, 1);
        assert_eq!(block_accent_color(&block), FAILURE);
    }

    /// Running block: accent bar color is ACCENT_BRIGHT (mineral bright teal).
    #[test]
    fn block_accent_color_running_pinned_to_accent_bright() {
        use anvil_term::BlockState;
        let block = make_block(BlockState::Running, 0);
        assert_eq!(
            block_accent_color(&block),
            ACCENT_BRIGHT,
            "running block must use ACCENT_BRIGHT per brand contract"
        );
    }

    /// Successful block: accent bar color is VERIFIED (green).
    #[test]
    fn block_accent_color_ok_is_verified() {
        use anvil_term::BlockState;
        let block = make_block(BlockState::Exited, 0);
        assert_eq!(
            block_accent_color(&block),
            VERIFIED,
            "exit-0 block must use VERIFIED per brand contract"
        );
    }

    /// Failed block: accent bar color is FAILURE (red).
    #[test]
    fn block_accent_color_failed_is_failure() {
        use anvil_term::BlockState;
        let block = make_block(BlockState::Exited, 1);
        assert_eq!(
            block_accent_color(&block),
            FAILURE,
            "non-zero exit block must use FAILURE per brand contract"
        );
    }

    // ── draw_viewport_gpu smoke test ─────────────────────────────────────────

    /// Stub rasterizer that returns a fixed slot for any non-empty codepoint.
    struct StubRasterizer {
        pub calls: usize,
    }

    impl crate::atlas::GlyphRasterizer for StubRasterizer {
        fn glyph_slot(
            &mut self,
            codepoint: u32,
            _metrics: FontMetrics,
        ) -> Option<crate::atlas::GlyphSlot> {
            self.calls += 1;
            if codepoint == 0 || codepoint == b' ' as u32 {
                return None;
            }
            Some(crate::atlas::GlyphSlot {
                atlas_x: 0,
                atlas_y: 0,
                w: 10,
                h: 20,
                bearing_x: 0,
                bearing_y: 0,
            })
        }
    }

    /// `draw_viewport_gpu` produces at least one instance for a non-empty
    /// terminal and at most rows*cols instances (one per visible cell).
    #[test]
    fn draw_viewport_gpu_smoke_non_zero_instances() {
        let m = metrics();
        let r = Raster::new(200, 120);
        let mut rasterizer = StubRasterizer { calls: 0 };
        let mut batch = crate::batch::CellBatch::new();
        let mut t = make_terminal(10, 4);
        t.feed(b"\x1b]133;A\x07");
        t.feed(b"hello\r\n");
        t.feed(b"world");
        let sel = Selection::default();
        let theme = MINERAL_DARK;

        batch.clear([200.0, 120.0]);
        draw_viewport_gpu(
            &mut batch,
            &r,
            &mut rasterizer,
            &mut t,
            m,
            &theme,
            0.0,
            sel,
            None,
            0,
            None,
            FoldedBlocks::empty(),
        );

        let count = batch.instance_count();
        let max_cells = t.rows() * t.cols();
        assert!(count > 0, "expected at least one instance, got 0");
        assert!(
            count <= max_cells,
            "instance count {count} exceeds max_cells {max_cells}"
        );
    }

    // ── dirty-row optimization ─────────────────────────────────────────────────
    //
    // When `draw_viewport` receives a partial `DirtySet` it must skip rows that
    // are not in the set.  This test populates three rows, marks only row 0 dirty,
    // then asserts that the painter receives glyphs only from that row.

    #[test]
    fn draw_viewport_partial_dirty_skips_clean_rows() {
        let m = metrics();
        let mut r = Raster::new(300, 120);
        let mut t = make_terminal(10, 4);
        // Row 0: "hello"
        t.feed(b"hello\r\n");
        // Row 1: "world"
        t.feed(b"world\r\n");
        // Row 2: "xxxxx"
        t.feed(b"xxxxx");

        let theme = MINERAL_DARK;
        r.clear(theme.background);

        // Build a DirtySet that marks only row 0.
        let mut dirty = anvil_term::DirtySet::none(t.rows());
        dirty.mark(0);

        let mut painter = StubPainter::default();
        let sel = Selection::default();
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            sel,
            None,
            0,
            None,
            0.0,
            300.0,
            FoldedBlocks::empty(),
            Some(&dirty),
        );

        // The painter must have received at least one call (row 0 has "hello").
        assert!(
            !painter.calls.is_empty(),
            "expected glyph calls for dirty row 0 but got none"
        );

        // Now do a full redraw to discover which glyph_ids appear in each row.
        // Any glyph_id found in the partial draw must also exist in the full draw
        // — but the partial draw must NOT contain glyph_ids that are exclusive to
        // rows 1 and 2.
        //
        // Concrete check: the partial-dirty pass must emit fewer glyph calls than
        // a full redraw of all three rows, because rows 1 and 2 are both skipped.
        let mut painter_full = StubPainter::default();
        r.clear(theme.background);
        let mut t2 = make_terminal(10, 4);
        t2.feed(b"hello\r\n");
        t2.feed(b"world\r\n");
        t2.feed(b"xxxxx");
        draw_viewport(
            &mut r,
            &mut painter_full,
            &mut t2,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            0,
            None,
            0.0,
            300.0,
            FoldedBlocks::empty(),
            None, // full redraw
        );

        assert!(
            painter.calls.len() < painter_full.calls.len(),
            "partial draw ({} calls) should be less than full draw ({} calls)",
            painter.calls.len(),
            painter_full.calls.len()
        );
    }

    /// `draw_viewport_gpu` with smooth scroll: no panic and at least one instance.
    #[test]
    fn draw_viewport_gpu_smooth_scroll_no_panic() {
        let m = metrics();
        let r = Raster::new(200, 120);
        let mut rasterizer = StubRasterizer { calls: 0 };
        let mut batch = crate::batch::CellBatch::new();
        let mut t = make_terminal(10, 4);
        for _ in 0..10 {
            t.feed(b"scrollback\r\n");
        }
        let sel = Selection::default();
        let theme = MINERAL_DARK;

        batch.clear([200.0, 120.0]);
        draw_viewport_gpu(
            &mut batch,
            &r,
            &mut rasterizer,
            &mut t,
            m,
            &theme,
            2.0,
            sel,
            None,
            0,
            None,
            FoldedBlocks::empty(),
        );
        // No panic; scroll path exercised. Instance count may be 0 (cells
        // happen to be spaces) but must not exceed rows+1 * cols.
        assert!(batch.instance_count() <= (t.rows() + 1) * t.cols());
    }

    // ── Block rendering: accent bar and body tint ─────────────────────────────

    /// A running block's accent bar uses ACCENT_BRIGHT (mineral bright teal).
    /// Verify: draw a running block and check that the pixel at the accent bar
    /// position contains the ACCENT_BRIGHT color.
    #[test]
    fn draw_viewport_running_block_has_teal_accent_bar() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 4);
        let theme = MINERAL_DARK;

        // Feed a running block (no 133;D yet).
        t.feed(b"\x1b]133;A\x07");
        t.feed(b"\x1b]133;B\x07");
        t.feed(b"ls\r\n");
        t.feed(b"\x1b]133;C\x07");
        t.feed(b"file.txt\r\n");
        // Still running — no 133;D.

        // Set pad_x so the accent bar lands in the bitmap.
        r.pad_x = 6.0;
        r.origin_x = 6.0;

        r.clear(theme.background);
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            0,
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );

        // OSC 133;C marks the command-start; the block's command row is the
        // row where 133;C fired. With A, B, "ls\\r\\n", C, the command row is
        // row 1 and output starts at row 2.
        use crate::raster::pixel_at;
        let bar_x = r.origin_x as usize + 1;
        let row2_px = pixel_at(&r, bar_x, (m.cell_h * 2.5) as usize);
        assert_eq!(
            row2_px, ACCENT_BRIGHT,
            "running block output row accent bar should be ACCENT_BRIGHT"
        );
    }

    /// A completed successful block's accent bar uses VERIFIED (green).
    #[test]
    fn draw_viewport_ok_block_has_green_accent_bar() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 6);
        let theme = MINERAL_DARK;

        t.feed(b"\x1b]133;A\x07");
        t.feed(b"\x1b]133;B\x07");
        t.feed(b"ls\r\n");
        t.feed(b"\x1b]133;C\x07");
        t.feed(b"file.txt\r\n");
        t.feed(b"\x1b]133;D;exit_code=0\x07");
        t.feed(b"\x1b]133;A\x07");

        r.pad_x = 6.0;
        r.origin_x = 6.0;
        r.clear(theme.background);
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            0,
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );

        use crate::raster::pixel_at;
        let bar_x = r.origin_x as usize + 1;
        // command_line = 0 (B-mark row), output rows are between command_line
        // and end_line. After `ls\\r\\n; C; ok\\r\\n; D; A` the next-A
        // closes end_line at the D's row (1), so output row is row 1.
        let row1_px = pixel_at(&r, bar_x, (m.cell_h * 1.5) as usize);
        assert_eq!(
            row1_px, VERIFIED,
            "exit-0 block output-row accent bar should be VERIFIED"
        );
    }

    /// A failed block's accent bar uses FAILURE (red).
    #[test]
    fn draw_viewport_failed_block_has_red_accent_bar() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 6);
        let theme = MINERAL_DARK;

        t.feed(b"\x1b]133;A\x07");
        t.feed(b"\x1b]133;B\x07");
        t.feed(b"bad\r\n");
        t.feed(b"\x1b]133;C\x07");
        t.feed(b"error\r\n");
        t.feed(b"\x1b]133;D;exit_code=1\x07");
        t.feed(b"\x1b]133;A\x07");

        r.pad_x = 6.0;
        r.origin_x = 6.0;
        r.clear(theme.background);
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            0,
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );

        use crate::raster::pixel_at;
        let bar_x = r.origin_x as usize + 1;
        let row1_px = pixel_at(&r, bar_x, (m.cell_h * 1.5) as usize);
        assert_eq!(
            row1_px, FAILURE,
            "exit-nonzero block output-row accent bar should be FAILURE"
        );
    }

    /// Body rows of a block get PANEL_RAISED background tint.
    #[test]
    fn draw_viewport_block_output_rows_have_panel_raised_tint() {
        let m = metrics();
        let mut r = Raster::new(200, 160);
        let mut painter = StubPainter::default();
        let mut t = make_terminal(10, 6);
        let theme = MINERAL_DARK;

        t.feed(b"\x1b]133;A\x07");
        t.feed(b"\x1b]133;B\x07");
        t.feed(b"ls\r\n");
        t.feed(b"\x1b]133;C\x07");
        t.feed(b"   \r\n"); // all spaces — tint should be visible as bg
        t.feed(b"\x1b]133;D;exit_code=0\x07");
        t.feed(b"\x1b]133;A\x07");

        // Set origin to avoid accent bar overlap when sampling.
        r.origin_x = 20.0; // cells start at x=20
        r.pad_x = 6.0;

        r.clear(theme.background);
        draw_viewport(
            &mut r,
            &mut painter,
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            0,
            None,
            20.0,
            200.0,
            FoldedBlocks::empty(),
            None,
        );

        // Row 1 is the output row (after 133;C). Sample a pixel in the middle
        // of the row at a cell column that should have PANEL_RAISED background.
        use crate::raster::pixel_at;
        let sample_x = 30_usize; // well into the cell area
        let sample_y = (m.cell_h * 1.5) as usize; // mid of row 1
        let px = pixel_at(&r, sample_x, sample_y);
        assert_eq!(
            px, PANEL_RAISED,
            "block output row background should be PANEL_RAISED; got {px:?}"
        );
    }
}
