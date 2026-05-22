//! The terminal model: the public face of the terminal core.
//!
//! A `Terminal` owns a primary grid, an alternate grid, the scrollback ring,
//! and a VT/ANSI parser. It implements the parser's handler interface,
//! translating parsed events into grid mutations. It also tracks a render
//! viewport that can be scrolled up into scrollback history.
//!
//! Pure Zig — no platform dependencies. The renderer reads `viewportRow`,
//! `cursor`, `cols`/`rows`, the window title, and the OSC-133 marks.

const std = @import("std");
const cell = @import("cell.zig");
const grid = @import("grid.zig");
const scrollback = @import("scrollback.zig");
const parser = @import("parser.zig");

pub const Cell = cell.Cell;
pub const Color = cell.Color;
pub const Attrs = cell.Attrs;

/// Cursor shape, as requested by the app via DECSCUSR.
pub const CursorShape = enum { block, underline, bar };

/// The cursor position and visibility, as the renderer needs it.
pub const Cursor = struct {
    x: usize,
    y: usize,
    visible: bool,
};

/// Monotonic clock in milliseconds. Used for shell-integration timing.
fn monoMs() i64 {
    var ts: std.c.timespec = undefined;
    _ = std.c.clock_gettime(std.c.CLOCK.MONOTONIC, &ts);
    return @as(i64, ts.sec) * 1000 + @divTrunc(ts.nsec, 1_000_000);
}

/// An OSC-133 semantic prompt mark, keyed to an absolute output line so the
/// command and output regions of the session are machine-identifiable.
pub const PromptMark = struct {
    /// Which `OSC 133` sub-command produced this mark.
    kind: enum { prompt_start, command_start, output_start, command_done },
    /// The absolute line index: scrollback rows ever evicted + scrollback
    /// length + the cursor's grid row at the time the mark was emitted.
    /// Monotonic for the session; survives scrollback eviction.
    line: usize,
};

/// DEC private modes the terminal records. Only modes that affect the model
/// are acted on; the rest are stored as flags for input handling / the host.
pub const PrivateModes = struct {
    autowrap: bool = true,
    cursor_visible: bool = true,
    alt_screen: bool = false,
    bracketed_paste: bool = false,
    /// ?1 application cursor keys.
    app_cursor_keys: bool = false,
    /// ?1000 / ?1002 / ?1006 mouse reporting flags.
    mouse_x10: bool = false,
    mouse_button: bool = false,
    mouse_sgr: bool = false,
};

/// Upper bound on retained OSC-133 marks. The session can outlive this; the
/// oldest marks are dropped first (they describe long-evicted history).
const max_marks = 4096;

