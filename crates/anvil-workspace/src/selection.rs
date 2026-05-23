//! Terminal text selection in content-coordinate space.
//!
//! A content row is an absolute index over scrollback + active grid, the same
//! space used by the terminal's `line_count()` / `line(i)` accessors.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Point {
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    pub active: bool,
    pub anchor: Point,
    pub head: Point,
}

impl Selection {
    pub fn clear(&mut self) {
        self.active = false;
    }

    /// Returns ordered `(start, end)` so `start <= end` in reading order.
    pub fn ordered(&self) -> (Point, Point) {
        let a = self.anchor;
        let h = self.head;
        if a.row < h.row || (a.row == h.row && a.col <= h.col) {
            (a, h)
        } else {
            (h, a)
        }
    }

    /// Is the content cell at `(row, col)` inside the selection?
    ///
    /// The range is half-open: `start.col..end.col` on a single row,
    /// `start.col..cols` on the first row, `0..cols` on middle rows,
    /// `0..end.col` on the last row.
    pub fn contains(&self, row: usize, col: usize) -> bool {
        if !self.active {
            return false;
        }
        let (s, e) = self.ordered();
        if row < s.row || row > e.row {
            return false;
        }
        if s.row == e.row {
            return col >= s.col && col < e.col;
        }
        if row == s.row {
            return col >= s.col;
        }
        if row == e.row {
            return col < e.col;
        }
        true // middle rows — entire row
    }
}

// ---------------------------------------------------------------------------
// Tests  (6 Zig tests → 6 Rust tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_selection_contains_nothing() {
        let sel = Selection::default();
        assert!(!sel.contains(0, 0));
    }

    #[test]
    fn single_row_selection_anchor_before_head() {
        let sel = Selection {
            active: true,
            anchor: Point { row: 5, col: 3 },
            head: Point { row: 5, col: 7 },
        };
        assert!(!sel.contains(5, 2));
        assert!(sel.contains(5, 3));
        assert!(sel.contains(5, 6));
        assert!(!sel.contains(5, 7)); // half-open
        assert!(!sel.contains(4, 5));
        assert!(!sel.contains(6, 5));
    }

    #[test]
    fn single_row_selection_reversed_drag() {
        let sel = Selection {
            active: true,
            anchor: Point { row: 5, col: 7 },
            head: Point { row: 5, col: 3 },
        };
        assert!(!sel.contains(5, 2));
        assert!(sel.contains(5, 3));
        assert!(sel.contains(5, 6));
        assert!(!sel.contains(5, 7));
    }

    #[test]
    fn multi_line_selection_first_row_from_start_col_last_up_to_end_col() {
        let sel = Selection {
            active: true,
            anchor: Point { row: 2, col: 4 },
            head: Point { row: 5, col: 10 },
        };
        // first row: col >= 4
        assert!(!sel.contains(2, 3));
        assert!(sel.contains(2, 4));
        assert!(sel.contains(2, 100));
        // middle rows: all cols
        assert!(sel.contains(3, 0));
        assert!(sel.contains(4, 999));
        // last row: col < 10
        assert!(sel.contains(5, 0));
        assert!(sel.contains(5, 9));
        assert!(!sel.contains(5, 10));
        // outside rows
        assert!(!sel.contains(1, 0));
        assert!(!sel.contains(6, 0));
    }

    #[test]
    fn multi_line_selection_reversed() {
        let sel = Selection {
            active: true,
            anchor: Point { row: 5, col: 10 },
            head: Point { row: 2, col: 4 },
        };
        assert!(!sel.contains(2, 3));
        assert!(sel.contains(2, 4));
        assert!(sel.contains(3, 0));
        assert!(sel.contains(5, 9));
        assert!(!sel.contains(5, 10));
    }

    #[test]
    fn empty_single_row_selection_zero_width() {
        let sel = Selection {
            active: true,
            anchor: Point { row: 3, col: 5 },
            head: Point { row: 3, col: 5 },
        };
        assert!(!sel.contains(3, 5)); // zero-width: nothing selected
        assert!(!sel.contains(3, 4));
    }

    #[test]
    fn clear_deactivates_selection() {
        let mut sel = Selection {
            active: true,
            anchor: Point { row: 0, col: 0 },
            head: Point { row: 0, col: 5 },
        };
        assert!(sel.contains(0, 0));
        sel.clear();
        assert!(!sel.active);
        assert!(!sel.contains(0, 0));
    }
}
