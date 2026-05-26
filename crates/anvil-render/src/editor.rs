//! Native editor pane render path — NE5, NE10.
//!
//! `draw_editor_into` paints a `Buffer`'s text into a pixel-raster pane area.
//! It mirrors the structure of `draw_viewport` in `draw.rs` but sources cells
//! from `Buffer` rows instead of a `Terminal` grid.
//!
//! Features:
//! - Solid background fill.
//! - Left gutter with right-aligned line numbers in `theme.text_muted`.
//! - Buffer rows rendered grapheme-by-grapheme in `theme.foreground`.
//! - Long-line clip: lines wider than the content area get a `▸` marker at
//!   the right edge in `theme.text_muted`.
//! - Cursor: 2 px-wide vertical bar at `(cursor.pos.line, cursor.pos.col)` in
//!   `theme.accent`.
//! - Selection wash: `fill_pixel_rect_alpha` over selected cells at α=0.18
//!   using `theme.accent_ember`.
//! - Per-grapheme syntax color via `SyntaxLayer::highlights_for_range` (NE8). No soft-wrap (long lines clip).
//! - Scroll is integer-row-aligned: `floor(editor_pane.scroll_pos)`.
//! - Diagnostics gutter stripe (4 px wide, colored by severity) + row tint α=0.06 (NE10).
//! - Hover popup: floating panel rendered via `fill_pixel_rect` + `glyph_at` (NE10).

use unicode_segmentation::UnicodeSegmentation as _;

use anvil_editor::{Buffer, GitChange, GitGutter, SyntaxRole};
use anvil_theme::Theme;
use anvil_workspace::{editor_pane::EditorPane, layout::Rect, selection::Selection};

use crate::raster::{FontMetrics, GlyphPainter, Raster};

// ── Render-side diagnostic type (NE10) ───────────────────────────────────────

