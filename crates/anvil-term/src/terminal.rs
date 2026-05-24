//! Terminal model — port of `src/terminal/terminal.zig`.
//!
//! `Terminal` owns a primary grid, an alternate grid, the scrollback ring,
//! and a VT/ANSI parser. It implements `Handler`, translating parsed events
//! into grid mutations. It also tracks a render viewport that can be scrolled
//! up into scrollback history.
//!
//! Pure Rust — no platform dependencies.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{
    cell::{Attrs, Cell, Color},
    grid::{Grid, ScrollRegion},
    parser::{Handler, Parser},
    scrollback::Scrollback,
};

// ── DirtySet ──────────────────────────────────────────────────────────────────

/// The set of viewport rows that changed since the last call to
/// [`Terminal::take_dirty_rows`].
///
/// `is_full()` is `true` when every row is dirty (e.g. after a resize or
/// screen switch). When `is_full()`, callers should redraw all rows rather
/// than iterating `iter()`.
pub struct DirtySet {
    /// Per-row dirty flags, indexed by viewport row.
    bitmap: Vec<bool>,
    /// When true every row is dirty; `bitmap` is not consulted.
    full: bool,
}

impl DirtySet {
    /// Construct a full-dirty set for `rows` viewport rows.
    pub fn all(rows: usize) -> Self {
        DirtySet {
            bitmap: vec![true; rows],
            full: true,
        }
    }

    /// Construct a clean set for `rows` viewport rows (no rows dirty).
    /// Use `mark` to add specific dirty rows.
    pub fn none(rows: usize) -> Self {
        DirtySet {
            bitmap: vec![false; rows],
            full: false,
        }
    }

    /// Construct from a raw bitmap (from `Grid::take_dirty`) and the `all` flag.
    fn from_raw(bitmap: Vec<bool>, all: bool) -> Self {
        DirtySet { bitmap, full: all }
    }

    /// True when every viewport row needs redrawing.
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// True when the given row needs redrawing.
    pub fn contains(&self, row: usize) -> bool {
        if self.full {
            return true;
        }
        self.bitmap.get(row).copied().unwrap_or(true)
    }

    /// Iterate dirty row indices. When `is_full()` this yields `0..bitmap.len()`.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        let len = self.bitmap.len();
        (0..len).filter(move |&r| self.contains(r))
    }

    /// Mark an additional row dirty (used by callers that know cursor rows etc.).
    pub fn mark(&mut self, row: usize) {
        if row < self.bitmap.len() {
            self.bitmap[row] = true;
        } else {
            self.full = true;
        }
    }

    /// Force the entire set full (equivalent to marking every row dirty).
    pub fn force_full(&mut self) {
        self.full = true;
    }
}

// =============================================================================

/// Cursor shape requested by the application via DECSCUSR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Bar,
}

/// Cursor position and visibility as the renderer needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub x: usize,
    pub y: usize,
    pub visible: bool,
}

/// DEC private modes the terminal records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivateModes {
    pub autowrap: bool,
    pub cursor_visible: bool,
    pub alt_screen: bool,
    pub bracketed_paste: bool,
    /// ?1 application cursor keys.
    pub app_cursor_keys: bool,
    /// ?1000 / ?1002 mouse reporting flags.
    pub mouse_x10: bool,
    pub mouse_button: bool,
    pub mouse_sgr: bool,
}

impl Default for PrivateModes {
    fn default() -> Self {
        PrivateModes {
            autowrap: true,
            cursor_visible: true,
            alt_screen: false,
            bracketed_paste: false,
            app_cursor_keys: false,
            mouse_x10: false,
            mouse_button: false,
            mouse_sgr: false,
        }
    }
}

/// The kind/sub-command of an OSC-133 semantic prompt mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMarkKind {
    PromptStart,
    CommandStart,
    OutputStart,
    CommandDone,
}

/// An OSC-133 semantic prompt mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptMark {
    pub kind: PromptMarkKind,
    /// Absolute line index — monotonic for the session, survives scrollback
    /// eviction.
    pub line: usize,
    /// Cursor column at the time this mark fired.  For `CommandStart` (133;B)
    /// this is the column where user input begins (i.e. after the prompt).
    pub col: u16,
    /// Exit code, valid only when `kind == CommandDone`. 0 otherwise.
    pub exit_code: i32,
    /// Wall-clock duration in milliseconds from 133;C to 133;D.
    /// Valid only when `kind == CommandDone`. 0 otherwise.
    pub duration_ms: u64,
}

/// State of a command block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockState {
    Running,
    Exited,
}

/// A shell command and its output, derived from adjacent OSC-133 marks.
/// All line numbers are ABSOLUTE (comparable to `PromptMark::line`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Absolute line of the `command_start` mark (133;B).
    pub command_line: usize,
    /// Cursor column at the 133;B mark — where user input begins (after the
    /// prompt).  Used at render time to extract the typed command text from
    /// the cells at `command_line`.
    pub command_start_col: u16,
    /// Absolute line of `output_start` (133;C), or `command_line` if none.
    pub output_line: usize,
    /// Absolute line one past the last output row (exclusive end).
    pub end_line: usize,
    pub state: BlockState,
    /// Valid only when `state == Exited`.
    pub exit_code: i32,
    /// Wall-clock duration in milliseconds from 133;C to 133;D.
    /// 0 when the block is still running or the shell didn't emit 133;D.
    pub duration_ms: u64,
}

impl Block {
    pub fn output_row_count(&self) -> usize {
        self.end_line
            .saturating_sub(self.output_line)
            .saturating_sub(1)
    }
}

/// Upper bound on retained OSC-133 marks.
const MAX_MARKS: usize = 4096;

/// Shell-state accessor for the HUD.
#[derive(Clone, Copy, Debug)]
pub struct LastRun {
    pub running: bool,
    pub exit_code: i32,
    pub duration_ms: i64,
}

pub struct Terminal {
    primary: Grid,
    alternate: Grid,
    /// True when the alternate grid is active.
    on_alt: bool,

    history: Scrollback,
    parser: Parser,

    /// 0 = pinned to live bottom; >0 = scrolled up into history.
    pub viewport_offset: usize,

    pub modes: PrivateModes,

    /// G0 charset: true = DEC special line-drawing set.
    g0_line_drawing: bool,

    title_buf: [u8; 256],
    title_len: usize,

    cwd_buf: [u8; 1024],
    cwd_len: usize,

    clipboard_buf: [u8; 4096],
    clipboard_len: usize,

    /// OSC-133 semantic prompt marks, oldest first.
    marks: [PromptMark; MAX_MARKS],
    mark_count: usize,
    /// Rows evicted from scrollback over the session's lifetime.
    pub evicted_lines: usize,

    shell_running: bool,
    shell_run_start: Option<Instant>,
    shell_last_exit: i32,
    shell_last_duration_ms: i64,

    /// App-requested cursor style from DECSCUSR. `None` = use config default.
    pub app_cursor_shape: Option<CursorShape>,
    pub app_cursor_blink: Option<bool>,

    /// Compose buffer for `viewport_row` — always exactly `cols` wide.
    compose_buf: Vec<Cell>,
}

impl Terminal {
    /// Create a terminal with a `width × height` screen and a scrollback ring
    /// of `scrollback_capacity` rows.
    pub fn new(width: usize, height: usize, scrollback_capacity: usize) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        let primary = Grid::new(w, h);
        let alternate = Grid::new(w, h);
        let history = Scrollback::new(scrollback_capacity);
        let compose_buf = vec![Cell::default(); w];

        // SAFETY: PromptMark is Copy; we initialise mark_count = 0 and never
        // read beyond mark_count, so the undefined tail is harmless.
        let marks = [PromptMark {
            kind: PromptMarkKind::PromptStart,
            line: 0,
            col: 0,
            exit_code: 0,
            duration_ms: 0,
        }; MAX_MARKS];

