//! In-buffer search state for native editor panes — NE11.
//!
//! `EditorSearch` holds the current query, compiled regex flag, and the list
//! of hit ranges (in buffer `Position` space). It is stored as
//! `Option<EditorSearch>` on `EditorPane`; `None` means the search bar is
//! closed for that pane.

use anvil_editor::{Buffer, Position, Range};

/// All search hits and navigation state for a native editor pane.
pub struct EditorSearch {
    pub query: String,
    pub is_regex: bool,
    pub hits: Vec<Range>,
    pub current: usize,
    /// When `Some`, the search bar shows a second "replace" row (item 9).
    /// `None` means plain find mode.
    pub replace_input: Option<String>,
}

impl EditorSearch {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            is_regex: false,
            hits: Vec::new(),
            current: 0,
            replace_input: None,
        }
    }

    /// Open in find+replace mode — sets `replace_input` to an empty string if
    /// not already set. Query is preserved.
    pub fn open_replace(&mut self) {
        if self.replace_input.is_none() {
            self.replace_input = Some(String::new());
        }
    }

    /// Close replace mode without closing find.
    pub fn close_replace(&mut self) {
        self.replace_input = None;
    }

    /// Recompute `hits` from the current `query` and `is_regex` against `buffer`.
    ///
    /// If the query is empty, hits are cleared.  On regex compile error the hits
    /// are cleared (silently — the user sees 0/0 in the bar).
    pub fn rescan(&mut self, buffer: &Buffer) {
        self.hits.clear();
        self.current = 0;
        if self.query.is_empty() {
            return;
        }
        let text = buffer.to_text();
        if self.is_regex {
            let Ok(re) = regex::Regex::new(&self.query) else {
                return;
            };
            for m in re.find_iter(&text) {
                if let Some(r) = byte_range_to_positions(buffer, &text, m.start(), m.end()) {
                    self.hits.push(r);
                }
            }
        } else {
            // Literal, case-sensitive.
            for (byte_start, _) in text.match_indices(self.query.as_str()) {
                let byte_end = byte_start + self.query.len();
                if let Some(r) = byte_range_to_positions(buffer, &text, byte_start, byte_end) {
                    self.hits.push(r);
                }
            }
        }
    }

    /// Advance to the next hit (wrapping).
    pub fn next(&mut self) {
        if self.hits.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.hits.len();
    }

    /// Retreat to the previous hit (wrapping).
    pub fn prev(&mut self) {
        if self.hits.is_empty() {
            return;
        }
        self.current = if self.current == 0 {
            self.hits.len() - 1
        } else {
            self.current - 1
        };
    }

    /// Return the current hit range, or `None` when there are no hits.
    pub fn current_hit(&self) -> Option<Range> {
        self.hits.get(self.current).copied()
    }

    /// Clear all hits and reset state (used by `SearchClose`).
    pub fn clear(&mut self) {
        self.hits.clear();
        self.current = 0;
        self.query.clear();
    }

    /// Number of hits.
    pub fn count(&self) -> usize {
        self.hits.len()
    }
}

impl Default for EditorSearch {
    fn default() -> Self {
        Self::new()
    }
}

// ── private helpers ───────────────────────────────────────────────────────────

/// Convert byte offsets into `text` to a `Range` of `Position`s.
///
/// Returns `None` if the byte offset is out of range (should not happen for
/// matches returned by `match_indices` / `regex`, but guard anyway).
fn byte_range_to_positions(
    buffer: &Buffer,
    text: &str,
    byte_start: usize,
    byte_end: usize,
) -> Option<Range> {
    if byte_start > text.len() || byte_end > text.len() {
        return None;
    }
    let char_start = text[..byte_start].chars().count();
    let char_end = text[..byte_end].chars().count();
    let start_line = buffer.char_to_line(char_start);
    let end_line = buffer.char_to_line(char_end.saturating_sub(1).max(char_start));
    let start_col = char_start - buffer.line_to_char(start_line);
    let end_col = char_end - buffer.line_to_char(end_line);
    Some(Range {
        start: Position {
            line: start_line,
            col: start_col,
        },
        end: Position {
            line: end_line,
            col: end_col,
        },
    })
}

// ── Bracket matching (item 14) ────────────────────────────────────────────────

