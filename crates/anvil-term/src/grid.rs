//! The active terminal screen: a fixed `cols × rows` cell matrix plus the
//! cursor, a scroll region, an SGR "pen" template, and mode flags.
//!
//! The grid knows nothing about parsing or scrollback — it exposes primitive
//! editing operations (print, line feed, erase, insert/delete, scroll) and
//! the `Terminal` composes them. `line_feed` returns the row that scrolled off
//! the top of the scroll region so the caller can archive it.

use crate::cell::{Cell, Color};

/// Tab stops sit every 8 columns, the conventional terminal default.
const TAB_WIDTH: usize = 8;

/// The mode flags a grid tracks. Higher-level DEC private modes live on the
/// `Terminal`; these are the ones that change cell-writing behavior.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Modes {
    pub autowrap: bool,
    pub origin: bool,
    pub cursor_visible: bool,
    pub insert: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            autowrap: true,
            origin: false,
            cursor_visible: true,
            insert: false,
        }
    }
}

/// An inclusive vertical scroll region. Defaults to the whole screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ScrollRegion {
    pub top: usize,
    pub bottom: usize,
}

/// The active terminal grid.
pub struct Grid {
    pub width: usize,
    pub height: usize,
    /// Row-major cell matrix, `width * height` cells.
    cells: Vec<Cell>,
    /// A stable copy of the most recently scrolled-off top row. `scroll_up`
    /// fills this before mutating `cells`, so the slice it returns stays
    /// valid for the caller to archive into scrollback.
    scrolled_off: Vec<Cell>,

    pub cur_x: usize,
    pub cur_y: usize,
    /// True once the cursor "should have wrapped" but autowrap is deferred —
    /// the next printable character wraps first. This is the standard
    /// pending-wrap (a.k.a. last-column) latch.
    pub wrap_pending: bool,

    pub saved_x: usize,
    pub saved_y: usize,
    pub saved_pen: Cell,

    pub region: ScrollRegion,
    pub modes: Modes,

    /// The SGR pen: a template cell whose color/attrs are stamped onto every
    /// printed character. `cp` is ignored.
    pub pen: Cell,

    /// Per-row dirty bitmap. `dirty[y]` is true if row `y` has been mutated
    /// since the last call to `take_dirty`. Always `height` elements long.
    dirty: Vec<bool>,
    /// When true the entire grid is dirty; `dirty` bitmap is ignored.
    dirty_all: bool,
}

impl Grid {
    /// Allocate a `cols × rows` grid of blank cells.
    pub fn new(cols: usize, rows: usize) -> Self {
        let w = cols.max(1);
        let h = rows.max(1);
        let cells = vec![Cell::default(); w * h];
        let scrolled_off = vec![Cell::default(); w];
        Grid {
            width: w,
            height: h,
            cells,
            scrolled_off,
            cur_x: 0,
            cur_y: 0,
            wrap_pending: false,
            saved_x: 0,
            saved_y: 0,
            saved_pen: Cell::default(),
            region: ScrollRegion {
                top: 0,
                bottom: h - 1,
            },
            modes: Modes::default(),
            pen: Cell::default(),
            dirty: vec![true; h], // start fully dirty
            dirty_all: true,
        }
    }

    // --- dirty tracking -------------------------------------------------------

    /// Mark row `y` dirty. Out-of-range rows set `dirty_all`.
    pub(crate) fn mark_dirty(&mut self, y: usize) {
        if y < self.height {
            self.dirty[y] = true;
        } else {
            self.dirty_all = true;
        }
    }

    /// Mark every row dirty.
    pub(crate) fn mark_all_dirty(&mut self) {
        self.dirty_all = true;
    }

    /// Drain the dirty state: returns `(dirty_bitmap, all_dirty)`.
    /// After the call every row is clean and `dirty_all` is false.
    pub(crate) fn take_dirty(&mut self) -> (Vec<bool>, bool) {
        let all = self.dirty_all;
        self.dirty_all = false;
        let bitmap = std::mem::replace(&mut self.dirty, vec![false; self.height]);
        (bitmap, all)
    }

    /// Borrow row `y` as a mutable slice of exactly `width` cells.
    pub fn row_mut(&mut self, y: usize) -> &mut [Cell] {
        let start = y * self.width;
        let end = start + self.width;
        &mut self.cells[start..end]
    }

    /// Borrow row `y` immutably.
    pub fn row(&self, y: usize) -> &[Cell] {
        let start = y * self.width;
        &self.cells[start..start + self.width]
    }

    fn cell_mut(&mut self, x: usize, y: usize) -> &mut Cell {
        &mut self.cells[y * self.width + x]
    }

    // --- printing ------------------------------------------------------------

