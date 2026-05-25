//! Native rope-backed text buffer — NE1 skeleton, NE2 file IO, NE3 undo/redo.
//!
//! `Buffer` is the primary editing model. All positions are grapheme-column
//! based (not byte offsets). UTF-8 only internally; UTF-16 BOM files are
//! decoded on read and saved back as UTF-8.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use unicode_segmentation::UnicodeSegmentation;

use crate::syntax::SyntaxLayer;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Opaque buffer identifier.
pub type BufferId = u64;

/// A grapheme-aware position in the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// 0-indexed line number.
    pub line: usize,
    /// 0-indexed grapheme column.
    pub col: usize,
}

/// A half-open range `[start, end)` in the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// A cursor with an optional selection anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Current cursor position.
    pub pos: Position,
    /// Anchor of the selection. When `anchor == pos` there is no selection.
    pub anchor: Position,
}

/// A single edit: replace `range` with `replacement`.
#[derive(Debug, Clone)]
pub struct Edit {
    pub range: Range,
    pub replacement: String,
}

// ---------------------------------------------------------------------------
// AI-native placeholder structs (reserved for NE14)
// ---------------------------------------------------------------------------

/// An AI-proposed edit — empty placeholder, reserved for NE14.
#[derive(Debug, Clone)]
pub struct EditProposal {}

/// A ghost-text span — empty placeholder, reserved for NE14.
#[derive(Debug, Clone)]
pub struct GhostTextSpan {}

// ---------------------------------------------------------------------------
// IO error types (NE2)
// ---------------------------------------------------------------------------

/// Error returned by [`Buffer::from_path`] and [`Buffer::save`].
#[derive(Debug)]
pub enum IoError {
    /// File exceeds the maximum allowed size (50 MB by default).
    TooLarge,
    /// Encoding could not be decoded.
    Encoding(EncodingError),
    /// Underlying OS I/O failure.
    Io(std::io::Error),
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoError::TooLarge => write!(f, "file too large (exceeds the 50 MB limit)"),
            IoError::Encoding(e) => write!(f, "encoding error: {e}"),
            IoError::Io(e) => write!(f, "IO error: {e}"),
        }
    }
}

impl std::error::Error for IoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IoError::Io(e) => Some(e),
            IoError::Encoding(e) => Some(e),
            IoError::TooLarge => None,
        }
    }
}

impl From<std::io::Error> for IoError {
    fn from(e: std::io::Error) -> Self {
        IoError::Io(e)
    }
}

/// Encoding-level decode failure.
#[derive(Debug)]
pub enum EncodingError {
    /// Bytes were not valid UTF-8 and no BOM indicated another encoding.
    InvalidUtf8,
}

impl std::fmt::Display for EncodingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodingError::InvalidUtf8 => write!(f, "invalid UTF-8 sequence"),
        }
    }
}

impl std::error::Error for EncodingError {}

// ---------------------------------------------------------------------------
// Undo / redo types (NE3)
// ---------------------------------------------------------------------------

/// One recorded edit plus its pre-computed inverse and a wall-clock timestamp.
///
/// Crate-private: lives only inside `Buffer::undo_stack`. Not part of the
/// public surface — the AI proposal layer (NE14) will own the public revision
/// shape.
pub(crate) struct EditRecord {
    /// The forward edit (already applied to the rope).
    pub edit: Edit,
    /// The inverse edit (restores the rope to the state before `edit`).
    pub inverse: Edit,
    /// When this edit was applied.
    pub at: Instant,
}

/// Groups of `EditRecord`s forming an undo/redo history.
///
/// Each `Vec<EditRecord>` is one undo group (e.g. a burst of typed characters).
pub(crate) struct UndoStack {
    undo: VecDeque<Vec<EditRecord>>,
    redo: VecDeque<Vec<EditRecord>>,
    /// Maximum number of undo groups retained. Oldest groups are evicted first.
    cap: usize,
}

impl UndoStack {
    fn new(cap: usize) -> Self {
        UndoStack {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            cap,
        }
    }
}

// ---------------------------------------------------------------------------
// Buffer
// ---------------------------------------------------------------------------

const DEFAULT_UNDO_CAP: usize = 1000;
/// Maximum gap between two single-char inserts for them to coalesce (ms).
const COALESCE_MS: u128 = 500;

/// A rope-backed UTF-8 text buffer.
pub struct Buffer {
    rope: ropey::Rope,
    /// Monotonic counter bumped on every applied edit. Used by NE14 proposals.
    pub revisions: u64,
    /// AI-native proposal slots — allocated empty in NE1 so NE14 doesn't break shapes.
    pub proposals: Vec<EditProposal>,
    pub ghost_text: Vec<GhostTextSpan>,
    /// Undo/redo history.
    pub(crate) undo_stack: UndoStack,
    /// When `true`, the next `apply_edit` always starts a new undo group.
    force_new_group: bool,
    /// Path this buffer was loaded from or last saved to (NE2).
    tracked_path: Option<PathBuf>,
    /// mtime recorded at last open or save (NE2).
    tracked_mtime: Option<SystemTime>,
    /// Tree-sitter syntax layer (NE8). Holds the parse tree and highlight cache.
    pub syntax: SyntaxLayer,
}