pub const Terminal = struct {
    alloc: std.mem.Allocator,

    primary: grid.Grid,
    alternate: grid.Grid,
    /// True when the alternate grid is the active one. Modeled as a flag —
    /// not a self-pointer — so a `Terminal` value can be moved/copied freely.
    on_alt: bool = false,

    history: scrollback.Scrollback,

    parser: parser.Parser,

    /// 0 = viewport pinned to the live bottom; >0 = scrolled up into history.
    viewport_offset: usize = 0,

    modes: PrivateModes = .{},

    /// G0 charset selection — true selects the DEC special line-drawing set.
    g0_line_drawing: bool = false,

    /// Window title from OSC 0 / OSC 2.
    title_buf: [256]u8 = undefined,
    title_len: usize = 0,

    /// Working directory from OSC 7.
    cwd_buf: [1024]u8 = undefined,
    cwd_len: usize = 0,

    /// Clipboard payload from OSC 52.
    clipboard_buf: [4096]u8 = undefined,
    clipboard_len: usize = 0,

    /// OSC-133 semantic prompt marks, oldest first.
    marks: [max_marks]PromptMark = undefined,
    mark_count: usize = 0,
    /// Count of scrollback rows evicted over the session's lifetime, so
    /// absolute line numbers in `marks` stay stable as history scrolls away.
    evicted_lines: usize = 0,

    /// Shell integration: last-run outcome. Updated by OSC 133;C (command
    /// starts) and 133;D (command ends, optionally carrying an exit code).
    shell_running: bool = false, // true between 133;C and 133;D
    shell_run_start_ms: i64 = 0, // milliTimestamp when 133;C was received
    shell_last_exit: i32 = 0, // exit code from most recent 133;D
    shell_last_duration_ms: i64 = 0, // duration of the most recent run in ms

    /// App-requested cursor style from DECSCUSR. Null = use the config default.
    app_cursor_shape: ?CursorShape = null,
    /// App-requested cursor blink from DECSCUSR. Null = use the config default.
    app_cursor_blink: ?bool = null,

    /// Reusable buffer backing `viewportRow` when it pads a short scrollback
    /// row — one row wide, reallocated on resize.
    compose_buf: []Cell,

    /// Create a terminal with a `width x height` screen and a scrollback ring
    /// of `scrollback_capacity` rows.
    pub fn init(alloc: std.mem.Allocator, width: usize, height: usize, scrollback_capacity: usize) !Terminal {
        var primary = try grid.Grid.init(alloc, width, height);
        errdefer primary.deinit();
        var alternate = try grid.Grid.init(alloc, width, height);
        errdefer alternate.deinit();
        var history = try scrollback.Scrollback.init(alloc, scrollback_capacity);
        errdefer history.deinit();
        const compose_buf = try alloc.alloc(Cell, primary.width);
        errdefer alloc.free(compose_buf);

        return Terminal{
            .alloc = alloc,
            .primary = primary,
            .alternate = alternate,
            .history = history,
            .parser = parser.Parser.init(),
            .compose_buf = compose_buf,
        };
    }

    pub fn deinit(self: *Terminal) void {
        self.primary.deinit();
        self.alternate.deinit();
        self.history.deinit();
        self.alloc.free(self.compose_buf);
        self.* = undefined;
    }

    /// The grid currently being drawn to — `primary` or `alternate`.
    fn active(self: *Terminal) *grid.Grid {
        return if (self.on_alt) &self.alternate else &self.primary;
    }

    fn activeConst(self: *const Terminal) *const grid.Grid {
        return if (self.on_alt) &self.alternate else &self.primary;
    }

    // --- dimensions --------------------------------------------------------

    pub fn cols(self: *const Terminal) usize {
        return self.primary.width;
    }

    pub fn rows(self: *const Terminal) usize {
        return self.primary.height;
    }

    pub fn cursor(self: *const Terminal) Cursor {
        const g = self.activeConst();
        return .{
            .x = g.cur_x,
            .y = g.cur_y,
            .visible = self.modes.cursor_visible,
        };
    }

    // --- feeding bytes -----------------------------------------------------

    /// Parse `bytes` and apply them to the active grid. Any new output pins
    /// the viewport back to the live bottom.
    pub fn feed(self: *Terminal, bytes: []const u8) void {
        if (bytes.len == 0) return;
        self.parser.feed(self, bytes);
        self.viewport_offset = 0;
    }

    // --- resize ------------------------------------------------------------

    /// Resize both grids to `cols x rows`. When shrinking past the cursor,
    /// pre-scroll the primary so the cursor stays anchored to the bottom;
    /// displaced rows are archived into scrollback exactly as a normal line
    /// feed would do.
    pub fn resize(self: *Terminal, new_cols: usize, new_rows: usize) void {
        const w = @max(new_cols, 1);
        const h = @max(new_rows, 1);

        // Pre-scroll the primary grid when the new height cannot fit the
        // cursor's current row.  Use a full-screen region for the scroll so
        // any active DECSTBM sub-region does not interfere; resize resets the
        // region anyway.
        if (h < self.primary.cur_y + 1) {
            const to_scroll = (self.primary.cur_y + 1) - h;
            self.primary.region = .{ .top = 0, .bottom = self.primary.height - 1 };
            var i: usize = 0;
            while (i < to_scroll) : (i += 1) {
                if (self.primary.scrollUp(1)) |displaced| self.archive(displaced);
                self.primary.cur_y -= 1;
            }
        }

        self.primary.resize(w, h);
        self.alternate.resize(w, h);
        if (self.compose_buf.len != w) {
            const fresh = self.alloc.alloc(Cell, w) catch return;
            self.alloc.free(self.compose_buf);
            self.compose_buf = fresh;
        }
        self.viewport_offset = @min(self.viewport_offset, self.history.len());
    }

    // --- viewport ----------------------------------------------------------

    pub fn scrollbackLen(self: *const Terminal) usize {
        return self.history.len();
    }

    pub fn viewportOffset(self: *const Terminal) usize {
        return self.viewport_offset;
    }

    /// Scroll the viewport by `delta` rows: positive moves up into history,
    /// negative moves down toward the live bottom. Clamped to valid range.
    pub fn scrollViewport(self: *Terminal, delta: isize) void {
        const max_offset = self.history.len();
        if (delta >= 0) {
            const up: usize = @intCast(delta);
            self.viewport_offset = @min(self.viewport_offset + up, max_offset);
        } else {
            const down: usize = @intCast(-delta);
            self.viewport_offset = if (self.viewport_offset > down)
                self.viewport_offset - down
            else
                0;
        }
    }

    pub fn scrollToBottom(self: *Terminal) void {
        self.viewport_offset = 0;
    }

    /// Set the viewport offset to an absolute row count, clamped to history.
    pub fn setViewportOffset(self: *Terminal, offset: usize) void {
        self.viewport_offset = @min(offset, self.history.len());
    }

    /// The row at viewport position `y` (0 = top of the viewport), always
    /// exactly `cols` cells wide. When the viewport is scrolled up, the top
    /// rows come from scrollback (short rows padded into `compose_buf`); the
    /// rest come from the active grid.
    pub fn viewportRow(self: *Terminal, y: usize) []const Cell {
        const g = self.active();
        if (y >= g.height) return g.rowConst(0);

        if (self.viewport_offset > y) {
            // This row is sourced from scrollback. The newest scrollback row
            // sits just above the grid's top.
            const from_oldest = self.history.len() - self.viewport_offset + y;
            const src = self.history.get(from_oldest);
            @memset(self.compose_buf, Cell{});
            const n = @min(src.len, g.width);
            @memcpy(self.compose_buf[0..n], src[0..n]);
            return self.compose_buf;
        }

        // Sourced from the active grid, shifted up by the remaining offset.
        const grid_y = y - self.viewport_offset;
        return g.rowConst(grid_y);
    }

    /// Like `viewportRow`, but for an explicit `offset` and a `y` that may equal
    /// `rows()` (one row past the viewport bottom). Used by smooth scrolling to
    /// render an in-between offset. Out-of-range rows return a blank row.
    pub fn viewportRowAt(self: *Terminal, offset: usize, y: usize) []const Cell {
        const g = self.active();
        if (offset > y) {
            const oldest = @as(isize, @intCast(self.history.len())) -
                @as(isize, @intCast(offset)) + @as(isize, @intCast(y));
            @memset(self.compose_buf, Cell{});
            if (oldest < 0) return self.compose_buf;
            const src = self.history.get(@intCast(oldest));
            const n = @min(src.len, g.width);
            @memcpy(self.compose_buf[0..n], src[0..n]);
            return self.compose_buf;
        }
        const grid_y = y - offset;
        if (grid_y >= g.height) {
            @memset(self.compose_buf, Cell{});
            return self.compose_buf;
        }
        return g.rowConst(grid_y);
    }

    // --- content-line access (for search) ---------------------------------

    /// Total content rows: scrollback length + active grid height.
    pub fn lineCount(self: *const Terminal) usize {
        return self.history.len() + self.activeConst().height;
    }

    /// Borrow content row `i`: 0..history.len() are scrollback rows (possibly
    /// shorter than grid width); the rest are active-grid rows (full width).
    /// Out-of-range returns an empty slice.
    pub fn line(self: *const Terminal, i: usize) []const Cell {
        const hist = self.history.len();
        if (i < hist) return self.history.get(i);
        const g = self.activeConst();
        const gy = i - hist;
        if (gy >= g.height) return &.{};
        return g.rowConst(gy);
    }

    /// The content row currently shown at viewport position `y`.
    pub fn contentRowOfViewport(self: *const Terminal, y: usize) usize {
        if (self.viewport_offset > y)
            return self.history.len() - self.viewport_offset + y;
        return self.history.len() + (y - self.viewport_offset);
    }

    /// Scroll the viewport so content row `target` is visible. Grid rows are
    /// always visible at offset 0; a scrollback row is brought near the top.
    pub fn scrollToLine(self: *Terminal, target: usize) void {
        const hist = self.history.len();
        if (target >= hist) {
            self.viewport_offset = 0;
        } else {
            self.viewport_offset = @min(hist - target, hist);
        }
    }

    // --- title / cwd / clipboard accessors ---------------------------------

    pub fn title(self: *const Terminal) []const u8 {
        return self.title_buf[0..self.title_len];
    }

    pub fn cwd(self: *const Terminal) []const u8 {
        return self.cwd_buf[0..self.cwd_len];
    }

    /// Return the filesystem path from the stored OSC 7 value.
    /// If the value starts with `file://`, the host component is stripped and
    /// the path from the first `/` after `file://` is returned.
    ///   "file:///home/dev"     -> "/home/dev"
    ///   "file://myhost/tmp"    -> "/tmp"
    ///   "file://"  (no path)   -> ""
    /// Anything else (bare path or empty) is returned as-is.
    /// Returns a sub-slice of `cwd_buf` — no allocation.
    pub fn cwdPath(self: *const Terminal) []const u8 {
        const raw = self.cwd_buf[0..self.cwd_len];
        const prefix = "file://";
        if (std.mem.startsWith(u8, raw, prefix)) {
            const after_prefix = raw[prefix.len..];
            // Find the first '/' — everything from there onward is the path.
            if (std.mem.indexOfScalar(u8, after_prefix, '/')) |slash| {
                return after_prefix[slash..];
            }
            return ""; // "file://" with no path component
        }
        return raw;
    }

    pub fn clipboard(self: *const Terminal) []const u8 {
        return self.clipboard_buf[0..self.clipboard_len];
    }

    /// The OSC-133 semantic prompt marks recorded so far, oldest first.
    pub fn promptMarks(self: *const Terminal) []const PromptMark {
        return self.marks[0..self.mark_count];
    }

    // === parser handler ====================================================
    // The methods below satisfy the `anytype` handler contract of `Parser`.

    /// A printable Unicode scalar. Applies G0 line-drawing translation.
    pub fn print(self: *Terminal, cp: u21) void {
        self.active().print(if (self.g0_line_drawing) translateLineDrawing(cp) else cp);
    }

    /// A C0/C1 control byte.
    pub fn execute(self: *Terminal, byte: u8) void {
        switch (byte) {
            0x07 => {}, // BEL — no audible bell in the model
            0x08 => self.active().backspace(), // BS
            0x09 => self.active().tab(), // HT
            0x0A, 0x0B, 0x0C => self.lineFeed(), // LF, VT, FF
            0x0D => self.active().carriageReturn(), // CR
            else => {},
        }
    }

    /// Apply a line feed, archiving any scrolled-off primary row.
    fn lineFeed(self: *Terminal) void {
        const scrolled = self.active().lineFeed();
        if (scrolled) |row| {
            if (!self.on_alt) self.archive(row);
        }
    }

    /// Push a scrolled-off row into scrollback, tracking eviction so absolute
    /// line numbers stay stable.
    fn archive(self: *Terminal, row: []const Cell) void {
        const was_full = self.history.len() == self.history.capacity();
        self.history.push(row);
        if (was_full) self.evicted_lines += 1;
    }

    /// A CSI sequence: `intermediates` may carry a private marker (`?` etc.).
    pub fn csiDispatch(self: *Terminal, intermediates: []const u8, params: []const u16, final: u8) void {
        // DECSCUSR: CSI Ps SP q — app-requested cursor style.
        if (intermediates.len == 1 and intermediates[0] == ' ' and final == 'q') {
            self.applyDecscusr(param(params, 0, 0));
            return;
        }
        const private = intermediates.len > 0 and intermediates[0] == '?';
        if (private) {
            self.csiPrivate(params, final);
            return;
        }
        self.csiStandard(params, final);
    }

    /// Handle DECSCUSR (CSI Ps SP q). Ps:
    ///   0 or 1 = blinking block (reset to default when 0)
    ///   2 = steady block
    ///   3 = blinking underline
    ///   4 = steady underline
    ///   5 = blinking bar
    ///   6 = steady bar
    fn applyDecscusr(self: *Terminal, ps: u16) void {
        switch (ps) {
            0 => { // reset to config default
                self.app_cursor_shape = null;
                self.app_cursor_blink = null;
            },
            1 => {
                self.app_cursor_shape = .block;
                self.app_cursor_blink = true;
            },
            2 => {
                self.app_cursor_shape = .block;
                self.app_cursor_blink = false;
            },
            3 => {
                self.app_cursor_shape = .underline;
                self.app_cursor_blink = true;
            },
            4 => {
                self.app_cursor_shape = .underline;
                self.app_cursor_blink = false;
            },
            5 => {
                self.app_cursor_shape = .bar;
                self.app_cursor_blink = true;
            },
            6 => {
                self.app_cursor_shape = .bar;
                self.app_cursor_blink = false;
            },
            else => {}, // unknown — ignore
        }
    }

    fn csiStandard(self: *Terminal, params: []const u16, final: u8) void {
        const g = self.active();
        const p0 = param(params, 0, 1);
        switch (final) {
            'A' => g.cursorUp(p0),
            'B', 'e' => g.cursorDown(p0),
            'C', 'a' => g.cursorForward(p0),
            'D' => g.cursorBack(p0),
            'E' => { // CNL — cursor next line
                g.carriageReturn();
                g.cursorDown(p0);
            },
            'F' => { // CPL — cursor previous line
                g.carriageReturn();
                g.cursorUp(p0);
            },
            'G', '`' => g.cursorToColumn(oneBased(param(params, 0, 1))),
            'd' => g.cursorToRow(oneBased(param(params, 0, 1))),
            'H', 'f' => g.cursorTo(
                oneBased(param(params, 1, 1)),
                oneBased(param(params, 0, 1)),
            ),
            'J' => {
                const mode = param(params, 0, 0);
                g.eraseDisplay(mode);
                // Erasing the whole display (`clear`) destroys the content the
                // grid-region prompt marks described — drop them, or stale
                // separators litter the blanked screen.
                if ((mode == 2 or mode == 3) and !self.on_alt) self.invalidateGridMarks();
            },
            'K' => g.eraseLine(param(params, 0, 0)),
            '@' => g.insertChars(p0),
            'P' => g.deleteChars(p0),
            'L' => g.insertLines(p0),
            'M' => g.deleteLines(p0),
            'X' => g.eraseChars(p0),
            'S' => _ = g.scrollUp(p0),
            'T' => g.scrollDown(p0),
            'r' => g.setScrollRegion(param(params, 0, 0), param(params, 1, 0)),
            'm' => self.applySgr(params),
            'h' => self.setStandardModes(params, true),
            'l' => self.setStandardModes(params, false),
            's' => g.saveCursor(),
            'u' => g.restoreCursor(),
            else => {},
        }
    }

    fn csiPrivate(self: *Terminal, params: []const u16, final: u8) void {
        switch (final) {
            'h' => for (params) |p| self.setPrivateMode(p, true),
            'l' => for (params) |p| self.setPrivateMode(p, false),
            else => {},
        }
    }

    /// An ESC sequence (no CSI). `intermediates` carries `(`/`)` charset
    /// designators and similar.
    pub fn escDispatch(self: *Terminal, intermediates: []const u8, final: u8) void {
        if (intermediates.len > 0 and intermediates[0] == '(') {
            // G0 charset: `0` = DEC line drawing, `B` = ASCII.
            self.g0_line_drawing = (final == '0');
            return;
        }
        switch (final) {
            '7' => self.active().saveCursor(), // DECSC
            '8' => self.active().restoreCursor(), // DECRC
            'c' => self.reset(), // RIS
            'D' => self.lineFeed(), // IND — index
            'M' => self.reverseIndex(), // RI — reverse index
            'E' => { // NEL — next line
                self.active().carriageReturn();
                self.lineFeed();
            },
            else => {},
        }
    }

    /// An OSC string: `code;payload`.
    pub fn oscDispatch(self: *Terminal, data: []const u8) void {
        const semi = std.mem.indexOfScalar(u8, data, ';') orelse {
            // OSC with no payload separator — ignore.
            return;
        };
        const code = data[0..semi];
        const payload = data[semi + 1 ..];

        if (eql(code, "0") or eql(code, "2")) {
            self.setTitle(payload);
        } else if (eql(code, "7")) {
            self.setCwd(payload);
        } else if (eql(code, "52")) {
            self.setClipboard(payload);
        } else if (eql(code, "133")) {
            self.recordPromptMark(payload);
        }
    }

    // --- mode handling -----------------------------------------------------

    fn setStandardModes(self: *Terminal, params: []const u16, on: bool) void {
        for (params) |p| {
            // SM/RM mode 4 = insert/replace.
            if (p == 4) self.active().modes.insert = on;
        }
    }

    fn setPrivateMode(self: *Terminal, mode: u16, on: bool) void {
        switch (mode) {
            1 => self.modes.app_cursor_keys = on,
            7 => {
                self.modes.autowrap = on;
                self.primary.modes.autowrap = on;
                self.alternate.modes.autowrap = on;
            },
            25 => {
                self.modes.cursor_visible = on;
                self.primary.modes.cursor_visible = on;
                self.alternate.modes.cursor_visible = on;
            },
            1000 => self.modes.mouse_button = on,
            1002 => self.modes.mouse_button = on,
            1006 => self.modes.mouse_sgr = on,
            2004 => self.modes.bracketed_paste = on,
            1049 => self.setAltScreen(on),
            else => {},
        }
    }

    /// Enter (`on`) or leave the alternate screen. Entering saves the primary
    /// cursor and clears the alt grid; it never feeds scrollback. Leaving
    /// restores the primary cursor.
    fn setAltScreen(self: *Terminal, on: bool) void {
        if (on == self.modes.alt_screen) return;
        if (on) {
            self.primary.saveCursor();
            self.alternate.cursorTo(0, 0);
            self.alternate.eraseDisplay(2);
            self.on_alt = true;
            self.modes.alt_screen = true;
        } else {
            self.on_alt = false;
            self.primary.restoreCursor();
            self.modes.alt_screen = false;
        }
    }

    // --- SGR ---------------------------------------------------------------

    /// Apply a Select Graphic Rendition sequence to the active grid's pen.
    fn applySgr(self: *Terminal, params: []const u16) void {
        const pen = &self.active().pen;
        if (params.len == 0) {
            resetPen(pen);
            return;
        }
        var i: usize = 0;
        while (i < params.len) : (i += 1) {
            i += applySgrAt(pen, params, i);
        }
    }

    /// Apply the SGR code at `params[i]`, returning how many *extra* params
    /// were consumed (for 38/48 extended-color sequences).
    fn applySgrAt(pen: *Cell, params: []const u16, i: usize) usize {
        switch (params[i]) {
            0 => resetPen(pen),
            1 => pen.attrs.bold = true,
            2 => pen.attrs.dim = true,
            3 => pen.attrs.italic = true,
            4 => pen.attrs.underline = true,
            5 => pen.attrs.blink = true,
            7 => pen.attrs.inverse = true,
            8 => pen.attrs.invisible = true,
            9 => pen.attrs.strikethrough = true,
            21, 22 => {
                pen.attrs.bold = false;
                pen.attrs.dim = false;
            },
            23 => pen.attrs.italic = false,
            24 => pen.attrs.underline = false,
            25 => pen.attrs.blink = false,
            27 => pen.attrs.inverse = false,
            28 => pen.attrs.invisible = false,
            29 => pen.attrs.strikethrough = false,
            30...37 => pen.fg = .{ .palette = @intCast(params[i] - 30) },
            39 => pen.fg = .default,
            40...47 => pen.bg = .{ .palette = @intCast(params[i] - 40) },
            49 => pen.bg = .default,
            90...97 => pen.fg = .{ .palette = @intCast(params[i] - 90 + 8) },
            100...107 => pen.bg = .{ .palette = @intCast(params[i] - 100 + 8) },
            38 => return applyExtendedColor(&pen.fg, params, i),
            48 => return applyExtendedColor(&pen.bg, params, i),
            else => {},
        }
        return 0;
    }

    /// Parse a `38`/`48` extended color: `;5;n` palette or `;2;r;g;b` rgb.
    /// Returns the count of extra params consumed beyond the `38`/`48` itself.
    fn applyExtendedColor(target: *Color, params: []const u16, i: usize) usize {
        if (i + 1 >= params.len) return 0;
        switch (params[i + 1]) {
            5 => {
                if (i + 2 >= params.len) return 1;
                target.* = .{ .palette = @intCast(params[i + 2] & 0xFF) };
                return 2;
            },
            2 => {
                if (i + 4 >= params.len) return @min(params.len - i - 1, 3);
                target.* = .{ .rgb = .{
                    @intCast(params[i + 2] & 0xFF),
                    @intCast(params[i + 3] & 0xFF),
                    @intCast(params[i + 4] & 0xFF),
                } };
                return 4;
            },
            else => return 1,
        }
    }

    fn resetPen(pen: *Cell) void {
        pen.* = Cell{};
    }

    // --- OSC payload handling ----------------------------------------------

    fn setTitle(self: *Terminal, text: []const u8) void {
        self.title_len = copyInto(&self.title_buf, text);
    }

    fn setCwd(self: *Terminal, text: []const u8) void {
        self.cwd_len = copyInto(&self.cwd_buf, text);
    }

    fn setClipboard(self: *Terminal, text: []const u8) void {
        // OSC 52 payload is `selection;base64data`; store the raw payload.
        const semi = std.mem.indexOfScalar(u8, text, ';');
        const data = if (semi) |s| text[s + 1 ..] else text;
        self.clipboard_len = copyInto(&self.clipboard_buf, data);
    }

    /// Record an OSC-133 semantic prompt mark keyed to the absolute line of
    /// the cursor at emit time.
    fn recordPromptMark(self: *Terminal, payload: []const u8) void {
        if (payload.len == 0) return;
        const kind: @TypeOf(@as(PromptMark, undefined).kind) = switch (payload[0]) {
            'A' => .prompt_start,
            'B' => .command_start,
            'C' => .output_start,
            'D' => .command_done,
            else => return,
        };

        // Update shell-state tracking for 133;C and 133;D.
        if (kind == .output_start) { // 133;C = command started
            self.shell_running = true;
            self.shell_run_start_ms = monoMs();
        } else if (kind == .command_done) { // 133;D = command finished
            if (self.shell_running) {
                self.shell_last_duration_ms = monoMs() - self.shell_run_start_ms;
            }
            self.shell_running = false;
            // Parse optional "exit_code=N" from the 133;D payload (e.g. "D;exit_code=1").
            const ec_key = "exit_code=";
            if (std.mem.indexOf(u8, payload, ec_key)) |idx| {
                const num_str = payload[idx + ec_key.len ..];
                var n: i32 = 0;
                var negative = false;
                var start: usize = 0;
                if (num_str.len > 0 and num_str[0] == '-') {
                    negative = true;
                    start = 1;
                }
                for (num_str[start..]) |ch| {
                    if (ch < '0' or ch > '9') break;
                    n = n * 10 + @as(i32, ch - '0');
                }
                self.shell_last_exit = if (negative) -n else n;
            } else {
                self.shell_last_exit = 0;
            }
        }

        const line_num = self.evicted_lines + self.history.len() + self.active().cur_y;
        // Suppress a duplicate prompt_start on the same line (precmd may fire
        // OSC 133;A more than once per prompt draw).
        if (kind == .prompt_start and self.mark_count > 0) {
            const last = self.marks[self.mark_count - 1];
            if (last.kind == .prompt_start and last.line == line_num) return;
        }
        if (self.mark_count == max_marks) {
            // Drop the oldest mark to make room.
            std.mem.copyForwards(PromptMark, self.marks[0 .. max_marks - 1], self.marks[1..]);
            self.mark_count -= 1;
        }
        self.marks[self.mark_count] = .{ .kind = kind, .line = line_num };
        self.mark_count += 1;
    }

    /// Shell-state accessor for the HUD: running state and last-run outcome.
    pub const LastRun = struct {
        running: bool,
        exit_code: i32,
        duration_ms: i64,
    };

    pub fn lastRun(self: *const Terminal) LastRun {
        return .{
            .running = self.shell_running,
            .exit_code = self.shell_last_exit,
            .duration_ms = self.shell_last_duration_ms,
        };
    }

    /// True when the given absolute line index was marked as a prompt start
    /// (OSC 133;A). Used by the renderer to draw a separator hairline.
    /// Drop prompt marks pointing into the live grid region. Called when the
    /// display is erased: the rows those marks described are now blank, so
    /// keeping the marks would draw separators across the cleared screen.
    /// Marks in scrollback (`line` below the grid's base) stay valid.
    fn invalidateGridMarks(self: *Terminal) void {
        const base = self.evicted_lines + self.history.len();
        var w: usize = 0;
        for (self.marks[0..self.mark_count]) |m| {
            if (m.line < base) {
                self.marks[w] = m;
                w += 1;
            }
        }
        self.mark_count = w;
    }

    pub fn isPromptStart(self: *const Terminal, abs_line: usize) bool {
        for (self.marks[0..self.mark_count]) |m| {
            if (m.kind == .prompt_start and m.line == abs_line) return true;
        }
        return false;
    }

    /// Convert a content-row index (scrollback rows followed by grid rows, as
    /// used by `line()` and `contentRowOfViewport()`) to an absolute line
    /// index that matches the values stored in `marks`.
    pub fn absoluteLineOfContent(self: *const Terminal, content_row: usize) usize {
        return self.evicted_lines + content_row;
    }

    // --- misc control ------------------------------------------------------

    /// Reverse index: move up one line, scrolling the region down at the top.
    fn reverseIndex(self: *Terminal) void {
        const g = self.active();
        if (g.cur_y == g.region.top) {
            g.scrollDown(1);
        } else if (g.cur_y > 0) {
            g.cur_y -= 1;
        }
    }

    /// Full reset (RIS): blank both grids, clear modes, drop the viewport to
    /// the live bottom. Scrollback and recorded marks are preserved.
    fn reset(self: *Terminal) void {
        self.primary.cursorTo(0, 0);
        self.primary.eraseDisplay(2);
        self.primary.pen = Cell{};
        self.primary.region = .{ .top = 0, .bottom = self.primary.height - 1 };
        self.alternate.cursorTo(0, 0);
        self.alternate.eraseDisplay(2);
        self.alternate.pen = Cell{};
        self.modes = .{};
        self.g0_line_drawing = false;
        self.on_alt = false;
        self.viewport_offset = 0;
    }
};

