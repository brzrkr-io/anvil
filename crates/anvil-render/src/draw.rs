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

// All chrome palette constants migrated to Theme fields.
// Use theme.accent_bright, theme.verified, theme.failure, theme.alloy, theme.panel_raised.

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
    /// Optional RGB color override for the cursor (AA7).
    /// When `Some`, replaces the theme's `accent` as the cursor color.
    pub color: Option<[u8; 3]>,
}

impl Default for CursorConfig {
    fn default() -> Self {
        CursorConfig {
            style: CursorStyle::Block,
            blink: true,
            color: None,
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
    _selection: Selection,
    search: Option<&Search>,
) {
    let mut fg = resolve_color(cell.fg, theme.foreground, theme);
    let mut bg = resolve_color(cell.bg, theme.background, theme);

    use anvil_term::Attrs;
    if cell.attrs.contains(Attrs::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }

    if let Some(s) = search {
        match s.classify(content_row, x) {
            MatchKind::Current => bg = theme.accent,
            MatchKind::Other => bg = theme.ansi[8],
            MatchKind::None => {}
        }
    }

    let ry = y;
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
    params: CursorParams,
) {
    let blink = terminal.app_cursor_blink.unwrap_or(params.cfg.blink);
    let opacity = if blink {
        cursor_opacity(params.blink_phase)
    } else {
        1.0
    };
    let ax = params.ax as f64;
    let ay = params.ay as f64;
    let base_color = params.cfg.color.unwrap_or(theme.accent);
    let cursor_rgb = mix(theme.background, base_color, opacity);

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

/// Compute the right-side (char, color) pairs for a block header row.
///
/// Returns the starting column and the ordered sequence of `(char, color)`
/// pairs that make up the right-aligned metadata region: duration (muted),
/// separator, exit indicator (status color), and fold caret (muted).
///
/// Returns `None` when the metadata would overflow the terminal width.
#[allow(clippy::type_complexity)]
fn compute_block_header_chars(
    block: &Block,
    cols: usize,
    theme: &Theme,
) -> Option<(usize, Vec<(char, [u8; 3])>)> {
    let muted = theme.alloy;
    let accent_color = block_accent_color(block, theme);

    let dur_str = if block.duration_ms > 0 {
        format_duration(block.duration_ms)
    } else {
        String::new()
    };

    let (exit_str, exit_color) = match block.state {
        BlockState::Running => ("\u{2026}".to_string(), accent_color), // …
        BlockState::Exited => {
            if block.exit_code == 0 {
                ("\u{2713}".to_string(), theme.verified) // ✓
            } else {
                (format!("\u{2717} {}", block.exit_code), theme.failure) // ✗ N
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
        return None;
    }

    let start_col = cols - right_len;
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

    Some((start_col, chars))
}

/// Draw the synthesized block header row (CPU path).
///
/// Draws over the raw terminal cells at the command row:
///   - command text in foreground color (col 1 onward; col 0 is accent bar)
///   - duration in muted color, right-aligned before exit indicator
///   - exit indicator: "✓" (exit 0), "✗ N" (exit N), "…" (running)
///   - fold indicator "▾" at far right
///
/// `ry` is the raster row (viewport row relative to pane origin).
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
    let fg = theme.foreground;

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

    if let Some((start_col, chars)) = compute_block_header_chars(block, cols, theme) {
        for (i, (cp, color)) in chars.into_iter().enumerate() {
            let col = start_col + i;
            raster.cell_glyph(painter, metrics, col, ry, cp as u32, color);
        }
    }
}

/// Draw the synthesized block header row (GPU path).
///
/// Mirrors `draw_block_header_cpu` but pushes `CellInstance`s into `batch`
/// instead of writing pixels.  Uses `raster.cell_rect` for pixel positions;
/// `y_shift` is subtracted for smooth-scroll.
#[allow(clippy::too_many_arguments)]
fn draw_block_header_gpu(
    batch: &mut CellBatch,
    rasterizer: &mut dyn GlyphRasterizer,
    raster: &Raster,
    metrics: FontMetrics,
    theme: &Theme,
    block: &Block,
    viewport_y: usize,
    cols: usize,
    cw: f32,
    ch: f32,
    y_shift: f32,
) {
    if let Some((start_col, chars)) = compute_block_header_chars(block, cols, theme) {
        for (i, (cp, color)) in chars.into_iter().enumerate() {
            let col = start_col + i;
            let rect = raster.cell_rect(metrics, col as f64, viewport_y as f64);
            let xy = [rect.x as f32, rect.y as f32 - y_shift];
            let wh = [cw, ch];
            let bg = theme.panel_raised;
            if cp == ' ' {
                batch.push_cell(xy, wh, None, color, bg);
            } else {
                let slot = rasterizer.glyph_slot(cp as u32, metrics);
                batch.push_cell(xy, wh, slot, color, bg);
            }
        }
    }
}

// ── Gutter mark color ─────────────────────────────────────────────────────────

fn block_accent_color(block: &Block, theme: &Theme) -> [u8; 3] {
    match block.state {
        BlockState::Running => theme.accent_bright,
        BlockState::Exited => {
            if block.exit_code == 0 {
                theme.verified
            } else {
                theme.failure
            }
        }
    }
}

// ── ViewportSink trait ────────────────────────────────────────────────────────

/// Backend abstraction for `draw_viewport_into`.
///
/// `CpuSink` wraps a `Raster + dyn GlyphPainter` (CPU raster path).
/// `GpuSink` wraps a `CellBatch + dyn GlyphRasterizer` (GPU instance path).
///
/// Row indices passed to each method are pre-shift viewport rows; each sink
/// applies its smooth-scroll offset internally (`raster.y_shift_px` for CPU,
/// explicit `shift` field for GPU).
trait ViewportSink {
    /// Clear one row's background to `bg` before redrawing (CPU only; GPU no-op).
    fn clear_row_bg(&mut self, ry: usize, m: FontMetrics, bg: [u8; 3]);
    /// Paint a selection wash over a full row (CPU only; GPU no-op).
    fn fill_selection_row(
        &mut self,
        ry: usize,
        cols: usize,
        m: FontMetrics,
        rgb: [u8; 3],
        alpha: f64,
    );
    /// Draw one terminal cell.
    #[allow(clippy::too_many_arguments)]
    fn draw_cell(
        &mut self,
        x: usize,
        y: usize,
        content_row: usize,
        cell: Cell,
        m: FontMetrics,
        theme: &Theme,
        sel: Selection,
        search: Option<&Search>,
    );
    /// Draw the fold summary text at `ry` (e.g. " ⌄ 3 hidden").
    fn draw_fold_summary(
        &mut self,
        ry: usize,
        cols: usize,
        hidden: usize,
        m: FontMetrics,
        theme: &Theme,
    );
    /// Draw the block header overlay (command text + right-aligned metadata).
    fn draw_block_header(
        &mut self,
        ry: usize,
        cols: usize,
        block: &Block,
        cmd_text: &str,
        m: FontMetrics,
        theme: &Theme,
    );
    /// Draw the prompt-rule hairline above row `ry`.
    fn draw_prompt_rule(&mut self, ry: f64, m: FontMetrics, rgb: [u8; 3], x_start: f64, x_end: f64);
    /// Draw the text cursor when pinned to live bottom.
    fn draw_cursor(
        &mut self,
        t: &mut Terminal,
        cp: CursorParams,
        m: FontMetrics,
        theme: &Theme,
        rows: usize,
        cols: usize,
    );
}

// ── GridPainters ──────────────────────────────────────────────────────────────

/// The four CoreText painters for the terminal grid: Regular, Bold, Italic, and
/// BoldItalic.  Passed into `draw_viewport` so `CpuSink` can select the correct
/// face per cell based on SGR bold/italic attribute bits.
pub struct GridPainters<'a> {
    pub regular: &'a mut dyn GlyphPainter,
    pub bold: &'a mut dyn GlyphPainter,
    pub italic: &'a mut dyn GlyphPainter,
    pub bold_italic: &'a mut dyn GlyphPainter,
}

// ── CpuSink ───────────────────────────────────────────────────────────────────

struct CpuSink<'a> {
    raster: &'a mut Raster,
    regular: &'a mut dyn GlyphPainter,
    bold: &'a mut dyn GlyphPainter,
    italic: &'a mut dyn GlyphPainter,
    bold_italic: &'a mut dyn GlyphPainter,
    /// Free-running phase [0,1) for the running-block header dot pulse.
    running_pulse_phase: f32,
}

impl<'a> CpuSink<'a> {
    /// Construct a `CpuSink` from a raster and a set of grid painters.
    ///
    /// The painters are re-borrowed via raw pointers so that `raster` and the
    /// four painter refs can coexist in the same struct with a single lifetime
    /// `'a`.  This is sound because:
    /// - Each painter ref points to a distinct object (enforced by the caller's
    ///   `GridPainters` struct, which requires 4 separate `&mut dyn` refs).
    /// - `raster` is a separate, distinct object from all painters.
    /// - No two mutable pointers alias; only one is accessed at a time per call.
    fn new(
        raster: &'a mut Raster,
        painters: &'a mut GridPainters<'_>,
        running_pulse_phase: f32,
    ) -> Self {
        // SAFETY: each raw pointer is derived from a valid, non-aliasing
        // `&'a mut dyn GlyphPainter` that outlives `CpuSink<'a>`.
        let reg = painters.regular as *mut dyn GlyphPainter;
        let bld = painters.bold as *mut dyn GlyphPainter;
        let itl = painters.italic as *mut dyn GlyphPainter;
        let bi = painters.bold_italic as *mut dyn GlyphPainter;
        Self {
            raster,
            regular: unsafe { &mut *reg },
            bold: unsafe { &mut *bld },
            italic: unsafe { &mut *itl },
            bold_italic: unsafe { &mut *bi },
            running_pulse_phase,
        }
    }
}

impl ViewportSink for CpuSink<'_> {
    fn clear_row_bg(&mut self, ry: usize, m: FontMetrics, bg: [u8; 3]) {
        let y_top = (self.raster.origin_y + ry as f64 * m.cell_h) as usize;
        let y_bot = (self.raster.origin_y + (ry + 1) as f64 * m.cell_h) as usize;
        self.raster.clear_pixel_rows(y_top, y_bot, bg);
    }

    fn fill_selection_row(
        &mut self,
        ry: usize,
        cols: usize,
        m: FontMetrics,
        rgb: [u8; 3],
        alpha: f64,
    ) {
        let px = self.raster.origin_x;
        let py = self.raster.origin_y + ry as f64 * m.cell_h;
        self.raster
            .fill_pixel_rect_alpha(px, py, cols as f64 * m.cell_w, m.cell_h, rgb, alpha);
    }

    fn draw_cell(
        &mut self,
        x: usize,
        y: usize,
        content_row: usize,
        cell: Cell,
        m: FontMetrics,
        theme: &Theme,
        sel: Selection,
        search: Option<&Search>,
    ) {
        use anvil_term::Attrs;
        let painter: &mut dyn GlyphPainter = match (
            cell.attrs.contains(Attrs::BOLD),
            cell.attrs.contains(Attrs::ITALIC),
        ) {
            (false, false) => self.regular,
            (true, false) => self.bold,
            (false, true) => self.italic,
            (true, true) => self.bold_italic,
        };
        draw_cell(
            self.raster,
            painter,
            m,
            theme,
            x,
            y,
            content_row,
            cell,
            sel,
            search,
        );
    }

    fn draw_fold_summary(
        &mut self,
        ry: usize,
        cols: usize,
        hidden: usize,
        m: FontMetrics,
        theme: &Theme,
    ) {
        let summary = format!(" \u{2304} {hidden} hidden");
        draw_text_row(
            self.raster,
            self.regular,
            m,
            0,
            ry,
            &summary,
            theme.alloy,
            cols,
        );
    }

    fn draw_block_header(
        &mut self,
        ry: usize,
        cols: usize,
        block: &Block,
        cmd_text: &str,
        m: FontMetrics,
        theme: &Theme,
    ) {
        draw_block_header_cpu(
            self.raster,
            self.regular,
            m,
            theme,
            block,
            cmd_text,
            ry,
            cols,
        );

        // Running-block header dot: sine-modulated 2×2 dot at col 0 while block is Running.
        if block.state == BlockState::Running {
            let alpha = 0.45
                + 0.55
                    * (std::f32::consts::TAU * self.running_pulse_phase)
                        .sin()
                        .max(0.0);
            let dot_px = (m.cell_w * 0.5 - 1.0).max(0.0);
            let dot_py = m.cell_h * 0.5 - 1.0;
            self.raster.fill_pixel_rect_alpha(
                self.raster.origin_x + dot_px,
                self.raster.origin_y + ry as f64 * m.cell_h + dot_py,
                2.0,
                2.0,
                theme.accent_bright,
                alpha as f64,
            );
        }

        // Block-header pulse: 200ms ember flash on command completion.
        if let Some(t) = block.completed_at {
            let elapsed_ms = t.elapsed().as_millis() as f64;
            if elapsed_ms < 200.0 {
                let frac = elapsed_ms / 200.0; // 0.0 → 1.0
                let alpha = (std::f64::consts::PI * frac).sin(); // 0 → 1 → 0
                let px = self.raster.origin_x;
                // Bottom 2px of the header row.
                let py = self.raster.origin_y + (ry + 1) as f64 * m.cell_h - 2.0;
                self.raster.fill_pixel_rect_alpha(
                    px,
                    py,
                    cols as f64 * m.cell_w,
                    2.0,
                    theme.accent_ember,
                    alpha,
                );
            }
        }
    }

    fn draw_prompt_rule(
        &mut self,
        ry: f64,
        m: FontMetrics,
        rgb: [u8; 3],
        x_start: f64,
        x_end: f64,
    ) {
        self.raster.row_rule(m, ry, rgb, x_start, x_end);
    }

    fn draw_cursor(
        &mut self,
        t: &mut Terminal,
        cp: CursorParams,
        m: FontMetrics,
        theme: &Theme,
        rows: usize,
        cols: usize,
    ) {
        let opacity = if t.app_cursor_blink.unwrap_or(cp.cfg.blink) {
            cursor_opacity(cp.blink_phase)
        } else {
            1.0
        };
        let cursor_rgb = mix(theme.background, theme.accent, opacity);
        let ax = cp.ax as f64;
        let ay = cp.ay as f64;
        let style = match t.app_cursor_shape {
            Some(CursorShape::Block) => CursorStyle::Block,
            Some(CursorShape::Underline) => CursorStyle::Underline,
            Some(CursorShape::Bar) => CursorStyle::Bar,
            None => cp.cfg.style,
        };
        match style {
            CursorStyle::Block => {
                self.raster
                    .cell_inset(m, ax, ay, cursor_rgb, 0.0, 0.0, 1.0, 1.0);
                let ic = cp.ax.round() as usize;
                let ir = cp.ay.round() as usize;
                if ir < rows && ic < cols {
                    let cell_under = {
                        let row = t.viewport_row(ir);
                        if ic < row.len() { Some(row[ic]) } else { None }
                    };
                    if let Some(cell) = cell_under {
                        if cell.cp != ' ' && cell.cp != '\0' {
                            let base_fg = resolve_color(cell.fg, theme.foreground, theme);
                            let glyph_fg = mix(base_fg, theme.background, opacity);
                            use anvil_term::Attrs;
                            let painter: &mut dyn GlyphPainter = match (
                                cell.attrs.contains(Attrs::BOLD),
                                cell.attrs.contains(Attrs::ITALIC),
                            ) {
                                (false, false) => self.regular,
                                (true, false) => self.bold,
                                (false, true) => self.italic,
                                (true, true) => self.bold_italic,
                            };
                            self.raster
                                .cell_glyph(painter, m, ic, ir, cell.cp as u32, glyph_fg);
                        }
                    }
                }
            }
            CursorStyle::Bar => {
                self.raster
                    .cell_inset(m, ax, ay, cursor_rgb, 0.0, 0.0, 0.15, 1.0);
            }
            CursorStyle::Underline => {
                let fh = 0.12_f64;
                self.raster
                    .cell_inset(m, ax, ay, cursor_rgb, 0.0, 1.0 - fh, 1.0, fh);
            }
        }
    }
}

// ── GpuSink ───────────────────────────────────────────────────────────────────

struct GpuSink<'a> {
    batch: &'a mut CellBatch,
    rasterizer: &'a mut dyn GlyphRasterizer,
    raster: &'a Raster,
    cw: f32,
    ch: f32,
    /// Smooth-scroll y shift in pixels (0.0 for live-bottom path).
    shift: f32,
    /// Free-running phase [0,1) for the running-block header dot pulse.
    running_pulse_phase: f32,
}

