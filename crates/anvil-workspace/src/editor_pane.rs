//! Native editor pane state and registry — NE4, NE6.
//!
//! `EditorPane` is the per-pane view state for a native editor.  `EditorPaneRegistry`
//! holds both the per-pane view state and the underlying `Buffer`s, keyed by
//! `PaneId` and `BufferId` respectively.  It lives alongside `PaneRegistry` on `Tab`.
//!
//! `EditorAction` is the typed action enum used by NE6 (keyboard dispatch).  A future
//! modal layer or vim plugin can sit as a thin keymap on top.

use std::collections::HashMap;

use anvil_editor::{Buffer, BufferId, Cursor, Position, Range};
use unicode_segmentation::UnicodeSegmentation;

use crate::layout::PaneId;
use crate::selection::Selection;

// ── FontMetrics (NE7) ────────────────────────────────────────────────────────

/// Font cell dimensions used by mouse hit-testing.  Mirrors the fields of
/// `anvil_render::FontMetrics`; kept here to avoid a dependency cycle
/// (anvil-render → anvil-workspace → anvil-render).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    /// Width of one monospace cell in device pixels.
    pub cell_w: f64,
    /// Height of one monospace cell in device pixels.
    pub cell_h: f64,
    /// Baseline descent in device pixels (positive = below baseline).
    pub descent: f64,
}

// ── EditorAction ──────────────────────────────────────────────────────────────

/// A typed editor action — the unit of currency between the keymap and the
/// buffer/cursor engine.  NE6 insert-mode only; a future modal layer adds a
/// keymap on top without touching this enum.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorAction {
    InsertChar(char),
    InsertNewline,
    Backspace,
    Delete,
    MoveLeft { extend: bool },
    MoveRight { extend: bool },
    MoveUp { extend: bool },
    MoveDown { extend: bool },
    MoveLineStart { extend: bool },
    MoveLineEnd { extend: bool },
    MoveBufferStart { extend: bool },
    MoveBufferEnd { extend: bool },
    PageUp { extend: bool },
    PageDown { extend: bool },
    Save,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste(String),
    SelectAll,
    GoToLine(usize),
    InsertTab,
    /// Place cursor at the given position; clears selection unless `extend` is true (NE7).
    MoveTo { pos: Position, extend: bool },
    /// Select the word containing `pos` (double-click, NE7).
    /// Word chars: alphanumeric + underscore.
    SelectWordAt(Position),
    /// Select the entire line containing `pos` (triple-click, NE7).
    SelectLineAt(Position),
}

/// Per-pane view state for a native editor pane.
pub struct EditorPane {
    pub buffer_id: BufferId,
    pub cursor: Cursor,
    pub selection: Selection,
    pub scroll_pos: f32,
    pub scroll_target: f32,
    pub scroll_vel: f32,
}

/// Registry of all native editor panes and their buffers for one `Tab`.
///
/// `panes` maps a `PaneId` to its `EditorPane` view state.
/// `buffers` maps a `BufferId` to the underlying `Buffer`.
/// `next_buffer_id` is a monotonic counter for allocating fresh `BufferId`s.
pub struct EditorPaneRegistry {
    panes: HashMap<PaneId, EditorPane>,
    buffers: HashMap<BufferId, Buffer>,
    next_buffer_id: BufferId,
}

impl Default for EditorPaneRegistry {
    fn default() -> Self {
        Self {
            panes: HashMap::new(),
            buffers: HashMap::new(),
            next_buffer_id: 1,
        }
    }
}

impl EditorPaneRegistry {
    /// Allocate a fresh `Buffer`, register an `EditorPane` for `pane_id`,
    /// and return the new `BufferId`.
    pub fn new_pane(&mut self, pane_id: PaneId) -> BufferId {
        let buffer_id = self.next_buffer_id;
        self.next_buffer_id += 1;
        let origin = Position { line: 0, col: 0 };
        let pane = EditorPane {
            buffer_id,
            cursor: Cursor {
                pos: origin,
                anchor: origin,
            },
            selection: Selection::default(),
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
        };
        self.panes.insert(pane_id, pane);
        self.buffers.insert(buffer_id, Buffer::new());
        buffer_id
    }

    /// Look up the `EditorPane` for `pane_id`.
    pub fn get_pane(&self, pane_id: PaneId) -> Option<&EditorPane> {
        self.panes.get(&pane_id)
    }