// --- free helpers ----------------------------------------------------------

/// Fetch parameter `idx`, returning `default_value` when absent or zero.
/// VT semantics treat a zero/omitted numeric param as its default.
fn param(params: []const u16, idx: usize, default_value: u16) u16 {
    if (idx >= params.len) return default_value;
    return if (params[idx] == 0) default_value else params[idx];
}

/// Convert a 1-based VT coordinate to a 0-based grid index (min 0).
fn oneBased(value: u16) usize {
    return if (value == 0) 0 else value - 1;
}

fn eql(a: []const u8, b: []const u8) bool {
    return std.mem.eql(u8, a, b);
}

/// Copy `src` into the start of `dst`, truncating to fit. Returns byte count.
fn copyInto(dst: []u8, src: []const u8) usize {
    const n = @min(dst.len, src.len);
    @memcpy(dst[0..n], src[0..n]);
    return n;
}

/// Map a codepoint through the DEC special graphics (line-drawing) set.
/// Only the box-drawing range `_`..`~` differs from ASCII.
fn translateLineDrawing(cp: u21) u21 {
    return switch (cp) {
        'j' => 0x2518, // ┘
        'k' => 0x2510, // ┐
        'l' => 0x250C, // ┌
        'm' => 0x2514, // └
        'n' => 0x253C, // ┼
        'q' => 0x2500, // ─
        't' => 0x251C, // ├
        'u' => 0x2524, // ┤
        'v' => 0x2534, // ┴
        'w' => 0x252C, // ┬
        'x' => 0x2502, // │
        '`' => 0x25C6, // ◆
        'a' => 0x2592, // ▒
        'f' => 0x00B0, // °
        'g' => 0x00B1, // ±
        '~' => 0x00B7, // ·
        else => cp,
    };
}