/// Severity of a diagnostic, mirroring `anvil_editor::DiagnosticSeverity`.
///
/// Kept in `anvil-render` so the render crate does not depend on
/// `anvil-editor::LspManager`.  `main.rs` translates `DocumentDiagnostic →
/// RenderDiagnostic` when building the `draw_workspace` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Minimal per-line diagnostic data needed by the render path.
#[derive(Debug, Clone)]
pub struct RenderDiagnostic {
    /// Zero-indexed buffer line this diagnostic applies to.
    pub line: usize,
    pub severity: RenderSeverity,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Draw the contents of `editor_pane` / `buffer` into `raster`.
///
/// `rect` is the pane area in device pixels (absolute, not relative to origin).
/// `diagnostics` is a slice of per-line diagnostic data for the current buffer
/// (translate from `LspManager::diagnostics_for` in `main.rs` before calling).
/// Pass an empty slice when LSP is unavailable or the buffer has no path.
///
/// `gutter` is the optional git gutter for the buffer.  Pass `None` for scratch
/// buffers or when git integration is unavailable.
///
/// After this call the raster's `origin_x`/`origin_y` are not changed; callers
/// are expected to set them before calling (matching the `draw_workspace` pattern).
#[allow(clippy::too_many_arguments)]
pub fn draw_editor_into(
    raster: &mut Raster,
    painter: &mut dyn GlyphPainter,
    editor_pane: &EditorPane,
    buffer: &Buffer,
    metrics: FontMetrics,
    theme: &Theme,
    rect: Rect,
    diagnostics: &[RenderDiagnostic],
    gutter: Option<&GitGutter>,
) {
    let cw = metrics.cell_w;
    let ch = metrics.cell_h;

    // ── Geometry ──────────────────────────────────────────────────────────────
    let line_count = buffer.line_count().max(1);
    // Gutter width: digits needed for highest line number + 2 padding cols.
    // When a git gutter is present, expand by 2 more columns (glyph + space).
    let digit_cols = line_count.to_string().len();
    let git_gutter_cols = if gutter.is_some() { 2 } else { 0 };
    let gutter_cols = digit_cols + 2 + git_gutter_cols;
    let gutter_w = gutter_cols as f64 * cw;

    // ── Background: native editor surface, not terminal canvas ────────────────
    raster.fill_pixel_rect(rect.x, rect.y, rect.w, rect.h, theme.surface);
    raster.fill_pixel_rect(rect.x, rect.y, gutter_w, rect.h, theme.charcoal);
    raster.fill_pixel_rect(rect.x + gutter_w - 1.0, rect.y, 1.0, rect.h, theme.hairline);

    // Available content columns to the right of the gutter.
    let content_cols = ((rect.w - gutter_w) / cw).floor() as usize;
    // Number of visible rows that fit in the pane height.
    let visible_rows = (rect.h / ch).ceil() as usize;

    // First visible buffer line (integer snap).
    let scroll_line = editor_pane.scroll_pos.floor() as usize;

    // ── Selection bounds (pre-compute for wash pass) ──────────────────────────
    let sel = &editor_pane.selection;

    // ── Full buffer text — computed once per frame for syntax queries. ────────
    // TODO: replace with streaming / viewport-only allocation when buffers
    // exceed typical sizes.
    let full_text = buffer.to_text();

    if buffer.byte_len() == 0 {
        let hint_x = rect.x + gutter_w + 2.0 * cw;
        let hint_y = rect.y + 2.0 * ch;
        let hints = [
            ("Anvil", theme.text_muted),
            ("Cmd+P open file", theme.text_subtle),
            ("Cmd+E new editor", theme.text_subtle),
        ];
        for (row, (hint, color)) in hints.iter().enumerate() {
            let y = hint_y + row as f64 * ch;
            for (i, ch_g) in hint.chars().enumerate() {
                let gx = hint_x + i as f64 * cw;
                if gx + cw > rect.x + rect.w {
                    break;
                }
                raster.glyph_at(painter, metrics, gx, y, ch_g as u32, *color);
            }
        }
    }

    // ── Row loop ──────────────────────────────────────────────────────────────
    for vrow in 0..visible_rows {
        let line_idx = scroll_line + vrow;
        if line_idx >= line_count {
            break;
        }

        let row_y = rect.y + vrow as f64 * ch;

        // ── Diagnostics: row tint + gutter stripe (NE10) ─────────────────────
        // Find the worst-severity diagnostic on this line (Error > Warning > Info > Hint).
        let row_diag: Option<RenderSeverity> = diagnostics
            .iter()
            .filter(|d| d.line == line_idx)
            .map(|d| d.severity)
            .fold(None, |acc, sev| {
                Some(match acc {
                    None => sev,
                    Some(prev) => worst_severity(prev, sev),
                })
            });

        if let Some(sev) = row_diag {
            let stripe_color = severity_color(sev, theme);
            // Row background tint α=0.06 over the full content area (gutter + text).
            raster.fill_pixel_rect_alpha(rect.x, row_y, rect.w, ch, stripe_color, 0.06);
            // 4 px wide stripe at the left edge of the gutter.
            raster.fill_pixel_rect(rect.x, row_y, 4.0, ch, stripe_color);
        }

        // ── Selection wash for this row ───────────────────────────────────────
        if sel.active {
            paint_selection_row(raster, sel, line_idx, vrow, rect, gutter_w, cw, ch, theme);
        }

        // ── Gutter: right-aligned line number ─────────────────────────────────
        let line_num_str = (line_idx + 1).to_string();
        // Right-align within the digit columns (+1 left-pad space).
        let pad_cols = digit_cols.saturating_sub(line_num_str.len()) + 1;
        for (i, ch_g) in line_num_str.chars().enumerate() {
            let gx = rect.x + (pad_cols + i) as f64 * cw;
            raster.glyph_at(painter, metrics, gx, row_y, ch_g as u32, theme.text_muted);
        }

        // ── Git gutter glyph (NE13) ───────────────────────────────────────────
        if let Some(gg) = gutter {
            // Glyph in column (digit_cols + 1); column (digit_cols + 2) is gap.
            let glyph_col = digit_cols + 1;
            let gx = rect.x + glyph_col as f64 * cw;
            let change = gg
                .per_line
                .get(line_idx)
                .copied()
                .unwrap_or(GitChange::None);
            let (cp, color) = match change {
                GitChange::None => (0u32, theme.text_muted),
                GitChange::Added => ('+' as u32, theme.verified),
                GitChange::Modified => ('~' as u32, theme.attention),
                GitChange::Removed => ('\u{25B4}' as u32, theme.failure), // ▴
            };
            if change != GitChange::None {
                raster.glyph_at(painter, metrics, gx, row_y, cp, color);
            }
        }

        // ── Buffer content ────────────────────────────────────────────────────
        let line_byte_start = buffer.line_to_byte(line_idx);
        let line_slice = buffer.line(line_idx);
        let line_str: String = line_slice.chars().collect();
        // Strip trailing newline before grapheme iteration.
        let line_content = line_str.trim_end_matches('\n').trim_end_matches('\r');
        let line_byte_end = line_byte_start + line_content.len();

        // Per-line syntax highlights (empty when no language is set).
        let highlights =
            buffer
                .syntax()
                .highlights_for_range(line_byte_start, line_byte_end, &full_text);

        let graphemes: Vec<&str> = line_content.graphemes(true).collect();
        let mut painted = 0usize;
        let overflow = graphemes.len() > content_cols;
        let paint_limit = if overflow {
            content_cols.saturating_sub(1)
        } else {
            content_cols
        };

        // Track the byte offset within the line as we walk graphemes.
        let mut grapheme_byte = line_byte_start;
        for (col, g) in graphemes.iter().enumerate() {
            if col >= paint_limit {
                break;
            }
            // Resolve syntax color for the byte position of this grapheme.
            let b = grapheme_byte;
            let role = highlights
                .iter()
                .find(|(r, _)| r.contains(&b))
                .map(|(_, role)| *role)
                .unwrap_or(SyntaxRole::Plain);
            let fg = match role {
                SyntaxRole::Plain => theme.foreground,
                SyntaxRole::Keyword => theme.syntax.keyword,
                SyntaxRole::String => theme.syntax.string,
                SyntaxRole::Number => theme.syntax.number,
                SyntaxRole::Comment => theme.syntax.comment,
                SyntaxRole::Function => theme.syntax.function,
                SyntaxRole::Type => theme.syntax.type_,
                SyntaxRole::Variable => theme.syntax.variable,
                SyntaxRole::Operator => theme.syntax.operator,
                SyntaxRole::Punctuation => theme.syntax.punctuation,
            };

            // Use the first scalar of the grapheme cluster as the glyph key.
            let cp = g.chars().next().unwrap_or(' ') as u32;
            let gx = rect.x + gutter_w + col as f64 * cw;
            raster.glyph_at(painter, metrics, gx, row_y, cp, fg);
            grapheme_byte += g.len();
            painted = col + 1;
        }

        // ── Long-line overflow marker ─────────────────────────────────────────
        if overflow {
            let marker_col = paint_limit;
            let gx = rect.x + gutter_w + marker_col as f64 * cw;
            raster.glyph_at(painter, metrics, gx, row_y, '▸' as u32, theme.text_muted);
        }
        let _ = painted; // suppress dead-code lint
    }

    // ── Cursor bars — primary + secondary (NE13) ─────────────────────────────
    for (i, cursor) in editor_pane.cursors.iter().enumerate() {
        let cursor_line = cursor.pos.line;
        let cursor_col = cursor.pos.col;
        if cursor_line >= scroll_line {
            let vrow = cursor_line - scroll_line;
            if vrow < visible_rows {
                let cx = rect.x + gutter_w + cursor_col as f64 * cw;
                let cy = rect.y + vrow as f64 * ch;
                // Primary cursor: full accent color; secondary: accent_ember.
                let color = if i == 0 {
                    theme.accent
                } else {
                    theme.accent_ember
                };
                // 2 px-wide vertical bar, full cell height.
                raster.fill_pixel_rect(cx, cy, 2.0, ch, color);
            }
        }
    }

    // ── Ghost-text suggestions (NE14) ────────────────────────────────────────
    // Paint the first ghost-text span whose anchor equals the cursor position.
    // Rendered in `theme.text_subtle` (lighter than text_muted) after the cursor.
    // Only paint when the anchor is exactly at the cursor (common completion case).
    let cursor_pos = editor_pane.primary_cursor().pos;
    if let Some(span) = buffer.ghost_text.iter().find(|s| s.anchor == cursor_pos) {
        if cursor_pos.line >= scroll_line {
            let vrow = cursor_pos.line - scroll_line;
            if vrow < visible_rows {
                let row_y = rect.y + vrow as f64 * ch;
                // Start painting ghost text at (cursor_col + 0) — the cursor bar
                // is 2px wide but occupies the same cell column as the ghost text
                // starts. Ghost text begins at the cell right after the cursor column.
                let start_col = cursor_pos.col;
                let chars: Vec<char> = span.text.chars().collect();
                for (i, &c) in chars.iter().enumerate() {
                    let col = start_col + i;
                    if col >= content_cols {
                        break;
                    }
                    let gx = rect.x + gutter_w + col as f64 * cw;
                    raster.glyph_at(painter, metrics, gx, row_y, c as u32, theme.text_subtle);
                }
            }
        }
    }

    // ── Hover popup (NE10) ────────────────────────────────────────────────────
    if let Some(popup) = &editor_pane.hover_popup {
        let anchor = popup.anchor;
        // Compute pixel anchor: position of the cursor cell that triggered hover.
        if anchor.line >= scroll_line {
            let av = anchor.line - scroll_line;
            // Show popup one row below the anchor line.
            let popup_y = rect.y + (av + 1) as f64 * ch;
            let popup_x = rect.x + gutter_w + anchor.col as f64 * cw;

            // Measure popup: wrap text at max 60 chars per line.
            const MAX_COLS: usize = 60;
            let lines: Vec<&str> = popup.text.lines().collect();
            let text_w = lines
                .iter()
                .map(|l| l.len().min(MAX_COLS))
                .max()
                .unwrap_or(0);
            let popup_w = (text_w + 2) as f64 * cw;
            let popup_h = (lines.len() + 1) as f64 * ch;

            // Clamp so popup doesn't overflow the right/bottom edges.
            let popup_x = popup_x.min(rect.x + rect.w - popup_w).max(rect.x);
            let popup_y = popup_y.min(rect.y + rect.h - popup_h).max(rect.y);

            // Background panel (surface color).
            raster.fill_pixel_rect(popup_x, popup_y, popup_w, popup_h, theme.surface);
            // 1 px border (border color).
            // Top edge.
            raster.fill_pixel_rect(popup_x, popup_y, popup_w, 1.0, theme.border);
            // Bottom edge.
            raster.fill_pixel_rect(popup_x, popup_y + popup_h - 1.0, popup_w, 1.0, theme.border);
            // Left edge.
            raster.fill_pixel_rect(popup_x, popup_y, 1.0, popup_h, theme.border);
            // Right edge.
            raster.fill_pixel_rect(popup_x + popup_w - 1.0, popup_y, 1.0, popup_h, theme.border);

            // Text: paint each line of the popup text.
            for (li, line) in lines.iter().enumerate() {
                let ty = popup_y + (li as f64 + 0.5) * ch;
                let chars: Vec<char> = line.chars().take(MAX_COLS).collect();
                for (ci, &c) in chars.iter().enumerate() {
                    let tx = popup_x + (ci + 1) as f64 * cw;
                    raster.glyph_at(painter, metrics, tx, ty, c as u32, theme.foreground);
                }
            }
        }
    }
}

// ── Severity helpers (NE10) ───────────────────────────────────────────────────

fn severity_color(sev: RenderSeverity, theme: &Theme) -> [u8; 3] {
    match sev {
        RenderSeverity::Error => theme.failure,
        RenderSeverity::Warning => theme.attention,
        RenderSeverity::Info => theme.info,
        RenderSeverity::Hint => theme.alloy,
    }
}

/// Return the worse of two severities (Error is worst, Hint is mildest).
fn worst_severity(a: RenderSeverity, b: RenderSeverity) -> RenderSeverity {
    let rank = |s: RenderSeverity| match s {
        RenderSeverity::Error => 3,
        RenderSeverity::Warning => 2,
        RenderSeverity::Info => 1,
        RenderSeverity::Hint => 0,
    };
    if rank(a) >= rank(b) { a } else { b }
}

// ── Selection wash helper ─────────────────────────────────────────────────────

/// Paint the selection wash for a single buffer row.
///
/// Computes the column range that is selected on `line_idx` and fills those
/// cell rects with `theme.accent_ember` at α=0.18.  First and last selected
/// rows use column-precise bounds; middle rows fill the full content width.
#[allow(clippy::too_many_arguments)]
fn paint_selection_row(
    raster: &mut Raster,
    sel: &Selection,
    line_idx: usize,
    vrow: usize,
    rect: Rect,
    gutter_w: f64,
    cw: f64,
    ch: f64,
    theme: &Theme,
) {
    use anvil_workspace::selection::SelectionMode;

    if !sel.active {
        return;
    }
    let (start, end) = sel.ordered();
    if line_idx < start.row || line_idx > end.row {
        return;
    }

    let row_y = rect.y + vrow as f64 * ch;

    let (col_start, col_end) = match sel.mode {
        SelectionMode::Rect => {
            let lo = start.col.min(end.col);
            let hi = start.col.max(end.col);
            (lo, hi)
        }
        SelectionMode::Linear => {
            if start.row == end.row {
                (start.col, end.col)
            } else if line_idx == start.row {
                // First row: from start.col to far right (use a large sentinel).
                (start.col, usize::MAX)
            } else if line_idx == end.row {
                // Last row: from col 0 to end.col.
                (0, end.col)
            } else {
                // Middle rows: full width.
                (0, usize::MAX)
            }
        }
    };

    if col_start >= col_end {
        return;
    }

    let content_w = rect.w - gutter_w;
    let x_start = rect.x + gutter_w + col_start as f64 * cw;
    let x_end = if col_end == usize::MAX {
        rect.x + rect.w
    } else {
        (rect.x + gutter_w + col_end as f64 * cw).min(rect.x + rect.w)
    };

    let wash_w = (x_end - x_start).max(0.0).min(content_w);
    if wash_w > 0.0 {
        raster.fill_pixel_rect_alpha(x_start, row_y, wash_w, ch, theme.accent_ember, 0.18);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_editor::{Buffer, Cursor, Position};
    use anvil_theme::MINERAL_DARK;
    use anvil_workspace::{editor_pane::EditorPane, layout::Rect, selection::Selection};

    use crate::raster::{FontMetrics, GlyphPainter, PixelRect, Raster};

    // ── Stub painter that captures every glyph call ───────────────────────────

    #[derive(Default)]
    struct CapturePainter {
        /// (codepoint, fg_color)
        pub calls: Vec<(u32, [u8; 3])>,
    }

    impl GlyphPainter for CapturePainter {
        #[allow(clippy::too_many_arguments)]
        fn draw_glyph(
            &mut self,
            codepoint: u32,
            _dest: PixelRect,
            fg: [u8; 3],
            _metrics: FontMetrics,
            _pixels: &mut [u8],
            _bw: usize,
            _bh: usize,
        ) {
            self.calls.push((codepoint, fg));
        }
    }

    fn metrics() -> FontMetrics {
        FontMetrics {
            cell_w: 8.0,
            cell_h: 16.0,
            descent: 3.0,
        }
    }

    fn rect() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
        }
    }