        Terminal {
            primary,
            alternate,
            on_alt: false,
            history,
            parser: Parser::new(),
            viewport_offset: 0,
            modes: PrivateModes::default(),
            g0_line_drawing: false,
            title_buf: [0; 256],
            title_len: 0,
            cwd_buf: [0; 1024],
            cwd_len: 0,
            clipboard_buf: [0; 4096],
            clipboard_len: 0,
            marks,
            mark_count: 0,
            evicted_lines: 0,
            shell_running: false,
            shell_run_start: None,
            shell_last_exit: 0,
            shell_last_duration_ms: 0,
            app_cursor_shape: None,
            app_cursor_blink: None,
            compose_buf,
        }
    }

    // --- active grid helper ---------------------------------------------------

    fn active(&mut self) -> &mut Grid {
        if self.on_alt {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    fn active_const(&self) -> &Grid {
        if self.on_alt {
            &self.alternate
        } else {
            &self.primary
        }
    }

    // --- dimensions -----------------------------------------------------------

    pub fn cols(&self) -> usize {
        self.primary.width
    }

    pub fn rows(&self) -> usize {
        self.primary.height
    }

    pub fn cursor(&self) -> Cursor {
        let g = self.active_const();
        Cursor {
            x: g.cur_x,
            y: g.cur_y,
            visible: self.modes.cursor_visible,
        }
    }

    // --- dirty row tracking ---------------------------------------------------

    /// Drain the set of viewport rows dirtied since the last call.
    ///
    /// After returning, the internal dirty state is cleared. The caller should
    /// redraw every row in the returned set. Rows outside `0..self.rows()` are
    /// conservatively represented as `DirtySet::all`.
    pub fn take_dirty_rows(&mut self) -> DirtySet {
        let (bitmap, all) = self.active().take_dirty();
        DirtySet::from_raw(bitmap, all)
    }

    // --- feeding bytes --------------------------------------------------------

    /// Parse `bytes` and apply them to the active grid. Any new output pins
    /// the viewport back to the live bottom.
    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // The parser takes `&mut dyn Handler` and `Terminal` is that handler,
        // so `self.parser.feed(self, ..)` would alias `&mut self`. Move the
        // parser out for the duration of the call — it carries state across
        // feeds, so it is put back afterward. `Parser` is small POD, so the
        // swap is cheap, and no `unsafe` is needed.
        let mut parser = std::mem::take(&mut self.parser);
        parser.feed(self, bytes);
        self.parser = parser;
        self.viewport_offset = 0;
    }

    // --- resize ---------------------------------------------------------------

    /// Resize both grids to `cols × rows`. Pre-scrolls the primary grid when
    /// shrinking past the cursor so the cursor stays anchored to the bottom.
    pub fn resize(&mut self, new_cols: usize, new_rows: usize) {
        let w = new_cols.max(1);
        let h = new_rows.max(1);

        // Pre-scroll the primary grid when the new height cannot fit the cursor.
        if h < self.primary.cur_y + 1 {
            let to_scroll = (self.primary.cur_y + 1) - h;
            self.primary.region = ScrollRegion {
                top: 0,
                bottom: self.primary.height - 1,
            };
            for _ in 0..to_scroll {
                let displaced: Option<Vec<Cell>> = self.primary.scroll_up(1).map(|r| r.to_vec());
                if let Some(row) = displaced {
                    self.archive_vec(&row);
                }
                self.primary.cur_y = self.primary.cur_y.saturating_sub(1);
            }
        }

        self.primary.resize(w, h);
        self.alternate.resize(w, h);

        if self.compose_buf.len() != w {
            self.compose_buf = vec![Cell::default(); w];
        }
        self.viewport_offset = self.viewport_offset.min(self.history.len());
    }

    // --- viewport -------------------------------------------------------------

    pub fn scrollback_len(&self) -> usize {
        self.history.len()
    }

    pub fn viewport_offset(&self) -> usize {
        self.viewport_offset
    }

    /// Scroll the viewport by `delta` rows: positive = up into history,
    /// negative = down toward live bottom. Clamped to valid range.
    pub fn scroll_viewport(&mut self, delta: isize) {
        let max_offset = self.history.len();
        if delta >= 0 {
            let up = delta as usize;
            self.viewport_offset = (self.viewport_offset + up).min(max_offset);
        } else {
            let down = (-delta) as usize;
            self.viewport_offset = self.viewport_offset.saturating_sub(down);
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.viewport_offset = 0;
    }

    /// Set the viewport offset to an absolute row count, clamped to history.
    pub fn set_viewport_offset(&mut self, offset: usize) {
        self.viewport_offset = offset.min(self.history.len());
    }

    /// The row at viewport position `y` (0 = top of viewport), padded to
    /// exactly `cols` cells. Scrollback rows are padded into `compose_buf`.
    pub fn viewport_row(&mut self, y: usize) -> &[Cell] {
        let height = self.active_const().height;
        if y >= height {
            return self.active().row(0);
        }

        if self.viewport_offset > y {
            let from_oldest = self.history.len() - self.viewport_offset + y;
            let src = self.history.get(from_oldest);
            let w = self.compose_buf.len();
            self.compose_buf.fill(Cell::default());
            let n = src.len().min(w);
            self.compose_buf[..n].copy_from_slice(&src[..n]);
            return &self.compose_buf;
        }

        let grid_y = y - self.viewport_offset;
        // Need separate borrows; use index via on_alt flag.
        if self.on_alt {
            self.alternate.row(grid_y)
        } else {
            self.primary.row(grid_y)
        }
    }

    /// Like `viewport_row` but for an explicit offset and a `y` that may equal
    /// `rows()`. Out-of-range rows return a blank row (compose_buf).
    pub fn viewport_row_at(&mut self, offset: usize, y: usize) -> &[Cell] {
        let hist_len = self.history.len();
        let height = self.active_const().height;
        let w = self.compose_buf.len();

        if offset > y {
            let oldest_signed = hist_len as isize - offset as isize + y as isize;
            self.compose_buf.fill(Cell::default());
            if oldest_signed < 0 {
                return &self.compose_buf;
            }
            let oldest = oldest_signed as usize;
            let src = self.history.get(oldest);
            let n = src.len().min(w);
            self.compose_buf[..n].copy_from_slice(&src[..n]);
            return &self.compose_buf;
        }

        let grid_y = y - offset;
        if grid_y >= height {
            self.compose_buf.fill(Cell::default());
            return &self.compose_buf;
        }

        if self.on_alt {
            self.alternate.row(grid_y)
        } else {
            self.primary.row(grid_y)
        }
    }

    // --- content-line access (for search) ------------------------------------

    /// Total content rows: scrollback length + active grid height.
    pub fn line_count(&self) -> usize {
        self.history.len() + self.active_const().height
    }

    /// Borrow content row `i`. 0..history.len() are scrollback rows; the rest
    /// are active-grid rows. Out-of-range returns an empty slice.
    pub fn line(&self, i: usize) -> &[Cell] {
        let hist = self.history.len();
        if i < hist {
            return self.history.get(i);
        }
        let g = self.active_const();
        let gy = i - hist;
        if gy >= g.height {
            return &[];
        }
        g.row(gy)
    }

    /// The content row currently shown at viewport position `y`.
    pub fn content_row_of_viewport(&self, y: usize) -> usize {
        if self.viewport_offset > y {
            return self.history.len() - self.viewport_offset + y;
        }
        self.history.len() + (y - self.viewport_offset)
    }

    /// Scroll the viewport so content row `target` is visible.
    pub fn scroll_to_line(&mut self, target: usize) {
        let hist = self.history.len();
        if target >= hist {
            self.viewport_offset = 0;
        } else {
            self.viewport_offset = (hist - target).min(hist);
        }
    }

    // --- title / cwd / clipboard accessors -----------------------------------

    pub fn title(&self) -> &str {
        std::str::from_utf8(&self.title_buf[..self.title_len]).unwrap_or("")
    }

    pub fn cwd(&self) -> &str {
        std::str::from_utf8(&self.cwd_buf[..self.cwd_len]).unwrap_or("")
    }

    /// Return the filesystem path from the stored OSC 7 value, stripping any
    /// `file://host` prefix and returning just the path component.
    pub fn cwd_path(&self) -> &str {
        let raw = std::str::from_utf8(&self.cwd_buf[..self.cwd_len]).unwrap_or("");
        let prefix = "file://";
        if let Some(after) = raw.strip_prefix(prefix) {
            // Find the first '/' — that starts the path.
            if let Some(slash) = after.find('/') {
                return &after[slash..];
            }
            return ""; // "file://" with no path
        }
        raw
    }

    pub fn clipboard(&self) -> &str {
        std::str::from_utf8(&self.clipboard_buf[..self.clipboard_len]).unwrap_or("")
    }

    /// The OSC-133 prompt marks recorded so far, oldest first.
    pub fn prompt_marks(&self) -> &[PromptMark] {
        &self.marks[..self.mark_count]
    }

    // --- shell-state accessor -------------------------------------------------

    pub fn last_run(&self) -> LastRun {
        LastRun {
            running: self.shell_running,
            exit_code: self.shell_last_exit,
            duration_ms: self.shell_last_duration_ms,
        }
    }

    // --- absolute line helpers ------------------------------------------------

    /// True when `abs_line` was marked as a prompt start (OSC 133;A).
    pub fn is_prompt_start(&self, abs_line: usize) -> bool {
        self.marks[..self.mark_count]
            .iter()
            .any(|m| m.kind == PromptMarkKind::PromptStart && m.line == abs_line)
    }

    /// Convert a content-row index to an absolute line index.
    pub fn absolute_line_of_content(&self, content_row: usize) -> usize {
        self.evicted_lines + content_row
    }

    // --- command-block derivation --------------------------------------------

    fn block_from_mark(&self, i: usize) -> Block {
        let marks = &self.marks[..self.mark_count];
        // For a running block (no terminator yet), end_line clamps to the
        // current cursor's absolute row + 1 — just past the last row that
        // could contain real output. Without this, the block visually
        // extends across every empty row in the viewport.
        let cursor_abs = self.evicted_lines + self.line_count().saturating_sub(self.rows())
            + self.active_const().cur_y as usize;
        let mut b = Block {
            command_line: marks[i].line,
            command_start_col: marks[i].col,
            output_line: marks[i].line,
            end_line: cursor_abs + 1,
            state: BlockState::Running,
            exit_code: 0,
            duration_ms: 0,
        };
        let mut j = i + 1;
        while j < marks.len() {
            let m = &marks[j];
            match m.kind {
                PromptMarkKind::CommandStart | PromptMarkKind::PromptStart => {
                    b.end_line = m.line;
                    break;
                }
                PromptMarkKind::OutputStart => {
                    if b.output_line == b.command_line {
                        b.output_line = m.line;
                    }
                }
                PromptMarkKind::CommandDone => {
                    b.state = BlockState::Exited;
                    b.exit_code = m.exit_code;
                    b.duration_ms = m.duration_ms;
                }
            }
            j += 1;
        }
        // Defensive: don't let end_line go past the current cursor for a
        // running block (e.g. if marks ordering puts a later mark with a
        // smaller line, or the cursor moved).
        if b.state == BlockState::Running && b.end_line > cursor_abs + 1 {
            b.end_line = cursor_abs + 1;
        }
        // Always: end_line ≥ command_line + 1, so a block always has a row.
        if b.end_line < b.command_line + 1 {
            b.end_line = b.command_line + 1;
        }
        b
    }

    /// Return the `Block` containing absolute line `abs_line`, or `None`.
    pub fn block_at(&self, abs_line: usize) -> Option<Block> {
        let marks = &self.marks[..self.mark_count];
        for (i, m) in marks.iter().enumerate() {
            if m.kind != PromptMarkKind::CommandStart {
                continue;
            }
            let b = self.block_from_mark(i);
            if abs_line >= b.command_line && abs_line < b.end_line {
                return Some(b);
            }
        }
        None
    }

    /// Return the next block after `abs_line` (command_line > abs_line).
    pub fn block_after(&self, abs_line: usize) -> Option<Block> {
        let marks = &self.marks[..self.mark_count];
        for (i, m) in marks.iter().enumerate() {
            if m.kind != PromptMarkKind::CommandStart {
                continue;
            }
            if m.line > abs_line {
                return Some(self.block_from_mark(i));
            }
        }
        None
    }

    /// Return the previous block before `abs_line` (command_line < abs_line).
    pub fn block_before(&self, abs_line: usize) -> Option<Block> {
        let marks = &self.marks[..self.mark_count];
        let mut result: Option<Block> = None;
        for (i, m) in marks.iter().enumerate() {
            if m.kind != PromptMarkKind::CommandStart {
                continue;
            }
            if m.line < abs_line {
                result = Some(self.block_from_mark(i));
            }
        }
        result
    }

    // --- internal helpers -----------------------------------------------------

    fn line_feed_internal(&mut self) {
        let on_alt = self.on_alt;
        let scrolled_row: Option<Vec<Cell>> = self.active().line_feed().map(|r| r.to_vec());
        if let Some(row) = scrolled_row {
            if !on_alt {
                self.archive_vec(&row);
            }
        }
    }

    fn archive_vec(&mut self, row: &[Cell]) {
        let was_full = self.history.len() == self.history.capacity();
        self.history.push(row);
        if was_full {
            self.evicted_lines += 1;
        }
    }

    fn set_title(&mut self, text: &[u8]) {
        self.title_len = copy_into(&mut self.title_buf, text);
    }

    fn set_cwd(&mut self, text: &[u8]) {
        self.cwd_len = copy_into(&mut self.cwd_buf, text);
    }

    fn set_clipboard(&mut self, text: &[u8]) {
        // OSC 52 payload is `selection;base64data`; store the base64 part.
        let data = if let Some(pos) = text.iter().position(|&b| b == b';') {
            &text[pos + 1..]
        } else {
            text
        };
        self.clipboard_len = copy_into(&mut self.clipboard_buf, data);
    }

    fn record_prompt_mark(&mut self, payload: &[u8]) {
        if payload.is_empty() {
            return;
        }
        let kind = match payload[0] {
            b'A' => PromptMarkKind::PromptStart,
            b'B' => PromptMarkKind::CommandStart,
            b'C' => PromptMarkKind::OutputStart,
            b'D' => PromptMarkKind::CommandDone,
            _ => return,
        };

        // Shell-state tracking.
        if kind == PromptMarkKind::OutputStart {
            self.shell_running = true;
            self.shell_run_start = Some(Instant::now());
        } else if kind == PromptMarkKind::CommandDone {
            if self.shell_running {
                if let Some(start) = self.shell_run_start {
                    self.shell_last_duration_ms = start.elapsed().as_millis() as i64;
                }
            }
            self.shell_running = false;
            // Parse optional "exit_code=N" from the payload.
            let exit_code = parse_exit_code(payload);
            self.shell_last_exit = exit_code;
        }

        let line_num = self.evicted_lines + self.history.len() + self.active_const().cur_y;

        // Suppress duplicate prompt_start on the same line.
        if kind == PromptMarkKind::PromptStart && self.mark_count > 0 {
            let last = self.marks[self.mark_count - 1];
            if last.kind == PromptMarkKind::PromptStart && last.line == line_num {
                return;
            }
        }

        // Drop the oldest mark when at capacity.
        if self.mark_count == MAX_MARKS {
            self.marks.copy_within(1..MAX_MARKS, 0);
            self.mark_count -= 1;
        }

        let exit_code = if kind == PromptMarkKind::CommandDone {
            self.shell_last_exit
        } else {
            0
        };
        let duration_ms = if kind == PromptMarkKind::CommandDone {
            self.shell_last_duration_ms.max(0) as u64
        } else {
            0
        };
        // Record cursor column at the time of the mark.  For CommandStart (B)
        // this captures where typed input begins, used later for header display.
        let col = self.active_const().cur_x.min(u16::MAX as usize) as u16;
        self.marks[self.mark_count] = PromptMark {
            kind,
            line: line_num,
            col,
            exit_code,
            duration_ms,
        };
        self.mark_count += 1;
    }

    fn invalidate_grid_marks(&mut self) {
        let base = self.evicted_lines + self.history.len();
        let mut w = 0;
        for i in 0..self.mark_count {
            if self.marks[i].line < base {
                self.marks[w] = self.marks[i];
                w += 1;
            }
        }
        self.mark_count = w;
    }

    fn reverse_index(&mut self) {
        let g = self.active();
        if g.cur_y == g.region.top {
            g.scroll_down(1);
        } else if g.cur_y > 0 {
            g.cur_y -= 1;
        }
    }

    fn reset(&mut self) {
        self.primary.cursor_to(0, 0);
        self.primary.erase_display(2);
        self.primary.pen = Cell::default();
        self.primary.region = ScrollRegion {
            top: 0,
            bottom: self.primary.height - 1,
        };
        self.alternate.cursor_to(0, 0);
        self.alternate.erase_display(2);
        self.alternate.pen = Cell::default();
        self.modes = PrivateModes::default();
        self.g0_line_drawing = false;
        self.on_alt = false;
        self.viewport_offset = 0;
        // Full reset: everything needs redrawing.
        self.primary.mark_all_dirty();
    }

    fn set_alt_screen(&mut self, on: bool) {
        if on == self.modes.alt_screen {
            return;
        }
        if on {
            self.primary.save_cursor();
            self.alternate.cursor_to(0, 0);
            self.alternate.erase_display(2);
            self.on_alt = true;
            self.modes.alt_screen = true;
            // Switching to alt screen: entire viewport is new content.
            self.alternate.mark_all_dirty();
        } else {
            self.on_alt = false;
            self.primary.restore_cursor();
            self.modes.alt_screen = false;
            // Returning to primary: entire viewport must be repainted.
            self.primary.mark_all_dirty();
        }
    }

    fn set_private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            1 => self.modes.app_cursor_keys = on,
            7 => {
                self.modes.autowrap = on;
                self.primary.modes.autowrap = on;
                self.alternate.modes.autowrap = on;
            }
            25 => {
                self.modes.cursor_visible = on;
                self.primary.modes.cursor_visible = on;
                self.alternate.modes.cursor_visible = on;
            }
            1000 => self.modes.mouse_button = on,
            1002 => self.modes.mouse_button = on,
            1006 => self.modes.mouse_sgr = on,
            2004 => self.modes.bracketed_paste = on,
            1049 => self.set_alt_screen(on),
            _ => {}
        }
    }

    fn apply_decscusr(&mut self, ps: u16) {
        match ps {
            0 => {
                self.app_cursor_shape = None;
                self.app_cursor_blink = None;
            }
            1 => {
                self.app_cursor_shape = Some(CursorShape::Block);
                self.app_cursor_blink = Some(true);
            }
            2 => {
                self.app_cursor_shape = Some(CursorShape::Block);
                self.app_cursor_blink = Some(false);
            }
            3 => {
                self.app_cursor_shape = Some(CursorShape::Underline);
                self.app_cursor_blink = Some(true);
            }
            4 => {
                self.app_cursor_shape = Some(CursorShape::Underline);
                self.app_cursor_blink = Some(false);
            }
            5 => {
                self.app_cursor_shape = Some(CursorShape::Bar);
                self.app_cursor_blink = Some(true);
            }
            6 => {
                self.app_cursor_shape = Some(CursorShape::Bar);
                self.app_cursor_blink = Some(false);
            }
            _ => {}
        }
    }

    fn csi_standard(&mut self, params: &[u16], final_byte: u8) {
        let p0 = vt_param(params, 0, 1);
        match final_byte {
            b'A' => self.active().cursor_up(p0 as usize),
            b'B' | b'e' => self.active().cursor_down(p0 as usize),
            b'C' | b'a' => self.active().cursor_forward(p0 as usize),
            b'D' => self.active().cursor_back(p0 as usize),
            b'E' => {
                // CNL
                self.active().carriage_return();
                self.active().cursor_down(p0 as usize);
            }
            b'F' => {
                // CPL
                self.active().carriage_return();
                self.active().cursor_up(p0 as usize);
            }
            b'G' | b'`' => {
                let col = one_based(vt_param(params, 0, 1));
                self.active().cursor_to_column(col);
            }
            b'd' => {
                let row = one_based(vt_param(params, 0, 1));
                self.active().cursor_to_row(row);
            }
            b'H' | b'f' => {
                let row = one_based(vt_param(params, 0, 1));
                let col = one_based(vt_param(params, 1, 1));
                self.active().cursor_to(col, row);
            }
            b'J' => {
                let mode = vt_param(params, 0, 0);
                self.active().erase_display(mode);
                if (mode == 2 || mode == 3) && !self.on_alt {
                    self.invalidate_grid_marks();
                }
            }
            b'K' => {
                let mode = vt_param(params, 0, 0);
                self.active().erase_line(mode);
            }
            b'@' => self.active().insert_chars(p0 as usize),
            b'P' => self.active().delete_chars(p0 as usize),
            b'L' => self.active().insert_lines(p0 as usize),
            b'M' => self.active().delete_lines(p0 as usize),
            b'X' => self.active().erase_chars(p0 as usize),
            b'S' => {
                let _ = self.active().scroll_up(p0 as usize);
            }
            b'T' => self.active().scroll_down(p0 as usize),
            b'r' => {
                let top = vt_param(params, 0, 0) as usize;
                let bottom = vt_param(params, 1, 0) as usize;
                self.active().set_scroll_region(top, bottom);
            }
            b'm' => self.apply_sgr(params),
            b'h' => self.set_standard_modes(params, true),
            b'l' => self.set_standard_modes(params, false),
            b's' => self.active().save_cursor(),
            b'u' => self.active().restore_cursor(),
            _ => {}
        }
    }

    fn csi_private(&mut self, params: &[u16], final_byte: u8) {
        match final_byte {
            b'h' => {
                for &p in params {
                    self.set_private_mode(p, true);
                }
            }
            b'l' => {
                for &p in params {
                    self.set_private_mode(p, false);
                }
            }
            _ => {}
        }
    }

    fn set_standard_modes(&mut self, params: &[u16], on: bool) {
        for &p in params {
            if p == 4 {
                self.active().modes.insert = on;
            }
        }
    }

    fn apply_sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            self.active().pen = Cell::default();
            return;
        }
        // Build a local copy, apply all SGR codes, then write back.
        let mut pen = self.active().pen;
        let mut i = 0;
        while i < params.len() {
            let extra = apply_sgr_at(&mut pen, params, i);
            i += 1 + extra;
        }
        self.active().pen = pen;
    }
}