// === tests =================================================================

const testing = std.testing;

// Run every terminal-core module's tests through this file, so
// `zig test src/terminal/terminal.zig` exercises the whole core.
test {
    _ = @import("cell.zig");
    _ = @import("parser.zig");
    _ = @import("grid.zig");
    _ = @import("scrollback.zig");
}

/// Build a terminal of `cols_n x rows_n`. Caller deinits.
fn makeTerminal(cols_n: usize, rows_n: usize) !Terminal {
    return Terminal.init(testing.allocator, cols_n, rows_n, scrollback.default_capacity);
}

/// Render viewport row `y` to a UTF-8 string in `buf`.
fn viewportText(term: *Terminal, y: usize, buf: []u8) []const u8 {
    var n: usize = 0;
    for (term.viewportRow(y)) |c| {
        n += std.unicode.utf8Encode(c.cp, buf[n..]) catch 0;
    }
    return buf[0..n];
}

test "init reports its dimensions" {
    var term = try makeTerminal(80, 24);
    defer term.deinit();
    try testing.expectEqual(@as(usize, 80), term.cols());
    try testing.expectEqual(@as(usize, 24), term.rows());
}

test "feeding plain text fills the first row" {
    var term = try makeTerminal(10, 3);
    defer term.deinit();
    term.feed("hello");
    var buf: [16]u8 = undefined;
    try testing.expectEqualStrings("hello     ", viewportText(&term, 0, &buf));
    try testing.expectEqual(@as(usize, 5), term.cursor().x);
}