    fn make_pane(buffer_id: u64) -> EditorPane {
        let origin = Position { line: 0, col: 0 };
        EditorPane {
            buffer_id,
            cursors: vec![Cursor {
                pos: origin,
                anchor: origin,
            }],
            selection: Selection::default(),
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
            search: None,
            hover_popup: None,
        }
    }

    // ── draw_editor_empty_buffer_paints_only_gutter_line_one ─────────────────

    /// An empty buffer renders the gutter line-number "1" and a muted editor
    /// placeholder in the content area (not terminal foreground text).
    #[test]
    fn draw_editor_empty_buffer_paints_gutter_and_placeholder() {
        let buf = Buffer::new();
        let pane = make_pane(1);
        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
            &[],
            None,
        );

        // The gutter should paint the digit '1' in text_muted.
        let gutter_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.text_muted)
            .collect();
        assert!(
            gutter_calls.iter().any(|(cp, _)| *cp == '1' as u32),
            "gutter must paint '1' for empty buffer, got: {gutter_calls:?}"
        );

        let hint_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.text_subtle)
            .map(|(cp, _)| *cp)
            .collect();
        assert!(
            hint_calls.contains(&('C' as u32)),
            "empty buffer must paint command placeholder hints in subtle text, got: {hint_calls:?}"
        );

        // No foreground calls (empty line has no graphemes and placeholder is subtle).
        let fg_calls: Vec<_> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.foreground)
            .collect();
        assert!(
            fg_calls.is_empty(),
            "empty buffer must produce no foreground glyph calls, got: {fg_calls:?}"
        );
    }

    // ── draw_editor_hello_world_paints_each_grapheme ──────────────────────────

    /// A buffer with "hello" on line 0 must produce a foreground glyph call for
    /// each of the 5 characters.
    #[test]
    fn draw_editor_hello_world_paints_each_grapheme() {
        let buf = Buffer::from_text("hello\n");
        let pane = make_pane(1);
        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
            &[],
            None,
        );

        let fg_cps: Vec<u32> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.foreground)
            .map(|(cp, _)| *cp)
            .collect();

        // Expect exactly h, e, l, l, o.
        let expected: Vec<u32> = "hello".chars().map(|c| c as u32).collect();
        assert_eq!(
            fg_cps, expected,
            "must paint each grapheme of 'hello' in foreground order"
        );
    }

    // ── draw_editor_cursor_at_row_5_col_3_paints_cursor_rect ─────────────────

    /// When the cursor is at (line=5, col=3), the pixel at the expected cursor
    /// x position carries `theme.accent`.
    #[test]
    fn draw_editor_cursor_at_row_5_col_3_paints_cursor_rect() {
        use crate::raster::pixel_at;

        // Build a buffer with 10 lines so line 5 exists.
        let text: String = (0..10).map(|i| format!("line{i}\n")).collect();
        let buf = Buffer::from_text(&text);
        let mut pane = make_pane(1);
        pane.cursors[0] = Cursor {
            pos: Position { line: 5, col: 3 },
            anchor: Position { line: 5, col: 3 },
        };

        let m = metrics();
        let r = rect();
        let line_count = buf.line_count().max(1);
        let digit_cols = line_count.to_string().len();
        let gutter_cols = digit_cols + 2;
        let gutter_w = gutter_cols as f64 * m.cell_w;

        let mut raster = Raster::new(400, 200);
        raster.clear(MINERAL_DARK.background);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            m,
            &theme,
            r,
            &[],
            None,
        );

        // Cursor pixel: x = gutter_w + col * cw, y = row * ch (row 5, col 3).
        let cx = (r.x + gutter_w + 3.0 * m.cell_w) as usize;
        let cy = (r.y + 5.0 * m.cell_h) as usize;
        let px = pixel_at(&raster, cx, cy);
        assert_eq!(
            px, theme.accent,
            "cursor pixel at ({cx},{cy}) should be accent, got {px:?}"
        );
    }

    // ── draw_editor_long_line_paints_overflow_marker ──────────────────────────

    /// A line longer than the available content columns must paint the `▸`
    /// overflow marker in `theme.text_muted`.
    #[test]
    fn draw_editor_long_line_paints_overflow_marker() {
        // content area = 400px, gutter ~3 cols * 8px = 24px, leaving 47 cols.
        // Build a line with 60 characters — well past 47.
        let long_line: String = "a".repeat(60) + "\n";
        let buf = Buffer::from_text(&long_line);
        let pane = make_pane(1);
        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
            &[],
            None,
        );

        let has_overflow_marker = painter
            .calls
            .iter()
            .any(|(cp, fg)| *cp == '▸' as u32 && *fg == theme.text_muted);
        assert!(
            has_overflow_marker,
            "overflow marker '▸' in text_muted must be painted for a 60-char line"
        );
    }

    // ── draw_editor_scroll_skips_top_rows ─────────────────────────────────────

    /// When `scroll_pos = 3.0`, lines 0-2 must not appear and line 3 must be
    /// the first painted row.
    #[test]
    fn draw_editor_scroll_skips_top_rows() {
        let text: String = (0..10).map(|i| format!("L{i}\n")).collect();
        let buf = Buffer::from_text(&text);
        let mut pane = make_pane(1);
        pane.scroll_pos = 3.0;

        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
            &[],
            None,
        );

        // Line 3 starts with 'L' followed by '3'. The foreground calls should
        // contain '4' (gutter line number for logical line 4 = 1-indexed 4).
        // More directly: there must be no foreground call for '0', '1', or '2'
        // — those are the content chars from lines 0-2.
        let fg_cps: Vec<u32> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.foreground)
            .map(|(cp, _)| *cp)
            .collect();

        // Lines 0–2 contain '0', '1', '2' as second char of "L0", "L1", "L2".
        // With scroll=3 those must be absent.
        assert!(
            !fg_cps.contains(&('0' as u32)),
            "scroll should skip line 0; '0' must not appear in fg glyphs"
        );
        assert!(
            !fg_cps.contains(&('1' as u32)),
            "scroll should skip line 1; '1' must not appear in fg glyphs"
        );
        assert!(
            !fg_cps.contains(&('2' as u32)),
            "scroll should skip line 2; '2' must not appear in fg glyphs"
        );
        // Line 3 ("L3") must be present.
        assert!(
            fg_cps.contains(&('L' as u32)),
            "'L' from visible lines must appear in fg glyphs"
        );
        assert!(
            fg_cps.contains(&('3' as u32)),
            "'3' from line 3 must appear in fg glyphs"
        );
    }

    // ── draw_editor_paints_keyword_color_on_fn_keyword ────────────────────────

    /// A Rust buffer containing "fn main() {}" must paint 'f' and 'n' with
    /// `theme.syntax.keyword`, not with `theme.foreground`.
    ///
    /// This verifies that `draw_editor_into` wires the per-grapheme syntax
    /// color lookup introduced in NE8.
    #[test]
    fn draw_editor_paints_keyword_color_on_fn_keyword() {
        let src = "fn main() {}\n";
        let mut buf = Buffer::from_text(src);
        // Set Rust language and parse so highlight data is available.
        buf.syntax
            .set_language_from_path(std::path::Path::new("x.rs"));
        buf.syntax.parse(src);

        let pane = make_pane(1);
        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
            &[],
            None,
        );

        // 'f' and 'n' must be painted with the keyword color.
        let keyword_cps: Vec<u32> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.syntax.keyword)
            .map(|(cp, _)| *cp)
            .collect();

        assert!(
            keyword_cps.contains(&('f' as u32)),
            "'f' must be painted with keyword color; keyword_cps: {keyword_cps:?}"
        );
        assert!(
            keyword_cps.contains(&('n' as u32)),
            "'n' must be painted with keyword color; keyword_cps: {keyword_cps:?}"
        );

        // Neither 'f' nor 'n' should appear in plain foreground calls.
        let fg_cps: Vec<u32> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.foreground)
            .map(|(cp, _)| *cp)
            .collect();
        assert!(
            !fg_cps.contains(&('f' as u32)),
            "'f' must not be painted as plain foreground; fg_cps: {fg_cps:?}"
        );
    }

    // ── draw_editor_diagnostic_gutter_stripe_painted ──────────────────────────

    /// When a `RenderDiagnostic` is supplied for line 0, the gutter stripe must
    /// be painted at `rect.x` (leftmost pixel) in `theme.failure` (Error severity).
    /// The test verifies via the pixel buffer that the stripe color is present.
    #[test]
    fn draw_editor_diagnostic_gutter_stripe_painted() {
        use crate::raster::pixel_at;

        let buf = Buffer::from_text("error here\n");
        let pane = make_pane(1);
        let m = metrics();
        let r = rect(); // 400x200, origin 0,0
        let mut raster = Raster::new(400, 200);
        raster.clear(MINERAL_DARK.background);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        let diag = vec![RenderDiagnostic {
            line: 0,
            severity: RenderSeverity::Error,
        }];

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            m,
            &theme,
            r,
            &diag,
            None,
        );

        // The 4 px gutter stripe is at rect.x=0, row 0.
        // Sample the stripe at x=2 (middle of the 4px stripe), y=middle of row 0.
        let px = pixel_at(&raster, 2, (m.cell_h * 0.5) as usize);
        assert_eq!(
            px, theme.failure,
            "gutter stripe at x=2,y=row0_mid should be failure color, got {px:?}"
        );
    }

    // ── draw_editor_paints_ghost_text_at_cursor ───────────────────────────────

    /// When the buffer has a ghost-text span at the cursor position, the ghost
    /// text glyphs must be painted in `theme.text_subtle` (not foreground).
    #[test]
    fn draw_editor_paints_ghost_text_at_cursor() {
        use anvil_editor::Position as BufPos;

        let mut buf = Buffer::from_text("hi\n");
        // Set ghost text anchored at the cursor (line 0, col 2 — end of "hi").
        buf.set_ghost_text("caldera".into(), BufPos { line: 0, col: 2 }, "xyz".into());

        let mut pane = make_pane(1);
        // Place cursor at line 0, col 2 to match the anchor.
        pane.cursors[0] = anvil_editor::Cursor {
            pos: BufPos { line: 0, col: 2 },
            anchor: BufPos { line: 0, col: 2 },
        };

        let mut raster = Raster::new(400, 200);
        let mut painter = CapturePainter::default();
        let theme = MINERAL_DARK;

        draw_editor_into(
            &mut raster,
            &mut painter,
            &pane,
            &buf,
            metrics(),
            &theme,
            rect(),
            &[],
            None,
        );

        // 'x', 'y', 'z' must appear in text_subtle calls.
        let subtle_cps: Vec<u32> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.text_subtle)
            .map(|(cp, _)| *cp)
            .collect();

        assert!(
            subtle_cps.contains(&('x' as u32)),
            "'x' of ghost text must be in text_subtle; subtle_cps: {subtle_cps:?}"
        );
        assert!(
            subtle_cps.contains(&('y' as u32)),
            "'y' of ghost text must be in text_subtle; subtle_cps: {subtle_cps:?}"
        );
        assert!(
            subtle_cps.contains(&('z' as u32)),
            "'z' of ghost text must be in text_subtle; subtle_cps: {subtle_cps:?}"
        );

        // Ghost text must NOT appear as plain foreground.
        let fg_cps: Vec<u32> = painter
            .calls
            .iter()
            .filter(|(_, fg)| *fg == theme.foreground)
            .map(|(cp, _)| *cp)
            .collect();
        assert!(
            !fg_cps.contains(&('x' as u32)),
            "'x' must not be plain foreground; fg_cps: {fg_cps:?}"
        );
    }
}