/// Find the matching bracket for the bracket at (or immediately before) `pos`.
///
/// Scans the buffer using a simple stack algorithm, capped to `max_lines` lines
/// to avoid hanging on huge files.
///
/// Returns `Some((open_pos, close_pos))` where `open_pos` ≤ `close_pos`.
/// Returns `None` when the cursor is not on a bracket, or the match is not
/// found within `max_lines`.
pub fn bracket_match_for(
    buffer: &Buffer,
    pos: Position,
    max_lines: usize,
) -> Option<(Position, Position)> {
    // Build a flat char list from the visible region (capped).
    let start_line = pos.line.saturating_sub(max_lines / 2);
    let end_line = (pos.line + max_lines / 2).min(buffer.line_count().saturating_sub(1));

    // Flatten the region into (line, col, char) triples.
    let mut chars: Vec<(usize, usize, char)> = Vec::new();
    for line_idx in start_line..=end_line {
        let line_str: String = buffer.line(line_idx).chars().collect();
        let trimmed: &str = line_str.trim_end_matches('\n').trim_end_matches('\r');
        for (col, ch) in trimmed.chars().enumerate() {
            chars.push((line_idx, col, ch));
        }
    }

    // Find the index of the bracket at `pos` (or immediately before if col > 0).
    fn is_bracket(c: char) -> bool {
        matches!(c, '(' | ')' | '{' | '}' | '[' | ']')
    }
    fn matching(c: char) -> char {
        match c {
            '(' => ')',
            ')' => '(',
            '{' => '}',
            '}' => '{',
            '[' => ']',
            ']' => '[',
            _ => c,
        }
    }
    fn is_open(c: char) -> bool {
        matches!(c, '(' | '{' | '[')
    }

    // Find the bracket position in our char list.
    let bracket_idx = chars
        .iter()
        .position(|&(l, c, ch)| l == pos.line && c == pos.col && is_bracket(ch))
        .or_else(|| {
            // Also try col - 1 (cursor immediately after a bracket).
            if pos.col > 0 {
                chars
                    .iter()
                    .position(|&(l, c, ch)| l == pos.line && c == pos.col - 1 && is_bracket(ch))
            } else {
                None
            }
        })?;

    let (bl, bc, bch) = chars[bracket_idx];
    let _ = bc;

    if is_open(bch) {
        // Scan forward for the matching close.
        let mut depth = 1i32;
        for &(l, c, ch) in &chars[bracket_idx + 1..] {
            if ch == bch {
                depth += 1;
            } else if ch == matching(bch) {
                depth -= 1;
                if depth == 0 {
                    return Some((Position { line: bl, col: bc }, Position { line: l, col: c }));
                }
            }
        }
    } else {
        // Scan backward for the matching open.
        let match_open = matching(bch);
        let mut depth = 1i32;
        for &(l, c, ch) in chars[..bracket_idx].iter().rev() {
            if ch == bch {
                depth += 1;
            } else if ch == match_open {
                depth -= 1;
                if depth == 0 {
                    return Some((Position { line: l, col: c }, Position { line: bl, col: bc }));
                }
            }
        }
    }

    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        Buffer::from_text(text)
    }

    #[test]
    fn editor_search_finds_all_literal_hits() {
        let b = buf("foo bar foo baz foo");
        let mut s = EditorSearch::new();
        s.query = "foo".into();
        s.rescan(&b);
        assert_eq!(s.count(), 3, "expected 3 literal hits");
    }

    #[test]
    fn editor_search_regex_matches() {
        let b = buf("cat123 dog456 cat789");
        let mut s = EditorSearch::new();
        s.query = r"cat\d+".into();
        s.is_regex = true;
        s.rescan(&b);
        assert_eq!(s.count(), 2, "expected 2 regex hits");
    }

    #[test]
    fn editor_search_next_wraps() {
        let b = buf("a a a");
        let mut s = EditorSearch::new();
        s.query = "a".into();
        s.rescan(&b);
        let total = s.count();
        assert!(total > 0);
        // Advance past the end — should wrap to 0.
        for _ in 0..total {
            s.next();
        }
        assert_eq!(s.current, 0, "next() should wrap to 0");
    }

    #[test]
    fn editor_search_prev_wraps() {
        let b = buf("a a a");
        let mut s = EditorSearch::new();
        s.query = "a".into();
        s.rescan(&b);
        let total = s.count();
        assert!(total > 0);
        // prev from 0 should wrap to last.
        s.prev();
        assert_eq!(s.current, total - 1, "prev() from 0 should wrap to last");
    }

    #[test]
    fn editor_search_clear_drops_hits() {
        let b = buf("hello hello");
        let mut s = EditorSearch::new();
        s.query = "hello".into();
        s.rescan(&b);
        assert!(s.count() > 0);
        s.clear();
        assert_eq!(s.count(), 0, "clear() should drop all hits");
        assert!(s.query.is_empty(), "clear() should reset query");
    }

    // ── item 14: bracket matching ─────────────────────────────────────────────

    #[test]
    fn bracket_match_finds_close_for_open() {
        // "fn f() { }" — '{' is at col 7, '}' is at col 9.
        let b = buf("fn f() { }");
        let pos = Position { line: 0, col: 7 };
        let result = bracket_match_for(&b, pos, 2000);
        assert!(result.is_some(), "should find a bracket match");
        let (a, z) = result.unwrap();
        assert_eq!(a, Position { line: 0, col: 7 });
        assert_eq!(z, Position { line: 0, col: 9 });
    }

    #[test]
    fn bracket_match_finds_open_for_close() {
        // "fn f() { x }" — '{' at col 7, '}' at col 11.
        let b = buf("fn f() { x }");
        let pos = Position { line: 0, col: 11 };
        let result = bracket_match_for(&b, pos, 2000);
        assert!(result.is_some());
        let (a, z) = result.unwrap();
        assert_eq!(a.col, 7, "open brace column");
        assert_eq!(z.col, 11, "close brace column");
    }

    #[test]
    fn bracket_match_returns_none_when_unmatched() {
        let b = buf("(unclosed");
        let result = bracket_match_for(&b, Position { line: 0, col: 0 }, 2000);
        assert!(result.is_none(), "unmatched bracket should return None");
    }
}