test "CR and LF reposition the cursor" {
    var term = try makeTerminal(10, 4);
    defer term.deinit();
    term.feed("ab\r\ncd");
    var buf: [16]u8 = undefined;
    try testing.expectEqualStrings("ab        ", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("cd        ", viewportText(&term, 1, &buf));
    try testing.expectEqual(@as(usize, 1), term.cursor().y);
}

test "CSI cursor position then print" {
    var term = try makeTerminal(10, 5);
    defer term.deinit();
    term.feed("\x1B[3;5HX");
    var buf: [16]u8 = undefined;
    try testing.expectEqualStrings("    X     ", viewportText(&term, 2, &buf));
}

test "CSI cursor moves clamp at bounds" {
    var term = try makeTerminal(10, 5);
    defer term.deinit();
    term.feed("\x1B[99;99H");
    try testing.expectEqual(@as(usize, 9), term.cursor().x);
    try testing.expectEqual(@as(usize, 4), term.cursor().y);
    term.feed("\x1B[99A");
    try testing.expectEqual(@as(usize, 0), term.cursor().y);
}

test "ED clears from cursor to end of screen" {
    var term = try makeTerminal(4, 3);
    defer term.deinit();
    term.feed("AAAA\r\nBBBB\r\nCC");
    term.feed("\x1B[0J");
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("AAAA", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("CC  ", viewportText(&term, 2, &buf));
}

test "EL clears the current line" {
    var term = try makeTerminal(6, 2);
    defer term.deinit();
    term.feed("abcdef\x1B[1G\x1B[0K");
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("      ", viewportText(&term, 0, &buf));
}

test "SGR sets bold and a palette color" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B[1;32mX");
    const c = term.viewportRow(0)[0];
    try testing.expect(c.attrs.bold);
    try testing.expectEqual(Color{ .palette = 2 }, c.fg);
}

test "SGR reset clears the pen" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B[1;31mA\x1B[0mB");
    const a = term.viewportRow(0)[0];
    const b = term.viewportRow(0)[1];
    try testing.expect(a.attrs.bold);
    try testing.expect(!b.attrs.bold);
    try testing.expectEqual(Color.default, b.fg);
}

test "SGR 256-color palette via 38;5;n" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B[38;5;200mX");
    try testing.expectEqual(Color{ .palette = 200 }, term.viewportRow(0)[0].fg);
}

test "SGR truecolor via 48;2;r;g;b" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B[48;2;10;20;30mX");
    try testing.expectEqual(Color{ .rgb = .{ 10, 20, 30 } }, term.viewportRow(0)[0].bg);
}

test "SGR truecolor mixed with other codes in one sequence" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    // bold, then fg truecolor, then underline — all in one CSI m.
    term.feed("\x1B[1;38;2;1;2;3;4mX");
    const c = term.viewportRow(0)[0];
    try testing.expect(c.attrs.bold);
    try testing.expect(c.attrs.underline);
    try testing.expectEqual(Color{ .rgb = .{ 1, 2, 3 } }, c.fg);
}

test "line feed past the bottom pushes rows into scrollback" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    term.feed("L1\r\nL2\r\nL3");
    // A 2-row screen: feeding three lines pushes "L1" into scrollback.
    try testing.expectEqual(@as(usize, 1), term.scrollbackLen());
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("L2  ", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("L3  ", viewportText(&term, 1, &buf));
}

test "viewport scrolls up into scrollback history" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    term.feed("L1\r\nL2\r\nL3\r\nL4");
    // Screen shows L3,L4; scrollback holds L1,L2.
    try testing.expectEqual(@as(usize, 2), term.scrollbackLen());

    term.scrollViewport(1); // up one row into history
    try testing.expectEqual(@as(usize, 1), term.viewportOffset());
    var buf: [8]u8 = undefined;
    // Top row now shows the newest scrollback row, L2.
    try testing.expectEqualStrings("L2  ", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("L3  ", viewportText(&term, 1, &buf));

    term.scrollViewport(1); // up one more
    try testing.expectEqualStrings("L1  ", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("L2  ", viewportText(&term, 1, &buf));
}

test "viewport scroll clamps and scrollToBottom resets it" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    term.feed("L1\r\nL2\r\nL3\r\nL4");
    term.scrollViewport(999); // clamps to scrollback length
    try testing.expectEqual(@as(usize, 2), term.viewportOffset());
    term.scrollViewport(-999); // clamps to zero
    try testing.expectEqual(@as(usize, 0), term.viewportOffset());
    term.scrollViewport(2);
    term.scrollToBottom();
    try testing.expectEqual(@as(usize, 0), term.viewportOffset());
}

test "setViewportOffset clamps to scrollback length and sets within range" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    term.feed("L1\r\nL2\r\nL3\r\nL4");
    // scrollback holds L1, L2 (len 2)
    try testing.expectEqual(@as(usize, 2), term.scrollbackLen());

    term.setViewportOffset(1);
    try testing.expectEqual(@as(usize, 1), term.viewportOffset());

    term.setViewportOffset(999); // clamps to scrollbackLen
    try testing.expectEqual(@as(usize, 2), term.viewportOffset());

    term.setViewportOffset(0);
    try testing.expectEqual(@as(usize, 0), term.viewportOffset());
}

test "new output snaps the viewport back to the live bottom" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    term.feed("L1\r\nL2\r\nL3");
    term.scrollViewport(1);
    try testing.expect(term.viewportOffset() > 0);
    term.feed("X");
    try testing.expectEqual(@as(usize, 0), term.viewportOffset());
}

test "alternate screen isolates scrollback and restores on exit" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    term.feed("L1\r\nL2\r\nL3"); // pushes L1 to scrollback
    try testing.expectEqual(@as(usize, 1), term.scrollbackLen());

    term.feed("\x1B[?1049h"); // enter alt screen
    // Filling and scrolling the alt screen must NOT touch scrollback.
    term.feed("A1\r\nA2\r\nA3\r\nA4");
    try testing.expectEqual(@as(usize, 1), term.scrollbackLen());

    term.feed("\x1B[?1049l"); // leave alt screen
    // The primary grid is intact.
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("L2  ", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("L3  ", viewportText(&term, 1, &buf));
}

test "cursor visibility toggles via DECSET ?25" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    try testing.expect(term.cursor().visible);
    term.feed("\x1B[?25l");
    try testing.expect(!term.cursor().visible);
    term.feed("\x1B[?25h");
    try testing.expect(term.cursor().visible);
}

test "bracketed paste flag follows DECSET ?2004" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    try testing.expect(!term.modes.bracketed_paste);
    term.feed("\x1B[?2004h");
    try testing.expect(term.modes.bracketed_paste);
}

test "autowrap can be disabled via DECRST ?7" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    term.feed("\x1B[?7l");
    term.feed("abcdef"); // 6 chars into a 4-wide grid, no wrap
    try testing.expectEqual(@as(usize, 0), term.cursor().y);
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("abcf", viewportText(&term, 0, &buf));
}

test "OSC 0 sets the window title" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B]0;Anvil\x07");
    try testing.expectEqualStrings("Anvil", term.title());
}

test "OSC 7 records the working directory" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B]7;file:///home/dev\x07");
    try testing.expectEqualStrings("file:///home/dev", term.cwd());
}