impl Buffer {
    /// Create an empty buffer.
    pub fn new() -> Buffer {
        Buffer {
            rope: ropey::Rope::new(),
            revisions: 0,
            proposals: Vec::new(),
            ghost_text: Vec::new(),
            undo_stack: UndoStack::new(DEFAULT_UNDO_CAP),
            force_new_group: false,
            tracked_path: None,
            tracked_mtime: None,
            syntax: SyntaxLayer::new(),
        }
    }

    /// Create a buffer pre-loaded with `text`.
    pub fn from_text(text: &str) -> Buffer {
        Buffer {
            rope: ropey::Rope::from_str(text),
            revisions: 0,
            proposals: Vec::new(),
            ghost_text: Vec::new(),
            undo_stack: UndoStack::new(DEFAULT_UNDO_CAP),
            force_new_group: false,
            tracked_path: None,
            tracked_mtime: None,
            syntax: SyntaxLayer::new(),
        }
    }

    // -----------------------------------------------------------------------
    // File IO (NE2)
    // -----------------------------------------------------------------------

    /// Load a buffer from `path`.
    ///
    /// Refuses files larger than 50 MB. Detects encoding from BOM:
    /// - UTF-8 BOM (`EF BB BF`): strip BOM, parse as UTF-8.
    /// - UTF-16 LE BOM (`FF FE`): decode as UTF-16 LE.
    /// - UTF-16 BE BOM (`FE FF`): decode as UTF-16 BE.
    /// - No BOM: attempt UTF-8; invalid bytes → `IoError::Encoding(EncodingError::InvalidUtf8)`.
    ///
    /// On success, records the file path and its mtime for change detection.
    pub fn from_path(path: &Path) -> Result<Buffer, IoError> {
        Buffer::from_path_with_limit(path, 50 * 1024 * 1024)
    }

    /// Like [`from_path`] but with a custom byte limit — used by tests to
    /// exercise the size cap without 50 MB temp files.
    pub(crate) fn from_path_with_limit(path: &Path, max_bytes: u64) -> Result<Buffer, IoError> {
        let meta = std::fs::metadata(path)?;
        if meta.len() > max_bytes {
            return Err(IoError::TooLarge);
        }
        let bytes = std::fs::read(path)?;
        let mtime = meta.modified().ok();
        let text = decode_bytes(&bytes)?;
        let mut buf = Buffer::from_text(&text);
        buf.tracked_path = Some(path.to_path_buf());
        buf.tracked_mtime = mtime;
        // NE8: detect language by extension and do an initial full parse.
        buf.syntax.set_language_from_path(path);
        buf.syntax.parse(&text);
        Ok(buf)
    }

    /// Serialize the buffer content to a UTF-8 `String`.
    pub fn to_text(&self) -> String {
        self.rope.to_string()
    }