    /// Write a printable scalar at the cursor, honoring autowrap and insert
    /// mode, then advance the cursor.
    pub fn print(&mut self, cp: char) {
        if self.wrap_pending && self.modes.autowrap {
            self.wrap_pending = false;
            self.carriage_return();
            self.line_feed_internal();
        }

        if self.modes.insert {
            self.shift_row_right(self.cur_y, self.cur_x, 1);
        }

        let mut written = self.pen;
        written.cp = cp;
        *self.cell_mut(self.cur_x, self.cur_y) = written;
        self.mark_dirty(self.cur_y);

        if self.cur_x + 1 >= self.width {
            // At the last column: latch a pending wrap rather than moving.
            self.wrap_pending = true;
        } else {
            self.cur_x += 1;
        }
    }

    // --- cursor motion -------------------------------------------------------

    pub fn carriage_return(&mut self) {
        self.cur_x = 0;
        self.wrap_pending = false;
    }

    /// Move down one line, scrolling the region when at its bottom. Returns
    /// a reference to the row that scrolled off the top, or `None` when no
    /// scroll occurred.
    pub fn line_feed(&mut self) -> Option<&[Cell]> {
        self.line_feed_internal()
    }

    fn line_feed_internal(&mut self) -> Option<&[Cell]> {
        self.wrap_pending = false;
        if self.cur_y == self.region.bottom {
            return self.scroll_up(1);
        }
        if self.cur_y + 1 < self.height {
            self.cur_y += 1;
        }
        None
    }

    pub fn backspace(&mut self) {
        self.wrap_pending = false;
        if self.cur_x > 0 {
            self.cur_x -= 1;
        }
    }

    /// Advance to the next 8-column tab stop, clamped to the last column.
    pub fn tab(&mut self) {
        self.wrap_pending = false;
        let next = ((self.cur_x / TAB_WIDTH) + 1) * TAB_WIDTH;
        self.cur_x = next.min(self.width - 1);
    }

    pub fn cursor_up(&mut self, n: usize) {
        self.wrap_pending = false;
        let limit = self.cursor_top_limit();
        let step = n.max(1);
        self.cur_y = if self.cur_y >= limit + step {
            self.cur_y - step
        } else {
            limit
        };
    }

    pub fn cursor_down(&mut self, n: usize) {
        self.wrap_pending = false;
        let limit = self.cursor_bottom_limit();
        let step = n.max(1);
        self.cur_y = (self.cur_y + step).min(limit);
    }

    pub fn cursor_forward(&mut self, n: usize) {
        self.wrap_pending = false;
        let step = n.max(1);
        self.cur_x = (self.cur_x + step).min(self.width - 1);
    }

    pub fn cursor_back(&mut self, n: usize) {
        self.wrap_pending = false;
        let step = n.max(1);
        self.cur_x = self.cur_x.saturating_sub(step);
    }

    /// Absolute cursor move. With origin mode on, `y` is relative to the
    /// scroll region top. Coordinates are clamped to the screen.
    pub fn cursor_to(&mut self, x: usize, y: usize) {
        self.wrap_pending = false;
        self.cur_x = x.min(self.width - 1);
        if self.modes.origin {
            let absolute = self.region.top + y;
            self.cur_y = absolute.min(self.region.bottom);
        } else {
            self.cur_y = y.min(self.height - 1);
        }
    }

    pub fn cursor_to_column(&mut self, x: usize) {
        self.wrap_pending = false;
        self.cur_x = x.min(self.width - 1);
    }

    pub fn cursor_to_row(&mut self, y: usize) {
        self.wrap_pending = false;
        if self.modes.origin {
            self.cur_y = (self.region.top + y).min(self.region.bottom);
        } else {
            self.cur_y = y.min(self.height - 1);
        }
    }

    fn cursor_top_limit(&self) -> usize {
        if self.modes.origin {
            self.region.top
        } else {
            0
        }
    }

    fn cursor_bottom_limit(&self) -> usize {
        if self.modes.origin {
            self.region.bottom
        } else {
            self.height - 1
        }
    }

    pub fn save_cursor(&mut self) {
        self.saved_x = self.cur_x;
        self.saved_y = self.cur_y;
        self.saved_pen = self.pen;
    }

    pub fn restore_cursor(&mut self) {
        self.cur_x = self.saved_x.min(self.width - 1);
        self.cur_y = self.saved_y.min(self.height - 1);
        self.pen = self.saved_pen;
        self.wrap_pending = false;
    }

    // --- erasing -------------------------------------------------------------

