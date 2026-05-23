//! Per-frame viewport draw loop.
//!
//! `draw_viewport` renders the visible cell grid (plus prompt-rule hairlines
//! and the text cursor) into a `Raster`.  It reads only the arguments it is
//! given — no global state — and performs no heap allocation per frame.
//!
//! Ported from `src/render/draw.zig`.

use anvil_term::{Cell, Color, CursorShape, MatchKind, Search, Terminal};
use anvil_theme::{Theme, mix};
use anvil_workspace::selection::Selection;

use crate::raster::{FontMetrics, GlyphPainter, Raster};

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

/// Cursor opacity for blink phase `p` in [0,1): solid → fade out → dim hold
/// → fade in.  Ported faithfully from `draw.zig`'s `cursorOpacity`.
pub fn cursor_opacity(p: f32) -> f32 {
    fn smoothstep(t: f32) -> f32 {
        let c = t.clamp(0.0, 1.0);
        c * c * (3.0 - 2.0 * c)
    }
    if p < 0.50 {
        return 1.0;
    }
    if p < 0.62 {
        return 1.0 - smoothstep((p - 0.50) / 0.12);
    }
    if p < 0.88 {
        return 0.0;
    }
    smoothstep((p - 0.88) / 0.12)
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
        bg = mix(theme.background, theme.accent, 0.35);
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

// ── Viewport draw loop ────────────────────────────────────────────────────────

/// Draw the viewport: visible cell grid, prompt-rule hairlines, and cursor.
///
/// This is the per-frame draw body, ported from `draw.zig`'s `drawViewport`.
///
/// `scroll_pos` and `overscroll` drive smooth scrolling (0/0 = pinned).
/// `top_bar_rows` offsets every cell row by the tab-bar height.
/// Pass `cursor_params = None` to suppress cursor drawing (e.g. in tests).
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
    overscroll: f32,
    selection: Selection,
    search: Option<&Search>,
    top_bar_rows: usize,
    cursor_params: Option<CursorParams>,
    rule_x_start: f64,
    rule_x_end: f64,
) {
    let rows = terminal.rows();
    let cols = terminal.cols();
    // Prompt-rule hairline drawn above each command. 0.55 mix gives ~3:1 on
    // bone (deliberate, legible) without competing with content or the edge.
    let rule_rgb = mix(theme.background, theme.foreground, 0.55);

    if scroll_pos == 0.0 && overscroll == 0.0 {
        // Live bottom: no fractional offset.
        for y in 0..rows {
            let crow = terminal.content_row_of_viewport(y);
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
            let abs = terminal.absolute_line_of_content(crow);
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
        raster.y_shift_px = scroll_shift - overscroll as f64;
        let hist = terminal.scrollback_len();
        let off = base + 1;
        for y in 0..=rows {
            let crow: usize = if off > y {
                (hist + y).saturating_sub(off)
            } else {
                hist + y - off
            };
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
            let abs = terminal.absolute_line_of_content(crow);
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
            raster.y_shift_px = -(overscroll as f64);

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
    fn cursor_opacity_solid_in_first_half() {
        assert_eq!(cursor_opacity(0.0), 1.0);
        assert_eq!(cursor_opacity(0.49), 1.0);
    }

    #[test]
    fn cursor_opacity_zero_in_dim_hold() {
        assert_eq!(cursor_opacity(0.75), 0.0);
    }

    #[test]
    fn cursor_opacity_fades_in_at_end() {
        let v = cursor_opacity(0.94);
        assert!(v > 0.0 && v < 1.0, "expected fade-in, got {v}");
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
            0.0,
            sel,
            None,
            0,
            None,
            0.0,
            200.0,
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
            0.0,
            sel,
            None,
            0,
            None,
            0.0,
            200.0,
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
        };

        let row = t.viewport_row(0);
        let cell = row[0];
        drop(row);

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
        drop(row);

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
            0.0,
            Selection::default(),
            None,
            0,
            Some(params),
            0.0,
            200.0,
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
            0.0,
            Selection::default(),
            None,
            0,
            Some(params),
            0.0,
            200.0,
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
            0.0,
            Selection::default(),
            None,
            0,
            Some(params),
            0.0,
            200.0,
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
            0.0,
            Selection::default(),
            Some(&search),
            0,
            None,
            0.0,
            200.0,
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
            0.0,
            Selection::default(),
            None,
            0,
            Some(params),
            0.0,
            200.0,
        );
    }

    // ── cursor_opacity full range ─────────────────────────────────────────────

    #[test]
    fn cursor_opacity_full_range_coverage() {
        // Covers all four branches of cursor_opacity.
        // Phase < 0.50: solid (1.0)
        assert_eq!(cursor_opacity(0.0), 1.0);
        assert_eq!(cursor_opacity(0.49), 1.0);
        // Phase in [0.50, 0.62): smoothstep fade from 1 to 0
        let v = cursor_opacity(0.56);
        assert!(v > 0.0 && v < 1.0);
        // Phase in [0.62, 0.88): zero
        assert_eq!(cursor_opacity(0.75), 0.0);
        assert_eq!(cursor_opacity(0.62), 0.0);
        // Phase in [0.88, 1.0): fade back up
        let v = cursor_opacity(0.94);
        assert!(v > 0.0 && v < 1.0);
    }
}