    /// Save the buffer to `path` atomically (write `<path>.tmp`, then rename).
    ///
    /// On success, records the path and new on-disk mtime.
    pub fn save(&mut self, path: &Path) -> Result<(), IoError> {
        // Build sibling tmp path by appending `.tmp` to the file name.
        // `path.with_extension("tmp")` would mangle multi-suffix names like
        // `archive.tar.gz` into `archive.tar.tmp`.
        let file_name = path.file_name().ok_or_else(|| {
            IoError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "save path has no file name",
            ))
        })?;
        let mut tmp_name = file_name.to_os_string();
        tmp_name.push(".tmp");
        let tmp = path.with_file_name(tmp_name);
        let text = self.to_text();
        std::fs::write(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, path)?;
        let mtime = std::fs::metadata(path)?.modified().ok();
        self.tracked_path = Some(path.to_path_buf());
        self.tracked_mtime = mtime;
        self.flush_undo_group();
        Ok(())
    }

    /// Return the path this buffer was last loaded from or saved to.
    ///
    /// `None` when no path has been set (e.g. a fresh scratch buffer).
    pub fn tracked_path(&self) -> Option<&std::path::Path> {
        self.tracked_path.as_deref()
    }

    /// Return the LSP language-id for this buffer, derived from the file
    /// extension of `tracked_path`.  `None` for scratch buffers or unknown
    /// extensions.
    pub fn language_id(&self) -> Option<&'static str> {
        let ext = self
            .tracked_path
            .as_ref()?
            .extension()
            .and_then(|e| e.to_str())?;
        crate::lsp::language_id_for_ext(ext)
    }

    /// Returns `true` if the file on disk has been modified since the buffer
    /// was last opened or saved.
    ///
    /// Returns `false` when no path has ever been tracked or the mtime cannot
    /// be read.
    pub fn is_externally_modified(&self) -> bool {
        let (Some(path), Some(tracked)) = (&self.tracked_path, &self.tracked_mtime) else {
            return false;
        };
        let Ok(meta) = std::fs::metadata(path) else {
            return false;
        };
        let Ok(disk_mtime) = meta.modified() else {
            return false;
        };
        disk_mtime != *tracked
    }

    // -----------------------------------------------------------------------
    // Read accessors
    // -----------------------------------------------------------------------

    /// Return line `n` as a `RopeSlice`. Panics if `n >= line_count()`.
    pub fn line(&self, n: usize) -> ropey::RopeSlice<'_> {
        self.rope.line(n)
    }

    /// Number of lines in the buffer (always >= 1 for a non-empty rope).
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Total byte length of the buffer content.
    pub fn byte_len(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Total char (Unicode scalar value) count.
    pub fn char_count(&self) -> usize {
        self.rope.len_chars()
    }

    /// Return the char at `char_idx`, or `None` if out of range.
    pub fn char_at(&self, char_idx: usize) -> Option<char> {
        if char_idx < self.rope.len_chars() {
            Some(self.rope.char(char_idx))
        } else {
            None
        }
    }

    /// Convert a char index to its 0-indexed line number.
    pub fn char_to_line(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx)
    }

    /// Convert a 0-indexed line number to the char index of its first char.
    pub fn line_to_char(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    /// Convert a 0-indexed line number to the byte offset of its first byte.
    pub fn line_to_byte(&self, line: usize) -> usize {
        self.rope.line_to_byte(line)
    }

    /// Return a reference to the syntax layer for this buffer.
    pub fn syntax(&self) -> &SyntaxLayer {
        &self.syntax
    }

    // -----------------------------------------------------------------------
    // Edit operations (all route through apply_edit)
    // -----------------------------------------------------------------------

    /// Insert a single char at `pos`.
    pub fn insert_char(&mut self, pos: Position, ch: char) {
        self.apply_edit(Edit {
            range: Range {
                start: pos,
                end: pos,
            },
            replacement: ch.to_string(),
        });
    }

    /// Insert `text` at `pos`.
    pub fn insert_str(&mut self, pos: Position, text: &str) {
        self.apply_edit(Edit {
            range: Range {
                start: pos,
                end: pos,
            },
            replacement: text.to_string(),
        });
    }

    /// Delete the text covered by `range`.
    pub fn delete_range(&mut self, range: Range) {
        self.apply_edit(Edit {
            range,
            replacement: String::new(),
        });
    }

    /// Replace the text covered by `range` with `replacement`.
    pub fn replace_range(&mut self, range: Range, replacement: &str) {
        self.apply_edit(Edit {
            range,
            replacement: replacement.to_string(),
        });
    }

    // -----------------------------------------------------------------------
    // Undo / redo
    // -----------------------------------------------------------------------

    /// Apply an edit, record its inverse on the undo stack, and clear redo.
    ///
    /// Coalesces with the previous group when:
    /// - the new edit is a single-char true insert (start == end, 1 char)
    /// - the prior group's last edit was also a single-char true insert
    /// - within `COALESCE_MS` ms of the prior group's last edit
    /// - adjacent in buffer (new start == position after prior insert)
    /// - `force_new_group` is not set
    pub fn apply_edit(&mut self, edit: Edit) {
        self.apply_edit_at(edit, Instant::now());
    }

    /// Force a new undo group boundary on the next `apply_edit`.
    ///
    /// Call this on cursor jumps, selection changes, and save() so subsequent
    /// edits do not coalesce into the prior group.
    pub fn flush_undo_group(&mut self) {
        self.force_new_group = true;
    }

    /// Undo the top group on the undo stack. No-op if the stack is empty.
    ///
    /// Applies each record's `inverse` in reverse group order, then pushes
    /// the group onto the redo stack (unchanged — redo re-applies `.edit`).
    pub fn undo(&mut self) {
        if let Some(group) = self.undo_stack.undo.pop_back() {
            // Apply inverses in reverse order.
            for record in group.iter().rev() {
                self.apply_edit_internal(&record.inverse);
            }
            // The group, with edit/inverse intact, becomes the redo group.
            self.undo_stack.redo.push_back(group);
            self.revisions += 1;
        }
    }

    /// Redo the top group on the redo stack. No-op if the stack is empty.
    ///
    /// Applies each record's `edit` in forward order, then pushes the group
    /// back onto the undo stack.
    pub fn redo(&mut self) {
        if let Some(group) = self.undo_stack.redo.pop_back() {
            // Apply forward edits in forward order.
            for record in group.iter() {
                self.apply_edit_internal(&record.edit);
            }
            self.undo_stack.undo.push_back(group);
            self.revisions += 1;
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Core of `apply_edit`; also used internally by `undo`/`redo`.
    /// Handles undo recording, coalescing, and rope mutation.
    fn apply_edit_at(&mut self, edit: Edit, at: Instant) {
        // Capture the text that will be overwritten BEFORE mutating the rope.
        let start_char = self.pos_to_char_idx(edit.range.start);
        let end_char = self.pos_to_char_idx(edit.range.end);
        let overwritten: String = self.rope.slice(start_char..end_char).chars().collect();

        // Check coalesce eligibility BEFORE mutating (needs current rope state).
        let should_coalesce = !self.force_new_group && self.can_coalesce(&edit, at);
        self.force_new_group = false;

        // Mutate the rope.
        self.apply_edit_internal(&edit);
        self.revisions += 1;

        // NE8: invalidate the syntax highlight cache on every edit.
        self.syntax.invalidate();

        // Clear redo — any new edit invalidates the redo path.
        self.undo_stack.redo.clear();

        // Build the inverse AFTER applying: the inverse range end is now the
        // position after the replacement text in the new rope.
        let n_replacement_chars = edit.replacement.chars().count();
        let inverse_end = self.position_after(edit.range.start, n_replacement_chars);
        let inverse = Edit {
            range: Range {
                start: edit.range.start,
                end: inverse_end,
            },
            replacement: overwritten,
        };

        let record = EditRecord { edit, inverse, at };

        if should_coalesce {
            // Safety: can_coalesce only returns true when undo is non-empty.
            self.undo_stack.undo.back_mut().unwrap().push(record);
        } else {
            self.undo_stack.undo.push_back(vec![record]);
            // Evict oldest group if over cap.
            while self.undo_stack.undo.len() > self.undo_stack.cap {
                self.undo_stack.undo.pop_front();
            }
        }
    }

    /// Apply the rope mutation for `edit` (no undo recording).
    fn apply_edit_internal(&mut self, edit: &Edit) {
        let start_char = self.pos_to_char_idx(edit.range.start);
        let end_char = self.pos_to_char_idx(edit.range.end);
        if start_char < end_char {
            self.rope.remove(start_char..end_char);
        }
        if !edit.replacement.is_empty() {
            self.rope.insert(start_char, &edit.replacement);
        }
    }

    /// Check whether `edit` can coalesce into the current top undo group.
    fn can_coalesce(&self, edit: &Edit, at: Instant) -> bool {
        // Must be a single-char true insert (no deletion).
        if edit.range.start != edit.range.end {
            return false;
        }
        if edit.replacement.chars().count() != 1 {
            return false;
        }
        let group = match self.undo_stack.undo.back() {
            Some(g) => g,
            None => return false,
        };
        let prior = match group.last() {
            Some(r) => r,
            None => return false,
        };
        // Prior must also be a single-char true insert.
        if prior.edit.range.start != prior.edit.range.end {
            return false;
        }
        if prior.edit.replacement.chars().count() != 1 {
            return false;
        }
        // Within 500ms.
        if at.duration_since(prior.at).as_millis() >= COALESCE_MS {
            return false;
        }
        // Adjacency: new edit's start must equal the position right after the
        // prior insert (prior.range.start advanced by 1 char).
        let expected_next = self.position_after(prior.edit.range.start, 1);
        edit.range.start == expected_next
    }

    /// Walk `n` Unicode scalar values forward from `start` through the rope.
    ///
    /// Used to compute the inverse range end (where the replacement ends up).
    /// This is called BEFORE applying the edit, so the rope is in the pre-edit
    /// state — but `start` is already a valid position in that state.
    fn position_after(&self, start: Position, n_chars: usize) -> Position {
        if n_chars == 0 {
            return start;
        }
        let start_idx = self.pos_to_char_idx(start);
        // Walk forward n_chars through the rope chars, counting newlines.
        let total_chars = self.rope.len_chars();
        let mut line = start.line;
        let mut remaining = n_chars;

        // Recompute col_chars from line start to start_idx so we can track col.
        let line_start_idx = self.rope.line_to_char(line);
        let already_walked = start_idx - line_start_idx;
        let mut col_chars = already_walked;

        let mut idx = start_idx;
        while remaining > 0 && idx < total_chars {
            let ch = self.rope.char(idx);
            idx += 1;
            remaining -= 1;
            if ch == '\n' {
                line += 1;
                col_chars = 0;
            } else {
                col_chars += 1;
            }
        }
        // col_chars is a raw scalar count; convert to grapheme column by re-reading
        // the line up to col_chars scalars.
        let grapheme_col = self.scalar_offset_to_grapheme_col(line, col_chars);
        Position {
            line,
            col: grapheme_col,
        }
    }

    /// Convert a scalar offset `n` from the start of `line` to a grapheme column.
    fn scalar_offset_to_grapheme_col(&self, line: usize, scalar_offset: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let line_slice = self.rope.line(line);
        let line_str: String = line_slice.chars().collect();
        let mut col = 0usize;
        let mut scalars_seen = 0usize;
        for grapheme in line_str.graphemes(true) {
            if scalars_seen >= scalar_offset {
                break;
            }
            scalars_seen += grapheme.chars().count();
            col += 1;
        }
        col
    }

    // -----------------------------------------------------------------------
    // Position conversion — grapheme-aware
    // -----------------------------------------------------------------------

    /// Convert a `Position` (line, grapheme-col) to a rope char index.
    ///
    /// If `pos.col` exceeds the grapheme count of the line, it is clamped to
    /// the end of the line (before any trailing newline).
    fn pos_to_char_idx(&self, pos: Position) -> usize {
        let line_count = self.rope.len_lines();
        // Clamp line to valid range.
        let line_idx = pos.line.min(line_count.saturating_sub(1));
        let line_char_start = self.rope.line_to_char(line_idx);
        let line_slice = self.rope.line(line_idx);

        // Build the line text to walk graphemes.
        let line_str: String = line_slice.chars().collect();

        // Walk graphemes, counting up to pos.col.
        let mut char_offset = 0usize;
        for (grapheme_count, grapheme) in line_str.graphemes(true).enumerate() {
            if grapheme_count == pos.col {
                break;
            }
            char_offset += grapheme.chars().count();
        }
        // char_offset now points to the target grapheme (or end of line content
        // if col was clamped). Don't go past a trailing newline.
        let line_char_len = line_slice.len_chars();
        let trailing_newline = if line_char_len > 0 {
            let last = line_slice.char(line_char_len - 1);
            (last == '\n' || last == '\r') as usize
        } else {
            0
        };
        let max_offset = line_char_len.saturating_sub(trailing_newline);
        line_char_start + char_offset.min(max_offset)
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Encoding helpers (NE2) — std only, no new crates
// ---------------------------------------------------------------------------

/// Detect BOM, strip it, and return the file text as UTF-8.
fn decode_bytes(bytes: &[u8]) -> Result<String, IoError> {
    // UTF-8 BOM: EF BB BF
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let without_bom = &bytes[3..];
        return std::str::from_utf8(without_bom)
            .map(|s| s.to_owned())
            .map_err(|_| IoError::Encoding(EncodingError::InvalidUtf8));
    }
    // UTF-16 LE BOM: FF FE
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16_le(&bytes[2..]);
    }
    // UTF-16 BE BOM: FE FF
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16_be(&bytes[2..]);
    }
    // No BOM: attempt UTF-8.
    std::str::from_utf8(bytes)
        .map(|s| s.to_owned())
        .map_err(|_| IoError::Encoding(EncodingError::InvalidUtf8))
}

/// Decode UTF-16 LE bytes (BOM already stripped) to a UTF-8 `String`.
fn decode_utf16_le(bytes: &[u8]) -> Result<String, IoError> {
    if bytes.len() % 2 != 0 {
        return Err(IoError::Encoding(EncodingError::InvalidUtf8));
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| IoError::Encoding(EncodingError::InvalidUtf8))
}

/// Decode UTF-16 BE bytes (BOM already stripped) to a UTF-8 `String`.
fn decode_utf16_be(bytes: &[u8]) -> Result<String, IoError> {
    if bytes.len() % 2 != 0 {
        return Err(IoError::Encoding(EncodingError::InvalidUtf8));
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| IoError::Encoding(EncodingError::InvalidUtf8))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
impl Buffer {
    /// Like `apply_edit` but with an injected timestamp — for testing the
    /// 500ms coalesce boundary without sleeping.
    pub fn apply_edit_at_ts(&mut self, edit: Edit, at: Instant) {
        self.apply_edit_at(edit, at);
    }

    /// Create a buffer with a custom undo cap (for cap-eviction tests).
    pub fn with_undo_cap(cap: usize) -> Buffer {
        Buffer {
            rope: ropey::Rope::new(),
            revisions: 0,
            proposals: Vec::new(),
            ghost_text: Vec::new(),
            undo_stack: UndoStack::new(cap),
            force_new_group: false,
            tracked_path: None,
            tracked_mtime: None,
            syntax: SyntaxLayer::new(),
        }
    }

    /// Return the current undo group depth.
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.undo.len()
    }

    /// Return the current redo group depth.
    pub fn redo_depth(&self) -> usize {
        self.undo_stack.redo.len()
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: usize, col: usize) -> Position {
        Position { line, col }
    }

    fn range(sl: usize, sc: usize, el: usize, ec: usize) -> Range {
        Range {
            start: pos(sl, sc),
            end: pos(el, ec),
        }
    }

    fn text(b: &Buffer) -> String {
        b.rope.chars().collect()
    }

    // ── Construction ─────────────────────────────────────────────────────────

    #[test]
    fn buffer_new_is_empty() {
        let b = Buffer::new();
        assert_eq!(b.char_count(), 0);
        assert_eq!(b.byte_len(), 0);
        assert_eq!(b.line_count(), 1); // ropey: empty rope has 1 "line"
        assert!(b.proposals.is_empty());
        assert!(b.ghost_text.is_empty());
        assert_eq!(b.revisions, 0);
    }

    #[test]
    fn buffer_from_text_round_trip() {
        let t = "hello\nworld\n";
        let b = Buffer::from_text(t);
        let collected: String = b.rope.chars().collect();
        assert_eq!(collected, t);
        assert_eq!(b.byte_len(), t.len());
    }

    // ── Line access ──────────────────────────────────────────────────────────

    #[test]
    fn buffer_line_access() {
        let b = Buffer::from_text("foo\nbar\nbaz\n");
        let line0: String = b.line(0).chars().collect();
        assert_eq!(line0, "foo\n");
        let line1: String = b.line(1).chars().collect();
        assert_eq!(line1, "bar\n");
    }

    #[test]
    fn buffer_line_count() {
        let b = Buffer::from_text("a\nb\nc\n");
        // ropey counts trailing newline as a 4th empty line.
        assert!(b.line_count() >= 3);
    }

    // ── char_to_line roundtrip ───────────────────────────────────────────────

    #[test]
    fn buffer_char_to_line_roundtrip() {
        let b = Buffer::from_text("abc\ndef\n");
        // char index 0 → line 0
        assert_eq!(b.char_to_line(0), 0);
        // char index 4 → line 1 ('d')
        assert_eq!(b.char_to_line(4), 1);
        // line_to_char(1) → 4
        assert_eq!(b.line_to_char(1), 4);
    }

    // ── Insert char ──────────────────────────────────────────────────────────

    #[test]
    fn buffer_insert_char_ascii() {
        let mut b = Buffer::from_text("hllo\n");
        b.insert_char(pos(0, 1), 'e');
        let result: String = b.rope.chars().collect();
        assert_eq!(result, "hello\n");
    }

    #[test]
    fn buffer_insert_char_at_line_split() {
        let mut b = Buffer::from_text("helloworld\n");
        // Insert newline at grapheme col 5.
        b.insert_char(pos(0, 5), '\n');
        assert_eq!(b.line_count(), 3); // "hello\n", "world\n", ""
        let line0: String = b.line(0).chars().collect();
        assert_eq!(line0, "hello\n");
        let line1: String = b.line(1).chars().collect();
        assert_eq!(line1, "world\n");
    }

    // ── Insert str — multibyte ───────────────────────────────────────────────

    #[test]
    fn buffer_insert_str_multibyte() {
        // 'é' is U+00E9, 2 UTF-8 bytes, 1 grapheme.
        let mut b = Buffer::from_text("hllo\n");
        b.insert_str(pos(0, 1), "é");
        let result: String = b.rope.chars().collect();
        assert_eq!(result, "héllo\n");
        // Grapheme col 1 should hold 'é'.
        let line: String = b.line(0).chars().collect();
        let graphemes: Vec<&str> = line.graphemes(true).collect();
        assert_eq!(graphemes[1], "é");
    }

    // ── Insert emoji ─────────────────────────────────────────────────────────

    #[test]
    fn buffer_insert_emoji() {
        // "🎉" is U+1F389, 4 UTF-8 bytes, 1 grapheme.
        let mut b = Buffer::from_text("ab\n");
        b.insert_char(pos(0, 1), '🎉');
        let result: String = b.rope.chars().collect();
        assert_eq!(result, "a🎉b\n");
        // Grapheme at col 1 should be the party popper.
        let line: String = b.line(0).chars().collect();
        let graphemes: Vec<&str> = line.graphemes(true).collect();
        assert_eq!(graphemes[1], "🎉");
    }

    // ── Insert CJK ───────────────────────────────────────────────────────────

    #[test]
    fn buffer_insert_cjk() {
        // "你好" — each char is 3 UTF-8 bytes, 1 grapheme each.
        let mut b = Buffer::from_text("\n");
        b.insert_str(pos(0, 0), "你好");
        let result: String = b.rope.chars().collect();
        assert_eq!(result, "你好\n");
        assert_eq!(b.byte_len(), "你好\n".len());
    }

    // ── Delete range ─────────────────────────────────────────────────────────

    #[test]
    fn buffer_delete_range_within_line() {
        let mut b = Buffer::from_text("hello world\n");
        // Delete " world" (cols 5..11).
        b.delete_range(range(0, 5, 0, 11));
        let result: String = b.rope.chars().collect();
        assert_eq!(result, "hello\n");
    }

    #[test]
    fn buffer_delete_range_across_lines() {
        let mut b = Buffer::from_text("line0\nline1\nline2\n");
        // Delete from end of line 0 (col 5) to start of line 2 (col 0).
        b.delete_range(range(0, 5, 2, 0));
        let result: String = b.rope.chars().collect();
        assert_eq!(result, "line0line2\n");
    }

    // ── Replace range ────────────────────────────────────────────────────────

    #[test]
    fn buffer_replace_range() {
        let mut b = Buffer::from_text("foo bar\n");
        b.replace_range(range(0, 4, 0, 7), "baz");
        let result: String = b.rope.chars().collect();
        assert_eq!(result, "foo baz\n");
    }

    // ── Edge cases ───────────────────────────────────────────────────────────

    #[test]
    fn buffer_empty_buffer_edge_cases() {
        // Insert into empty buffer.
        let mut b = Buffer::new();
        b.insert_char(pos(0, 0), 'x');
        assert_eq!(b.char_count(), 1);

        // Delete zero-width range is a no-op.
        let mut b2 = Buffer::from_text("abc\n");
        b2.delete_range(range(0, 1, 0, 1));
        assert_eq!(b2.char_count(), 4);

        // line(0) on empty buffer.
        let b3 = Buffer::new();
        let s: String = b3.line(0).chars().collect();
        assert_eq!(s, "");
    }

    // ── NE3: Undo / redo ─────────────────────────────────────────────────────

    #[test]
    fn undo_single_char_insert() {
        let mut b = Buffer::new();
        b.insert_char(pos(0, 0), 'h');
        assert_eq!(text(&b), "h");
        b.undo();
        assert_eq!(text(&b), "");
        assert_eq!(b.undo_depth(), 0);
    }

    #[test]
    fn undo_coalesces_consecutive_typing() {
        // Type "hello" letter by letter — should form one undo group.
        let mut b = Buffer::new();
        let t0 = Instant::now();
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 0),
                    end: pos(0, 0),
                },
                replacement: "h".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 1),
                    end: pos(0, 1),
                },
                replacement: "e".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 2),
                    end: pos(0, 2),
                },
                replacement: "l".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 3),
                    end: pos(0, 3),
                },
                replacement: "l".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 4),
                    end: pos(0, 4),
                },
                replacement: "o".into(),
            },
            t0,
        );
        assert_eq!(text(&b), "hello");
        assert_eq!(
            b.undo_depth(),
            1,
            "all 5 chars should coalesce into one group"
        );
        b.undo();
        assert_eq!(text(&b), "");
    }

    #[test]
    fn undo_breaks_on_cursor_jump() {
        let mut b = Buffer::new();
        let t0 = Instant::now();
        // Type "hi".
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 0),
                    end: pos(0, 0),
                },
                replacement: "h".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 1),
                    end: pos(0, 1),
                },
                replacement: "i".into(),
            },
            t0,
        );
        // Simulate cursor jump.
        b.flush_undo_group();
        // Type "lo" at a non-adjacent position (simulate cursor moved to col 0).
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 0),
                    end: pos(0, 0),
                },
                replacement: "l".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 1),
                    end: pos(0, 1),
                },
                replacement: "o".into(),
            },
            t0,
        );
        assert_eq!(b.undo_depth(), 2, "flush_undo_group must break coalescing");
        // First undo: removes the "lo" group.
        b.undo();
        // Second undo: removes the "hi" group.
        b.undo();
        assert_eq!(text(&b), "");
        assert_eq!(b.undo_depth(), 0);
    }

    #[test]
    fn undo_breaks_on_500ms_gap() {
        let mut b = Buffer::new();
        let t0 = Instant::now();
        // Type "h" at t0.
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 0),
                    end: pos(0, 0),
                },
                replacement: "h".into(),
            },
            t0,
        );
        // Type "i" at t0 + 600ms — beyond the 500ms window.
        let t1 = t0 + std::time::Duration::from_millis(600);
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 1),
                    end: pos(0, 1),
                },
                replacement: "i".into(),
            },
            t1,
        );
        assert_eq!(b.undo_depth(), 2, "600ms gap must break coalescing");
        assert_eq!(text(&b), "hi");
    }

    #[test]
    fn redo_after_undo_restores() {
        let mut b = Buffer::new();
        let t0 = Instant::now();
        // Type "abc" — one coalesced group.
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 0),
                    end: pos(0, 0),
                },
                replacement: "a".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 1),
                    end: pos(0, 1),
                },
                replacement: "b".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 2),
                    end: pos(0, 2),
                },
                replacement: "c".into(),
            },
            t0,
        );
        assert_eq!(text(&b), "abc");
        b.undo();
        assert_eq!(text(&b), "");
        b.redo();
        assert_eq!(text(&b), "abc");
    }

    #[test]
    fn new_edit_after_undo_clears_redo() {
        let mut b = Buffer::new();
        let t0 = Instant::now();
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 0),
                    end: pos(0, 0),
                },
                replacement: "a".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 1),
                    end: pos(0, 1),
                },
                replacement: "b".into(),
            },
            t0,
        );
        b.apply_edit_at_ts(
            Edit {
                range: Range {
                    start: pos(0, 2),
                    end: pos(0, 2),
                },
                replacement: "c".into(),
            },
            t0,
        );
        b.undo();
        assert_eq!(b.redo_depth(), 1);
        // New edit clears the redo stack.
        b.insert_char(pos(0, 0), 'x');
        assert_eq!(
            b.redo_depth(),
            0,
            "redo stack must be cleared after a new edit"
        );
        // Redo is a no-op.
        b.redo();
        assert_eq!(b.redo_depth(), 0);
    }

    #[test]
    fn undo_cap_evicts_oldest() {
        let mut b = Buffer::with_undo_cap(3);
        // Create 4 distinct undo groups by flushing between each.
        for i in 0u8..4 {
            b.flush_undo_group();
            b.insert_char(pos(0, i as usize), 'a');
        }
        assert_eq!(
            b.undo_depth(),
            3,
            "oldest group evicted when cap == 3 and 4 groups inserted"
        );
    }

    // ── NE2: File IO ──────────────────────────────────────────────────────────

    #[test]
    fn io_round_trip_ascii() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ascii.txt");
        let content = "hello world\n";
        std::fs::write(&path, content).unwrap();
        let buf = Buffer::from_path(&path).unwrap();
        assert_eq!(buf.to_text(), content);
    }

    #[test]
    fn io_round_trip_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("utf8.txt");
        let content = "日本語テスト\nこんにちは\n";
        std::fs::write(&path, content).unwrap();
        let buf = Buffer::from_path(&path).unwrap();
        assert_eq!(buf.to_text(), content);
    }

    #[test]
    fn io_utf8_bom_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bom_utf8.txt");
        // UTF-8 BOM (EF BB BF) followed by content.
        let mut bytes = vec![0xEF_u8, 0xBB, 0xBF];
        bytes.extend_from_slice(b"hello\n");
        std::fs::write(&path, &bytes).unwrap();
        let buf = Buffer::from_path(&path).unwrap();
        // BOM must be stripped; text starts at "hello".
        assert_eq!(buf.to_text(), "hello\n");
    }

    #[test]
    fn io_utf16_le_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("utf16le.txt");
        let content = "hi\n";
        // BOM (FF FE) + code units in little-endian order.
        let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
        for unit in content.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();
        let buf = Buffer::from_path(&path).unwrap();
        assert_eq!(buf.to_text(), content);
    }

    #[test]
    fn io_utf16_be_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("utf16be.txt");
        let content = "hi\n";
        // BOM (FE FF) + code units in big-endian order.
        let mut bytes: Vec<u8> = vec![0xFE, 0xFF];
        for unit in content.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();
        let buf = Buffer::from_path(&path).unwrap();
        assert_eq!(buf.to_text(), content);
    }

    #[test]
    fn io_too_large_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.txt");
        // 3 bytes; limit is 2 bytes.
        std::fs::write(&path, b"abc").unwrap();
        let result = Buffer::from_path_with_limit(&path, 2);
        assert!(
            matches!(result, Err(IoError::TooLarge)),
            "expected TooLarge"
        );
    }

    #[test]
    fn io_invalid_utf8_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.txt");
        // No BOM; bytes are not valid UTF-8.
        std::fs::write(&path, b"\x80\x81invalid").unwrap();
        let result = Buffer::from_path(&path);
        assert!(
            matches!(result, Err(IoError::Encoding(EncodingError::InvalidUtf8))),
            "expected InvalidUtf8"
        );
    }

    #[test]
    fn io_atomic_save_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("save_test.txt");
        let content = "saved content\n";
        let mut buf = Buffer::from_text(content);
        buf.save(&path).unwrap();
        // Verify file on disk contains the expected bytes.
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk, content);
        // Reload and confirm round-trip.
        let buf2 = Buffer::from_path(&path).unwrap();
        assert_eq!(buf2.to_text(), content);
    }

    #[test]
    fn io_external_modification_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("watch.txt");
        std::fs::write(&path, b"original\n").unwrap();
        let buf = Buffer::from_path(&path).unwrap();
        // No external modification yet.
        assert!(!buf.is_externally_modified());
        // Sleep ≥ 1 second to advance the filesystem mtime resolution.
        std::thread::sleep(std::time::Duration::from_secs(1));
        std::fs::write(&path, b"modified\n").unwrap();
        assert!(buf.is_externally_modified());
    }
}