test "cwdPath strips file:// prefix and host" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();

    // Empty host (three slashes): file:///home/dev -> /home/dev
    term.feed("\x1B]7;file:///home/dev\x07");
    try testing.expectEqualStrings("/home/dev", term.cwdPath());

    // Named host: file://somehost/var/log -> /var/log
    term.feed("\x1B]7;file://somehost/var/log\x07");
    try testing.expectEqualStrings("/var/log", term.cwdPath());

    // Bare path passthrough
    term.feed("\x1B]7;/plain/path\x07");
    try testing.expectEqualStrings("/plain/path", term.cwdPath());

    // Empty value passthrough
    term.feed("\x1B]7;\x07");
    try testing.expectEqualStrings("", term.cwdPath());

    // file:// with no path component -> empty
    term.feed("\x1B]7;file://\x07");
    try testing.expectEqualStrings("", term.cwdPath());
}

test "OSC 133 prompt marks are recorded with absolute lines" {
    var term = try makeTerminal(6, 3);
    defer term.deinit();
    term.feed("\x1B]133;A\x07"); // prompt start at line 0
    term.feed("$ ls\r\n"); // move to line 1
    term.feed("\x1B]133;B\x07"); // command start at line 1
    term.feed("\x1B]133;C\x07"); // output start at line 1
    term.feed("out\r\n"); // move to line 2
    term.feed("\x1B]133;D\x07"); // command done at line 2

    const marks = term.promptMarks();
    try testing.expectEqual(@as(usize, 4), marks.len);
    try testing.expectEqual(@as(usize, 0), marks[0].line);
    try testing.expect(marks[0].kind == .prompt_start);
    try testing.expectEqual(@as(usize, 1), marks[1].line);
    try testing.expect(marks[1].kind == .command_start);
    try testing.expect(marks[2].kind == .output_start);
    try testing.expectEqual(@as(usize, 2), marks[3].line);
    try testing.expect(marks[3].kind == .command_done);
}

test "erasing the display drops prompt marks in the grid region" {
    var term = try makeTerminal(6, 4);
    defer term.deinit();
    term.feed("\x1B]133;A\x07"); // prompt start at line 0 (grid region)
    term.feed("$ x\r\n");
    try testing.expectEqual(@as(usize, 1), term.promptMarks().len);
    term.feed("\x1B[2J"); // clear the display
    // The mark pointed at a now-blank grid row — it must be gone.
    try testing.expectEqual(@as(usize, 0), term.promptMarks().len);
    try testing.expect(!term.isPromptStart(0));
}

test "OSC 133 absolute line survives scrollback eviction" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    // Fill so the screen is at the bottom, then mark and scroll past it.
    term.feed("L1\r\nL2"); // cursor at row 1
    term.feed("\x1B]133;A\x07"); // mark at absolute line 1
    try testing.expectEqual(@as(usize, 1), term.promptMarks()[0].line);
    // Scroll several lines; the mark's absolute line stays fixed.
    term.feed("\r\nL3\r\nL4\r\nL5");
    try testing.expectEqual(@as(usize, 1), term.promptMarks()[0].line);
}

test "isPromptStart returns true for a prompt_start mark and false elsewhere" {
    var term = try makeTerminal(6, 3);
    defer term.deinit();
    term.feed("\x1B]133;A\x07"); // prompt start at abs line 0
    term.feed("$ ls\r\n"); // move to line 1
    term.feed("\x1B]133;B\x07"); // command start (not prompt_start) at line 1
    try testing.expect(term.isPromptStart(0));
    try testing.expect(!term.isPromptStart(1)); // command_start, not prompt_start
    try testing.expect(!term.isPromptStart(2)); // no mark here
}

test "isPromptStart deduplicates repeated OSC 133;A on the same line" {
    var term = try makeTerminal(6, 3);
    defer term.deinit();
    term.feed("\x1B]133;A\x07");
    term.feed("\x1B]133;A\x07"); // second fire on same line — must be a no-op
    // Only one mark recorded.
    try testing.expectEqual(@as(usize, 1), term.promptMarks().len);
    try testing.expect(term.isPromptStart(0));
}

test "absoluteLineOfContent converts content row to absolute line" {
    var term = try makeTerminal(4, 3);
    defer term.deinit();
    // Fresh: no evictions; content row equals absolute line.
    try testing.expectEqual(@as(usize, 0), term.absoluteLineOfContent(0));
    try testing.expectEqual(@as(usize, 2), term.absoluteLineOfContent(2));
    // Feed enough lines to cause scrollback eviction (cap=default, but 4-col
    // 3-row grid will evict once we push a 4th line).
    term.feed("A\r\nB\r\nC\r\nD"); // pushes first row into scrollback
    // evicted_lines is still 0 until scrollback itself is full; for a fresh
    // terminal with default capacity the first eviction takes much longer.
    // Just confirm the formula holds: absoluteLineOfContent = evicted + row.
    const ev = term.evicted_lines;
    try testing.expectEqual(ev + 1, term.absoluteLineOfContent(1));
}

test "isPromptStart ring wraps without error past max_marks" {
    var term = try makeTerminal(6, 3);
    defer term.deinit();
    // Feed enough distinct prompt-starts to overflow the ring. Each one
    // advances the cursor to the next line so the absolute line differs.
    var i: usize = 0;
    while (i < max_marks + 10) : (i += 1) {
        term.feed("\x1B]133;A\x07");
        term.feed("\r\n"); // next line so absolute index differs each iteration
    }
    // Ring is at capacity; no crash and count is capped.
    try testing.expectEqual(@as(usize, max_marks), term.promptMarks().len);
}

test "ESC c performs a full reset" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    term.feed("\x1B[1;31mABCD");
    term.feed("\x1Bc");
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("    ", viewportText(&term, 0, &buf));
    try testing.expectEqual(@as(usize, 0), term.cursor().x);
    // The pen was reset, so fresh output is unstyled.
    term.feed("Z");
    try testing.expect(!term.viewportRow(0)[0].attrs.bold);
}

test "DEC line-drawing charset maps lowercase letters to box glyphs" {
    var term = try makeTerminal(6, 2);
    defer term.deinit();
    term.feed("\x1B(0qqq\x1B(Bqq");
    // First three q's are line-drawing horizontals; last two are ASCII 'q'.
    const r = term.viewportRow(0);
    try testing.expectEqual(@as(u21, 0x2500), r[0].cp);
    try testing.expectEqual(@as(u21, 0x2500), r[2].cp);
    try testing.expectEqual(@as(u21, 'q'), r[3].cp);
}