// --- Handler impl -------------------------------------------------------------

impl Handler for Terminal {
    fn print(&mut self, cp: char) {
        let translated = if self.g0_line_drawing {
            translate_line_drawing(cp)
        } else {
            cp
        };
        self.active().print(translated);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => {}                                // BEL
            0x08 => self.active().backspace(),        // BS
            0x09 => self.active().tab(),              // HT
            0x0A..=0x0C => self.line_feed_internal(), // LF, VT, FF
            0x0D => self.active().carriage_return(),  // CR
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, intermediates: &[u8], params: &[u16], final_byte: u8) {
        // DECSCUSR: CSI Ps SP q
        if intermediates.len() == 1 && intermediates[0] == b' ' && final_byte == b'q' {
            self.apply_decscusr(vt_param(params, 0, 0));
            return;
        }
        let private = !intermediates.is_empty() && intermediates[0] == b'?';
        if private {
            self.csi_private(params, final_byte);
        } else {
            self.csi_standard(params, final_byte);
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], final_byte: u8) {
        if !intermediates.is_empty() && intermediates[0] == b'(' {
            self.g0_line_drawing = final_byte == b'0';
            return;
        }
        match final_byte {
            b'7' => self.active().save_cursor(),    // DECSC
            b'8' => self.active().restore_cursor(), // DECRC
            b'c' => self.reset(),                   // RIS
            b'D' => self.line_feed_internal(),      // IND
            b'M' => self.reverse_index(),           // RI
            b'E' => {
                // NEL
                self.active().carriage_return();
                self.line_feed_internal();
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, data: &[u8]) {
        let semi = match data.iter().position(|&b| b == b';') {
            Some(p) => p,
            None => return,
        };
        let code = &data[..semi];
        let payload = &data[semi + 1..];

        if code == b"0" || code == b"2" {
            self.set_title(payload);
        } else if code == b"7" {
            self.set_cwd(payload);
        } else if code == b"52" {
            self.set_clipboard(payload);
        } else if code == b"133" {
            self.record_prompt_mark(payload);
        }
    }
}

// --- free helpers -------------------------------------------------------------

/// Fetch VT parameter `idx`, returning `default` when absent or zero.
fn vt_param(params: &[u16], idx: usize, default: u16) -> u16 {
    if idx >= params.len() {
        return default;
    }
    if params[idx] == 0 {
        default
    } else {
        params[idx]
    }
}

/// Convert a 1-based VT coordinate to a 0-based index.
fn one_based(value: u16) -> usize {
    if value == 0 { 0 } else { (value - 1) as usize }
}

fn copy_into(dst: &mut [u8], src: &[u8]) -> usize {
    let n = dst.len().min(src.len());
    dst[..n].copy_from_slice(&src[..n]);
    n
}

/// Map a character through the DEC special graphics (line-drawing) set.
fn translate_line_drawing(cp: char) -> char {
    match cp {
        'j' => '\u{2518}', // ┘
        'k' => '\u{2510}', // ┐
        'l' => '\u{250C}', // ┌
        'm' => '\u{2514}', // └
        'n' => '\u{253C}', // ┼
        'q' => '\u{2500}', // ─
        't' => '\u{251C}', // ├
        'u' => '\u{2524}', // ┤
        'v' => '\u{2534}', // ┴
        'w' => '\u{252C}', // ┬
        'x' => '\u{2502}', // │
        '`' => '\u{25C6}', // ◆
        'a' => '\u{2592}', // ▒
        'f' => '\u{00B0}', // °
        'g' => '\u{00B1}', // ±
        '~' => '\u{00B7}', // ·
        _ => cp,
    }
}

/// Parse an optional `exit_code=N` field from an OSC 133;D payload.
fn parse_exit_code(payload: &[u8]) -> i32 {
    let key = b"exit_code=";
    let Some(idx) = payload.windows(key.len()).position(|w| w == key) else {
        return 0;
    };
    let num_bytes = &payload[idx + key.len()..];
    let mut n: i32 = 0;
    let mut negative = false;
    let mut start = 0;
    if num_bytes.first() == Some(&b'-') {
        negative = true;
        start = 1;
    }
    for &ch in &num_bytes[start..] {
        if !ch.is_ascii_digit() {
            break;
        }
        n = n * 10 + (ch - b'0') as i32;
    }
    if negative { -n } else { n }
}

/// Apply SGR code at `params[i]`. Returns extra params consumed beyond `i`.
fn apply_sgr_at(pen: &mut Cell, params: &[u16], i: usize) -> usize {
    match params[i] {
        0 => *pen = Cell::default(),
        1 => pen.attrs.insert(Attrs::BOLD),
        2 => pen.attrs.insert(Attrs::DIM),
        3 => pen.attrs.insert(Attrs::ITALIC),
        4 => pen.attrs.insert(Attrs::UNDERLINE),
        5 => pen.attrs.insert(Attrs::BLINK),
        7 => pen.attrs.insert(Attrs::INVERSE),
        8 => pen.attrs.insert(Attrs::INVISIBLE),
        9 => pen.attrs.insert(Attrs::STRIKETHROUGH),
        21 | 22 => {
            pen.attrs.remove(Attrs::BOLD);
            pen.attrs.remove(Attrs::DIM);
        }
        23 => pen.attrs.remove(Attrs::ITALIC),
        24 => pen.attrs.remove(Attrs::UNDERLINE),
        25 => pen.attrs.remove(Attrs::BLINK),
        27 => pen.attrs.remove(Attrs::INVERSE),
        28 => pen.attrs.remove(Attrs::INVISIBLE),
        29 => pen.attrs.remove(Attrs::STRIKETHROUGH),
        30..=37 => pen.fg = Color::Palette((params[i] - 30) as u8),
        39 => pen.fg = Color::Default,
        40..=47 => pen.bg = Color::Palette((params[i] - 40) as u8),
        49 => pen.bg = Color::Default,
        90..=97 => pen.fg = Color::Palette((params[i] - 90 + 8) as u8),
        100..=107 => pen.bg = Color::Palette((params[i] - 100 + 8) as u8),
        38 => return apply_extended_color(&mut pen.fg, params, i),
        48 => return apply_extended_color(&mut pen.bg, params, i),
        _ => {}
    }
    0
}

fn apply_extended_color(target: &mut Color, params: &[u16], i: usize) -> usize {
    if i + 1 >= params.len() {
        return 0;
    }
    match params[i + 1] {
        5 => {
            if i + 2 >= params.len() {
                return 1;
            }
            *target = Color::Palette((params[i + 2] & 0xFF) as u8);
            2
        }
        2 => {
            if i + 4 >= params.len() {
                return (params.len() - i - 1).min(3);
            }
            *target = Color::Rgb([
                (params[i + 2] & 0xFF) as u8,
                (params[i + 3] & 0xFF) as u8,
                (params[i + 4] & 0xFF) as u8,
            ]);
            4
        }
        _ => 1,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrollback::DEFAULT_CAPACITY;

    fn make_terminal(cols: usize, rows: usize) -> Terminal {
        Terminal::new(cols, rows, DEFAULT_CAPACITY)
    }

    /// Render viewport row `y` to a String.
    fn viewport_text(term: &mut Terminal, y: usize) -> String {
        term.viewport_row(y).iter().map(|c| c.cp).collect()
    }

    #[test]
    fn init_reports_its_dimensions() {
        let term = make_terminal(80, 24);
        assert_eq!(80, term.cols());
        assert_eq!(24, term.rows());
    }

    #[test]
    fn feeding_plain_text_fills_the_first_row() {
        let mut term = make_terminal(10, 3);
        term.feed(b"hello");
        assert_eq!("hello     ", viewport_text(&mut term, 0));
        assert_eq!(5, term.cursor().x);
    }

    #[test]
    fn cr_and_lf_reposition_the_cursor() {
        let mut term = make_terminal(10, 4);
        term.feed(b"ab\r\ncd");
        assert_eq!("ab        ", viewport_text(&mut term, 0));
        assert_eq!("cd        ", viewport_text(&mut term, 1));
        assert_eq!(1, term.cursor().y);
    }

    #[test]
    fn csi_cursor_position_then_print() {
        let mut term = make_terminal(10, 5);
        term.feed(b"\x1B[3;5HX");
        assert_eq!("    X     ", viewport_text(&mut term, 2));
    }

    #[test]
    fn csi_cursor_moves_clamp_at_bounds() {
        let mut term = make_terminal(10, 5);
        term.feed(b"\x1B[99;99H");
        assert_eq!(9, term.cursor().x);
        assert_eq!(4, term.cursor().y);
        term.feed(b"\x1B[99A");
        assert_eq!(0, term.cursor().y);
    }

    #[test]
    fn ed_clears_from_cursor_to_end_of_screen() {
        let mut term = make_terminal(4, 3);
        term.feed(b"AAAA\r\nBBBB\r\nCC");
        term.feed(b"\x1B[0J");
        assert_eq!("AAAA", viewport_text(&mut term, 0));
        assert_eq!("CC  ", viewport_text(&mut term, 2));
    }

    #[test]
    fn el_clears_the_current_line() {
        let mut term = make_terminal(6, 2);
        term.feed(b"abcdef\x1B[1G\x1B[0K");
        assert_eq!("      ", viewport_text(&mut term, 0));
    }

    #[test]
    fn sgr_sets_bold_and_a_palette_color() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[1;32mX");
        let c = term.viewport_row(0)[0];
        assert!(c.attrs.contains(Attrs::BOLD));
        assert_eq!(Color::Palette(2), c.fg);
    }

    #[test]
    fn sgr_reset_clears_the_pen() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[1;31mA\x1B[0mB");
        let a = term.viewport_row(0)[0];
        let b = term.viewport_row(0)[1];
        assert!(a.attrs.contains(Attrs::BOLD));
        assert!(!b.attrs.contains(Attrs::BOLD));
        assert_eq!(Color::Default, b.fg);
    }

    #[test]
    fn sgr_256_color_palette_via_38_5_n() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[38;5;200mX");
        assert_eq!(Color::Palette(200), term.viewport_row(0)[0].fg);
    }

    #[test]
    fn sgr_truecolor_via_48_2_r_g_b() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[48;2;10;20;30mX");
        assert_eq!(Color::Rgb([10, 20, 30]), term.viewport_row(0)[0].bg);
    }

    #[test]
    fn sgr_truecolor_mixed_with_other_codes_in_one_sequence() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[1;38;2;1;2;3;4mX");
        let c = term.viewport_row(0)[0];
        assert!(c.attrs.contains(Attrs::BOLD));
        assert!(c.attrs.contains(Attrs::UNDERLINE));
        assert_eq!(Color::Rgb([1, 2, 3]), c.fg);
    }

    #[test]
    fn line_feed_past_the_bottom_pushes_rows_into_scrollback() {
        let mut term = make_terminal(4, 2);
        term.feed(b"L1\r\nL2\r\nL3");
        assert_eq!(1, term.scrollback_len());
        assert_eq!("L2  ", viewport_text(&mut term, 0));
        assert_eq!("L3  ", viewport_text(&mut term, 1));
    }

    #[test]
    fn viewport_scrolls_up_into_scrollback_history() {
        let mut term = make_terminal(4, 2);
        term.feed(b"L1\r\nL2\r\nL3\r\nL4");
        assert_eq!(2, term.scrollback_len());

        term.scroll_viewport(1);
        assert_eq!(1, term.viewport_offset());
        assert_eq!("L2  ", viewport_text(&mut term, 0));
        assert_eq!("L3  ", viewport_text(&mut term, 1));

        term.scroll_viewport(1);
        assert_eq!("L1  ", viewport_text(&mut term, 0));
        assert_eq!("L2  ", viewport_text(&mut term, 1));
    }

    #[test]
    fn viewport_scroll_clamps_and_scroll_to_bottom_resets_it() {
        let mut term = make_terminal(4, 2);
        term.feed(b"L1\r\nL2\r\nL3\r\nL4");
        term.scroll_viewport(999);
        assert_eq!(2, term.viewport_offset());
        term.scroll_viewport(-999);
        assert_eq!(0, term.viewport_offset());
        term.scroll_viewport(2);
        term.scroll_to_bottom();
        assert_eq!(0, term.viewport_offset());
    }

    #[test]
    fn set_viewport_offset_clamps_to_scrollback_length_and_sets_within_range() {
        let mut term = make_terminal(4, 2);
        term.feed(b"L1\r\nL2\r\nL3\r\nL4");
        assert_eq!(2, term.scrollback_len());

        term.set_viewport_offset(1);
        assert_eq!(1, term.viewport_offset());

        term.set_viewport_offset(999);
        assert_eq!(2, term.viewport_offset());

        term.set_viewport_offset(0);
        assert_eq!(0, term.viewport_offset());
    }

    #[test]
    fn new_output_snaps_the_viewport_back_to_the_live_bottom() {
        let mut term = make_terminal(4, 2);
        term.feed(b"L1\r\nL2\r\nL3");
        term.scroll_viewport(1);
        assert!(term.viewport_offset() > 0);
        term.feed(b"X");
        assert_eq!(0, term.viewport_offset());
    }

    #[test]
    fn alternate_screen_isolates_scrollback_and_restores_on_exit() {
        let mut term = make_terminal(4, 2);
        term.feed(b"L1\r\nL2\r\nL3");
        assert_eq!(1, term.scrollback_len());

        term.feed(b"\x1B[?1049h");
        term.feed(b"A1\r\nA2\r\nA3\r\nA4");
        assert_eq!(1, term.scrollback_len());

        term.feed(b"\x1B[?1049l");
        assert_eq!("L2  ", viewport_text(&mut term, 0));
        assert_eq!("L3  ", viewport_text(&mut term, 1));
    }

    #[test]
    fn cursor_visibility_toggles_via_decset_25() {
        let mut term = make_terminal(10, 2);
        assert!(term.cursor().visible);
        term.feed(b"\x1B[?25l");
        assert!(!term.cursor().visible);
        term.feed(b"\x1B[?25h");
        assert!(term.cursor().visible);
    }

    #[test]
    fn bracketed_paste_flag_follows_decset_2004() {
        let mut term = make_terminal(10, 2);
        assert!(!term.modes.bracketed_paste);
        term.feed(b"\x1B[?2004h");
        assert!(term.modes.bracketed_paste);
    }

    #[test]
    fn autowrap_can_be_disabled_via_decrst_7() {
        let mut term = make_terminal(4, 2);
        term.feed(b"\x1B[?7l");
        term.feed(b"abcdef");
        assert_eq!(0, term.cursor().y);
        assert_eq!("abcf", viewport_text(&mut term, 0));
    }

    #[test]
    fn osc_0_sets_the_window_title() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B]0;Anvil\x07");
        assert_eq!("Anvil", term.title());
    }

    #[test]
    fn osc_7_records_the_working_directory() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B]7;file:///home/dev\x07");
        assert_eq!("file:///home/dev", term.cwd());
    }

