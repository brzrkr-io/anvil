//! Incremental substring search over a `Terminal`'s content rows.

use regex::Regex;

use crate::{cell::Cell, terminal::Terminal};

/// A run of matched cells on one content row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub row: usize,
    pub col: usize,
    pub len: usize,
}

/// How a cell should be tinted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    None,
    Other,
    Current,
}

/// Upper bound on stored matches.
pub const MAX_MATCHES: usize = 2048;

pub struct Search {
    query_buf: [u8; 256],
    query_len: usize,
    matches: Vec<Match>,
    pub current: usize,
    /// When true, the query is treated as a regular expression.
    regex_mode: bool,
}

impl Search {
    pub fn new() -> Self {
        Search {
            query_buf: [0; 256],
            query_len: 0,
            matches: Vec::new(),
            current: 0,
            regex_mode: false,
        }
    }

    /// Enable or disable regex mode. Callers must call `rescan` (or
    /// `set_query`) afterwards to update matches.
    pub fn set_regex(&mut self, enabled: bool) {
        self.regex_mode = enabled;
    }

    /// Returns true when regex mode is active.
    pub fn is_regex(&self) -> bool {
        self.regex_mode
    }

    pub fn query(&self) -> &str {
        std::str::from_utf8(&self.query_buf[..self.query_len]).unwrap_or("")
    }

    pub fn count(&self) -> usize {
        self.matches.len()
    }

    pub fn current_match(&self) -> Option<Match> {
        if self.matches.is_empty() {
            return None;
        }
        Some(self.matches[self.current])
    }

    /// Replace the query and re-scan `term`. Truncates past 256 bytes.
    /// `current` resets to 0.
    pub fn set_query(&mut self, term: &Terminal, text: &str) {
        let bytes = text.as_bytes();
        let n = self.query_buf.len().min(bytes.len());
        self.query_buf[..n].copy_from_slice(&bytes[..n]);
        self.query_len = n;
        self.rescan(term);
    }

    /// Re-run the scan with the existing query (e.g. after new shell output).
    ///
    /// In regex mode: if the query doesn't compile as a valid regex the
    /// existing matches are left unchanged (no crash, no clear).
    pub fn rescan(&mut self, term: &Terminal) {
        // Clone the query off `self.query_buf` so the immutable borrow of
        // self ends before we call the `&mut self` rescan_* helpers.
        let query_str = match std::str::from_utf8(&self.query_buf[..self.query_len]) {
            Ok(s) => s.to_owned(),
            Err(_) => return,
        };

        if self.regex_mode {
            self.rescan_regex(term, &query_str);
        } else {
            self.rescan_literal(term, &query_str);
        }
    }

    fn rescan_literal(&mut self, term: &Terminal, query_str: &str) {
        self.matches.clear();
        self.current = 0;

        // Decode the query to codepoints; detect smart-case.
        let mut q: Vec<char> = Vec::new();
        let mut case_sensitive = false;
        for cp in query_str.chars() {
            q.push(cp);
            if cp.is_ascii_uppercase() {
                case_sensitive = true;
            }
        }
        if q.is_empty() {
            return;
        }

        let qn = q.len();
        let total = term.line_count();
        for r in 0..total {
            let row = term.line(r);
            if row.len() < qn {
                continue;
            }
            let mut c = 0;
            while c + qn <= row.len() {
                if row_matches_at(row, c, &q, case_sensitive) {
                    self.matches.push(Match {
                        row: r,
                        col: c,
                        len: qn,
                    });
                    if self.matches.len() >= MAX_MATCHES {
                        return;
                    }
                }
                c += 1;
            }
        }
    }

    fn rescan_regex(&mut self, term: &Terminal, query_str: &str) {
        if query_str.is_empty() {
            self.matches.clear();
            self.current = 0;
            return;
        }

        // If the pattern is invalid, preserve the last-good matches.
        let re = match Regex::new(query_str) {
            Ok(r) => r,
            Err(_) => return,
        };

        self.matches.clear();
        self.current = 0;

        let total = term.line_count();
        for r in 0..total {
            let row = term.line(r);
            // Build a String from the row's cells so the regex can operate on it.
            let mut row_str = String::with_capacity(row.len());
            // Track where each char starts so we can map byte offset → col index.
            let mut col_for_byte: Vec<usize> = Vec::with_capacity(row.len() * 4);
            for (col_idx, cell) in row.iter().enumerate() {
                let byte_start = row_str.len();
                row_str.push(cell.cp);
                // Fill byte → col map for every byte the codepoint occupies.
                let encoded_len = row_str.len() - byte_start;
                for _ in 0..encoded_len {
                    col_for_byte.push(col_idx);
                }
            }

            for m in re.find_iter(&row_str) {
                let col_start = col_for_byte.get(m.start()).copied().unwrap_or(0);
                // End byte is exclusive; the col of the last matched cell is
                // col_for_byte[m.end()-1], so match length spans col_start to that col.
                let last_byte = m.end().saturating_sub(1);
                let col_end = col_for_byte.get(last_byte).copied().unwrap_or(col_start);
                let len = col_end + 1 - col_start;
                self.matches.push(Match {
                    row: r,
                    col: col_start,
                    len,
                });
                if self.matches.len() >= MAX_MATCHES {
                    return;
                }
            }
        }
    }