test "insert and delete lines via CSI L and M" {
    var term = try makeTerminal(4, 4);
    defer term.deinit();
    term.feed("11\r\n22\r\n33\r\n44");
    term.feed("\x1B[2;1H"); // row index 1
    term.feed("\x1B[L"); // insert one blank line
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("11  ", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("    ", viewportText(&term, 1, &buf));
    try testing.expectEqualStrings("22  ", viewportText(&term, 2, &buf));
}

test "DECSTBM scroll region limits line feeds" {
    var term = try makeTerminal(4, 4);
    defer term.deinit();
    term.feed("11\r\n22\r\n33\r\n44");
    term.feed("\x1B[2;3r"); // region = rows index 1..2
    term.feed("\x1B[3;1H"); // bottom of region
    term.feed("\r\nXX"); // line feed scrolls only the region
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("11  ", viewportText(&term, 0, &buf)); // untouched
    try testing.expectEqualStrings("33  ", viewportText(&term, 1, &buf));
    try testing.expectEqualStrings("XX  ", viewportText(&term, 2, &buf));
    try testing.expectEqualStrings("44  ", viewportText(&term, 3, &buf)); // untouched
}

test "resize keeps content and re-clamps the viewport" {
    var term = try makeTerminal(6, 3);
    defer term.deinit();
    term.feed("hello\r\nworld");
    term.resize(3, 2);
    try testing.expectEqual(@as(usize, 3), term.cols());
    try testing.expectEqual(@as(usize, 2), term.rows());
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("hel", viewportText(&term, 0, &buf));
}

test "autowrap moves to the next line at the right edge" {
    var term = try makeTerminal(3, 3);
    defer term.deinit();
    term.feed("abcdef"); // 6 chars wrap across 3-wide rows
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("abc", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("def", viewportText(&term, 1, &buf));
}

test "save and restore cursor via ESC 7 / ESC 8" {
    var term = try makeTerminal(10, 4);
    defer term.deinit();
    term.feed("\x1B[3;5H"); // row 2, col 4
    term.feed("\x1B7"); // save
    term.feed("\x1B[1;1H"); // home
    term.feed("\x1B8"); // restore
    try testing.expectEqual(@as(usize, 4), term.cursor().x);
    try testing.expectEqual(@as(usize, 2), term.cursor().y);
}

test "viewportRow always returns exactly cols cells" {
    var term = try makeTerminal(8, 2);
    defer term.deinit();
    term.feed("hi\r\nthere\r\nmore"); // forces scrollback
    term.scrollViewport(1);
    // The composed scrollback row is padded to the full width.
    try testing.expectEqual(@as(usize, 8), term.viewportRow(0).len);
    try testing.expectEqual(@as(usize, 8), term.viewportRow(1).len);
}

test "init releases partial state on allocation failure" {
    const H = struct {
        fn run(alloc: std.mem.Allocator) !void {
            var term = try Terminal.init(alloc, 8, 4, scrollback.default_capacity);
            term.deinit();
        }
    };
    // Fails each allocation in turn; init's errdefers must leave no leaks.
    try testing.checkAllAllocationFailures(testing.allocator, H.run, .{});
}

test "BS and HT execute as cursor controls" {
    var term = try makeTerminal(20, 2);
    defer term.deinit();
    term.feed("ab\x08"); // backspace after 'ab'
    try testing.expectEqual(@as(usize, 1), term.cursor().x);
    term.feed("\r\x09"); // CR home, then HT to the next 8-column tab stop
    try testing.expectEqual(@as(usize, 8), term.cursor().x);
}

test "CSI relative cursor moves B, C, D" {
    var term = try makeTerminal(10, 5);
    defer term.deinit();
    term.feed("\x1B[3;5H"); // row 2, col 4
    term.feed("\x1B[1B"); // down 1
    try testing.expectEqual(@as(usize, 3), term.cursor().y);
    term.feed("\x1B[2C"); // forward 2
    try testing.expectEqual(@as(usize, 6), term.cursor().x);
    term.feed("\x1B[3D"); // back 3
    try testing.expectEqual(@as(usize, 3), term.cursor().x);
}

test "CSI E and F move to line start, down and up" {
    var term = try makeTerminal(10, 5);
    defer term.deinit();
    term.feed("\x1B[3;5H\x1B[1E"); // CNL from row 2 -> col 0, row 3
    try testing.expectEqual(@as(usize, 0), term.cursor().x);
    try testing.expectEqual(@as(usize, 3), term.cursor().y);
    term.feed("\x1B[5;5H\x1B[2F"); // CPL from row 4 -> col 0, row 2
    try testing.expectEqual(@as(usize, 0), term.cursor().x);
    try testing.expectEqual(@as(usize, 2), term.cursor().y);
}

test "CSI d sets the cursor row absolutely" {
    var term = try makeTerminal(10, 5);
    defer term.deinit();
    term.feed("\x1B[3d"); // line position absolute -> row index 2
    try testing.expectEqual(@as(usize, 2), term.cursor().y);
}

test "CSI @ and P insert and delete characters" {
    var term = try makeTerminal(6, 2);
    defer term.deinit();
    term.feed("abcdef\x1B[1G\x1B[2@"); // insert 2 blanks at the line start
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("  abcd", viewportText(&term, 0, &buf));
    term.feed("\x1B[1G\x1B[3P"); // delete 3 characters from the start
    try testing.expectEqualStrings("bcd   ", viewportText(&term, 0, &buf));
}

test "CSI X erases characters in place" {
    var term = try makeTerminal(6, 2);
    defer term.deinit();
    term.feed("abcdef\x1B[1G\x1B[3X");
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("   def", viewportText(&term, 0, &buf));
}

test "CSI S and T scroll the screen up and down" {
    var term = try makeTerminal(4, 3);
    defer term.deinit();
    term.feed("11\r\n22\r\n33");
    var buf: [8]u8 = undefined;
    term.feed("\x1B[1S"); // scroll up
    try testing.expectEqualStrings("22  ", viewportText(&term, 0, &buf));
    term.feed("\x1B[1T"); // scroll down
    try testing.expectEqualStrings("    ", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("22  ", viewportText(&term, 1, &buf));
}

test "CSI M deletes lines" {
    var term = try makeTerminal(4, 4);
    defer term.deinit();
    term.feed("11\r\n22\r\n33\r\n44");
    term.feed("\x1B[1;1H\x1B[1M"); // delete the line at row 0
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("22  ", viewportText(&term, 0, &buf));
}

test "CSI 4h and 4l toggle the grid insert mode" {
    var term = try makeTerminal(6, 2);
    defer term.deinit();
    term.feed("\x1B[4h"); // SM: insert mode on
    try testing.expect(term.primary.modes.insert);
    term.feed("\x1B[4l"); // RM: insert mode off
    try testing.expect(!term.primary.modes.insert);
}

test "ESC D indexes down and ESC E moves to the next line" {
    var term = try makeTerminal(6, 3);
    defer term.deinit();
    term.feed("\x1B[2;3H\x1BD"); // IND from row 1 -> row 2, column kept
    try testing.expectEqual(@as(usize, 2), term.cursor().y);
    try testing.expectEqual(@as(usize, 2), term.cursor().x);
    term.feed("\x1B[1;3H\x1BE"); // NEL from row 0 -> col 0, row 1
    try testing.expectEqual(@as(usize, 0), term.cursor().x);
    try testing.expectEqual(@as(usize, 1), term.cursor().y);
}

test "ESC M reverse-indexes, scrolling down at the top" {
    var term = try makeTerminal(4, 3);
    defer term.deinit();
    term.feed("11\r\n22\r\n33");
    term.feed("\x1B[2;1H\x1BM"); // RI from row 1 -> cursor up to row 0
    try testing.expectEqual(@as(usize, 0), term.cursor().y);
    term.feed("\x1BM"); // RI at the top -> scroll the region down
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("    ", viewportText(&term, 0, &buf));
    try testing.expectEqualStrings("11  ", viewportText(&term, 1, &buf));
}

test "OSC 52 stores the clipboard payload" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B]52;c;SGVsbG8=\x07"); // selection 'c', base64 payload
    try testing.expectEqualStrings("SGVsbG8=", term.clipboard());
}

test "DECSET ?1 toggles application cursor keys" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B[?1h");
    try testing.expect(term.modes.app_cursor_keys);
    term.feed("\x1B[?1l");
    try testing.expect(!term.modes.app_cursor_keys);
}

test "DECSET mouse modes ?1000 ?1002 ?1006" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B[?1000h");
    try testing.expect(term.modes.mouse_button);
    term.feed("\x1B[?1000l\x1B[?1002h");
    try testing.expect(term.modes.mouse_button);
    term.feed("\x1B[?1006h");
    try testing.expect(term.modes.mouse_sgr);
}

test "SGR with no parameters resets the pen" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    term.feed("\x1B[1mA\x1B[mB"); // bare CSI m resets
    try testing.expect(term.viewportRow(0)[0].attrs.bold);
    try testing.expect(!term.viewportRow(0)[1].attrs.bold);
}

test "SGR applies and clears every text attribute" {
    var term = try makeTerminal(20, 2);
    defer term.deinit();
    term.feed("\x1B[2;3;5;7;8;9mA"); // dim, italic, blink, inverse, invisible, strike
    const a = term.viewportRow(0)[0];
    try testing.expect(a.attrs.dim and a.attrs.italic and a.attrs.blink);
    try testing.expect(a.attrs.inverse and a.attrs.invisible and a.attrs.strikethrough);
    // 22 clears bold+dim; 23/24/25/27/28/29 clear the remaining attributes.
    term.feed("\x1B[1;2;22;23;24;25;27;28;29mB");
    const b = term.viewportRow(0)[1];
    try testing.expect(!b.attrs.bold and !b.attrs.dim and !b.attrs.italic);
    try testing.expect(!b.attrs.underline and !b.attrs.blink and !b.attrs.inverse);
    try testing.expect(!b.attrs.invisible and !b.attrs.strikethrough);
}

