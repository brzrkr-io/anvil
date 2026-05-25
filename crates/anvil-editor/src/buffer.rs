//! Native rope-backed text buffer — NE1 skeleton.
//!
//! `Buffer` is the primary editing model. All positions are grapheme-column
//! based (not byte offsets). UTF-8 only.

use unicode_segmentation::UnicodeSegmentation;

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

/// A revision tag — empty placeholder, reserved for NE14.
#[derive(Debug, Clone)]
pub struct RevisionTag {}

// ---------------------------------------------------------------------------
// Buffer
// ---------------------------------------------------------------------------

/// A rope-backed UTF-8 text buffer.
pub struct Buffer {
    rope: ropey::Rope,
    /// AI-native proposal slots — allocated empty in NE1 so NE14 doesn't break shapes.
    pub proposals: Vec<EditProposal>,
    pub ghost_text: Vec<GhostTextSpan>,
    pub revisions: Vec<RevisionTag>,
}

impl Buffer {
    /// Create an empty buffer.
    pub fn new() -> Buffer {
        Buffer {
            rope: ropey::Rope::new(),
            proposals: Vec::new(),
            ghost_text: Vec::new(),
            revisions: Vec::new(),
        }
    }

    /// Create a buffer pre-loaded with `text`.
    pub fn from_text(text: &str) -> Buffer {
        Buffer {
            rope: ropey::Rope::from_str(text),
            proposals: Vec::new(),
            ghost_text: Vec::new(),
            revisions: Vec::new(),
        }
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

    // -----------------------------------------------------------------------
    // Edit operations
    // -----------------------------------------------------------------------

    /// Insert a single char at `pos`.
    pub fn insert_char(&mut self, pos: Position, ch: char) {
        let idx = self.pos_to_char_idx(pos);
        self.rope.insert_char(idx, ch);
    }

    /// Insert `text` at `pos`.
    pub fn insert_str(&mut self, pos: Position, text: &str) {
        let idx = self.pos_to_char_idx(pos);
        self.rope.insert(idx, text);
    }

    /// Delete the text covered by `range`.
    pub fn delete_range(&mut self, range: Range) {
        let start = self.pos_to_char_idx(range.start);
        let end = self.pos_to_char_idx(range.end);
        if start < end {
            self.rope.remove(start..end);
        }
    }

    /// Replace the text covered by `range` with `replacement`.
    pub fn replace_range(&mut self, range: Range, replacement: &str) {
        let start = self.pos_to_char_idx(range.start);
        let end = self.pos_to_char_idx(range.end);
        if start < end {
            self.rope.remove(start..end);
        }
        self.rope.insert(start, replacement);
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
// Tests
// ---------------------------------------------------------------------------

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

    // ── Construction ─────────────────────────────────────────────────────────

    #[test]
    fn buffer_new_is_empty() {
        let b = Buffer::new();
        assert_eq!(b.char_count(), 0);
        assert_eq!(b.byte_len(), 0);
        assert_eq!(b.line_count(), 1); // ropey: empty rope has 1 "line"
        assert!(b.proposals.is_empty());
        assert!(b.ghost_text.is_empty());
        assert!(b.revisions.is_empty());
    }

    #[test]
    fn buffer_from_text_round_trip() {
        let text = "hello\nworld\n";
        let b = Buffer::from_text(text);
        let collected: String = b.rope.chars().collect();
        assert_eq!(collected, text);
        assert_eq!(b.byte_len(), text.len());
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
}