    /// Erase Display (ED). `mode` 0 = cursor to end, 1 = start to cursor,
    /// 2/3 = whole screen.
    pub fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_line(0);
                let cur_y = self.cur_y;
                for y in cur_y + 1..self.height {
                    self.blank_row(y);
                }
            }
            1 => {
                let cur_y = self.cur_y;
                for y in 0..cur_y {
                    self.blank_row(y);
                }
                self.erase_line(1);
            }
            _ => {
                self.mark_all_dirty();
                for y in 0..self.height {
                    self.blank_row(y);
                }
            }
        }
    }

    /// Erase in Line (EL). `mode` 0 = cursor to end, 1 = start to cursor,
    /// 2 = whole line.
    pub fn erase_line(&mut self, mode: u16) {
        let cur_x = self.cur_x;
        let cur_y = self.cur_y;
        let width = self.width;
        let pen_bg = self.pen.bg;
        let r = self.row_mut(cur_y);
        match mode {
            0 => blank_cells_with_bg(&mut r[cur_x..], pen_bg),
            1 => blank_cells_with_bg(&mut r[..cur_x.saturating_add(1).min(width)], pen_bg),
            _ => blank_cells_with_bg(r, pen_bg),
        }
        self.mark_dirty(cur_y);
    }

    /// Erase Character (ECH): blank `n` cells from the cursor without moving.
    pub fn erase_chars(&mut self, n: usize) {
        let count = n.max(1);
        let cur_x = self.cur_x;
        let cur_y = self.cur_y;
        let width = self.width;
        let pen_bg = self.pen.bg;
        let r = self.row_mut(cur_y);
        let end = (cur_x + count).min(width);
        blank_cells_with_bg(&mut r[cur_x..end], pen_bg);
        self.mark_dirty(cur_y);
    }

    fn blank_row(&mut self, y: usize) {
        let pen_bg = self.pen.bg;
        let r = self.row_mut(y);
        blank_cells_with_bg(r, pen_bg);
        self.mark_dirty(y);
    }

    // --- insert / delete -----------------------------------------------------

    /// Insert Character (ICH): shift the cursor row right by `n`, blanking
    /// the gap. Cells pushed past the right edge are lost.
    pub fn insert_chars(&mut self, n: usize) {
        let cur_x = self.cur_x;
        let cur_y = self.cur_y;
        self.shift_row_right(cur_y, cur_x, n.max(1));
        self.mark_dirty(cur_y);
    }

    /// Delete Character (DCH): shift the cursor row left by `n`, blanking the
    /// vacated tail.
    pub fn delete_chars(&mut self, n: usize) {
        let count = n.max(1);
        let cur_x = self.cur_x;
        let cur_y = self.cur_y;
        let width = self.width;
        let pen_bg = self.pen.bg;
        let src_start = (cur_x + count).min(width);
        let move_len = width - src_start;
        // Shift left within the row.
        let row_start = cur_y * width;
        self.cells.copy_within(
            row_start + src_start..row_start + src_start + move_len,
            row_start + cur_x,
        );
        // Blank the vacated tail.
        let tail_start = row_start + cur_x + move_len;
        blank_cells_with_bg(&mut self.cells[tail_start..row_start + width], pen_bg);
        self.mark_dirty(cur_y);
    }

    /// Insert `n` blank lines at the cursor row, pushing lower lines down
    /// within the scroll region. No effect outside the region.
    pub fn insert_lines(&mut self, n: usize) {
        if self.cur_y < self.region.top || self.cur_y > self.region.bottom {
            return;
        }
        let count = n.max(1).min(self.region.bottom - self.cur_y + 1);
        let width = self.width;
        // Shift rows down within region (from bottom up).
        let mut y = self.region.bottom;
        loop {
            if y < self.cur_y + count {
                break;
            }
            let src_start = (y - count) * width;
            let dst_start = y * width;
            self.cells
                .copy_within(src_start..src_start + width, dst_start);
            self.mark_dirty(y);
            if y == self.cur_y + count {
                break;
            }
            y -= 1;
        }
        // Blank the inserted rows.
        let pen_bg = self.pen.bg;
        for blank in self.cur_y..self.cur_y + count {
            let row_start = blank * width;
            blank_cells_with_bg(&mut self.cells[row_start..row_start + width], pen_bg);
            self.mark_dirty(blank);
        }
    }

    /// Delete `n` lines at the cursor row, pulling lower lines up within the
    /// scroll region. No effect outside the region.
    pub fn delete_lines(&mut self, n: usize) {
        if self.cur_y < self.region.top || self.cur_y > self.region.bottom {
            return;
        }
        let count = n.max(1).min(self.region.bottom - self.cur_y + 1);
        let width = self.width;
        let mut y = self.cur_y;
        while y + count <= self.region.bottom {
            let src_start = (y + count) * width;
            let dst_start = y * width;
            self.cells
                .copy_within(src_start..src_start + width, dst_start);
            self.mark_dirty(y);
            y += 1;
        }
        let pen_bg = self.pen.bg;
        while y <= self.region.bottom {
            let row_start = y * width;
            blank_cells_with_bg(&mut self.cells[row_start..row_start + width], pen_bg);
            self.mark_dirty(y);
            y += 1;
        }
    }

    // --- region scrolling ----------------------------------------------------

    /// Scroll the region up by `n` lines (SU). Returns a reference to the
    /// grid-owned `scrolled_off` buffer containing a copy of the first line
    /// that scrolled off the top. Valid until the next `scroll_up` call.
    /// Returns `None` only when n == 0 (which never happens in practice).
    pub fn scroll_up(&mut self, n: usize) -> Option<&[Cell]> {
        let span = self.region.bottom - self.region.top + 1;
        let count = n.max(1).min(span);
        let width = self.width;

        // Snapshot the top row before the copy loop overwrites it.
        let top_start = self.region.top * width;
        self.scrolled_off
            .copy_from_slice(&self.cells[top_start..top_start + width]);

        // All visible rows shift: mark the whole region dirty.
        self.mark_all_dirty();

        // Shift rows up.
        let mut y = self.region.top;
        while y + count <= self.region.bottom {
            let src_start = (y + count) * width;
            let dst_start = y * width;
            self.cells
                .copy_within(src_start..src_start + width, dst_start);
            y += 1;
        }
        // Blank the vacated bottom rows.
        let pen_bg = self.pen.bg;
        while y <= self.region.bottom {
            let row_start = y * width;
            blank_cells_with_bg(&mut self.cells[row_start..row_start + width], pen_bg);
            y += 1;
        }
        Some(&self.scrolled_off)
    }

    /// Scroll the region down by `n` lines (SD).
    pub fn scroll_down(&mut self, n: usize) {
        let span = self.region.bottom - self.region.top + 1;
        let count = n.max(1).min(span);
        let width = self.width;
        let pen_bg = self.pen.bg;

        // All visible rows shift: mark the whole region dirty.
        self.mark_all_dirty();

        let mut y = self.region.bottom;
        loop {
            if y < self.region.top + count {
                break;
            }
            let src_start = (y - count) * width;
            let dst_start = y * width;
            self.cells
                .copy_within(src_start..src_start + width, dst_start);
            if y == self.region.top + count {
                break;
            }
            y -= 1;
        }
        for blank in self.region.top..self.region.top + count {
            let row_start = blank * width;
            blank_cells_with_bg(&mut self.cells[row_start..row_start + width], pen_bg);
        }
    }

    /// Set the DECSTBM scroll region (1-based, inclusive). An invalid or
    /// empty range resets to the whole screen. The cursor homes afterward.
    pub fn set_scroll_region(&mut self, top_1based: usize, bottom_1based: usize) {
        let top = if top_1based == 0 { 0 } else { top_1based - 1 };
        let bottom = if bottom_1based == 0 {
            self.height - 1
        } else {
            bottom_1based - 1
        };
        if top >= bottom || bottom >= self.height {
            self.region = ScrollRegion {
                top: 0,
                bottom: self.height - 1,
            };
        } else {
            self.region = ScrollRegion { top, bottom };
        }
        self.cursor_to(0, 0);
    }

    // --- helpers -------------------------------------------------------------

    /// Shift row `y` right by `n` starting at column `from`, blanking the gap.
    fn shift_row_right(&mut self, y: usize, from: usize, n: usize) {
        if from >= self.width {
            return;
        }
        let count = n.min(self.width - from);
        let width = self.width;
        let pen_bg = self.pen.bg;
        let row_start = y * width;
        // Shift existing cells right (from the end toward `from + count`).
        let mut i = width;
        while i > from + count {
            i -= 1;
            self.cells[row_start + i] = self.cells[row_start + i - count];
        }
        blank_cells_with_bg(
            &mut self.cells[row_start + from..row_start + from + count],
            pen_bg,
        );
    }

    // --- resize --------------------------------------------------------------

    /// Resize to `cols × rows`, preserving overlapping content from the top
    /// left. The cursor and scroll region are clamped to the new bounds.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let w = cols.max(1);
        let h = rows.max(1);
        if w == self.width && h == self.height {
            return;
        }

        let mut fresh = vec![Cell::default(); w * h];
        let fresh_scratch = vec![Cell::default(); w];

        let copy_h = h.min(self.height);
        let copy_w = w.min(self.width);
        for y in 0..copy_h {
            let src_start = y * self.width;
            let dst_start = y * w;
            fresh[dst_start..dst_start + copy_w]
                .copy_from_slice(&self.cells[src_start..src_start + copy_w]);
        }

        self.cells = fresh;
        self.scrolled_off = fresh_scratch;
        self.width = w;
        self.height = h;
        self.region = ScrollRegion {
            top: 0,
            bottom: h - 1,
        };
        self.cur_x = self.cur_x.min(w - 1);
        self.cur_y = self.cur_y.min(h - 1);
        self.wrap_pending = false;
        // After resize the whole visible area must be redrawn.
        self.dirty = vec![true; h];
        self.dirty_all = true;
    }

    /// The current `scrolled_off` buffer length — equals `width` after any
    /// scroll_up or after construction.
    pub fn scrolled_off_len(&self) -> usize {
        self.scrolled_off.len()
    }
}