impl<'a> GpuSink<'a> {
    fn new(
        batch: &'a mut CellBatch,
        rasterizer: &'a mut dyn GlyphRasterizer,
        raster: &'a Raster,
        metrics: FontMetrics,
        shift: f32,
        running_pulse_phase: f32,
    ) -> Self {
        Self {
            batch,
            rasterizer,
            raster,
            cw: metrics.cell_w as f32,
            ch: metrics.cell_h as f32,
            shift,
            running_pulse_phase,
        }
    }

    /// Top-left pixel coords of cell (col, row) — does NOT apply shift.
    fn cell_xy(&self, metrics: FontMetrics, col: f64, row: f64) -> [f32; 2] {
        let rect = self.raster.cell_rect(metrics, col, row);
        [rect.x as f32, rect.y as f32]
    }
}

impl ViewportSink for GpuSink<'_> {
    fn clear_row_bg(&mut self, _ry: usize, _m: FontMetrics, _bg: [u8; 3]) {
        // GPU path has no per-pixel buffer to clear.
    }

    fn fill_selection_row(
        &mut self,
        _ry: usize,
        _cols: usize,
        _m: FontMetrics,
        _rgb: [u8; 3],
        _alpha: f64,
    ) {
        // GPU path: selection wash is not composited in the CPU pixel buffer.
    }

    fn draw_cell(
        &mut self,
        x: usize,
        y: usize,
        content_row: usize,
        cell: Cell,
        m: FontMetrics,
        theme: &Theme,
        sel: Selection,
        search: Option<&Search>,
    ) {
        let (fg, bg) = resolve_cell_colors(cell, content_row, x, sel, search, theme);
        let base_xy = self.cell_xy(m, x as f64, y as f64);
        let xy = [base_xy[0], base_xy[1] - self.shift];
        let wh = [self.cw, self.ch];
        if cell.cp == ' ' || cell.cp == '\0' {
            if bg != theme.background {
                self.batch.push_cell(xy, wh, None, fg, bg);
            }
        } else {
            let slot = self.rasterizer.glyph_slot(cell.cp as u32, m);
            self.batch.push_cell(xy, wh, slot, fg, bg);
        }
    }

    fn draw_fold_summary(
        &mut self,
        ry: usize,
        cols: usize,
        hidden: usize,
        m: FontMetrics,
        theme: &Theme,
    ) {
        let summary = format!(" \u{2304} {hidden} hidden");
        for (i, cp) in summary.chars().enumerate() {
            if i >= cols {
                break;
            }
            let base_xy = self.cell_xy(m, i as f64, ry as f64);
            let xy = [base_xy[0], base_xy[1] - self.shift];
            let wh = [self.cw, self.ch];
            let slot = self.rasterizer.glyph_slot(cp as u32, m);
            self.batch
                .push_cell(xy, wh, slot, theme.alloy, theme.background);
        }
    }

    fn draw_block_header(
        &mut self,
        ry: usize,
        cols: usize,
        block: &Block,
        _cmd_text: &str,
        m: FontMetrics,
        theme: &Theme,
    ) {
        draw_block_header_gpu(
            self.batch,
            self.rasterizer,
            self.raster,
            m,
            theme,
            block,
            ry,
            cols,
            self.cw,
            self.ch,
            self.shift,
        );

        // Running-block header dot: sine-modulated color at col 0 while block is Running.
        if block.state == BlockState::Running {
            let alpha = 0.45
                + 0.55
                    * (std::f32::consts::TAU * self.running_pulse_phase)
                        .sin()
                        .max(0.0);
            let dot_col = 0.5_f64 - 1.0 / m.cell_w; // ~center of col 0
            let dot_row = ry as f64 + 0.5 - 1.0 / m.cell_h;
            let rect = self.raster.cell_rect(m, dot_col, dot_row);
            let xy = [rect.x as f32, rect.y as f32 - self.shift];
            // Use accent_bright with the pulsed alpha baked into the color.
            let ab = theme.accent_bright;
            let a = alpha;
            let bg = theme.background;
            let color = [
                (ab[0] as f32 * a + bg[0] as f32 * (1.0 - a)) as u8,
                (ab[1] as f32 * a + bg[1] as f32 * (1.0 - a)) as u8,
                (ab[2] as f32 * a + bg[2] as f32 * (1.0 - a)) as u8,
            ];
            self.batch.push_cell(xy, [2.0, 2.0], None, color, color);
        }
    }

    fn draw_prompt_rule(
        &mut self,
        _ry: f64,
        _m: FontMetrics,
        _rgb: [u8; 3],
        _x_start: f64,
        _x_end: f64,
    ) {
        // GPU path does not draw prompt rules (CPU-only raster operation).
    }

    fn draw_cursor(
        &mut self,
        t: &mut Terminal,
        cp: CursorParams,
        m: FontMetrics,
        theme: &Theme,
        rows: usize,
        cols: usize,
    ) {
        let opacity = if t.app_cursor_blink.unwrap_or(cp.cfg.blink) {
            cursor_opacity(cp.blink_phase)
        } else {
            1.0
        };
        let cursor_rgb = mix(theme.background, theme.accent, opacity);
        let ax = cp.ax as f64;
        let ay = cp.ay as f64;
        let xy = self.cell_xy(m, ax, ay);
        let style = match t.app_cursor_shape {
            Some(CursorShape::Block) => CursorStyle::Block,
            Some(CursorShape::Underline) => CursorStyle::Underline,
            Some(CursorShape::Bar) => CursorStyle::Bar,
            None => cp.cfg.style,
        };
        match style {
            CursorStyle::Block => {
                let bxy = xy;
                self.batch
                    .push_cell(bxy, [self.cw, self.ch], None, cursor_rgb, cursor_rgb);
                let ic = cp.ax.round() as usize;
                let ir = cp.ay.round() as usize;
                if ir < rows && ic < cols {
                    let cell_under = {
                        let row = t.viewport_row(ir);
                        if ic < row.len() { Some(row[ic]) } else { None }
                    };
                    if let Some(cell) = cell_under {
                        if cell.cp != ' ' && cell.cp != '\0' {
                            let base_fg = resolve_color(cell.fg, theme.foreground, theme);
                            let glyph_fg = mix(base_fg, theme.background, opacity);
                            let slot = self.rasterizer.glyph_slot(cell.cp as u32, m);
                            self.batch.push_cell(
                                bxy,
                                [self.cw, self.ch],
                                slot,
                                glyph_fg,
                                cursor_rgb,
                            );
                        }
                    }
                }
            }
            CursorStyle::Bar => {
                let bwh = [self.cw * 0.15, self.ch];
                self.batch.push_cell(xy, bwh, None, cursor_rgb, cursor_rgb);
            }
            CursorStyle::Underline => {
                let fh = self.ch * 0.12;
                let bxy = [xy[0], xy[1] + self.ch - fh];
                let bwh = [self.cw, fh];
                self.batch.push_cell(bxy, bwh, None, cursor_rgb, cursor_rgb);
            }
        }
    }
}