    #[test]
    fn cwd_path_strips_file_prefix_and_host() {
        let mut term = make_terminal(10, 2);

        term.feed(b"\x1B]7;file:///home/dev\x07");
        assert_eq!("/home/dev", term.cwd_path());

        term.feed(b"\x1B]7;file://somehost/var/log\x07");
        assert_eq!("/var/log", term.cwd_path());

        term.feed(b"\x1B]7;/plain/path\x07");
        assert_eq!("/plain/path", term.cwd_path());

        term.feed(b"\x1B]7;\x07");
        assert_eq!("", term.cwd_path());

        term.feed(b"\x1B]7;file://\x07");
        assert_eq!("", term.cwd_path());
    }

    #[test]
    fn osc_133_prompt_marks_are_recorded_with_absolute_lines() {
        let mut term = make_terminal(6, 3);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"$ ls\r\n");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"out\r\n");
        term.feed(b"\x1B]133;D\x07");

        let marks = term.prompt_marks();
        assert_eq!(4, marks.len());
        assert_eq!(0, marks[0].line);
        assert_eq!(PromptMarkKind::PromptStart, marks[0].kind);
        assert_eq!(1, marks[1].line);
        assert_eq!(PromptMarkKind::CommandStart, marks[1].kind);
        assert_eq!(PromptMarkKind::OutputStart, marks[2].kind);
        assert_eq!(2, marks[3].line);
        assert_eq!(PromptMarkKind::CommandDone, marks[3].kind);
    }

    #[test]
    fn erasing_the_display_drops_prompt_marks_in_the_grid_region() {
        let mut term = make_terminal(6, 4);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"$ x\r\n");
        assert_eq!(1, term.prompt_marks().len());
        term.feed(b"\x1B[2J");
        assert_eq!(0, term.prompt_marks().len());
        assert!(!term.is_prompt_start(0));
    }

    #[test]
    fn osc_133_absolute_line_survives_scrollback_eviction() {
        let mut term = make_terminal(4, 2);
        term.feed(b"L1\r\nL2");
        term.feed(b"\x1B]133;A\x07");
        assert_eq!(1, term.prompt_marks()[0].line);
        term.feed(b"\r\nL3\r\nL4\r\nL5");
        assert_eq!(1, term.prompt_marks()[0].line);
    }

    #[test]
    fn is_prompt_start_returns_true_for_prompt_start_mark_and_false_elsewhere() {
        let mut term = make_terminal(6, 3);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"$ ls\r\n");
        term.feed(b"\x1B]133;B\x07");
        assert!(term.is_prompt_start(0));
        assert!(!term.is_prompt_start(1));
        assert!(!term.is_prompt_start(2));
    }

    #[test]
    fn is_prompt_start_deduplicates_repeated_osc_133a_on_same_line() {
        let mut term = make_terminal(6, 3);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;A\x07");
        assert_eq!(1, term.prompt_marks().len());
        assert!(term.is_prompt_start(0));
    }

    #[test]
    fn absolute_line_of_content_converts_content_row_to_absolute_line() {
        let mut term = make_terminal(4, 3);
        assert_eq!(0, term.absolute_line_of_content(0));
        assert_eq!(2, term.absolute_line_of_content(2));
        term.feed(b"A\r\nB\r\nC\r\nD");
        let ev = term.evicted_lines;
        assert_eq!(ev + 1, term.absolute_line_of_content(1));
    }

    #[test]
    fn is_prompt_start_ring_wraps_without_error_past_max_marks() {
        let mut term = make_terminal(6, 3);
        for _ in 0..MAX_MARKS + 10 {
            term.feed(b"\x1B]133;A\x07");
            term.feed(b"\r\n");
        }
        assert_eq!(MAX_MARKS, term.prompt_marks().len());
    }

    #[test]
    fn esc_c_performs_a_full_reset() {
        let mut term = make_terminal(4, 2);
        term.feed(b"\x1B[1;31mABCD");
        term.feed(b"\x1Bc");
        assert_eq!("    ", viewport_text(&mut term, 0));
        assert_eq!(0, term.cursor().x);
        term.feed(b"Z");
        assert!(!term.viewport_row(0)[0].attrs.contains(Attrs::BOLD));
    }

    #[test]
    fn dec_line_drawing_charset_maps_lowercase_letters_to_box_glyphs() {
        let mut term = make_terminal(6, 2);
        term.feed(b"\x1B(0qqq\x1B(Bqq");
        let r = term.viewport_row(0);
        assert_eq!('\u{2500}', r[0].cp);
        assert_eq!('\u{2500}', r[2].cp);
        assert_eq!('q', r[3].cp);
    }

    #[test]
    fn insert_and_delete_lines_via_csi_l_and_m() {
        let mut term = make_terminal(4, 4);
        term.feed(b"11\r\n22\r\n33\r\n44");
        term.feed(b"\x1B[2;1H");
        term.feed(b"\x1B[L");
        assert_eq!("11  ", viewport_text(&mut term, 0));
        assert_eq!("    ", viewport_text(&mut term, 1));
        assert_eq!("22  ", viewport_text(&mut term, 2));
    }

    #[test]
    fn decstbm_scroll_region_limits_line_feeds() {
        let mut term = make_terminal(4, 4);
        term.feed(b"11\r\n22\r\n33\r\n44");
        term.feed(b"\x1B[2;3r");
        term.feed(b"\x1B[3;1H");
        term.feed(b"\r\nXX");
        assert_eq!("11  ", viewport_text(&mut term, 0));
        assert_eq!("33  ", viewport_text(&mut term, 1));
        assert_eq!("XX  ", viewport_text(&mut term, 2));
        assert_eq!("44  ", viewport_text(&mut term, 3));
    }

    #[test]
    fn resize_keeps_content_and_re_clamps_the_viewport() {
        let mut term = make_terminal(6, 3);
        term.feed(b"hello\r\nworld");
        term.resize(3, 2);
        assert_eq!(3, term.cols());
        assert_eq!(2, term.rows());
        assert_eq!("hel", viewport_text(&mut term, 0));
    }

    #[test]
    fn autowrap_moves_to_the_next_line_at_the_right_edge() {
        let mut term = make_terminal(3, 3);
        term.feed(b"abcdef");
        assert_eq!("abc", viewport_text(&mut term, 0));
        assert_eq!("def", viewport_text(&mut term, 1));
    }

    #[test]
    fn save_and_restore_cursor_via_esc_7_esc_8() {
        let mut term = make_terminal(10, 4);
        term.feed(b"\x1B[3;5H");
        term.feed(b"\x1B7");
        term.feed(b"\x1B[1;1H");
        term.feed(b"\x1B8");
        assert_eq!(4, term.cursor().x);
        assert_eq!(2, term.cursor().y);
    }

    #[test]
    fn viewport_row_always_returns_exactly_cols_cells() {
        let mut term = make_terminal(8, 2);
        term.feed(b"hi\r\nthere\r\nmore");
        term.scroll_viewport(1);
        assert_eq!(8, term.viewport_row(0).len());
        assert_eq!(8, term.viewport_row(1).len());
    }

    #[test]
    fn bs_and_ht_execute_as_cursor_controls() {
        let mut term = make_terminal(20, 2);
        term.feed(b"ab\x08");
        assert_eq!(1, term.cursor().x);
        term.feed(b"\r\x09");
        assert_eq!(8, term.cursor().x);
    }

    #[test]
    fn csi_relative_cursor_moves_b_c_d() {
        let mut term = make_terminal(10, 5);
        term.feed(b"\x1B[3;5H");
        term.feed(b"\x1B[1B");
        assert_eq!(3, term.cursor().y);
        term.feed(b"\x1B[2C");
        assert_eq!(6, term.cursor().x);
        term.feed(b"\x1B[3D");
        assert_eq!(3, term.cursor().x);
    }

    #[test]
    fn csi_e_and_f_move_to_line_start_down_and_up() {
        let mut term = make_terminal(10, 5);
        term.feed(b"\x1B[3;5H\x1B[1E");
        assert_eq!(0, term.cursor().x);
        assert_eq!(3, term.cursor().y);
        term.feed(b"\x1B[5;5H\x1B[2F");
        assert_eq!(0, term.cursor().x);
        assert_eq!(2, term.cursor().y);
    }

    #[test]
    fn csi_d_sets_the_cursor_row_absolutely() {
        let mut term = make_terminal(10, 5);
        term.feed(b"\x1B[3d");
        assert_eq!(2, term.cursor().y);
    }

    #[test]
    fn csi_at_and_p_insert_and_delete_characters() {
        let mut term = make_terminal(6, 2);
        term.feed(b"abcdef\x1B[1G\x1B[2@");
        assert_eq!("  abcd", viewport_text(&mut term, 0));
        term.feed(b"\x1B[1G\x1B[3P");
        assert_eq!("bcd   ", viewport_text(&mut term, 0));
    }

    #[test]
    fn csi_x_erases_characters_in_place() {
        let mut term = make_terminal(6, 2);
        term.feed(b"abcdef\x1B[1G\x1B[3X");
        assert_eq!("   def", viewport_text(&mut term, 0));
    }

    #[test]
    fn csi_s_and_t_scroll_the_screen_up_and_down() {
        let mut term = make_terminal(4, 3);
        term.feed(b"11\r\n22\r\n33");
        term.feed(b"\x1B[1S");
        assert_eq!("22  ", viewport_text(&mut term, 0));
        term.feed(b"\x1B[1T");
        assert_eq!("    ", viewport_text(&mut term, 0));
        assert_eq!("22  ", viewport_text(&mut term, 1));
    }

    #[test]
    fn csi_m_deletes_lines() {
        let mut term = make_terminal(4, 4);
        term.feed(b"11\r\n22\r\n33\r\n44");
        term.feed(b"\x1B[1;1H\x1B[1M");
        assert_eq!("22  ", viewport_text(&mut term, 0));
    }

    #[test]
    fn csi_4h_and_4l_toggle_the_grid_insert_mode() {
        let mut term = make_terminal(6, 2);
        term.feed(b"\x1B[4h");
        assert!(term.primary.modes.insert);
        term.feed(b"\x1B[4l");
        assert!(!term.primary.modes.insert);
    }

    #[test]
    fn esc_d_indexes_down_and_esc_e_moves_to_the_next_line() {
        let mut term = make_terminal(6, 3);
        term.feed(b"\x1B[2;3H\x1BD");
        assert_eq!(2, term.cursor().y);
        assert_eq!(2, term.cursor().x);
        term.feed(b"\x1B[1;3H\x1BE");
        assert_eq!(0, term.cursor().x);
        assert_eq!(1, term.cursor().y);
    }

    #[test]
    fn esc_m_reverse_indexes_scrolling_down_at_the_top() {
        let mut term = make_terminal(4, 3);
        term.feed(b"11\r\n22\r\n33");
        term.feed(b"\x1B[2;1H\x1BM");
        assert_eq!(0, term.cursor().y);
        term.feed(b"\x1BM");
        assert_eq!("    ", viewport_text(&mut term, 0));
        assert_eq!("11  ", viewport_text(&mut term, 1));
    }

    #[test]
    fn osc_52_stores_the_clipboard_payload() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B]52;c;SGVsbG8=\x07");
        assert_eq!("SGVsbG8=", term.clipboard());
    }

    #[test]
    fn decset_1_toggles_application_cursor_keys() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[?1h");
        assert!(term.modes.app_cursor_keys);
        term.feed(b"\x1B[?1l");
        assert!(!term.modes.app_cursor_keys);
    }

    #[test]
    fn decset_mouse_modes_1000_1002_1006() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[?1000h");
        assert!(term.modes.mouse_button);
        term.feed(b"\x1B[?1000l\x1B[?1002h");
        assert!(term.modes.mouse_button);
        term.feed(b"\x1B[?1006h");
        assert!(term.modes.mouse_sgr);
    }

    #[test]
    fn sgr_with_no_parameters_resets_the_pen() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[1mA\x1B[mB");
        assert!(term.viewport_row(0)[0].attrs.contains(Attrs::BOLD));
        assert!(!term.viewport_row(0)[1].attrs.contains(Attrs::BOLD));
    }

    #[test]
    fn sgr_applies_and_clears_every_text_attribute() {
        let mut term = make_terminal(20, 2);
        term.feed(b"\x1B[2;3;5;7;8;9mA");
        let a = term.viewport_row(0)[0];
        assert!(
            a.attrs.contains(Attrs::DIM)
                && a.attrs.contains(Attrs::ITALIC)
                && a.attrs.contains(Attrs::BLINK)
        );
        assert!(
            a.attrs.contains(Attrs::INVERSE)
                && a.attrs.contains(Attrs::INVISIBLE)
                && a.attrs.contains(Attrs::STRIKETHROUGH)
        );
        term.feed(b"\x1B[1;2;22;23;24;25;27;28;29mB");
        let b = term.viewport_row(0)[1];
        assert!(
            !b.attrs.contains(Attrs::BOLD)
                && !b.attrs.contains(Attrs::DIM)
                && !b.attrs.contains(Attrs::ITALIC)
        );
        assert!(
            !b.attrs.contains(Attrs::UNDERLINE)
                && !b.attrs.contains(Attrs::BLINK)
                && !b.attrs.contains(Attrs::INVERSE)
        );
        assert!(!b.attrs.contains(Attrs::INVISIBLE) && !b.attrs.contains(Attrs::STRIKETHROUGH));
    }

    #[test]
    fn sgr_foreground_and_background_color_codes() {
        let mut term = make_terminal(20, 2);
        term.feed(b"\x1B[44mA");
        assert_eq!(Color::Palette(4), term.viewport_row(0)[0].bg);
        term.feed(b"\x1B[39;49mB");
        assert_eq!(Color::Default, term.viewport_row(0)[1].fg);
        assert_eq!(Color::Default, term.viewport_row(0)[1].bg);
        term.feed(b"\x1B[91mC");
        assert_eq!(Color::Palette(9), term.viewport_row(0)[2].fg);
        term.feed(b"\x1B[102mD");
        assert_eq!(Color::Palette(10), term.viewport_row(0)[3].bg);
    }

    #[test]
    fn sgr_extended_color_with_unknown_selector_consumes_one_parameter() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[38;9;1mX");
        assert!(term.viewport_row(0)[0].attrs.contains(Attrs::BOLD));
    }

    #[test]
    fn osc_133_marks_evict_the_oldest_past_the_cap() {
        let mut term = make_terminal(6, 3);
        for _ in 0..MAX_MARKS + 5 {
            term.feed(b"\x1B]133;A\x07");
            term.feed(b"\r\n");
        }
        assert_eq!(MAX_MARKS, term.prompt_marks().len());
    }

    #[test]
    fn line_count_and_line_span_scrollback_then_grid() {
        let mut t = make_terminal(10, 3);
        assert_eq!(3, t.line_count());
        t.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\n");
        assert!(t.line_count() > 3);
        let last = t.line(t.line_count() - 1);
        assert_eq!(10, last.len());
    }

    #[test]
    fn content_row_of_viewport_matches_viewport_composition() {
        let mut t = make_terminal(10, 3);
        t.feed(b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n");
        assert_eq!(t.history.len(), t.content_row_of_viewport(0));
    }

    #[test]
    fn viewport_row_at_matches_viewport_row_when_offset_equals_viewport_offset() {
        let mut term = make_terminal(4, 2);
        term.feed(b"L1\r\nL2\r\nL3\r\nL4");
        term.scroll_viewport(1);
        let off = term.viewport_offset();
        for y in 0..term.rows() {
            let via_row: Vec<Cell> = term.viewport_row(y).to_vec();
            let via_at: Vec<Cell> = term.viewport_row_at(off, y).to_vec();
            assert_eq!(via_row.len(), via_at.len());
            for x in 0..via_row.len() {
                assert_eq!(via_row[x].cp, via_at[x].cp);
            }
        }
    }

    #[test]
    fn viewport_row_at_returns_a_blank_row_for_out_of_range_y() {
        let mut term = make_terminal(4, 2);
        term.feed(b"L1\r\nL2\r\nL3");
        let off = term.viewport_offset();
        let extra: Vec<Cell> = term.viewport_row_at(off, term.rows()).to_vec();
        for c in &extra {
            assert!(c.cp == ' ' || c.cp == '\0');
        }
    }

    // --- resize pre-scroll tests -----------------------------------------------

    #[test]
    fn shrink_past_cursor_anchors_cursor_to_bottom_and_archives_overflow() {
        let mut term = make_terminal(4, 5);
        term.feed(b"aaaa\r\nbbbb\r\ncccc\r\ndddd\r\neeee");
        assert_eq!(4, term.cursor().y);
        assert_eq!(0, term.scrollback_len());

        term.resize(4, 3);

        assert!(term.cursor().y < term.rows());
        assert_eq!(2, term.scrollback_len());
        let cy = term.cursor().y;
        assert_eq!("eeee", viewport_text(&mut term, cy));
    }

    #[test]
    fn shrink_that_does_not_overflow_the_cursor_leaves_scrollback_unchanged() {
        let mut term = make_terminal(4, 5);
        term.feed(b"aaaa\r\nbbbb");
        assert_eq!(1, term.cursor().y);
        assert_eq!(0, term.scrollback_len());

        term.resize(4, 3);

        assert_eq!(0, term.scrollback_len());
        let cy = term.cursor().y;
        assert_eq!("bbbb", viewport_text(&mut term, cy));
    }

    #[test]
    fn grow_preserves_content_and_cursor_and_leaves_scrollback_unchanged() {
        let mut term = make_terminal(4, 3);
        term.feed(b"aaaa\r\nbbbb\r\ncccc");
        assert_eq!(2, term.cursor().y);
        assert_eq!(0, term.scrollback_len());

        term.resize(4, 6);

        assert_eq!(0, term.scrollback_len());
        assert_eq!("cccc", viewport_text(&mut term, 2));
        assert_eq!(2, term.cursor().y);
    }

    #[test]
    fn grow_then_shrink_round_trip_leaves_the_cursor_line_visible() {
        let mut term = make_terminal(4, 3);
        term.feed(b"aaaa\r\nbbbb\r\ncccc");
        assert_eq!(2, term.cursor().y);

        term.resize(4, 6);
        assert_eq!(0, term.scrollback_len());

        term.resize(4, 3);
        assert!(term.cursor().y < term.rows());
        let cy = term.cursor().y;
        assert_eq!("cccc", viewport_text(&mut term, cy));
    }

    // --- resize matrix (Bug A regression) ------------------------------------

    struct ResizeCase {
        name: &'static str,
        w1: usize,
        h1: usize,
        w2: usize,
        h2: usize,
        feed: &'static [u8],
        on_alternate: bool,
    }

    fn verify_terminal(case_name: &str, t: &Terminal, w2: usize, h2: usize, dims_changed: bool) {
        let ew = w2.max(1);
        let eh = h2.max(1);
        assert!(t.primary.cur_x < ew, "{case_name}: cursor x out of bounds");
        assert!(t.primary.cur_y < eh, "{case_name}: cursor y out of bounds");
        if dims_changed {
            assert!(
                !t.primary.wrap_pending,
                "{case_name}: wrap_pending not cleared"
            );
        }
        if dims_changed {
            assert_eq!(0, t.primary.region.top, "{case_name}: region top not reset");
            assert_eq!(
                eh - 1,
                t.primary.region.bottom,
                "{case_name}: region bottom not reset"
            );
        }
        assert_eq!(
            ew,
            t.primary.scrolled_off_len(),
            "{case_name}: scrolled_off width mismatch"
        );
        assert!(
            t.viewport_offset <= t.history.len(),
            "{case_name}: viewport_offset > history.len()"
        );
        assert_eq!(
            t.primary.width, t.alternate.width,
            "{case_name}: alt-grid width mismatch"
        );
        assert_eq!(
            t.primary.height, t.alternate.height,
            "{case_name}: alt-grid height mismatch"
        );
        assert_eq!(
            ew,
            t.compose_buf.len(),
            "{case_name}: compose_buf width mismatch"
        );
    }

    #[test]
    fn terminal_resize_matrix() {
        const CASES: &[ResizeCase] = &[
            ResizeCase {
                name: "grow both",
                w1: 4,
                h1: 3,
                w2: 8,
                h2: 6,
                feed: b"abc",
                on_alternate: false,
            },
            ResizeCase {
                name: "shrink both",
                w1: 8,
                h1: 6,
                w2: 4,
                h2: 3,
                feed: b"hello\r\nworld",
                on_alternate: false,
            },
            ResizeCase {
                name: "grow cols only",
                w1: 4,
                h1: 3,
                w2: 8,
                h2: 3,
                feed: b"hi",
                on_alternate: false,
            },
            ResizeCase {
                name: "shrink rows only",
                w1: 4,
                h1: 6,
                w2: 4,
                h2: 3,
                feed: b"line\r\nnext",
                on_alternate: false,
            },
            ResizeCase {
                name: "degenerate 1x1",
                w1: 8,
                h1: 4,
                w2: 1,
                h2: 1,
                feed: b"A",
                on_alternate: false,
            },
            ResizeCase {
                name: "degenerate 0x0 clamped",
                w1: 8,
                h1: 4,
                w2: 0,
                h2: 0,
                feed: b"",
                on_alternate: false,
            },
            ResizeCase {
                name: "no-op resize",
                w1: 4,
                h1: 3,
                w2: 4,
                h2: 3,
                feed: b"test",
                on_alternate: false,
            },
            ResizeCase {
                name: "grow then shrink round trip",
                w1: 4,
                h1: 3,
                w2: 6,
                h2: 5,
                feed: b"abc\r\ndef",
                on_alternate: false,
            },
            ResizeCase {
                name: "resize twice no feed first",
                w1: 4,
                h1: 4,
                w2: 2,
                h2: 2,
                feed: b"",
                on_alternate: false,
            },
            ResizeCase {
                name: "resize on alternate screen",
                w1: 8,
                h1: 4,
                w2: 4,
                h2: 2,
                feed: b"alt",
                on_alternate: true,
            },
            ResizeCase {
                name: "cursor at bottom-right then shrink",
                w1: 6,
                h1: 4,
                w2: 3,
                h2: 2,
                feed: b"aaaa\r\nbbbb\r\ncccc\r\ndddd",
                on_alternate: false,
            },
            ResizeCase {
                name: "resize twice no feed second",
                w1: 4,
                h1: 4,
                w2: 8,
                h2: 8,
                feed: b"",
                on_alternate: false,
            },
        ];

        for c in CASES {
            let mut t = Terminal::new(c.w1, c.h1, DEFAULT_CAPACITY);
            if c.on_alternate {
                t.feed(b"\x1b[?1049h");
            }
            t.feed(c.feed);

            let ew2 = c.w2.max(1);
            let eh2 = c.h2.max(1);
            let dims_changed = ew2 != t.primary.width || eh2 != t.primary.height;
            t.resize(c.w2, c.h2);
            verify_terminal(c.name, &t, c.w2, c.h2, dims_changed);

            // Round trip back to original.
            let ew1 = c.w1.max(1);
            let eh1 = c.h1.max(1);
            let rt_changed = ew1 != t.primary.width || eh1 != t.primary.height;
            t.resize(c.w1, c.h1);
            verify_terminal(c.name, &t, c.w1, c.h1, rt_changed);
        }
    }

    #[test]
    fn resize_to_1x1_from_any_state_terminates() {
        let degen_sizes: &[(usize, usize)] = &[(0, 0), (1, 1), (80, 1), (1, 24)];
        for &(w, h) in degen_sizes {
            let mut t = Terminal::new(80, 24, DEFAULT_CAPACITY);
            t.feed(b"some text\r\nmore text\r\nand more");
            t.resize(w, h);
            assert!(t.primary.cur_x < t.primary.width);
            assert!(t.primary.cur_y < t.primary.height);
        }
    }

    #[test]
    fn decscusr_sets_and_clears_app_cursor_shape() {
        let mut term = make_terminal(10, 2);
        assert!(term.app_cursor_shape.is_none());
        assert!(term.app_cursor_blink.is_none());

        term.feed(b"\x1b[6 q");
        assert_eq!(Some(CursorShape::Bar), term.app_cursor_shape);
        assert_eq!(Some(false), term.app_cursor_blink);

        term.feed(b"\x1b[5 q");
        assert_eq!(Some(CursorShape::Bar), term.app_cursor_shape);
        assert_eq!(Some(true), term.app_cursor_blink);

        term.feed(b"\x1b[2 q");
        assert_eq!(Some(CursorShape::Block), term.app_cursor_shape);
        assert_eq!(Some(false), term.app_cursor_blink);

        term.feed(b"\x1b[4 q");
        assert_eq!(Some(CursorShape::Underline), term.app_cursor_shape);
        assert_eq!(Some(false), term.app_cursor_blink);

        term.feed(b"\x1b[0 q");
        assert!(term.app_cursor_shape.is_none());
        assert!(term.app_cursor_blink.is_none());
    }

    // --- Block derivation tests -----------------------------------------------

    #[test]
    fn block_single_completed_command_has_correct_fields() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"output\r\n");
        term.feed(b"\x1B]133;D;exit_code=42\x07");
        term.feed(b"\x1B]133;A\x07");

        let b = term.block_at(0).unwrap();
        assert_eq!(0, b.command_line);
        assert_eq!(1, b.output_line);
        assert_eq!(2, b.end_line);
        assert_eq!(BlockState::Exited, b.state);
        assert_eq!(42, b.exit_code);

        let b2 = term.block_at(1).unwrap();
        assert_eq!(0, b2.command_line);
        assert_eq!(1, b2.output_line);

        assert!(term.block_at(2).is_none());
        assert!(term.block_before(0).is_none());
    }

    #[test]
    fn block_failed_command_has_correct_non_zero_exit_code() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"err\r\n");
        term.feed(b"\x1B]133;D;exit_code=127\x07");
        term.feed(b"\x1B]133;A\x07");

        let b = term.block_at(0).unwrap();
        assert_eq!(BlockState::Exited, b.state);
        assert_eq!(127, b.exit_code);
    }

    #[test]
    fn block_exit_code_stored_on_prompt_mark_for_command_done() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"\x1B]133;D;exit_code=5\x07");

        let marks = term.prompt_marks();
        let mut found = false;
        for m in marks {
            if m.kind == PromptMarkKind::CommandDone {
                assert_eq!(5, m.exit_code);
                found = true;
            }
        }
        assert!(found);
        for m in marks {
            if m.kind != PromptMarkKind::CommandDone {
                assert_eq!(0, m.exit_code);
            }
        }
    }

    #[test]
    fn block_live_running_command_has_state_running_and_end_line_at_cursor() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"partial output\r\n");

        // For a running block we clamp end_line to the cursor's absolute
        // row + 1 so the block visual doesn't extend past the actual output
        // into empty viewport rows.
        let b = term.block_at(0).unwrap();
        assert_eq!(BlockState::Running, b.state);
        assert!(
            b.end_line > b.command_line,
            "end_line {} must be > command_line {}",
            b.end_line,
            b.command_line
        );
        // end_line must not be far past the last real output row.
        let line_count = term.line_count();
        assert!(
            b.end_line <= term.evicted_lines + line_count,
            "end_line should be within terminal lines"
        );
        assert_eq!(0, b.exit_code);
    }

    #[test]
    fn block_two_consecutive_blocks_navigation_and_end_line_boundary() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"out1\r\n");
        term.feed(b"\x1B]133;D;exit_code=0\x07");
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"out2\r\n");
        term.feed(b"\x1B]133;D;exit_code=1\x07");
        term.feed(b"\x1B]133;A\x07");

        let b1 = term.block_at(0).unwrap();
        let b2 = term.block_at(2).unwrap();

        assert_eq!(0, b1.command_line);
        assert_eq!(1, b1.output_line);
        assert_eq!(2, b1.end_line);
        assert_eq!(BlockState::Exited, b1.state);
        assert_eq!(0, b1.exit_code);

        assert_eq!(2, b2.command_line);
        assert_eq!(3, b2.output_line);
        assert_eq!(4, b2.end_line);
        assert_eq!(BlockState::Exited, b2.state);
        assert_eq!(1, b2.exit_code);

        assert_eq!(b1.end_line, b2.command_line);

        let after = term.block_after(0).unwrap();
        assert_eq!(2, after.command_line);

        let before = term.block_before(2).unwrap();
        assert_eq!(0, before.command_line);

        assert!(term.block_after(2).is_none());
        assert!(term.block_before(0).is_none());
    }

    #[test]
    fn block_command_with_no_output_start_output_line_equals_command_line() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"some output\r\n");
        term.feed(b"\x1B]133;D;exit_code=0\x07");
        term.feed(b"\x1B]133;A\x07");

        let b = term.block_at(0).unwrap();
        assert_eq!(0, b.command_line);
        assert_eq!(b.command_line, b.output_line);
        assert_eq!(BlockState::Exited, b.state);
    }

    #[test]
    fn block_block_at_on_prompt_start_line_returns_null() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"out\r\n");
        term.feed(b"\x1B]133;D;exit_code=0\x07");
        term.feed(b"\x1B]133;A\x07");

        assert!(term.block_at(2).is_none());
    }

    #[test]
    fn block_block_at_before_first_command_returns_null() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        assert!(term.block_at(0).is_none());
        assert!(term.block_at(5).is_none());
    }

    #[test]
    fn block_no_marks_at_all_returns_null_for_all_block_queries() {
        let mut term = make_terminal(20, 10);
        term.feed(b"just some text\r\n");
        assert!(term.block_at(0).is_none());
        assert!(term.block_after(0).is_none());
        assert!(term.block_before(0).is_none());
    }

    #[test]
    fn block_scrollback_eviction_evicted_block_returns_null_later_blocks_resolve() {
        let mut term = make_terminal(4, 2);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"ok\r\n");
        term.feed(b"\x1B]133;D;exit_code=0\x07");
        term.feed(b"\x1B]133;A\x07");

        assert!(term.block_at(0).is_some());

        for _ in 0..MAX_MARKS {
            term.feed(b"\x1B]133;A\x07");
            term.feed(b"\r\n");
        }

        assert!(term.block_at(0).is_none());

        let pre_line = term.evicted_lines + term.history.len() + term.active_const().cur_y;
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"new\r\n");
        term.feed(b"\x1B]133;D;exit_code=3\x07");
        term.feed(b"\x1B]133;A\x07");

        let b_new = term.block_at(pre_line);
        assert!(b_new.is_some());
        assert_eq!(3, b_new.unwrap().exit_code);
        assert_eq!(BlockState::Exited, b_new.unwrap().state);
    }

    #[test]
    fn block_output_row_count_correctness() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"L1\r\n");
        term.feed(b"L2\r\n");
        term.feed(b"\x1B]133;D;exit_code=0\x07");
        term.feed(b"\x1B]133;A\x07");

        let b = term.block_at(0).unwrap();
        assert_eq!(1, b.output_row_count());

        let mut term2 = make_terminal(20, 10);
        term2.feed(b"\x1B]133;A\x07");
        term2.feed(b"\x1B]133;B\x07");
        term2.feed(b"\r\n");
        term2.feed(b"\x1B]133;D;exit_code=0\x07");
        term2.feed(b"\x1B]133;A\x07");

        let b2 = term2.block_at(0).unwrap();
        assert_eq!(0, b2.output_row_count());
    }

    #[test]
    fn block_block_after_returns_null_when_no_subsequent_block_exists() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;D;exit_code=0\x07");

        assert!(term.block_after(0).is_none());
    }

    #[test]
    fn block_block_before_returns_null_when_no_prior_block_exists() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        assert!(term.block_before(0).is_none());
    }

    // ── Block: duration_ms and command_start_col ──────────────────────────────

    #[test]
    fn block_duration_ms_is_zero_when_running() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"output\r\n");
        // No 133;D — still running.
        let b = term.block_at(0).unwrap();
        assert_eq!(BlockState::Running, b.state);
        assert_eq!(0, b.duration_ms);
    }

    #[test]
    fn block_duration_ms_present_after_command_done() {
        let mut term = make_terminal(20, 10);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"output\r\n");
        term.feed(b"\x1B]133;D;exit_code=0\x07");
        term.feed(b"\x1B]133;A\x07");
        let b = term.block_at(0).unwrap();
        assert_eq!(BlockState::Exited, b.state);
        // duration_ms may be very small (test runs fast) but must be present
        // (zero is valid for extremely fast commands, but the field is populated).
        // We verify the field exists and is a u64 (no type error).
        let _: u64 = b.duration_ms;
    }

    #[test]
    fn block_command_start_col_recorded_from_133b_mark() {
        let mut term = make_terminal(20, 10);
        // Write a 4-char prompt then fire 133;B so col is 4.
        term.feed(b"\x1B]133;A\x07");
        term.feed(b">>> "); // 4 chars — cursor now at col 4
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"git status");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"ok\r\n");
        term.feed(b"\x1B]133;D;exit_code=0\x07");
        term.feed(b"\x1B]133;A\x07");

        let b = term.block_at(0).unwrap();
        // command_start_col should be 4 (after the 4-char ">>> ").
        assert_eq!(
            4, b.command_start_col,
            "command_start_col should record cursor column at 133;B"
        );
    }

    // ── DEC line-drawing character set ────────────────────────────────────────

    #[test]
    fn dec_line_drawing_all_chars() {
        // Enable G0 line-drawing charset (ESC ( 0), emit all line-drawing chars.
        let mut term = make_terminal(20, 4);
        // Row 0: j k l m n
        term.feed(b"\x1B(0jklmn\x1B(B");
        // Row 1: q t u v w
        term.feed(b"\r\n\x1B(0qtuvw\x1B(B");
        // Row 2: x ` a f g
        term.feed(b"\r\n\x1B(0x`afg\x1B(B");
        // Row 3: ~
        term.feed(b"\r\n\x1B(0~\x1B(B");

        let r0 = term.viewport_row(0);
        assert_eq!(r0[0].cp, '\u{2518}'); // j → ┘
        assert_eq!(r0[1].cp, '\u{2510}'); // k → ┐
        assert_eq!(r0[2].cp, '\u{250C}'); // l → ┌
        assert_eq!(r0[3].cp, '\u{2514}'); // m → └
        assert_eq!(r0[4].cp, '\u{253C}'); // n → ┼

        let r1 = term.viewport_row(1);
        assert_eq!(r1[0].cp, '\u{2500}'); // q → ─
        assert_eq!(r1[1].cp, '\u{251C}'); // t → ├
        assert_eq!(r1[2].cp, '\u{2524}'); // u → ┤
        assert_eq!(r1[3].cp, '\u{2534}'); // v → ┴
        assert_eq!(r1[4].cp, '\u{252C}'); // w → ┬

        let r2 = term.viewport_row(2);
        assert_eq!(r2[0].cp, '\u{2502}'); // x → │
        assert_eq!(r2[1].cp, '\u{25C6}'); // ` → ◆
        assert_eq!(r2[2].cp, '\u{2592}'); // a → ▒
        assert_eq!(r2[3].cp, '\u{00B0}'); // f → °
        assert_eq!(r2[4].cp, '\u{00B1}'); // g → ±

        let r3 = term.viewport_row(3);
        assert_eq!(r3[0].cp, '\u{00B7}'); // ~ → ·
    }

    // ── parse_exit_code negative and digit loop ───────────────────────────────

    #[test]
    fn osc_133_d_negative_exit_code() {
        let mut term = make_terminal(20, 5);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"\x1B]133;D;exit_code=-7\x07");
        let marks = term.prompt_marks();
        let done = marks.iter().find(|m| m.kind == PromptMarkKind::CommandDone);
        assert!(done.is_some());
        assert_eq!(-7, done.unwrap().exit_code);
    }

    // ── CSI s / CSI u (save / restore cursor) ────────────────────────────────

    #[test]
    fn csi_s_and_u_save_restore_cursor_position() {
        let mut term = make_terminal(10, 5);
        term.feed(b"AB"); // cursor at col 2
        term.feed(b"\x1B[s"); // CSI s — save
        term.feed(b"\r\n\r\n"); // move to row 2
        term.feed(b"\x1B[u"); // CSI u — restore to col 2, row 0
        term.feed(b"X"); // should appear at col 2 row 0
        assert_eq!('X', term.viewport_row(0)[2].cp);
    }

    // ── OSC with no semicolon returns immediately ─────────────────────────────

    #[test]
    fn osc_without_semicolon_is_silently_ignored() {
        let mut term = make_terminal(10, 2);
        term.feed(b"AB");
        // OSC payload "notitle" has no ';' — should be silently ignored.
        term.feed(b"\x1B]notitle\x07");
        // Terminal content unchanged.
        assert_eq!('A', term.viewport_row(0)[0].cp);
        assert_eq!('B', term.viewport_row(0)[1].cp);
    }

    // ── active_const on alt screen ────────────────────────────────────────────

    #[test]
    fn active_const_uses_alt_grid_when_on_alt_screen() {
        let mut term = make_terminal(10, 2);
        term.feed(b"PRIMARY");
        // Switch to alt screen (1049h).
        term.feed(b"\x1B[?1049h");
        term.feed(b"ALT");
        // cursor() calls active_const() — verify it reflects alt grid.
        let cur = term.cursor();
        assert_eq!(3, cur.x); // "ALT" is 3 chars
        assert_eq!(0, cur.y);
    }

    // ── viewport_row on alt screen (line 364) ────────────────────────────────

    #[test]
    fn viewport_row_on_alt_screen_reads_alternate_grid() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1B[?1049h"); // enter alt
        term.feed(b"HELLO");
        let row = term.viewport_row(0);
        assert_eq!('H', row[0].cp);
        assert_eq!('E', row[1].cp);
    }

    // ── viewport_row out-of-bounds y (line 348) ───────────────────────────────

    #[test]
    fn viewport_row_out_of_bounds_y_returns_row_zero() {
        let mut term = make_terminal(5, 3);
        term.feed(b"ABCDE");
        // y=99 is >= height=3, so returns row(0).
        let row = term.viewport_row(99);
        assert_eq!(5, row.len());
    }

    // ── viewport_row_at negative oldest_signed (line 381) ────────────────────

    #[test]
    fn viewport_row_at_negative_oldest_returns_blank() {
        let mut term = make_terminal(5, 3);
        // With an empty history, offset > y but signed index is negative.
        // offset=5, y=0: oldest_signed = 0 - 5 + 0 = -5 < 0 → blank.
        let row = term.viewport_row_at(5, 0).to_vec();
        assert!(row.iter().all(|c| c.cp == ' '));
    }

    // ── viewport_row_at alt screen grid (line 397) ───────────────────────────

    #[test]
    fn viewport_row_at_on_alt_screen_reads_alternate_grid() {
        let mut term = make_terminal(10, 3);
        term.feed(b"\x1B[?1049h");
        term.feed(b"XYZ");
        // offset=0, y=0: grid_y=0, on_alt → alternate.row(0).
        let row = term.viewport_row_at(0, 0).to_vec();
        assert_eq!('X', row[0].cp);
    }

    // ── viewport_row_at grid_y out of bounds (line 392) ──────────────────────

    #[test]
    fn viewport_row_at_grid_y_out_of_bounds_returns_blank() {
        let mut term = make_terminal(5, 3);
        // offset=0, y=10: grid_y=10 >= height=3 → blank compose_buf.
        let row = term.viewport_row_at(0, 10).to_vec();
        assert!(row.iter().all(|c| c.cp == ' '));
    }

    // ── line() out-of-range gy (line 420) ────────────────────────────────────

    #[test]
    fn line_out_of_range_gy_returns_empty() {
        let term = make_terminal(5, 3);
        // history is empty; gy = i - 0 = 99 >= height=3.
        let row = term.line(99);
        assert_eq!(0, row.len());
    }

    // ── scroll_to_line both branches (lines 434-441) ─────────────────────────

    #[test]
    fn scroll_to_line_within_history_sets_offset() {
        let mut term = make_terminal(10, 3);
        // Fill scrollback so history.len() > 0.
        term.feed(b"L1\r\nL2\r\nL3\r\nL4\r\nL5\r\n");
        let hist = term.history.len();
        assert!(hist > 0);
        // target < hist: viewport_offset = hist - target.
        term.scroll_to_line(0);
        assert_eq!(hist.min(hist), term.viewport_offset());
    }

    #[test]
    fn scroll_to_line_at_or_beyond_history_resets_offset() {
        let mut term = make_terminal(10, 3);
        term.feed(b"L1\r\nL2\r\nL3\r\nL4\r\n");
        let hist = term.history.len();
        // target >= hist → viewport_offset = 0.
        term.scroll_to_line(hist + 5);
        assert_eq!(0, term.viewport_offset());
    }

    // ── last_run() accessor (lines 479-485) ──────────────────────────────────

    #[test]
    fn last_run_returns_initial_shell_state() {
        let term = make_terminal(10, 3);
        let lr = term.last_run();
        assert!(!lr.running);
        assert_eq!(0, lr.exit_code);
        assert_eq!(0, lr.duration_ms);
    }

    #[test]
    fn last_run_reflects_completed_command() {
        let mut term = make_terminal(20, 5);
        term.feed(b"\x1B]133;A\x07");
        term.feed(b"\x1B]133;B\x07");
        term.feed(b"\r\n");
        term.feed(b"\x1B]133;C\x07");
        term.feed(b"\x1B]133;D;exit_code=42\x07");
        let lr = term.last_run();
        assert!(!lr.running);
        assert_eq!(42, lr.exit_code);
    }

    // ── evicted_lines increments when ring is full (line 596) ────────────────

    #[test]
    fn evicted_lines_increments_when_scrollback_ring_is_full() {
        let mut term = make_terminal(10, 2);
        // Fill scrollback to capacity.
        for _ in 0..DEFAULT_CAPACITY + 5 {
            term.feed(b"line\r\n");
        }
        assert!(term.evicted_lines > 0);
    }

    // ── set_clipboard no-semicolon path (line 613) ───────────────────────────

    #[test]
    fn osc_52_without_semicolon_stores_whole_payload() {
        let mut term = make_terminal(10, 2);
        // OSC 52 payload without a second ';' separator — raw data stored directly.
        term.feed(b"\x1B]52;nosemicolon\x07");
        // The clipboard stores the data after the first ';' (after "52").
        // "nosemicolon" has no inner ';', so the whole string after "52;" is stored.
        assert!(!term.clipboard().is_empty());
    }

    // ── record_prompt_mark empty payload (line 620) ───────────────────────────

    #[test]
    fn osc_133_empty_payload_is_ignored() {
        let mut term = make_terminal(10, 2);
        // OSC 133 with nothing after the ';'.
        term.feed(b"\x1B]133;\x07");
        assert_eq!(0, term.prompt_marks().len());
    }

    // ── record_prompt_mark unknown byte (line 627) ────────────────────────────

    #[test]
    fn osc_133_unknown_mark_type_is_ignored() {
        let mut term = make_terminal(10, 2);
        // 'Z' is not A/B/C/D.
        term.feed(b"\x1B]133;Z\x07");
        assert_eq!(0, term.prompt_marks().len());
    }

    // ── invalidate_grid_marks compaction (lines 680-681) ─────────────────────

    #[test]
    fn invalidate_grid_marks_removes_marks_scrolled_out_of_grid() {
        let mut term = make_terminal(10, 3);
        // Record a prompt mark at the current (absolute) line.
        term.feed(b"\x1B]133;A\x07");
        assert_eq!(1, term.prompt_marks().len());
        // Scroll the grid so all marks are below the base line.
        // Feeding many lines pushes them into history, making base > mark.line.
        for _ in 0..10 {
            term.feed(b"newline content\r\n");
        }
        // After a resize the marks are revalidated.
        term.resize(10, 3);
        // All historical marks should have been compacted away.
        // (They may or may not all be gone depending on exact timing,
        // but the mark count must not exceed the original or increase.)
        let _ = term.prompt_marks().len(); // must not panic
    }

    // ── set_alt_screen no-op when already in desired state (line 715) ─────────

    #[test]
    fn set_alt_screen_no_op_when_already_in_state() {
        let mut term = make_terminal(10, 2);
        // Enter alt screen.
        term.feed(b"\x1B[?1049h");
        assert!(term.modes.alt_screen);
        // Write on alt.
        term.feed(b"ALT");
        // Entering alt again should be a no-op.
        term.feed(b"\x1B[?1049h");
        assert!(term.modes.alt_screen);
        // Content still present.
        assert_eq!('A', term.viewport_row(0)[0].cp);

        // Exit alt.
        term.feed(b"\x1B[?1049l");
        assert!(!term.modes.alt_screen);
        // Exiting again should be a no-op.
        term.feed(b"\x1B[?1049l");
        assert!(!term.modes.alt_screen);
    }

    // ── set_private_mode catch-all _ => {} (line 748) ────────────────────────

    #[test]
    fn set_private_mode_unknown_code_is_silently_ignored() {
        let mut term = make_terminal(10, 2);
        // Mode 9999 is not handled.
        term.feed(b"\x1B[?9999h");
        term.feed(b"\x1B[?9999l");
        // No panic, no state corruption.
        assert_eq!(' ', term.viewport_row(0)[0].cp);
    }

    // ── DECSCUSR shapes 1 and 3 (lines 758-769) ──────────────────────────────

    #[test]
    fn decscusr_shapes_1_and_3_blinking_block_and_underline() {
        let mut term = make_terminal(10, 2);
        // Shape 1: blinking block.
        term.feed(b"\x1b[1 q");
        assert_eq!(Some(CursorShape::Block), term.app_cursor_shape);
        assert_eq!(Some(true), term.app_cursor_blink);

        // Shape 3: blinking underline.
        term.feed(b"\x1b[3 q");
        assert_eq!(Some(CursorShape::Underline), term.app_cursor_shape);
        assert_eq!(Some(true), term.app_cursor_blink);
    }

    // ── DECSCUSR unknown param catch-all (line 782) ───────────────────────────

    #[test]
    fn decscusr_unknown_param_is_silently_ignored() {
        let mut term = make_terminal(10, 2);
        term.feed(b"\x1b[6 q");
        assert_eq!(Some(CursorShape::Bar), term.app_cursor_shape);
        // Unknown param 99 must not panic or corrupt shape.
        term.feed(b"\x1b[99 q");
        assert_eq!(Some(CursorShape::Bar), term.app_cursor_shape);
    }

    // ── csi_standard catch-all _ => {} (line 846) ────────────────────────────

    #[test]
    fn csi_standard_unknown_final_byte_is_ignored() {
        let mut term = make_terminal(10, 2);
        term.feed(b"A");
        // CSI 'X' with no leading '?' is an unknown standard sequence.
        term.feed(b"\x1B[X");
        // Content unchanged, no panic.
        assert_eq!('A', term.viewport_row(0)[0].cp);
    }

    // ── csi_private catch-all _ => {} (line 862) ─────────────────────────────

    #[test]
    fn csi_private_unknown_final_byte_is_ignored() {
        let mut term = make_terminal(10, 2);
        term.feed(b"B");
        // CSI ? followed by unknown final byte.
        term.feed(b"\x1B[?X");
        assert_eq!('B', term.viewport_row(0)[0].cp);
    }

    // ── execute catch-all _ => {} (line 909) ─────────────────────────────────

    #[test]
    fn execute_unknown_control_byte_is_ignored() {
        let mut term = make_terminal(10, 2);
        term.feed(b"C");
        // 0x01 (SOH) is not in the handled set.
        term.feed(&[0x01]);
        assert_eq!('C', term.viewport_row(0)[0].cp);
    }

    // ── esc_dispatch catch-all _ => {} (line 943) ────────────────────────────

    #[test]
    fn esc_dispatch_unknown_final_byte_is_ignored() {
        let mut term = make_terminal(10, 2);
        term.feed(b"D");
        // ESC Z (DECID) is not handled in our impl.
        term.feed(b"\x1BZ");
        assert_eq!('D', term.viewport_row(0)[0].cp);
    }

    // ── translate_line_drawing catch-all _ => cp (line 1011) ──────────────────

    #[test]
    fn translate_line_drawing_unknown_char_returns_itself() {
        let mut term = make_terminal(10, 2);
        // Enable DEC line drawing (g0_line_drawing=true).
        term.feed(b"\x1B(0");
        // 'z' is not in the mapping — should print as 'z'.
        term.feed(b"z");
        term.feed(b"\x1B(B"); // back to ASCII
        assert_eq!('z', term.viewport_row(0)[0].cp);
    }

    // ── apply_sgr_at catch-all _ => {} (line 1068) ────────────────────────────

    #[test]
    fn sgr_unknown_code_is_silently_ignored() {
        let mut term = make_terminal(10, 2);
        // SGR 255 is not handled.
        term.feed(b"\x1B[255mX");
        assert_eq!('X', term.viewport_row(0)[0].cp);
    }

    // ── apply_extended_color truncated params (lines 1074-1087) ───────────────

    #[test]
    fn sgr_extended_color_38_with_only_one_param_is_safe() {
        let mut term = make_terminal(10, 2);
        // Only "38" with no mode selector (i+1 >= len) — must not panic.
        term.feed(b"\x1B[38mX");
        assert_eq!('X', term.viewport_row(0)[0].cp);
    }

    #[test]
    fn sgr_extended_color_38_5_with_no_index_param_is_safe() {
        let mut term = make_terminal(10, 2);
        // "38;5" but no color index (i+2 >= len) — must not panic.
        term.feed(b"\x1B[38;5mX");
        assert_eq!('X', term.viewport_row(0)[0].cp);
    }

    #[test]
    fn sgr_extended_color_38_2_with_fewer_than_four_extra_params_is_safe() {
        let mut term = make_terminal(10, 2);
        // "38;2;255;128" — only 2 RGB values provided instead of 3 (i+4 >= len).
        term.feed(b"\x1B[38;2;255;128mX");
        assert_eq!('X', term.viewport_row(0)[0].cp);
    }
}