/// Reset every cell in `slice` to a blank carrying `bg_color` — so an erase
/// after `CSI 44m` paints a blue field.
fn blank_cells_with_bg(slice: &mut [Cell], bg_color: Color) {
    let blank = Cell {
        bg: bg_color,
        ..Cell::default()
    };
    slice.fill(blank);
}

// --- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::{Attrs, Cell, Color};

    /// Collect the characters of row `y` into a `String`.
    fn row_text(g: &Grid, y: usize) -> String {
        g.row(y).iter().map(|c| c.cp).collect()
    }

    #[test]
    fn print_advances_the_cursor_and_stamps_the_pen() {
        let mut g = Grid::new(10, 3);
        g.pen.fg = Color::Palette(2);
        g.print('h');
        g.print('i');
        assert_eq!(g.cur_x, 2);
        assert_eq!(g.row(0)[0].cp, 'h');
        assert_eq!(g.row(0)[0].fg, Color::Palette(2));
    }

    #[test]
    fn autowrap_latches_at_the_last_column_and_wraps_on_next_print() {
        let mut g = Grid::new(4, 3);
        for ch in "abcd".chars() {
            g.print(ch);
        }
        // After 4 prints in a 4-wide grid the wrap is pending, not yet applied.
        assert!(g.wrap_pending);
        assert_eq!(g.cur_y, 0);
        g.print('e');
        assert_eq!(g.cur_y, 1);
        assert_eq!(g.cur_x, 1);
        assert_eq!(g.row(1)[0].cp, 'e');
    }

    #[test]
    fn autowrap_disabled_overwrites_the_last_column() {
        let mut g = Grid::new(4, 3);
        g.modes.autowrap = false;
        for ch in "abcd".chars() {
            g.print(ch);
        }
        g.print('X');
        assert_eq!(g.cur_y, 0);
        assert_eq!(g.row(0)[3].cp, 'X');
    }

    #[test]
    fn carriage_return_and_line_feed() {
        let mut g = Grid::new(8, 4);
        g.print('a');
        g.carriage_return();
        assert_eq!(g.cur_x, 0);
        let _ = g.line_feed();
        assert_eq!(g.cur_y, 1);
    }

    #[test]
    fn line_feed_at_region_bottom_scrolls_and_returns_the_lost_row() {
        let mut g = Grid::new(8, 3);
        for ch in "top".chars() {
            g.print(ch);
        } // row 0
        g.cursor_to(0, 1);
        for ch in "mid".chars() {
            g.print(ch);
        } // row 1
        g.cursor_to(0, 2); // bottom row

        let scrolled = g.line_feed();
        // The line feed scrolled the whole screen up; "top" fell off.
        assert!(scrolled.is_some());
        let s = scrolled.unwrap();
        let text: String = s.iter().map(|c| c.cp).collect();
        assert_eq!(&text, "top     ");
        assert_eq!(&row_text(&g, 0), "mid     ");
        assert_eq!(g.cur_y, 2);
    }

    #[test]
    fn cursor_moves_clamp_at_screen_bounds() {
        let mut g = Grid::new(10, 5);
        g.cursor_up(99);
        assert_eq!(g.cur_y, 0);
        g.cursor_down(99);
        assert_eq!(g.cur_y, 4);
        g.cursor_forward(99);
        assert_eq!(g.cur_x, 9);
        g.cursor_back(99);
        assert_eq!(g.cur_x, 0);
    }

    #[test]
    fn cursor_to_positions_absolutely_and_clamps() {
        let mut g = Grid::new(10, 5);
        g.cursor_to(3, 2);
        assert_eq!(g.cur_x, 3);
        assert_eq!(g.cur_y, 2);
        g.cursor_to(99, 99);
        assert_eq!(g.cur_x, 9);
        assert_eq!(g.cur_y, 4);
    }

    #[test]
    fn tab_advances_to_8_column_stops() {
        let mut g = Grid::new(30, 2);
        g.tab();
        assert_eq!(g.cur_x, 8);
        g.cursor_to_column(10);
        g.tab();
        assert_eq!(g.cur_x, 16);
    }

    #[test]
    fn erase_line_variants() {
        let mut g = Grid::new(6, 2);
        for ch in "abcdef".chars() {
            g.print(ch);
        }
        g.cursor_to_column(3);
        g.erase_line(0); // cursor to end
        assert_eq!(&row_text(&g, 0), "abc   ");

        for (i, ch) in "ABCDEF".chars().enumerate() {
            g.row_mut(0)[i].cp = ch;
        }
        g.cursor_to_column(2);
        g.erase_line(1); // start to cursor inclusive
        assert_eq!(&row_text(&g, 0), "   DEF");
    }

    #[test]
    fn erase_display_clears_below_the_cursor() {
        let mut g = Grid::new(4, 3);
        for y in 0..3 {
            for x in 0..4 {
                g.row_mut(y)[x].cp = 'x';
            }
        }
        g.cursor_to(2, 1);
        g.erase_display(0);
        assert_eq!(&row_text(&g, 0), "xxxx");
        assert_eq!(&row_text(&g, 1), "xx  ");
        assert_eq!(&row_text(&g, 2), "    ");
    }

    #[test]
    fn insert_and_delete_characters() {
        let mut g = Grid::new(6, 2);
        for ch in "abcdef".chars() {
            g.print(ch);
        }
        g.cursor_to_column(2);
        g.insert_chars(2);
        assert_eq!(&row_text(&g, 0), "ab  cd");

        for (i, ch) in "abcdef".chars().enumerate() {
            g.row_mut(0)[i].cp = ch;
        }
        g.cursor_to_column(1);
        g.delete_chars(2);
        assert_eq!(&row_text(&g, 0), "adef  ");
    }

    #[test]
    fn insert_and_delete_lines_within_the_scroll_region() {
        let mut g = Grid::new(4, 4);
        for y in 0..4 {
            let ch = char::from_u32('1' as u32 + y as u32).unwrap();
            for x in 0..4 {
                g.row_mut(y)[x].cp = ch;
            }
        }
        g.cursor_to(0, 1);
        g.insert_lines(1);
        assert_eq!(&row_text(&g, 0), "1111");
        assert_eq!(&row_text(&g, 1), "    ");
        assert_eq!(&row_text(&g, 2), "2222");

        for y in 0..4 {
            let ch = char::from_u32('1' as u32 + y as u32).unwrap();
            for x in 0..4 {
                g.row_mut(y)[x].cp = ch;
            }
        }
        g.cursor_to(0, 1);
        g.delete_lines(1);
        assert_eq!(&row_text(&g, 0), "1111");
        assert_eq!(&row_text(&g, 1), "3333");
    }

    #[test]
    fn scroll_region_restricts_line_feed() {
        let mut g = Grid::new(4, 5);
        for y in 0..5 {
            let ch = char::from_u32('1' as u32 + y as u32).unwrap();
            for x in 0..4 {
                g.row_mut(y)[x].cp = ch;
            }
        }
        g.set_scroll_region(2, 4); // rows index 1..3
        g.cursor_to(0, 3); // bottom of region
        let _ = g.line_feed();
        // Row 0 untouched, region scrolled, row 4 untouched.
        assert_eq!(&row_text(&g, 0), "1111");
        assert_eq!(&row_text(&g, 1), "3333");
        assert_eq!(&row_text(&g, 2), "4444");
        assert_eq!(&row_text(&g, 3), "    ");
        assert_eq!(&row_text(&g, 4), "5555");
    }

    #[test]
    fn resize_preserves_top_left_content_and_clamps_cursor() {
        let mut g = Grid::new(6, 3);
        for ch in "hello".chars() {
            g.print(ch);
        }
        g.cursor_to(5, 2);
        g.resize(3, 2);
        assert_eq!(g.width, 3);
        assert_eq!(g.height, 2);
        assert_eq!(g.cur_x, 2);
        assert_eq!(g.cur_y, 1);
        assert_eq!(&row_text(&g, 0), "hel");
    }

    #[test]
    fn save_and_restore_cursor_round_trips_position_and_pen() {
        let mut g = Grid::new(10, 4);
        g.cursor_to(4, 2);
        g.pen.attrs |= Attrs::BOLD;
        g.save_cursor();
        g.cursor_to(0, 0);
        g.pen.attrs.remove(Attrs::BOLD);
        g.restore_cursor();
        assert_eq!(g.cur_x, 4);
        assert_eq!(g.cur_y, 2);
        assert!(g.pen.attrs.contains(Attrs::BOLD));
    }

    #[test]
    fn erase_paints_the_pen_background() {
        let mut g = Grid::new(4, 2);
        g.pen.bg = Color::Palette(4);
        g.erase_line(2);
        assert_eq!(g.row(0)[0].bg, Color::Palette(4));
    }

    // --- Grid-level resize matrix (Bug A regression) -------------------------

    /// Verify grid-level invariants after every resize case.
    /// I-G1: cursor in bounds; I-G2: wrap_pending cleared;
    /// I-G3: region reset (when dimensions changed);
    /// I-G4: scrolled_off.len == width; I-G5: top-left content preserved.
    fn verify_grid(
        g: &Grid,
        prev_cells: &[Cell],
        prev_w: usize,
        prev_h: usize,
        dims_changed: bool,
    ) {
        // I-G1 cursor in bounds
        assert!(
            g.cur_x < g.width,
            "I-G1: cur_x {} >= width {}",
            g.cur_x,
            g.width
        );
        assert!(
            g.cur_y < g.height,
            "I-G1: cur_y {} >= height {}",
            g.cur_y,
            g.height
        );
        // I-G2 wrap_pending cleared (only when resize actually ran)
        if dims_changed {
            assert!(!g.wrap_pending, "I-G2: wrap_pending should be cleared");
        }
        // I-G3 region reset (only when resize actually ran)
        if dims_changed {
            assert_eq!(g.region.top, 0, "I-G3: region.top");
            assert_eq!(g.region.bottom, g.height - 1, "I-G3: region.bottom");
        }
        // I-G4 scrolled_off width
        assert_eq!(g.scrolled_off_len(), g.width, "I-G4: scrolled_off.len");
        // I-G5 top-left content preserved in the overlap rect
        let copy_w = g.width.min(prev_w);
        let copy_h = g.height.min(prev_h);
        for y in 0..copy_h {
            for x in 0..copy_w {
                let got = g.row(y)[x];
                let exp = prev_cells[y * prev_w + x];
                assert_eq!(
                    got.cp, exp.cp,
                    "I-G5: cell [{x},{y}] got {:?} expected {:?}",
                    got.cp, exp.cp
                );
            }
        }
    }

    struct GridResizeCase {
        name: &'static str,
        w1: usize,
        h1: usize,
        w2: usize,
        h2: usize,
        feed: &'static str,
    }

    const RESIZE_CASES: &[GridResizeCase] = &[
        GridResizeCase {
            name: "grow both",
            w1: 4,
            h1: 3,
            w2: 8,
            h2: 6,
            feed: "abc",
        },
        GridResizeCase {
            name: "shrink both",
            w1: 8,
            h1: 6,
            w2: 4,
            h2: 3,
            feed: "hello",
        },
        GridResizeCase {
            name: "grow cols only",
            w1: 4,
            h1: 3,
            w2: 8,
            h2: 3,
            feed: "hi",
        },
        GridResizeCase {
            name: "shrink rows only",
            w1: 4,
            h1: 6,
            w2: 4,
            h2: 3,
            feed: "xy",
        },
        GridResizeCase {
            name: "degenerate 1x1",
            w1: 8,
            h1: 4,
            w2: 1,
            h2: 1,
            feed: "A",
        },
        GridResizeCase {
            name: "degenerate 0x0 (clamped to 1x1)",
            w1: 8,
            h1: 4,
            w2: 0,
            h2: 0,
            feed: "",
        },
        GridResizeCase {
            name: "no-op resize",
            w1: 4,
            h1: 3,
            w2: 4,
            h2: 3,
            feed: "test",
        },
        GridResizeCase {
            name: "grow then shrink round trip",
            w1: 4,
            h1: 3,
            w2: 6,
            h2: 5,
            feed: "abc",
        },
        GridResizeCase {
            name: "resize twice no feed",
            w1: 4,
            h1: 4,
            w2: 2,
            h2: 2,
            feed: "",
        },
        GridResizeCase {
            name: "cursor at bottom-right then shrink",
            w1: 6,
            h1: 4,
            w2: 3,
            h2: 2,
            feed: "",
        },
    ];

    // ── insert mode shifts cells right on print (line 128) ───────────────────

    #[test]
    fn insert_mode_shifts_existing_cells_right_on_print() {
        let mut g = Grid::new(5, 2);
        g.print('A');
        g.print('B');
        g.print('C');
        g.cursor_to(0, 0);
        g.modes.insert = true;
        g.print('X');
        // 'X' inserted at col 0; 'A','B','C' shift right.
        assert_eq!(g.row(0)[0].cp, 'X');
        assert_eq!(g.row(0)[1].cp, 'A');
        assert_eq!(g.row(0)[2].cp, 'B');
    }

    // ── cursor_to with origin mode (lines 218-219) ────────────────────────────

    #[test]
    fn cursor_to_with_origin_mode_is_relative_to_region_top() {
        let mut g = Grid::new(10, 10);
        g.region = ScrollRegion { top: 3, bottom: 7 };
        g.modes.origin = true;
        // y=0 with origin mode → absolute row 3.
        g.cursor_to(0, 0);
        assert_eq!(3, g.cur_y);
        // y=4 → absolute 3+4=7 (clamped to region.bottom=7).
        g.cursor_to(0, 4);
        assert_eq!(7, g.cur_y);
        // y=10 → 3+10=13, clamped to 7.
        g.cursor_to(0, 10);
        assert_eq!(7, g.cur_y);
    }

    // ── cursor_to_row with origin mode (line 233) ─────────────────────────────

    #[test]
    fn cursor_to_row_with_origin_mode_is_relative_to_region_top() {
        let mut g = Grid::new(10, 10);
        g.region = ScrollRegion { top: 2, bottom: 8 };
        g.modes.origin = true;
        g.cursor_to_row(1);
        assert_eq!(3, g.cur_y); // 2 + 1
        g.cursor_to_row(6);
        assert_eq!(8, g.cur_y); // 2 + 6 = 8 (= bottom)
        g.cursor_to_row(99);
        assert_eq!(8, g.cur_y); // clamped to bottom
    }

    // ── cursor_top_limit and cursor_bottom_limit with origin mode (241, 249) ──

    #[test]
    fn cursor_limits_respect_origin_mode() {
        let mut g = Grid::new(10, 10);
        g.region = ScrollRegion { top: 2, bottom: 7 };

        // Without origin mode: cursor_up stops at row 0.
        g.modes.origin = false;
        g.cur_y = 5;
        g.cursor_up(10);
        assert_eq!(0, g.cur_y);

        // With origin mode: cursor_up stops at region.top (2).
        g.modes.origin = true;
        g.cur_y = 5;
        g.cursor_up(10);
        assert_eq!(2, g.cur_y);

        // cursor_down uses cursor_bottom_limit: should stop at region.bottom (7).
        g.cur_y = 5;
        g.cursor_down(10);
        assert_eq!(7, g.cur_y);
    }

    // ── erase_display mode 1 (erase above, lines 282-286) ────────────────────

    #[test]
    fn erase_display_mode_1_erases_above_cursor() {
        let mut g = Grid::new(5, 3);
        g.print('A');
        g.print('B');
        g.cursor_to(0, 1);
        g.print('C'); // col 0, row 1
        g.print('D'); // col 1, row 1
        g.print('E'); // col 2, row 1
        g.cursor_to(1, 1); // cursor at col 1, row 1
        // Erase from top to cursor inclusive (mode 1).
        g.erase_display(1);
        // Row 0 fully erased (was in rows 0..cur_y).
        assert_eq!(' ', g.row(0)[0].cp);
        assert_eq!(' ', g.row(0)[1].cp);
        // Row 1: cols 0..=cur_x erased (erase_line(1) with cur_x=1).
        assert_eq!(' ', g.row(1)[0].cp);
        assert_eq!(' ', g.row(1)[1].cp);
        // Col 2 is beyond cursor — untouched.
        assert_eq!('E', g.row(1)[2].cp);
    }

    // ── insert_lines outside region is no-op (line 364) ──────────────────────

    #[test]
    fn insert_lines_outside_scroll_region_is_noop() {
        let mut g = Grid::new(5, 6);
        g.region = ScrollRegion { top: 2, bottom: 4 };
        // Print something at row 0 (outside region.top=2).
        g.cursor_to(0, 0);
        g.print('X');
        // insert_lines with cursor outside region must not move anything.
        g.insert_lines(1);
        assert_eq!('X', g.row(0)[0].cp);
    }

    // ── delete_lines outside region is no-op (line 395) ──────────────────────

    #[test]
    fn delete_lines_outside_scroll_region_is_noop() {
        let mut g = Grid::new(5, 6);
        g.region = ScrollRegion { top: 2, bottom: 4 };
        g.cursor_to(0, 5); // row 5 is above bottom (4).
        g.print('Y');
        g.delete_lines(1);
        // Row 5 content unchanged.
        assert_eq!('Y', g.row(5)[0].cp);
    }

    // ── scroll_down early break (line 460) ────────────────────────────────────

    #[test]
    fn scroll_down_by_full_region_span_blanks_all_rows() {
        let mut g = Grid::new(5, 4);
        g.region = ScrollRegion { top: 0, bottom: 3 };
        g.print('A');
        g.cursor_to(0, 1);
        g.print('B');
        // Scroll down by the entire height — all rows become blank.
        g.scroll_down(4);
        assert_eq!(' ', g.row(0)[0].cp);
        assert_eq!(' ', g.row(1)[0].cp);
        assert_eq!(' ', g.row(3)[0].cp);
    }

    // ── set_scroll_region with zero bottom resets to full screen (482-490) ────

    #[test]
    fn set_scroll_region_zero_params_reset_to_full_screen() {
        let mut g = Grid::new(10, 8);
        // Set a tight region first.
        g.set_scroll_region(2, 5);
        assert_eq!(1, g.region.top);
        assert_eq!(4, g.region.bottom);
        // Both zero → reset to full screen.
        g.set_scroll_region(0, 0);
        assert_eq!(0, g.region.top);
        assert_eq!(7, g.region.bottom);
    }

    #[test]
    fn set_scroll_region_invalid_top_gte_bottom_resets_to_full_screen() {
        let mut g = Grid::new(10, 8);
        g.set_scroll_region(5, 3); // top_1based=5 > bottom_1based=3 → invalid
        assert_eq!(0, g.region.top);
        assert_eq!(7, g.region.bottom);
    }

    #[test]
    fn set_scroll_region_bottom_beyond_height_resets_to_full_screen() {
        let mut g = Grid::new(10, 4);
        g.set_scroll_region(1, 99); // bottom >= height → invalid
        assert_eq!(0, g.region.top);
        assert_eq!(3, g.region.bottom);
    }

    #[test]
    fn grid_resize_matrix() {
        for c in RESIZE_CASES {
            let mut g = Grid::new(c.w1, c.h1);

            // Feed characters.
            for ch in c.feed.chars() {
                g.print(ch);
            }

            // Snapshot cells before resize.
            let snap: Vec<Cell> = g.cells.clone();
            let prev_w = g.width;
            let prev_h = g.height;

            // For the "cursor at bottom-right" case, position cursor there.
            if c.name == "cursor at bottom-right then shrink" {
                g.cursor_to(c.w1 - 1, c.h1 - 1);
            }

            let ew2 = c.w2.max(1);
            let eh2 = c.h2.max(1);
            let dims_changed = ew2 != prev_w || eh2 != prev_h;
            g.resize(c.w2, c.h2);

            verify_grid(&g, &snap, prev_w, prev_h, dims_changed);

            // Round-trip: grow back if we shrank.
            if ew2 < prev_w || eh2 < prev_h {
                g.resize(c.w1, c.h1);
                // Only check invariants (content is lost on shrink + regrow).
                assert!(
                    g.cur_x < g.width,
                    "case '{}' round-trip: cur_x out of bounds",
                    c.name
                );
                assert!(
                    g.cur_y < g.height,
                    "case '{}' round-trip: cur_y out of bounds",
                    c.name
                );
            }
        }
    }
}