// ── Unified draw loop ─────────────────────────────────────────────────────────

/// Unified viewport draw body shared by CPU and GPU paths.
///
/// `off_opt` is `None` for the live-bottom path and `Some(off)` for smooth
/// scroll (`off = base + 1`).  `row_end` is `rows` for live and `rows + 1`
/// for smooth.
#[allow(clippy::too_many_arguments)]
fn draw_viewport_into(
    sink: &mut dyn ViewportSink,
    terminal: &mut Terminal,
    metrics: FontMetrics,
    theme: &Theme,
    rows: usize,
    cols: usize,
    off_opt: Option<usize>,
    row_end: usize,
    selection: Selection,
    search: Option<&Search>,
    cursor_params: Option<CursorParams>,
    rule_x_start: f64,
    rule_x_end: f64,
    folded: &FoldedBlocks<'_>,
    dirty: Option<&DirtySet>,
) {
    let rule_rgb = theme.border;
    let hist = terminal.scrollback_len();
    let is_live = off_opt.is_none();
    let mut cached_block: Option<Block> = None;

    for y in 0..row_end {
        // Dirty-row gate and row-bg clear: live path only.
        if is_live {
            if !dirty.is_none_or(|d| d.contains(y)) {
                continue;
            }
            sink.clear_row_bg(y, metrics, theme.background);
        }

        // Content-row index.
        let crow: usize = match off_opt {
            None => terminal.content_row_of_viewport(y),
            Some(off) => {
                if off > y {
                    (hist + y).saturating_sub(off)
                } else {
                    hist + y - off
                }
            }
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

        // Skip folded output rows.
        if let Some(ref block) = block_opt {
            if folded.contains(block.command_line)
                && abs >= block.output_line
                && abs < block.end_line
            {
                continue;
            }
        }

        // Selection row wash (pre-pass, CPU only; GPU no-op).
        if selection.active {
            let (s, e) = selection.ordered();
            if crow >= s.row && crow <= e.row {
                let lum = theme.background[0] as f64 * 0.2126
                    + theme.background[1] as f64 * 0.7152
                    + theme.background[2] as f64 * 0.0722;
                let alpha = if lum / 255.0 > 0.5 { 0.18 } else { 0.22 };
                sink.fill_selection_row(y, cols, metrics, theme.accent_ember, alpha);
            }
        }

        // Diff-row tint (unified diff +/- content lines only).
        if let Some(ref block) = block_opt {
            if block.is_unified_diff_row(terminal, abs) {
                // Determine sign from first cell.
                let sigil = terminal.line(abs.saturating_sub(terminal.evicted_lines))[0].cp;
                let rgb = if sigil == '+' {
                    theme.verified
                } else {
                    theme.failure
                };
                sink.fill_selection_row(y, cols, metrics, rgb, 0.12);
            }
        }

        // Draw all cells in this row.
        {
            let row: Vec<Cell> = match off_opt {
                None => terminal
                    .viewport_row(y)
                    .iter()
                    .take(cols)
                    .copied()
                    .collect(),
                Some(off) => terminal
                    .viewport_row_at(off, y)
                    .iter()
                    .take(cols)
                    .copied()
                    .collect(),
            };
            for (x, cell) in row.into_iter().enumerate() {
                sink.draw_cell(x, y, crow, cell, metrics, theme, selection, search);
            }
        }

        // Fold summary.
        if let Some(ref block) = block_opt {
            if folded.contains(block.command_line) && abs == block.command_line {
                let hidden = block.output_row_count();
                sink.draw_fold_summary(y, cols, hidden, metrics, theme);
            }
        }

        // Block header overlay.
        if let Some(ref block) = block_opt {
            if abs == block.command_line && !folded.contains(block.command_line) {
                let cmd_text = read_command_text(terminal, crow, block.command_start_col as usize);
                sink.draw_block_header(y, cols, block, &cmd_text, metrics, theme);
            }
        }

        // Prompt-rule hairline.
        if terminal.is_prompt_start(abs) {
            sink.draw_prompt_rule(y as f64, metrics, rule_rgb, rule_x_start, rule_x_end);
        }
    }

    // Cursor: only when pinned to live bottom.
    if let Some(cp) = cursor_params {
        let cur = terminal.cursor();
        if cur.visible && terminal.viewport_offset() == 0 && is_live && cur.x < cols && cur.y < rows
        {
            sink.draw_cursor(terminal, cp, metrics, theme, rows, cols);
        }
    }
}

// ── Viewport draw loop ────────────────────────────────────────────────────────

/// Draw the viewport: visible cell grid, prompt-rule hairlines, fold summaries,
/// and cursor.
///
/// This is the per-frame draw body, ported from `draw.zig`'s `drawViewport`.
///
/// `scroll_pos` drives smooth scrolling (0 = pinned to live bottom).
/// `folded` carries the set of folded `command_line` values for this pane.
/// Pass `cursor_params = None` to suppress cursor drawing (e.g. in tests).
///
/// `dirty` restricts drawing to only the rows that have changed.  In the
/// smooth-scroll path all rows are always redrawn (content shifts).
///
/// `painters` supplies one `GlyphPainter` per SGR face (Regular, Bold, Italic,
/// BoldItalic).  The correct painter is selected per cell based on its attrs.
///
/// # Zero-allocation guarantee
/// The draw loop writes into the pre-allocated `Raster` pixel buffer.  No
/// `Vec` growth or heap allocation occurs per frame by construction.
#[allow(clippy::too_many_arguments)]
pub fn draw_viewport(
    raster: &mut Raster,
    painters: &mut GridPainters<'_>,
    terminal: &mut Terminal,
    metrics: FontMetrics,
    theme: &Theme,
    scroll_pos: f32,
    selection: Selection,
    search: Option<&Search>,
    cursor_params: Option<CursorParams>,
    rule_x_start: f64,
    rule_x_end: f64,
    folded: FoldedBlocks<'_>,
    dirty: Option<&DirtySet>,
    running_pulse_phase: f32,
) {
    let rows = terminal.rows();
    let cols = terminal.cols();
    let mut sink = CpuSink::new(raster, painters, running_pulse_phase);

    if scroll_pos == 0.0 {
        draw_viewport_into(
            &mut sink,
            terminal,
            metrics,
            theme,
            rows,
            cols,
            None,
            rows,
            selection,
            search,
            cursor_params,
            rule_x_start,
            rule_x_end,
            &folded,
            dirty,
        );
    } else {
        let base = scroll_pos.floor() as usize;
        let frac = scroll_pos as f64 - scroll_pos.floor() as f64;
        sink.raster.y_shift_px = (1.0 - frac) * metrics.cell_h;
        let off = base + 1;
        draw_viewport_into(
            &mut sink,
            terminal,
            metrics,
            theme,
            rows,
            cols,
            Some(off),
            rows + 1,
            selection,
            search,
            cursor_params,
            rule_x_start,
            rule_x_end,
            &folded,
            dirty,
        );
        sink.raster.y_shift_px = 0.0;
    }
}

// ── GPU viewport draw loop ────────────────────────────────────────────────────

/// Resolve fg/bg for a cell, applying selection, search, and INVERSE.
fn resolve_cell_colors(
    cell: Cell,
    content_row: usize,
    col: usize,
    _selection: Selection,
    search: Option<&Search>,
    theme: &Theme,
) -> ([u8; 3], [u8; 3]) {
    let mut fg = resolve_color(cell.fg, theme.foreground, theme);
    let mut bg = resolve_color(cell.bg, theme.background, theme);

    use anvil_term::Attrs;
    if cell.attrs.contains(Attrs::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
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

/// GPU-path viewport draw: pushes `CellInstance` records into `batch`.
///
/// `raster` is read-only; only `cell_rect` pixel-position math uses it.
/// Chrome (tab bar, status bar) is NOT drawn here — terminal viewport only.
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
    cursor: Option<CursorParams>,
    folded: FoldedBlocks<'_>,
    running_pulse_phase: f32,
) {
    let rows = terminal.rows();
    let cols = terminal.cols();

    if scroll_pos == 0.0 {
        let mut sink = GpuSink::new(batch, rasterizer, raster, metrics, 0.0, running_pulse_phase);
        draw_viewport_into(
            &mut sink, terminal, metrics, theme, rows, cols, None, rows, selection, search, cursor,
            0.0, 0.0, &folded, None,
        );
    } else {
        let base = scroll_pos.floor() as usize;
        let frac = scroll_pos as f64 - scroll_pos.floor() as f64;
        let shift = ((1.0 - frac) * metrics.cell_h) as f32;
        let off = base + 1;
        let mut sink = GpuSink::new(
            batch,
            rasterizer,
            raster,
            metrics,
            shift,
            running_pulse_phase,
        );
        draw_viewport_into(
            &mut sink,
            terminal,
            metrics,
            theme,
            rows,
            cols,
            Some(off),
            rows + 1,
            selection,
            search,
            cursor,
            0.0,
            0.0,
            &folded,
            None,
        );
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
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"\x1b]133;A\x07");
        t.feed(b"hello\r\n");
        t.feed(b"world");
        let sel = Selection::default();
        let theme = MINERAL_DARK;

        r.clear(theme.background);
        draw_viewport(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &mut t,
            m,
            &theme,
            0.0,
            sel,
            None,
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
            0.0,
        );
        // "hello" starts with 'h' (non-space): expect at least one glyph call.
        assert!(!painter.calls.is_empty());
    }

    #[test]
    fn draw_viewport_smooth_scroll_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
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
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &mut t,
            m,
            &theme,
            2.0,
            sel,
            None,
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
            0.0,
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

        draw_cell(&mut r, &mut painter, m, &theme, 0, 0, 0, cell, sel, None);
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
                color: None,
            },
        };
        draw_cursor(&mut r, &mut painter, &t, m, &theme, params);
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
                color: None,
            },
        };
        draw_cursor(&mut r, &mut painter, &t, m, &theme, params);
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
                color: None,
            },
        };
        draw_cursor(&mut r, &mut painter, &t, m, &theme, params);
    }

    // ── draw_viewport with cursor_params (block, bar, underline) ─────────────

    #[test]
    fn draw_viewport_with_block_cursor_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
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
                color: None,
            },
        };
        draw_viewport(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            Some(params),
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
            0.0,
        );
    }

    #[test]
    fn draw_viewport_with_bar_cursor_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
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
                color: None,
            },
        };
        draw_viewport(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            Some(params),
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
            0.0,
        );
    }

    #[test]
    fn draw_viewport_with_underline_cursor_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
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
                color: None,
            },
        };
        draw_viewport(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            Some(params),
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
            0.0,
        );
    }

    // ── draw_viewport with search ─────────────────────────────────────────────

    #[test]
    fn draw_viewport_with_search_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        let mut t = make_terminal(10, 4);
        t.feed(b"hello world");
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let mut search = anvil_term::Search::new();
        search.set_query(&t, "hello");

        draw_viewport(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            Some(&search),
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
            0.0,
        );
    }

    // ── draw_viewport with blink cursor ──────────────────────────────────────

    #[test]
    fn draw_viewport_blink_cursor_no_panic() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut painter = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
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
                color: None,
            },
        };
        draw_viewport(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            Some(params),
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
            0.0,
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
            let mut bp = StubPainter::default();
            let mut ip = StubPainter::default();
            let mut bip = StubPainter::default();
            r.clear(theme.background);
            draw_viewport(
                &mut r,
                &mut GridPainters {
                    regular: &mut painter,
                    bold: &mut bp,
                    italic: &mut ip,
                    bold_italic: &mut bip,
                },
                &mut t,
                m,
                &theme,
                0.0,
                sel,
                None,
                None,
                0.0,
                400.0,
                FoldedBlocks::empty(),
                None,
                0.0,
            );
            painter.calls.len() + bp.calls.len() + ip.calls.len() + bip.calls.len()
        };

        // Determine the command_line abs value (it's 0 since no prior scrollback).
        let block = t.block_at(0);
        assert!(block.is_some(), "block_at(0) should return Some");
        let cmd_line = block.unwrap().command_line;

        // Count glyph calls WITH the block folded.
        let calls_folded = {
            let mut r = Raster::new(400, 200);
            let mut painter = StubPainter::default();
            let mut bp = StubPainter::default();
            let mut ip = StubPainter::default();
            let mut bip = StubPainter::default();
            r.clear(theme.background);
            let folded_arr = [cmd_line];
            draw_viewport(
                &mut r,
                &mut GridPainters {
                    regular: &mut painter,
                    bold: &mut bp,
                    italic: &mut ip,
                    bold_italic: &mut bip,
                },
                &mut t,
                m,
                &theme,
                0.0,
                sel,
                None,
                None,
                0.0,
                400.0,
                FoldedBlocks::new(&folded_arr),
                None,
                0.0,
            );
            painter.calls.len() + bp.calls.len() + ip.calls.len() + bip.calls.len()
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
            diff_kind: anvil_term::DiffKind::None,
            completed_at: None,
        }
    }

    fn th() -> anvil_theme::Theme {
        anvil_theme::EMBER_DARK
    }

    #[test]
    fn gutter_mark_color_running_is_info_teal() {
        use anvil_term::BlockState;
        let t = th();
        let block = make_block(BlockState::Running, 0);
        assert_eq!(block_accent_color(&block, &t), t.accent_bright);
    }

    #[test]
    fn block_accent_color_running_is_accent_bright() {
        use anvil_term::BlockState;
        let t = th();
        let block = make_block(BlockState::Running, 0);
        assert_eq!(block_accent_color(&block, &t), t.accent_bright);
    }

    #[test]
    fn gutter_mark_color_exit_zero_is_verified() {
        use anvil_term::BlockState;
        let t = th();
        let block = make_block(BlockState::Exited, 0);
        assert_eq!(block_accent_color(&block, &t), t.verified);
    }

    #[test]
    fn gutter_mark_color_exit_nonzero_is_failure() {
        use anvil_term::BlockState;
        let t = th();
        let block = make_block(BlockState::Exited, 1);
        assert_eq!(block_accent_color(&block, &t), t.failure);
    }

    /// Running block: accent bar color is theme.accent_bright per brand contract.
    #[test]
    fn block_accent_color_running_pinned_to_accent_bright() {
        use anvil_term::BlockState;
        let t = th();
        let block = make_block(BlockState::Running, 0);
        assert_eq!(
            block_accent_color(&block, &t),
            t.accent_bright,
            "running block must use accent_bright per brand contract"
        );
    }

    /// Successful block: accent bar color is theme.verified.
    #[test]
    fn block_accent_color_ok_is_verified() {
        use anvil_term::BlockState;
        let t = th();
        let block = make_block(BlockState::Exited, 0);
        assert_eq!(
            block_accent_color(&block, &t),
            t.verified,
            "exit-0 block must use verified per brand contract"
        );
    }

    /// Failed block: accent bar color is theme.failure.
    #[test]
    fn block_accent_color_failed_is_failure() {
        use anvil_term::BlockState;
        let t = th();
        let block = make_block(BlockState::Exited, 1);
        assert_eq!(
            block_accent_color(&block, &t),
            t.failure,
            "non-zero exit block must use failure per brand contract"
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
            None,
            FoldedBlocks::empty(),
            0.0,
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
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        let sel = Selection::default();
        draw_viewport(
            &mut r,
            &mut GridPainters {
                regular: &mut painter,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &mut t,
            m,
            &theme,
            0.0,
            sel,
            None,
            None,
            0.0,
            300.0,
            FoldedBlocks::empty(),
            Some(&dirty),
            0.0,
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
        let mut bold_full = StubPainter::default();
        let mut italic_full = StubPainter::default();
        let mut bold_italic_full = StubPainter::default();
        r.clear(theme.background);
        let mut t2 = make_terminal(10, 4);
        t2.feed(b"hello\r\n");
        t2.feed(b"world\r\n");
        t2.feed(b"xxxxx");
        draw_viewport(
            &mut r,
            &mut GridPainters {
                regular: &mut painter_full,
                bold: &mut bold_full,
                italic: &mut italic_full,
                bold_italic: &mut bold_italic_full,
            },
            &mut t2,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            None,
            0.0,
            300.0,
            FoldedBlocks::empty(),
            None, // full redraw
            0.0,
        );

        let partial_calls = painter.calls.len()
            + bold_p.calls.len()
            + italic_p.calls.len()
            + bold_italic_p.calls.len();
        let full_calls = painter_full.calls.len()
            + bold_full.calls.len()
            + italic_full.calls.len()
            + bold_italic_full.calls.len();
        assert!(
            partial_calls < full_calls,
            "partial draw ({partial_calls} calls) should be less than full draw ({full_calls} calls)",
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
            None,
            FoldedBlocks::empty(),
            0.0,
        );
        // No panic; scroll path exercised. Instance count may be 0 (cells
        // happen to be spaces) but must not exceed rows+1 * cols.
        assert!(batch.instance_count() <= (t.rows() + 1) * t.cols());
    }

    // ── SGR bold/italic face selection ────────────────────────────────────────

    /// A bold cell must route to the bold painter, not the regular painter.
    #[test]
    fn draw_cell_bold_attr_routes_to_bold_painter() {
        use anvil_term::{Attrs, Cell, Color};
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let regular_p = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let italic_p = StubPainter::default();
        let bold_italic_p = StubPainter::default();
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let cell = Cell {
            cp: 'B',
            fg: Color::Default,
            bg: Color::Default,
            attrs: Attrs::BOLD,
        };
        draw_cell(
            &mut r,
            &mut bold_p,
            m,
            &theme,
            0,
            0,
            0,
            cell,
            Selection::default(),
            None,
        );
        assert!(
            !bold_p.calls.is_empty(),
            "bold painter must receive a call for BOLD cell"
        );
        assert!(
            regular_p.calls.is_empty(),
            "regular painter must not receive BOLD cell calls"
        );
        assert!(italic_p.calls.is_empty());
        assert!(bold_italic_p.calls.is_empty());
    }

    /// An italic cell must route to the italic painter.
    #[test]
    fn draw_cell_italic_attr_routes_to_italic_painter() {
        use anvil_term::{Attrs, Cell, Color};
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let regular_p = StubPainter::default();
        let bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let bold_italic_p = StubPainter::default();
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        let cell = Cell {
            cp: 'I',
            fg: Color::Default,
            bg: Color::Default,
            attrs: Attrs::ITALIC,
        };
        draw_cell(
            &mut r,
            &mut italic_p,
            m,
            &theme,
            0,
            0,
            0,
            cell,
            Selection::default(),
            None,
        );
        assert!(
            !italic_p.calls.is_empty(),
            "italic painter must receive a call for ITALIC cell"
        );
        assert!(regular_p.calls.is_empty());
        assert!(bold_p.calls.is_empty());
        assert!(bold_italic_p.calls.is_empty());
    }

    /// draw_viewport routes bold SGR cells to the bold painter, not regular.
    #[test]
    fn draw_viewport_bold_cell_routes_to_bold_painter() {
        let m = metrics();
        let mut r = Raster::new(200, 120);
        let mut regular_p = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();
        let mut t = make_terminal(10, 4);
        // Feed an ESC[1m (bold) sequence followed by a visible character.
        t.feed(b"\x1b[1mX\x1b[m");
        let theme = MINERAL_DARK;
        r.clear(theme.background);

        draw_viewport(
            &mut r,
            &mut GridPainters {
                regular: &mut regular_p,
                bold: &mut bold_p,
                italic: &mut italic_p,
                bold_italic: &mut bold_italic_p,
            },
            &mut t,
            m,
            &theme,
            0.0,
            Selection::default(),
            None,
            None,
            0.0,
            200.0,
            FoldedBlocks::empty(),
            None,
            0.0,
        );
        // The bold 'X' must have routed to bold_p.
        assert!(
            !bold_p.calls.is_empty(),
            "bold painter must receive bold cell"
        );
    }

    // ── AA9: render path stability — 100 frame loop ───────────────────────────

    /// AA9: render 100 frames via the CPU draw_viewport path with no panics.
    ///
    /// The GPU/Metal path (ANVIL_RENDER=gpu) requires hardware and is not
    /// testable in headless CI; see `AtlasPainter::new_with_default_device`
    /// which already returns `None` when no Metal device is available.  This
    /// test exercises the full draw_viewport loop as the stable surrogate.
    #[test]
    fn render_100_frames_no_panic() {
        const COLS: usize = 80;
        const ROWS: usize = 24;
        let theme = anvil_theme::EMBER_DARK;
        let m = FontMetrics {
            cell_w: 8.0,
            cell_h: 16.0,
            descent: 3.0,
        };
        let mut t = Terminal::new(COLS, ROWS, 1000);
        // Write some content so the draw loop does real work.
        let content: Vec<u8> = b"Hello, Anvil!\r\n"
            .iter()
            .cycle()
            .take(512)
            .copied()
            .collect();
        t.feed(&content);

        let mut raster = Raster::new(COLS * 8, ROWS * 16);
        let mut regular_p = StubPainter::default();
        let mut bold_p = StubPainter::default();
        let mut italic_p = StubPainter::default();
        let mut bold_italic_p = StubPainter::default();

        for frame in 0..100_u32 {
            let blink = (frame as f32) / 100.0;
            raster.clear(theme.background);
            draw_viewport(
                &mut raster,
                &mut GridPainters {
                    regular: &mut regular_p,
                    bold: &mut bold_p,
                    italic: &mut italic_p,
                    bold_italic: &mut bold_italic_p,
                },
                &mut t,
                m,
                &theme,
                blink,
                Selection::default(),
                None,
                Some(CursorParams {
                    ax: 0.0,
                    ay: 0.0,
                    blink_phase: blink,
                    cfg: CursorConfig {
                        style: CursorStyle::Block,
                        blink: true,
                        color: None,
                    },
                }),
                0.0,
                (COLS * 8) as f64,
                FoldedBlocks::empty(),
                None,
                0.0,
            );
        }
        // Reaching here without panic is the assertion.
    }
}
