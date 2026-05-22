//! Deep terminal scrollback: a ring buffer of *trimmed* rows.
//!
//! Each pushed row is copied into a fresh `Vec` sized to only its used
//! width — trailing blank cells are dropped. Most terminal lines are short,
//! so this keeps memory tiny even at very large capacities. When the ring is
//! full the oldest row is evicted to make room.

use crate::cell::Cell;

/// Default ring capacity in rows. Deliberately enormous — trimmed storage
/// makes this cheap — and trivially raised by callers that want even more.
pub const DEFAULT_CAPACITY: usize = 100_000;

/// A ring buffer of trimmed terminal rows.
pub struct Scrollback {
    /// Ring of trimmed rows. `rows[i]` owns its `Vec<Cell>`.
    rows: Vec<Option<Vec<Cell>>>,
    /// Index of the oldest row within `rows`.
    head: usize,
    /// Number of rows currently stored (0..rows.len()).
    count: usize,
}

impl Scrollback {
    /// Create a scrollback holding up to `row_capacity` rows. A capacity of 0
    /// is raised to 1 so the ring math always has somewhere to put a row.
    pub fn new(row_capacity: usize) -> Self {
        let cap = row_capacity.max(1);
        let rows = (0..cap).map(|_| None).collect();
        Scrollback {
            rows,
            head: 0,
            count: 0,
        }
    }

    /// Number of rows currently held.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns true if no rows are stored.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Maximum number of rows the ring can hold.
    pub fn capacity(&self) -> usize {
        self.rows.len()
    }

    /// Copy `row` into the ring, trimming trailing blank cells. When the ring
    /// is full the oldest row is evicted. A push silently no-ops if `row` is
    /// empty and all blank (the allocation is still performed for non-blank rows).
    pub fn push(&mut self, row: &[Cell]) {
        let used = trimmed_len(row);
        let copy: Vec<Cell> = row[..used].to_vec();

        let slot = (self.head + self.count) % self.rows.len();
        if self.count == self.rows.len() {
            // Ring full: evict the oldest row (it sits at `slot`).
            self.rows[slot] = None;
            self.head = (self.head + 1) % self.rows.len();
        } else {
            self.count += 1;
        }
        self.rows[slot] = Some(copy);
    }

    /// Borrow row `index` counting from the oldest (0) to the newest
    /// (`len() - 1`). The slice may be shorter than the grid width — callers
    /// pad with blanks when rendering. Returns an empty slice if out of range.
    pub fn get(&self, index_from_oldest: usize) -> &[Cell] {
        if index_from_oldest >= self.count {
            return &[];
        }
        let slot = (self.head + index_from_oldest) % self.rows.len();
        match &self.rows[slot] {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }
}

/// Length of `row` with trailing blank cells removed.
fn trimmed_len(row: &[Cell]) -> usize {
    let mut n = row.len();
    while n > 0 && row[n - 1].is_blank() {
        n -= 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Cell;

    /// Build a row of `width` cells whose first `text.len()` cells carry the
    /// ASCII characters from `text`.
    fn make_row(width: usize, text: &[u8]) -> Vec<Cell> {
        let mut row = vec![Cell::default(); width];
        for (i, &b) in text.iter().enumerate() {
            if i < width {
                row[i].cp = b as char;
            }
        }
        row
    }

    #[test]
    fn push_then_get_round_trips_trimmed_content() {
        let mut sb = Scrollback::new(8);
        sb.push(&make_row(80, b"hello"));
        assert_eq!(sb.len(), 1);
        let row = sb.get(0);
        // Trailing blanks dropped: the row shrank to its 5 used cells.
        assert_eq!(row.len(), 5);
        assert_eq!(row[0].cp, 'h');
        assert_eq!(row[4].cp, 'o');
    }

    #[test]
    fn an_all_blank_row_trims_to_length_zero() {
        let mut sb = Scrollback::new(4);
        sb.push(&make_row(40, b""));
        assert_eq!(sb.len(), 1);
        assert_eq!(sb.get(0).len(), 0);
    }

    #[test]
    fn ring_evicts_the_oldest_row_at_capacity() {
        let mut sb = Scrollback::new(3);
        sb.push(&make_row(10, b"A"));
        sb.push(&make_row(10, b"B"));
        sb.push(&make_row(10, b"C"));
        assert_eq!(sb.len(), 3);
        assert_eq!(sb.get(0)[0].cp, 'A');

        // Pushing a fourth row evicts "A"; the window slides forward.
        sb.push(&make_row(10, b"D"));
        assert_eq!(sb.len(), 3);
        assert_eq!(sb.get(0)[0].cp, 'B');
        assert_eq!(sb.get(1)[0].cp, 'C');
        assert_eq!(sb.get(2)[0].cp, 'D');
    }

    #[test]
    fn get_out_of_range_returns_an_empty_slice() {
        let mut sb = Scrollback::new(4);
        assert_eq!(sb.get(0).len(), 0);

        sb.push(&make_row(10, b"x"));
        assert_eq!(sb.get(1).len(), 0);
        assert_eq!(sb.get(99).len(), 0);
    }

    #[test]
    fn many_pushes_past_capacity_keep_memory_bounded_and_ordering_correct() {
        let mut sb = Scrollback::new(5);
        for i in 0u8..100 {
            let cell = Cell {
                cp: i as char,
                ..Cell::default()
            };
            sb.push(&[cell]);
        }
        assert_eq!(sb.len(), 5);
        // The retained window is the last five values: 95..99.
        assert_eq!(sb.get(0)[0].cp, 95u8 as char);
        assert_eq!(sb.get(4)[0].cp, 99u8 as char);
    }

    #[test]
    fn default_capacity_constant_is_the_deep_value() {
        assert_eq!(DEFAULT_CAPACITY, 100_000);
    }

    #[test]
    fn zero_capacity_is_raised_to_one() {
        let mut sb = Scrollback::new(0);
        assert_eq!(sb.capacity(), 1);

        sb.push(&make_row(4, b"z"));
        assert_eq!(sb.len(), 1);
        assert_eq!(sb.get(0)[0].cp, 'z');
    }
}