    /// Advance the current match (wraps). No-op when there are no matches.
    pub fn next(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.matches.len();
    }

    /// Step the current match back (wraps). No-op when there are no matches.
    pub fn prev(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.current = (self.current + self.matches.len() - 1) % self.matches.len();
    }

    /// How the cell at content (`row`, `col`) should be tinted.
    pub fn classify(&self, row: usize, col: usize) -> MatchKind {
        if self.matches.is_empty() {
            return MatchKind::None;
        }
        // Current match wins.
        let cur = &self.matches[self.current];
        if cur.row == row && col >= cur.col && col < cur.col + cur.len {
            return MatchKind::Current;
        }
        // Other matches.
        for (i, m) in self.matches.iter().enumerate() {
            if i == self.current {
                continue;
            }
            if m.row != row {
                continue;
            }
            if col >= m.col && col < m.col + m.len {
                return MatchKind::Other;
            }
        }
        MatchKind::None
    }
}

impl Default for Search {
    fn default() -> Self {
        Self::new()
    }
}

/// True when `row[col..col+q.len]` equals `q` (codepoint-wise, case-folded
/// for ASCII letters unless `case_sensitive`).
fn row_matches_at(row: &[Cell], col: usize, q: &[char], case_sensitive: bool) -> bool {
    for (i, &qc) in q.iter().enumerate() {
        let cc = row[col + i].cp;
        if case_sensitive {
            if cc != qc {
                return false;
            }
        } else if lower_cp(cc) != lower_cp(qc) {
            return false;
        }
    }
    true
}