    /// Look up the `EditorPane` mutably for `pane_id`.
    pub fn get_pane_mut(&mut self, pane_id: PaneId) -> Option<&mut EditorPane> {
        self.panes.get_mut(&pane_id)
    }

    /// Look up the `Buffer` by `buffer_id`.
    pub fn get_buffer(&self, buffer_id: BufferId) -> Option<&Buffer> {
        self.buffers.get(&buffer_id)
    }

    /// Look up the `Buffer` mutably by `buffer_id`.
    pub fn get_buffer_mut(&mut self, buffer_id: BufferId) -> Option<&mut Buffer> {
        self.buffers.get_mut(&buffer_id)
    }

    /// Remove the `EditorPane` for `pane_id` and drop its buffer.
    ///
    /// No-op if `pane_id` is not registered.
    pub fn remove_pane(&mut self, pane_id: PaneId) {
        if let Some(pane) = self.panes.remove(&pane_id) {
            self.buffers.remove(&pane.buffer_id);
        }
    }

    /// Number of registered editor panes.
    pub fn count(&self) -> usize {
        self.panes.len()
    }

    /// Apply `action` to the pane identified by `pane_id`.
    ///
    /// Returns `true` when the buffer was mutated (caller should mark the pane
    /// dirty for re-render).  Returns `false` for pure cursor/selection moves,
    /// no-ops, or when `pane_id` is not registered.
    ///
    /// Copy and Cut return the selected text through `clipboard_out`.  The
    /// caller is responsible for writing it to the system clipboard.
    pub fn apply(
        &mut self,
        pane_id: PaneId,
        action: EditorAction,
        clipboard_out: &mut Option<String>,
    ) -> bool {
        *clipboard_out = None;

        let pane = match self.panes.get_mut(&pane_id) {
            Some(p) => p,
            None => return false,
        };
        let buffer_id = pane.buffer_id;

        match action {
            // ── Insert ───────────────────────────────────────────────────────
            EditorAction::InsertChar(ch) => {
                let pos = pane.cursor.pos;
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_char(pos, ch);
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let new_pos = advance_col(pos, 1);
                set_cursor(pane, new_pos, false);
                true
            }
            EditorAction::InsertNewline => {
                let pos = pane.cursor.pos;
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_char(pos, '\n');
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let new_pos = Position { line: pos.line + 1, col: 0 };
                set_cursor(pane, new_pos, false);
                true
            }
            EditorAction::InsertTab => {
                let pos = pane.cursor.pos;
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_str(pos, "    ");
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let new_pos = advance_col(pos, 4);
                set_cursor(pane, new_pos, false);
                true
            }
            EditorAction::Paste(text) => {
                let pos = pane.cursor.pos;
                let line_delta = text.lines().count().saturating_sub(1);
                let last_line_len = text
                    .lines()
                    .last()
                    .map(|l| l.graphemes(true).count())
                    .unwrap_or(0);
                {
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.insert_str(pos, &text);
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let new_pos = if line_delta == 0 {
                    advance_col(pos, last_line_len)
                } else {
                    Position { line: pos.line + line_delta, col: last_line_len }
                };
                set_cursor(pane, new_pos, false);
                true
            }

            // ── Delete ───────────────────────────────────────────────────────
            EditorAction::Backspace => {
                let pos = pane.cursor.pos;
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                let prev = prev_position(buf, pos);
                if prev == pos {
                    return false; // at buffer start
                }
                buf.delete_range(Range { start: prev, end: pos });
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, prev, false);
                true
            }
            EditorAction::Delete => {
                let pos = pane.cursor.pos;
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                let next = next_position(buf, pos);
                if next == pos {
                    return false; // at buffer end
                }
                buf.delete_range(Range { start: pos, end: next });
                // cursor stays at pos (now points to former next char)
                true
            }

            // ── Cursor movement ──────────────────────────────────────────────
            EditorAction::MoveLeft { extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let new_pos = prev_position(buf, pane.cursor.pos);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveRight { extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let new_pos = next_position(buf, pane.cursor.pos);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveUp { extend } => {
                let pos = pane.cursor.pos;
                let new_pos = if pos.line == 0 {
                    Position { line: 0, col: 0 }
                } else {
                    let buf = self.buffers.get(&buffer_id).unwrap();
                    let target_line = pos.line - 1;
                    let max_col = line_grapheme_len(buf, target_line);
                    Position { line: target_line, col: pos.col.min(max_col) }
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveDown { extend } => {
                let pos = pane.cursor.pos;
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let new_pos = if pos.line >= last_line {
                    let max_col = line_grapheme_len(buf, last_line);
                    Position { line: last_line, col: max_col }
                } else {
                    let target_line = pos.line + 1;
                    let max_col = line_grapheme_len(buf, target_line);
                    Position { line: target_line, col: pos.col.min(max_col) }
                };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveLineStart { extend } => {
                let new_pos = Position { line: pane.cursor.pos.line, col: 0 };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveLineEnd { extend } => {
                let line = pane.cursor.pos.line;
                let buf = self.buffers.get(&buffer_id).unwrap();
                let col = line_grapheme_len(buf, line);
                let new_pos = Position { line, col };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveBufferStart { extend } => {
                let new_pos = Position { line: 0, col: 0 };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::MoveBufferEnd { extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let col = line_grapheme_len(buf, last_line);
                let new_pos = Position { line: last_line, col };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::PageUp { extend } => {
                let pos = pane.cursor.pos;
                let page = pane.scroll_pos as usize;
                let new_line = pos.line.saturating_sub(page.max(1));
                let buf = self.buffers.get(&buffer_id).unwrap();
                let col = pos.col.min(line_grapheme_len(buf, new_line));
                let new_pos = Position { line: new_line, col };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }
            EditorAction::PageDown { extend } => {
                let pos = pane.cursor.pos;
                let page = pane.scroll_pos as usize;
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let new_line = (pos.line + page.max(1)).min(last_line);
                let col = pos.col.min(line_grapheme_len(buf, new_line));
                let new_pos = Position { line: new_line, col };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, extend);
                false
            }

            // ── Select all ──────────────────────────────────────────────────
            EditorAction::SelectAll => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let last_col = line_grapheme_len(buf, last_line);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursor.anchor = Position { line: 0, col: 0 };
                pane.cursor.pos = Position { line: last_line, col: last_col };
                false
            }

            // ── GoToLine ────────────────────────────────────────────────────
            EditorAction::GoToLine(target) => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let line = target.min(last_line);
                let new_pos = Position { line, col: 0 };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                buf.flush_undo_group();
                false
            }

            // ── Save ────────────────────────────────────────────────────────
            EditorAction::Save => {
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                if let Some(path) = buf.tracked_path().map(|p| p.to_path_buf()) {
                    if let Err(e) = buf.save(&path) {
                        eprintln!("anvil: editor save failed: {e}");
                    }
                } else {
                    eprintln!("anvil: editor save: no tracked path (file-open is NE7+ scope)");
                }
                false
            }

            // ── Undo / Redo ─────────────────────────────────────────────────
            EditorAction::Undo => {
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                buf.undo();
                // Clamp cursor to valid position after undo.
                let last_line = buf.line_count().saturating_sub(1);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let line = pane.cursor.pos.line.min(last_line);
                let buf = self.buffers.get(&buffer_id).unwrap();
                let col = pane.cursor.pos.col.min(line_grapheme_len(buf, line));
                let new_pos = Position { line, col };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                true
            }
            EditorAction::Redo => {
                let buf = self.buffers.get_mut(&buffer_id).unwrap();
                buf.redo();
                let last_line = buf.line_count().saturating_sub(1);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                let line = pane.cursor.pos.line.min(last_line);
                let buf = self.buffers.get(&buffer_id).unwrap();
                let col = pane.cursor.pos.col.min(line_grapheme_len(buf, line));
                let new_pos = Position { line, col };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, new_pos, false);
                true
            }

            // ── Mouse actions (NE7) ─────────────────────────────────────────
            EditorAction::MoveTo { pos, extend } => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let clamped_line = pos.line.min(last_line);
                let max_col = line_grapheme_len(buf, clamped_line);
                let clamped = Position { line: clamped_line, col: pos.col.min(max_col) };
                let pane = self.panes.get_mut(&pane_id).unwrap();
                set_cursor(pane, clamped, extend);
                false
            }
            EditorAction::SelectWordAt(pos) => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let line = pos.line.min(last_line);
                let line_str: String = buf.line(line).chars().collect();
                let graphemes: Vec<&str> = line_str
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .graphemes(true)
                    .collect();
                let col = pos.col.min(graphemes.len().saturating_sub(1));
                let is_word = |g: &str| {
                    g.chars().all(|c| c.is_alphanumeric() || c == '_')
                };
                // Walk left to find word start.
                let mut lo = col;
                while lo > 0 && is_word(graphemes[lo - 1]) {
                    lo -= 1;
                }
                // Walk right to find word end.
                let mut hi = col;
                if hi < graphemes.len() && is_word(graphemes[hi]) {
                    hi += 1;
                    while hi < graphemes.len() && is_word(graphemes[hi]) {
                        hi += 1;
                    }
                }
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursor.anchor = Position { line, col: lo };
                pane.cursor.pos = Position { line, col: hi };
                false
            }
            EditorAction::SelectLineAt(pos) => {
                let buf = self.buffers.get(&buffer_id).unwrap();
                let last_line = buf.line_count().saturating_sub(1);
                let line = pos.line.min(last_line);
                let line_len = line_grapheme_len(buf, line);
                let pane = self.panes.get_mut(&pane_id).unwrap();
                pane.cursor.anchor = Position { line, col: 0 };
                pane.cursor.pos = Position { line, col: line_len };
                false
            }

            // ── Copy / Cut ──────────────────────────────────────────────────
            EditorAction::Copy => {
                if let Some(text) = selected_text(pane, self.buffers.get(&buffer_id).unwrap()) {
                    *clipboard_out = Some(text);
                }
                false
            }
            EditorAction::Cut => {
                if let Some(text) = selected_text(pane, self.buffers.get(&buffer_id).unwrap()) {
                    let (start, end) = selection_range(pane);
                    *clipboard_out = Some(text);
                    let buf = self.buffers.get_mut(&buffer_id).unwrap();
                    buf.delete_range(Range { start, end });
                    let pane = self.panes.get_mut(&pane_id).unwrap();
                    set_cursor(pane, start, false);
                    return true;
                }
                false
            }
        }
    }
}

// ── Public helpers (NE7) ─────────────────────────────────────────────────────

/// Convert a click pixel (relative to pane top-left) to a buffer `Position`.
///
/// Accounts for gutter width, scroll offset, and grapheme column walk.
///
/// - `rel_x`, `rel_y`: pointer position in device pixels relative to the
///   top-left corner of the pane's draw area.
/// - `metrics`: font cell dimensions in device pixels.
/// - `gutter_cols`: width of the line-number gutter in character columns.
pub fn pixel_to_position(
    editor_pane: &EditorPane,
    buffer: &Buffer,
    rel_x: f64,
    rel_y: f64,
    metrics: FontMetrics,
    gutter_cols: usize,
) -> Position {
    // Row: floor(rel_y / cell_h) + scroll_pos, clamped to buffer bounds.
    let row_raw = (rel_y / metrics.cell_h).floor() as usize;
    let line_count = buffer.line_count();
    let last_line = line_count.saturating_sub(1);
    let row = (row_raw + editor_pane.scroll_pos as usize).min(last_line);

    // Column: subtract gutter pixels, then walk graphemes.
    let col_pixel = rel_x - gutter_cols as f64 * metrics.cell_w;
    if col_pixel < 0.0 {
        return Position { line: row, col: 0 };
    }
    let cell_col = (col_pixel / metrics.cell_w).round() as usize;

    // Walk line graphemes and clamp to line length.
    let line_len = if line_count == 0 {
        0
    } else {
        let line_str: String = buffer.line(row).chars().collect();
        line_str
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .graphemes(true)
            .count()
    };
    Position { line: row, col: cell_col.min(line_len) }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Set the cursor position. When `extend` is false, the anchor snaps to `pos`.
fn set_cursor(pane: &mut EditorPane, pos: Position, extend: bool) {
    pane.cursor.pos = pos;
    if !extend {
        pane.cursor.anchor = pos;
    }
}

/// Advance `pos` by `n` grapheme columns on the same line (no line wrapping).
fn advance_col(pos: Position, n: usize) -> Position {
    Position { line: pos.line, col: pos.col + n }
}

/// Number of grapheme clusters on `line` (excluding any trailing newline).
fn line_grapheme_len(buf: &Buffer, line: usize) -> usize {
    if line >= buf.line_count() {
        return 0;
    }
    let line_str: String = buf.line(line).chars().collect();
    line_str
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .graphemes(true)
        .count()
}

/// Move one grapheme cluster backward from `pos`.  Returns `pos` unchanged at
/// the buffer start.
fn prev_position(buf: &Buffer, pos: Position) -> Position {
    if pos.col > 0 {
        return Position { line: pos.line, col: pos.col - 1 };
    }
    if pos.line == 0 {
        return pos;
    }
    // Move to end of previous line.
    let prev_line = pos.line - 1;
    let col = line_grapheme_len(buf, prev_line);
    Position { line: prev_line, col }
}

/// Move one grapheme cluster forward from `pos`.  Returns `pos` unchanged at
/// the buffer end.
fn next_position(buf: &Buffer, pos: Position) -> Position {
    let line_len = line_grapheme_len(buf, pos.line);
    if pos.col < line_len {
        return Position { line: pos.line, col: pos.col + 1 };
    }
    // Move to start of next line if there is one.
    let last_line = buf.line_count().saturating_sub(1);
    if pos.line < last_line {
        Position { line: pos.line + 1, col: 0 }
    } else {
        pos
    }
}

/// Extract the text covered by the cursor's selection anchor→pos range.
/// Returns `None` when anchor == pos (no selection).
fn selected_text(pane: &EditorPane, buf: &Buffer) -> Option<String> {
    let (start, end) = selection_range(pane);
    if start == end {
        return None;
    }
    // Walk every line from start to end, collecting grapheme clusters.
    let mut out = String::new();
    for line_idx in start.line..=end.line {
        if line_idx >= buf.line_count() {
            break;
        }
        let line_str: String = buf.line(line_idx).chars().collect();
        let graphemes: Vec<&str> = line_str
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .graphemes(true)
            .collect();
        let lo = if line_idx == start.line { start.col } else { 0 };
        let hi = if line_idx == end.line {
            end.col.min(graphemes.len())
        } else {
            graphemes.len()
        };
        for g in &graphemes[lo.min(graphemes.len())..hi.min(graphemes.len())] {
            out.push_str(g);
        }
        if line_idx < end.line {
            out.push('\n');
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Return the ordered `(start, end)` pair of the cursor's anchor→pos range.
fn selection_range(pane: &EditorPane) -> (Position, Position) {
    let a = pane.cursor.anchor;
    let p = pane.cursor.pos;
    // Compare by (line, col).
    if (a.line, a.col) <= (p.line, p.col) {
        (a, p)
    } else {
        (p, a)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_reg_with_text(text: &str) -> (EditorPaneRegistry, PaneId) {
        let mut reg = EditorPaneRegistry::default();
        let pane_id: PaneId = 1;
        reg.new_pane(pane_id);
        // Load text into the buffer.
        let bid = reg.get_pane(pane_id).unwrap().buffer_id;
        let buf = reg.get_buffer_mut(bid).unwrap();
        *buf = anvil_editor::Buffer::from_text(text);
        (reg, pane_id)
    }

    fn buf_text(reg: &EditorPaneRegistry, pane_id: PaneId) -> String {
        let bid = reg.get_pane(pane_id).unwrap().buffer_id;
        reg.get_buffer(bid).unwrap().to_text()
    }

    fn test_metrics() -> FontMetrics {
        FontMetrics { cell_w: 8.0, cell_h: 16.0, descent: 3.0 }
    }

    fn make_pane_with_text(text: &str) -> (EditorPane, Buffer) {
        let origin = Position { line: 0, col: 0 };
        let pane = EditorPane {
            buffer_id: 1,
            cursor: Cursor { pos: origin, anchor: origin },
            selection: Selection::default(),
            scroll_pos: 0.0,
            scroll_target: 0.0,
            scroll_vel: 0.0,
        };
        let buf = anvil_editor::Buffer::from_text(text);
        (pane, buf)
    }

    // ── NE7 pixel_to_position tests ──────────────────────────────────────────

    #[test]
    fn pixel_to_position_origin_returns_0_0() {
        let (pane, buf) = make_pane_with_text("hello\nworld\n");
        let pos = pixel_to_position(&pane, &buf, 0.0, 0.0, test_metrics(), 0);
        assert_eq!(pos, Position { line: 0, col: 0 });
    }

    #[test]
    fn pixel_to_position_row_3_col_5_with_gutter() {
        // row = floor(3*16 / 16) + 0 = 3
        // rel_x = gutter(2)*8 + col(5)*8 = 16 + 40 = 56
        // col_pixel = 56 - 16 = 40; 40/8 = 5.0 → 5
        // Line 3 is "barnacle" (8 chars) so col 5 is within bounds.
        let (pane, buf) = make_pane_with_text("hello\nworld\nfoobar\nbarnacle\n");
        let rel_x = (2 + 5) as f64 * 8.0; // gutter 2 cols + col 5
        let rel_y = 3.0 * 16.0;           // row 3
        let pos = pixel_to_position(&pane, &buf, rel_x, rel_y, test_metrics(), 2);
        assert_eq!(pos, Position { line: 3, col: 5 });
    }

    #[test]
    fn pixel_to_position_clamps_overflow() {
        // Buffer has 2 lines of "hi". Click far past end.
        let (pane, buf) = make_pane_with_text("hi\nhi\n");
        let pos = pixel_to_position(&pane, &buf, 9999.0, 9999.0, test_metrics(), 0);
        let last_line = buf.line_count().saturating_sub(1);
        assert_eq!(pos.line, last_line);
        // col clamped to "hi" length = 2
        assert!(pos.col <= 2);
    }

    // ── NE7 action tests ─────────────────────────────────────────────────────

    #[test]
    fn apply_move_to_clears_selection() {
        let (mut reg, pid) = make_reg_with_text("hello world");
        // Put cursor at col 5 with anchor at 0 (simulating a selection).
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursor.anchor = Position { line: 0, col: 0 };
            pane.cursor.pos = Position { line: 0, col: 5 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::MoveTo { pos: Position { line: 0, col: 3 }, extend: false }, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        // Anchor should equal pos (selection collapsed).
        assert_eq!(pane.cursor.pos, Position { line: 0, col: 3 });
        assert_eq!(pane.cursor.anchor, Position { line: 0, col: 3 });
    }

    #[test]
    fn apply_move_to_with_extend_preserves_anchor() {
        let (mut reg, pid) = make_reg_with_text("hello world");
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursor.anchor = Position { line: 0, col: 2 };
            pane.cursor.pos = Position { line: 0, col: 2 };
        }
        let mut clip = None;
        reg.apply(pid, EditorAction::MoveTo { pos: Position { line: 0, col: 7 }, extend: true }, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        // Anchor stays at 2; pos moves to 7.
        assert_eq!(pane.cursor.anchor, Position { line: 0, col: 2 });
        assert_eq!(pane.cursor.pos, Position { line: 0, col: 7 });
    }

    #[test]
    fn apply_select_word_at_picks_word_span() {
        // "hello world" — click on 'o' (col 4) should select "hello" (0..5).
        let (mut reg, pid) = make_reg_with_text("hello world");
        let mut clip = None;
        reg.apply(pid, EditorAction::SelectWordAt(Position { line: 0, col: 4 }), &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursor.anchor, Position { line: 0, col: 0 });
        assert_eq!(pane.cursor.pos, Position { line: 0, col: 5 });
    }

    #[test]
    fn apply_select_line_at_picks_full_line() {
        // "foo\nbar\n" — select line 1 ("bar") → col 0..3.
        let (mut reg, pid) = make_reg_with_text("foo\nbar\n");
        let mut clip = None;
        reg.apply(pid, EditorAction::SelectLineAt(Position { line: 1, col: 0 }), &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursor.anchor, Position { line: 1, col: 0 });
        assert_eq!(pane.cursor.pos, Position { line: 1, col: 3 });
    }

    // ── NE6 apply tests ──────────────────────────────────────────────────────

    #[test]
    fn apply_insert_char_appends_to_buffer() {
        let (mut reg, pid) = make_reg_with_text("");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::InsertChar('x'), &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "x");
        // Cursor should advance one col.
        assert_eq!(reg.get_pane(pid).unwrap().cursor.pos.col, 1);
    }

    #[test]
    fn apply_backspace_removes_prior_char() {
        let (mut reg, pid) = make_reg_with_text("ab");
        // Move cursor to col 2 (end of "ab").
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursor.pos = Position { line: 0, col: 2 };
            pane.cursor.anchor = Position { line: 0, col: 2 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::Backspace, &mut clip);
        assert!(mutated);
        // "ab" → "a" (col 2 − 1 = col 1 now has nothing).
        let text = buf_text(&reg, pid);
        assert_eq!(text.trim_end_matches('\n'), "a");
        assert_eq!(reg.get_pane(pid).unwrap().cursor.pos.col, 1);
    }

    #[test]
    fn apply_move_right_advances_cursor() {
        let (mut reg, pid) = make_reg_with_text("hello");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::MoveRight { extend: false }, &mut clip);
        assert!(!mutated);
        assert_eq!(reg.get_pane(pid).unwrap().cursor.pos.col, 1);
        // Anchor snaps to pos.
        assert_eq!(reg.get_pane(pid).unwrap().cursor.anchor.col, 1);
    }

    #[test]
    fn apply_move_right_with_extend_grows_selection() {
        let (mut reg, pid) = make_reg_with_text("hello");
        let mut clip = None;
        reg.apply(pid, EditorAction::MoveRight { extend: true }, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        // pos advanced, anchor stayed.
        assert_eq!(pane.cursor.pos.col, 1);
        assert_eq!(pane.cursor.anchor.col, 0);
    }

    #[test]
    fn apply_select_all_anchors_at_origin() {
        let (mut reg, pid) = make_reg_with_text("line1\nline2\n");
        let mut clip = None;
        reg.apply(pid, EditorAction::SelectAll, &mut clip);
        let pane = reg.get_pane(pid).unwrap();
        assert_eq!(pane.cursor.anchor, Position { line: 0, col: 0 });
        // pos should be at last line.
        assert!(pane.cursor.pos.line > 0);
    }

    #[test]
    fn apply_paste_inserts_string() {
        let (mut reg, pid) = make_reg_with_text("ab");
        // Cursor at col 1.
        {
            let pane = reg.get_pane_mut(pid).unwrap();
            pane.cursor.pos = Position { line: 0, col: 1 };
            pane.cursor.anchor = Position { line: 0, col: 1 };
        }
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::Paste("XY".to_string()), &mut clip);
        assert!(mutated);
        let text = buf_text(&reg, pid);
        assert!(text.starts_with("aXYb"), "expected aXYb, got {text}");
        assert_eq!(reg.get_pane(pid).unwrap().cursor.pos.col, 3);
    }

    #[test]
    fn apply_undo_reverts_last_edit() {
        let (mut reg, pid) = make_reg_with_text("");
        let mut clip = None;
        reg.apply(pid, EditorAction::InsertChar('z'), &mut clip);
        assert_eq!(buf_text(&reg, pid), "z");
        let mutated = reg.apply(pid, EditorAction::Undo, &mut clip);
        assert!(mutated);
        assert_eq!(buf_text(&reg, pid), "");
    }

    #[test]
    fn apply_save_with_no_path_is_noop() {
        // A fresh buffer has no tracked path — Save should be a silent no-op
        // (logs to stderr but doesn't panic).
        let (mut reg, pid) = make_reg_with_text("content");
        let mut clip = None;
        let mutated = reg.apply(pid, EditorAction::Save, &mut clip);
        // Buffer not mutated; text unchanged.
        assert!(!mutated);
        assert_eq!(buf_text(&reg, pid), "content");
    }

    // ── Existing registry tests ──────────────────────────────────────────────

    #[test]
    fn editor_pane_registry_new_pane_returns_buffer_id() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(1);
        // Buffer id must be a non-zero sentinel (our counter starts at 1).
        assert!(bid > 0);
        // The pane must be findable.
        assert!(reg.get_pane(1).is_some());
    }

    #[test]
    fn editor_pane_registry_get_buffer_round_trip() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(42);
        // Buffer must be present and empty.
        let buf = reg
            .get_buffer(bid)
            .expect("buffer should exist after new_pane");
        assert_eq!(buf.char_count(), 0);
    }

    #[test]
    fn editor_pane_registry_remove_pane_drops_buffer() {
        let mut reg = EditorPaneRegistry::default();
        let bid = reg.new_pane(7);
        assert_eq!(reg.count(), 1);
        reg.remove_pane(7);
        assert_eq!(reg.count(), 0);
        // Buffer is gone.
        assert!(reg.get_buffer(bid).is_none());
    }

    #[test]
    fn editor_pane_registry_multiple_panes_independent() {
        let mut reg = EditorPaneRegistry::default();
        let bid1 = reg.new_pane(1);
        let bid2 = reg.new_pane(2);
        assert_ne!(bid1, bid2, "each pane must get a unique buffer id");
        assert_eq!(reg.count(), 2);
        // Remove one; other stays.
        reg.remove_pane(1);
        assert_eq!(reg.count(), 1);
        assert!(reg.get_pane(2).is_some());
        assert!(reg.get_buffer(bid2).is_some());
    }
}