test "SGR foreground and background color codes" {
    var term = try makeTerminal(20, 2);
    defer term.deinit();
    term.feed("\x1B[44mA"); // background palette 4
    try testing.expectEqual(Color{ .palette = 4 }, term.viewportRow(0)[0].bg);
    term.feed("\x1B[39;49mB"); // default foreground and background
    try testing.expectEqual(Color.default, term.viewportRow(0)[1].fg);
    try testing.expectEqual(Color.default, term.viewportRow(0)[1].bg);
    term.feed("\x1B[91mC"); // bright foreground -> palette 9
    try testing.expectEqual(Color{ .palette = 9 }, term.viewportRow(0)[2].fg);
    term.feed("\x1B[102mD"); // bright background -> palette 10
    try testing.expectEqual(Color{ .palette = 10 }, term.viewportRow(0)[3].bg);
}

test "SGR extended-color with an unknown selector consumes one parameter" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();
    // 38 then 9 (neither 5 nor 2): the selector is skipped, the trailing 1m applies.
    term.feed("\x1B[38;9;1mX");
    try testing.expect(term.viewportRow(0)[0].attrs.bold);
}

test "OSC 133 marks evict the oldest past the cap" {
    var term = try makeTerminal(6, 3);
    defer term.deinit();
    var i: usize = 0;
    while (i < max_marks + 5) : (i += 1) {
        term.feed("\x1B]133;A\x07");
        term.feed("\r\n"); // advance cursor so each mark lands on a distinct line
    }
    // The ring is capped; the most recent max_marks are retained.
    try testing.expectEqual(@as(usize, max_marks), term.promptMarks().len);
}

test "lineCount and line span scrollback then grid" {
    var t = try makeTerminal(10, 3);
    defer t.deinit();
    // Fresh terminal: no scrollback, grid height 3.
    try testing.expectEqual(@as(usize, 3), t.lineCount());
    // Feed enough newlines to push rows into scrollback.
    t.feed("a\r\nb\r\nc\r\nd\r\ne\r\n");
    try testing.expect(t.lineCount() > 3);
    // The oldest content row is a scrollback row; the last is a grid row.
    const last = t.line(t.lineCount() - 1);
    try testing.expectEqual(@as(usize, 10), last.len); // grid row is full width
}

test "contentRowOfViewport matches viewport composition" {
    var t = try makeTerminal(10, 3);
    defer t.deinit();
    t.feed("1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n");
    // At offset 0, viewport row 0 is the first grid row.
    try testing.expectEqual(t.history.len(), t.contentRowOfViewport(0));
}

test "viewportRowAt matches viewportRow when offset equals viewportOffset" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    // Feed lines to push some into scrollback.
    term.feed("L1\r\nL2\r\nL3\r\nL4");
    // Screen shows L3, L4; scrollback has L1, L2.
    term.scrollViewport(1); // offset = 1
    const off = term.viewportOffset();
    // viewportRowAt with the same offset must return the same cells as viewportRow.
    for (0..term.rows()) |y| {
        const via_row = term.viewportRow(y);
        const via_at = term.viewportRowAt(off, y);
        try testing.expectEqual(via_row.len, via_at.len);
        for (0..via_row.len) |x| {
            try testing.expectEqual(via_row[x].cp, via_at[x].cp);
        }
    }
}

test "viewportRowAt returns a blank row for out-of-range y" {
    var term = try makeTerminal(4, 2);
    defer term.deinit();
    term.feed("L1\r\nL2\r\nL3");
    // y == rows() is one past the viewport bottom — should yield a blank row.
    const off = term.viewportOffset();
    const extra = term.viewportRowAt(off, term.rows());
    for (extra) |c| {
        // A blank cell has cp = ' ' (space); no non-space content expected.
        try testing.expect(c.cp == ' ' or c.cp == 0);
    }
}

// --- resize pre-scroll tests -----------------------------------------------

test "shrink past cursor anchors cursor to bottom and archives overflow" {
    // 4-wide, 5-row terminal. Feed 5 lines so cursor is on row 4 (last).
    // Shrink to 3 rows: 2 rows overflow into scrollback, cursor on row 2.
    var term = try makeTerminal(4, 5);
    defer term.deinit();
    // Write 5 lines; cursor ends up on the last row with known content.
    term.feed("aaaa\r\nbbbb\r\ncccc\r\ndddd\r\neeee");
    try testing.expectEqual(@as(usize, 4), term.cursor().y);
    try testing.expectEqual(@as(usize, 0), term.scrollbackLen());

    term.resize(4, 3);

    // Cursor must be within the new height.
    try testing.expect(term.cursor().y < term.rows());
    // 2 rows (row 0 and row 1 from the original screen) overflowed.
    try testing.expectEqual(@as(usize, 2), term.scrollbackLen());
    // The cursor row must still contain the last-written text "eeee".
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("eeee", viewportText(&term, term.cursor().y, &buf));
}

test "shrink that does not overflow the cursor leaves scrollback unchanged" {
    // 4-wide, 5-row terminal. Cursor at row 1; rows 2..4 are blank.
    // Shrink to 3 rows: cursor still fits, no overflow.
    var term = try makeTerminal(4, 5);
    defer term.deinit();
    term.feed("aaaa\r\nbbbb");
    try testing.expectEqual(@as(usize, 1), term.cursor().y);
    try testing.expectEqual(@as(usize, 0), term.scrollbackLen());

    term.resize(4, 3);

    try testing.expectEqual(@as(usize, 0), term.scrollbackLen());
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("bbbb", viewportText(&term, term.cursor().y, &buf));
}

test "grow preserves content and cursor and leaves scrollback unchanged" {
    var term = try makeTerminal(4, 3);
    defer term.deinit();
    term.feed("aaaa\r\nbbbb\r\ncccc");
    // cccc is on row 2, cursor there.
    try testing.expectEqual(@as(usize, 2), term.cursor().y);
    try testing.expectEqual(@as(usize, 0), term.scrollbackLen());

    term.resize(4, 6);

    try testing.expectEqual(@as(usize, 0), term.scrollbackLen());
    var buf: [8]u8 = undefined;
    // cccc must still be on row 2.
    try testing.expectEqualStrings("cccc", viewportText(&term, 2, &buf));
    try testing.expectEqual(@as(usize, 2), term.cursor().y);
}

test "grow then shrink round trip leaves the cursor line visible" {
    // 4-wide, 3-row terminal. Fill all rows; cursor on row 2.
    var term = try makeTerminal(4, 3);
    defer term.deinit();
    term.feed("aaaa\r\nbbbb\r\ncccc");
    try testing.expectEqual(@as(usize, 2), term.cursor().y);

    // Grow to 6 rows — cursor stays at row 2, content intact.
    term.resize(4, 6);
    try testing.expectEqual(@as(usize, 0), term.scrollbackLen());

    // Shrink back to 3 rows — cccc must still be visible at the cursor.
    term.resize(4, 3);
    try testing.expect(term.cursor().y < term.rows());
    var buf: [8]u8 = undefined;
    try testing.expectEqualStrings("cccc", viewportText(&term, term.cursor().y, &buf));
}

test "DECSCUSR sets and clears app cursor shape" {
    var term = try makeTerminal(10, 2);
    defer term.deinit();

    // Default: no app request.
    try testing.expect(term.app_cursor_shape == null);
    try testing.expect(term.app_cursor_blink == null);

    // Ps=6: steady bar.
    term.feed("\x1b[6 q");
    try testing.expectEqual(CursorShape.bar, term.app_cursor_shape.?);
    try testing.expectEqual(false, term.app_cursor_blink.?);

    // Ps=5: blinking bar.
    term.feed("\x1b[5 q");
    try testing.expectEqual(CursorShape.bar, term.app_cursor_shape.?);
    try testing.expectEqual(true, term.app_cursor_blink.?);

    // Ps=2: steady block.
    term.feed("\x1b[2 q");
    try testing.expectEqual(CursorShape.block, term.app_cursor_shape.?);
    try testing.expectEqual(false, term.app_cursor_blink.?);

    // Ps=4: steady underline.
    term.feed("\x1b[4 q");
    try testing.expectEqual(CursorShape.underline, term.app_cursor_shape.?);
    try testing.expectEqual(false, term.app_cursor_blink.?);

    // Ps=0: reset to config default.
    term.feed("\x1b[0 q");
    try testing.expect(term.app_cursor_shape == null);
    try testing.expect(term.app_cursor_blink == null);
}