/// Lowercase an ASCII letter; everything else unchanged.
fn lower_cp(cp: char) -> char {
    if cp.is_ascii_uppercase() {
        (cp as u8 + 32) as char
    } else {
        cp
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    fn make_terminal(cols: usize, rows: usize) -> Terminal {
        Terminal::new(cols, rows, 1000)
    }

    #[test]
    fn finds_a_substring_in_the_grid() {
        let mut t = make_terminal(40, 5);
        t.feed(b"hello world");
        let mut s = Search::new();
        s.set_query(&t, "world");
        assert_eq!(1, s.count());
        let m = s.current_match().unwrap();
        assert_eq!(6, m.col);
        assert_eq!(5, m.len);
    }

    #[test]
    fn smart_case_lowercase_query_is_case_insensitive() {
        let mut t = make_terminal(40, 5);
        t.feed(b"The Cat SAT");
        let mut s = Search::new();
        s.set_query(&t, "cat");
        assert_eq!(1, s.count());
    }

    #[test]
    fn smart_case_uppercase_letter_forces_case_sensitive() {
        let mut t = make_terminal(40, 5);
        t.feed(b"the cat Cat");
        let mut s = Search::new();
        s.set_query(&t, "Cat");
        assert_eq!(1, s.count());
    }

    #[test]
    fn empty_query_yields_no_matches() {
        let mut t = make_terminal(40, 5);
        t.feed(b"anything");
        let mut s = Search::new();
        s.set_query(&t, "");
        assert_eq!(0, s.count());
        assert!(s.current_match().is_none());
    }

    #[test]
    fn finds_multiple_matches_left_to_right() {
        let mut t = make_terminal(40, 5);
        t.feed(b"aa aa aa");
        let mut s = Search::new();
        s.set_query(&t, "aa");
        assert_eq!(3, s.count());
    }

    #[test]
    fn finds_a_match_in_scrollback() {
        let mut t = make_terminal(20, 3);
        t.feed(b"findme\r\n");
        t.feed(b"x\r\n");
        t.feed(b"x\r\n");
        t.feed(b"x\r\n");
        t.feed(b"x\r\n");
        t.feed(b"x\r\n");
        let mut s = Search::new();
        s.set_query(&t, "findme");
        assert_eq!(1, s.count());
        let m = s.current_match().unwrap();
        assert!(m.row < t.scrollback_len());
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let mut t = make_terminal(40, 5);
        t.feed(b"x x x");
        let mut s = Search::new();
        s.set_query(&t, "x");
        assert_eq!(3, s.count());
        assert_eq!(0, s.current);
        s.next();
        assert_eq!(1, s.current);
        s.next();
        s.next();
        assert_eq!(0, s.current);
        s.prev();
        assert_eq!(2, s.current);
    }

    #[test]
    fn next_prev_are_no_ops_with_no_matches() {
        let mut t = make_terminal(40, 5);
        t.feed(b"abc");
        let mut s = Search::new();
        s.set_query(&t, "zzz");
        s.next();
        s.prev();
        assert_eq!(0, s.current);
    }

    #[test]
    fn classify_tags_current_other_and_none() {
        let mut t = make_terminal(40, 5);
        t.feed(b"ab ab");
        let mut s = Search::new();
        s.set_query(&t, "ab");
        let r0 = t.content_row_of_viewport(0);
        assert_eq!(MatchKind::Current, s.classify(r0, 0));
        assert_eq!(MatchKind::Current, s.classify(r0, 1));
        assert_eq!(MatchKind::Other, s.classify(r0, 3));
        assert_eq!(MatchKind::None, s.classify(r0, 2));
    }

    #[test]
    fn classify_current_match_wins_on_overlap() {
        let mut t = make_terminal(40, 5);
        t.feed(b"aaa");
        let mut s = Search::new();
        s.set_query(&t, "aa");
        assert_eq!(2, s.count());
        s.next();
        assert_eq!(1, s.current);
        let r0 = t.content_row_of_viewport(0);
        assert_eq!(MatchKind::Current, s.classify(r0, 1));
    }

    #[test]
    fn search_default_impl_matches_new() {
        let s: Search = Default::default();
        assert_eq!(0, s.count());
        assert_eq!(0, s.current);
    }

    #[test]
    fn classify_with_no_matches_returns_none() {
        let mut t = make_terminal(10, 2);
        t.feed(b"hello");
        let mut s = Search::new();
        s.set_query(&t, "zzz"); // no matches
        assert_eq!(MatchKind::None, s.classify(0, 0));
    }

    #[test]
    fn classify_row_mismatch_returns_none_for_that_row() {
        // Match on row 0; classify on a different row returns None.
        let mut t = make_terminal(40, 5);
        t.feed(b"ab\r\ncd");
        let mut s = Search::new();
        s.set_query(&t, "ab");
        assert_eq!(1, s.count());
        // Row 1 has 'cd', not 'ab'. classify(1, 0) should be None.
        let r1 = t.content_row_of_viewport(1);
        assert_eq!(MatchKind::None, s.classify(r1, 0));
    }

    #[test]
    fn max_matches_cap_stops_scan() {
        // Fill a tall terminal with 'a' to exceed MAX_MATCHES = 2048.
        // With a 100-col terminal, each row gives 97 matches for "aa".
        // We need roughly 22 rows of filled 'a' to exceed 2048 (22*97=2134).
        let mut t = make_terminal(100, 25);
        for _ in 0..25 {
            // Fill row with 'a' then LF.
            let row: Vec<u8> = b"a".repeat(100);
            t.feed(&row);
            t.feed(b"\r\n");
        }
        let mut s = Search::new();
        s.set_query(&t, "aa");
        // Must not exceed MAX_MATCHES.
        assert_eq!(MAX_MATCHES, s.count());
    }

    // --- Regex mode tests ---------------------------------------------------

    #[test]
    fn regex_mode_finds_pattern_match() {
        let mut t = make_terminal(40, 5);
        t.feed(b"hello world");
        let mut s = Search::new();
        s.set_regex(true);
        assert!(s.is_regex());
        s.set_query(&t, r"w\w+"); // matches "world"
        assert_eq!(1, s.count());
        let m = s.current_match().unwrap();
        assert_eq!(6, m.col);
        assert_eq!(5, m.len);
    }

    #[test]
    fn regex_invalid_pattern_preserves_last_good_matches() {
        let mut t = make_terminal(40, 5);
        t.feed(b"hello world");
        let mut s = Search::new();
        s.set_regex(true);
        // First, a valid query that finds matches.
        s.set_query(&t, "world");
        assert_eq!(1, s.count());
        // Now set an invalid regex — matches should not change.
        s.set_query(&t, "[invalid(");
        assert_eq!(1, s.count(), "bad regex should preserve last-good matches");
    }

    #[test]
    fn regex_empty_pattern_yields_no_matches() {
        let mut t = make_terminal(40, 5);
        t.feed(b"hello world");
        let mut s = Search::new();
        s.set_regex(true);
        s.set_query(&t, "");
        assert_eq!(0, s.count());
    }

    #[test]
    fn regex_mode_can_be_toggled() {
        let mut s = Search::new();
        assert!(!s.is_regex());
        s.set_regex(true);
        assert!(s.is_regex());
        s.set_regex(false);
        assert!(!s.is_regex());
    }

    #[test]
    fn literal_mode_unaffected_by_regex_toggle() {
        let mut t = make_terminal(40, 5);
        // "a.b" is a regex wildcard but a literal non-match for "acb".
        t.feed(b"a.b acb");
        let mut s = Search::new();
        s.set_regex(false);
        s.set_query(&t, "a.b");
        // Literal: only matches "a.b" exactly (col 0).
        assert_eq!(1, s.count());
        assert_eq!(0, s.current_match().unwrap().col);
    }
}
