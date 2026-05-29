const std = @import("std");
const cell = @import("cell.zig");
const Cell = cell.Cell;
const Color = cell.Color;
const Grid = @import("grid.zig").Grid;
const Scrollback = @import("scrollback.zig").Scrollback;

const scrollback_cap = 5000;

pub const Pos = struct { row: u16, col: u16 };

/// Mouse-tracking level requested by the program (DEC private modes).
/// off = none, normal = 1000 (press/release), button = 1002 (+drag while
/// pressed), any = 1003 (+all motion).
pub const MouseMode = enum { off, normal, button, any };

/// Linear (text-flow) selection in visible-grid coordinates.
pub const Selection = struct {
    anchor: Pos,
    head: Pos,

    pub fn ordered(self: Selection) struct { start: Pos, end: Pos } {
        const a = self.anchor;
        const h = self.head;
        const a_first = a.row < h.row or (a.row == h.row and a.col <= h.col);
        return if (a_first) .{ .start = a, .end = h } else .{ .start = h, .end = a };
    }
};

pub const Terminal = struct {
    grid: Grid,
    cx: u16 = 0,
    cy: u16 = 0,
    saved_cx: u16 = 0,
    saved_cy: u16 = 0,
    stash: ?Grid = null, // primary grid, parked while the alt screen is active
    alt_cx: u16 = 0,
    alt_cy: u16 = 0,
    scrollback: Scrollback,
    view_offset: usize = 0, // lines scrolled up into history; 0 = live bottom
    selection: ?Selection = null,
    pen: Cell = .{},
    title_buf: [256]u8 = undefined,
    title_len: usize = 0,
    cwd_buf: [1024]u8 = undefined,
    cwd_len: usize = 0,
    mouse: MouseMode = .off,
    mouse_sgr: bool = false, // SGR (1006) extended encoding

    /// Window title set via OSC 0/2 (empty until the shell sets one).
    pub fn title(self: *const Terminal) []const u8 {
        return self.title_buf[0..self.title_len];
    }

    /// Working directory set via OSC 7 (empty until the shell reports one).
    pub fn cwd(self: *const Terminal) []const u8 {
        return self.cwd_buf[0..self.cwd_len];
    }

    pub fn setTitle(self: *Terminal, s: []const u8) void {
        const n = @min(s.len, self.title_buf.len);
        @memcpy(self.title_buf[0..n], s[0..n]);
        self.title_len = n;
    }

    /// OSC 7 reports cwd as a `file://host/path` URI; store just the path.
    pub fn setCwd(self: *Terminal, uri: []const u8) void {
        var path = uri;
        if (std.mem.startsWith(u8, path, "file://")) {
            path = path[7..];
            if (std.mem.indexOfScalar(u8, path, '/')) |slash| path = path[slash..];
        }
        const n = @min(path.len, self.cwd_buf.len);
        @memcpy(self.cwd_buf[0..n], path[0..n]);
        self.cwd_len = n;
    }

    pub fn init(alloc: std.mem.Allocator, rows: u16, cols: u16) !Terminal {
        return .{
            .grid = try Grid.init(alloc, rows, cols),
            .scrollback = try Scrollback.init(alloc, scrollback_cap),
        };
    }

    pub fn deinit(self: *Terminal) void {
        self.grid.deinit();
        if (self.stash) |*g| g.deinit();
        self.scrollback.deinit();
    }

    /// Cells for visible row `r`, drawn from scrollback when scrolled up.
    pub fn viewRow(self: *Terminal, r: u16) []Cell {
        const sb = self.scrollback.len();
        const logical = (sb - self.view_offset) + r;
        if (logical < sb) return self.scrollback.at(logical);
        return self.grid.row(@intCast(logical - sb));
    }

    /// Scroll the viewport: positive = back into history, negative = toward live.
    pub fn scrollView(self: *Terminal, delta: i32) void {
        const sb: i64 = @intCast(self.scrollback.len());
        var off: i64 = @as(i64, @intCast(self.view_offset)) + delta;
        off = std.math.clamp(off, 0, sb);
        self.view_offset = @intCast(off);
    }

    pub fn selectStart(self: *Terminal, row: u16, col: u16) void {
        self.selection = .{ .anchor = .{ .row = row, .col = col }, .head = .{ .row = row, .col = col } };
    }

    pub fn selectExtend(self: *Terminal, row: u16, col: u16) void {
        if (self.selection) |*s| s.head = .{ .row = row, .col = col };
    }

    pub fn clearSelection(self: *Terminal) void {
        self.selection = null;
    }

    pub fn isSelected(self: *Terminal, row: u16, col: u16) bool {
        const sel = self.selection orelse return false;
        const o = sel.ordered();
        const after = row > o.start.row or (row == o.start.row and col >= o.start.col);
        const before = row < o.end.row or (row == o.end.row and col <= o.end.col);
        return after and before;
    }

    /// Write the selection as UTF-8 into `out`, trimming trailing blanks per
    /// row and joining rows with '\n'. Returns bytes written (truncated to fit).
    pub fn selectionText(self: *Terminal, out: []u8) usize {
        const sel = self.selection orelse return 0;
        const o = sel.ordered();
        var n: usize = 0;
        var r: u16 = o.start.row;
        while (r <= o.end.row) : (r += 1) {
            const cells = self.viewRow(r);
            const c0: u16 = if (r == o.start.row) o.start.col else 0;
            const c1: u16 = if (r == o.end.row) o.end.col else self.grid.cols - 1;
            var last: i32 = -1;
            var c: u16 = c0;
            while (c <= c1) : (c += 1) if (cells[c].cp != ' ') {
                last = c;
            };
            c = c0;
            while (c <= c1) : (c += 1) {
                if (last < 0 or c > @as(u16, @intCast(last))) break;
                var tmp: [4]u8 = undefined;
                const ln = std.unicode.utf8Encode(cells[c].cp, &tmp) catch 1;
                if (n + ln > out.len) return n;
                @memcpy(out[n .. n + ln], tmp[0..ln]);
                n += ln;
            }
            if (r < o.end.row and n < out.len) {
                out[n] = '\n';
                n += 1;
            }
        }
        return n;
    }

    /// Resize preserving content, bottom-anchored. On shrink the overflowing
    /// top rows spill into scrollback; on grow, history fills the new top rows.
    /// Does not rewrap wrapped lines.
    pub fn resize(self: *Terminal, new_rows: u16, new_cols: u16) !void {
        if (new_rows == self.grid.rows and new_cols == self.grid.cols) return;

        if (self.stash) |*primary| {
            const a = try copyResized(&self.grid, new_rows, new_cols);
            self.grid.deinit();
            self.grid = a;
            const p = try copyResized(primary, new_rows, new_cols);
            primary.deinit();
            self.stash = p;
            self.cx = @min(self.cx, new_cols - 1);
            self.cy = @min(self.cy, new_rows - 1);
            return;
        }

        try self.reflow(new_rows, new_cols);
        if (self.view_offset > self.scrollback.len()) self.view_offset = self.scrollback.len();
    }

    /// Rewrap the primary grid to a new width and height. Logical lines (runs of
    /// soft-wrapped rows) are re-chunked at new_cols; the result is bottom-
    /// anchored, top overflow spills to scrollback, and slack pulls history back.
    fn reflow(self: *Terminal, new_rows: u16, new_cols: u16) !void {
        const alloc = self.grid.alloc;
        const old = &self.grid;

        // 1. Flatten into logical lines (owned []Cell), tracking the cursor.
        var lines: std.ArrayListUnmanaged([]Cell) = .empty;
        defer {
            for (lines.items) |l| alloc.free(l);
            lines.deinit(alloc);
        }
        var cur: std.ArrayListUnmanaged(Cell) = .empty;
        defer cur.deinit(alloc);

        // Content extends to the cursor row or the last non-blank row, whichever
        // is lower. Trailing blank rows are screen space, not content.
        var content_rows: u16 = self.cy + 1;
        var rr: u16 = old.rows;
        while (rr > 0) {
            rr -= 1;
            if (old.wrapped[rr] or trimLen(old.row(rr)) > 0) {
                content_rows = @max(content_rows, rr + 1);
                break;
            }
        }

        var cursor_line: usize = 0;
        var cursor_abscol: usize = 0;
        var r: u16 = 0;
        while (r < content_rows) : (r += 1) {
            if (r == self.cy) {
                cursor_line = lines.items.len;
                cursor_abscol = cur.items.len + self.cx;
            }
            const cells = old.row(r);
            const take: usize = if (old.wrapped[r]) old.cols else trimLen(cells);
            try cur.appendSlice(alloc, cells[0..take]);
            if (!old.wrapped[r]) {
                try lines.append(alloc, try cur.toOwnedSlice(alloc));
            }
        }
        if (cur.items.len > 0) try lines.append(alloc, try cur.toOwnedSlice(alloc));

        // 2. Re-chunk each logical line into new_cols-wide rows.
        var outrows: std.ArrayListUnmanaged([]Cell) = .empty;
        var outwrap: std.ArrayListUnmanaged(bool) = .empty;
        defer {
            for (outrows.items) |l| alloc.free(l);
            outrows.deinit(alloc);
            outwrap.deinit(alloc);
        }
        var new_cy: ?usize = null;
        var new_cx: u16 = 0;
        for (lines.items, 0..) |line, li| {
            const nchunks: usize = if (line.len == 0) 1 else (line.len + new_cols - 1) / new_cols;
            var ci: usize = 0;
            while (ci < nchunks) : (ci += 1) {
                const off = ci * new_cols;
                const buf = try alloc.alloc(Cell, new_cols);
                @memset(buf, Cell.blank);
                const n = if (line.len > off) @min(@as(usize, new_cols), line.len - off) else 0;
                @memcpy(buf[0..n], line[off .. off + n]);
                try outrows.append(alloc, buf);
                try outwrap.append(alloc, ci < nchunks - 1);
            }
            if (li == cursor_line and new_cy == null) {
                var chunk = cursor_abscol / new_cols;
                if (chunk >= nchunks) chunk = nchunks - 1;
                new_cy = outrows.items.len - nchunks + chunk;
                new_cx = @intCast(@min(cursor_abscol - chunk * new_cols, new_cols - 1));
            }
        }

        // 3. Place bottom-anchored into a fresh grid.
        var ng = try Grid.init(alloc, new_rows, new_cols);
        const total = outrows.items.len;
        if (total >= new_rows) {
            const spill = total - new_rows;
            var i: usize = 0;
            while (i < spill) : (i += 1) self.scrollback.push(outrows.items[i]);
            i = 0;
            while (i < new_rows) : (i += 1) {
                @memcpy(ng.row(@intCast(i)), outrows.items[spill + i]);
                ng.wrapped[i] = outwrap.items[spill + i];
            }
            const cl = new_cy orelse (total - 1);
            self.cy = @intCast(@min(if (cl >= spill) cl - spill else 0, new_rows - 1));
        } else {
            const top_blank: u16 = @intCast(new_rows - total);
            var i: usize = 0;
            while (i < total) : (i += 1) {
                @memcpy(ng.row(top_blank + @as(u16, @intCast(i))), outrows.items[i]);
                ng.wrapped[top_blank + i] = outwrap.items[i];
            }
            var t: u16 = top_blank;
            while (t > 0) {
                const line = self.scrollback.pop() orelse break;
                t -= 1;
                const n = @min(@as(u16, @intCast(line.len)), new_cols);
                @memcpy(ng.row(t)[0..n], line[0..n]);
                alloc.free(line);
            }
            const cl = new_cy orelse (if (total > 0) total - 1 else 0);
            self.cy = @intCast(@min(top_blank + cl, new_rows - 1));
        }
        self.cx = @min(new_cx, new_cols - 1);

        old.deinit();
        self.grid = ng;
    }

    pub fn print(self: *Terminal, cp: u21) void {
        const w = @import("width.zig").charWidth(cp);
        if (w == 0) return; // combining mark: drop (no shaping yet)
        // A wide glyph needs two columns; wrap early if only one is left.
        if (self.cx + w > self.grid.cols) {
            self.grid.wrapped[self.cy] = true; // soft wrap, not a hard line break
            self.cx = 0;
            self.lineFeed();
        }
        const c = self.grid.at(self.cy, self.cx);
        c.* = .{ .cp = cp, .fg = self.pen.fg, .bg = self.pen.bg, .attrs = self.pen.attrs };
        self.cx += 1;
        if (w == 2) {
            // Trailing spacer: blank glyph carrying the pen background.
            const s = self.grid.at(self.cy, self.cx);
            s.* = .{ .cp = ' ', .fg = self.pen.fg, .bg = self.pen.bg, .attrs = self.pen.attrs };
            self.cx += 1;
        }
    }

    pub fn lineFeed(self: *Terminal) void {
        if (self.cy + 1 >= self.grid.rows) {
            if (self.stash == null) {
                self.scrollback.push(self.grid.row(0));
                if (self.view_offset > 0 and self.view_offset < self.scrollback.len())
                    self.view_offset += 1; // stay anchored to history while scrolled
            }
            self.grid.scrollUp(1);
        } else {
            self.cy += 1;
        }
    }

    pub fn carriageReturn(self: *Terminal) void {
        self.cx = 0;
    }

    pub fn backspace(self: *Terminal) void {
        if (self.cx > 0) self.cx -= 1;
    }

    pub fn tab(self: *Terminal) void {
        const next = (self.cx / 8 + 1) * 8;
        self.cx = @min(next, self.grid.cols - 1);
    }

    pub fn cursorUp(self: *Terminal, n: u16) void {
        self.cy -= @min(n, self.cy);
    }

    pub fn cursorDown(self: *Terminal, n: u16) void {
        self.cy = @min(self.cy + n, self.grid.rows - 1);
    }

    pub fn cursorForward(self: *Terminal, n: u16) void {
        self.cx = @min(self.cx + n, self.grid.cols - 1);
    }

    pub fn cursorBack(self: *Terminal, n: u16) void {
        self.cx -= @min(n, self.cx);
    }

    pub fn setCursor(self: *Terminal, row1: u16, col1: u16) void {
        const r = if (row1 > 0) row1 - 1 else 0;
        const c = if (col1 > 0) col1 - 1 else 0;
        self.cy = @min(r, self.grid.rows - 1);
        self.cx = @min(c, self.grid.cols - 1);
    }

    pub fn cursorCol(self: *Terminal, col1: u16) void {
        const c = if (col1 > 0) col1 - 1 else 0;
        self.cx = @min(c, self.grid.cols - 1);
    }

    pub fn cursorRow(self: *Terminal, row1: u16) void {
        const r = if (row1 > 0) row1 - 1 else 0;
        self.cy = @min(r, self.grid.rows - 1);
    }

    pub fn saveCursor(self: *Terminal) void {
        self.saved_cx = self.cx;
        self.saved_cy = self.cy;
    }

    pub fn restoreCursor(self: *Terminal) void {
        self.cx = @min(self.saved_cx, self.grid.cols - 1);
        self.cy = @min(self.saved_cy, self.grid.rows - 1);
    }

    pub fn deleteChars(self: *Terminal, n: u16) void {
        const r = self.grid.row(self.cy);
        const cnt = @min(n, self.grid.cols - self.cx);
        std.mem.copyForwards(Cell, r[self.cx .. self.grid.cols - cnt], r[self.cx + cnt ..]);
        @memset(r[self.grid.cols - cnt ..], Cell.blank);
    }

    pub fn insertChars(self: *Terminal, n: u16) void {
        const r = self.grid.row(self.cy);
        const cnt = @min(n, self.grid.cols - self.cx);
        std.mem.copyBackwards(Cell, r[self.cx + cnt ..], r[self.cx .. self.grid.cols - cnt]);
        @memset(r[self.cx .. self.cx + cnt], Cell.blank);
    }

    pub fn eraseChars(self: *Terminal, n: u16) void {
        const r = self.grid.row(self.cy);
        const end = @min(self.cx + n, self.grid.cols);
        @memset(r[self.cx..end], Cell.blank);
    }

    pub fn setMode(self: *Terminal, mode: u16, enable: bool) void {
        switch (mode) {
            47, 1047, 1049 => if (enable) self.enterAlt() else self.exitAlt(),
            1000 => self.mouse = if (enable) .normal else .off,
            1002 => self.mouse = if (enable) .button else .off,
            1003 => self.mouse = if (enable) .any else .off,
            1006 => self.mouse_sgr = enable,
            else => {},
        }
    }

    fn enterAlt(self: *Terminal) void {
        if (self.stash != null) return;
        const g = Grid.init(self.grid.alloc, self.grid.rows, self.grid.cols) catch return;
        self.alt_cx = self.cx;
        self.alt_cy = self.cy;
        self.stash = self.grid;
        self.grid = g;
        self.cx = 0;
        self.cy = 0;
        self.view_offset = 0;
    }

    fn exitAlt(self: *Terminal) void {
        if (self.stash) |primary| {
            self.grid.deinit();
            self.grid = primary;
            self.stash = null;
            self.cx = @min(self.alt_cx, self.grid.cols - 1);
            self.cy = @min(self.alt_cy, self.grid.rows - 1);
        }
    }

    fn trimLen(cells: []const Cell) usize {
        var n: usize = cells.len;
        while (n > 0 and cells[n - 1].cp == ' ') n -= 1;
        return n;
    }

    fn copyResized(old: *Grid, new_rows: u16, new_cols: u16) !Grid {
        var ng = try Grid.init(old.alloc, new_rows, new_cols);
        const copy_cols = @min(old.cols, new_cols);
        const keep = @min(old.rows, new_rows);
        var i: u16 = 0;
        while (i < keep) : (i += 1) {
            const src = old.row(old.rows - keep + i);
            @memcpy(ng.row(new_rows - keep + i)[0..copy_cols], src[0..copy_cols]);
            ng.wrapped[new_rows - keep + i] = old.wrapped[old.rows - keep + i];
        }
        return ng;
    }

    pub fn eraseInLine(self: *Terminal, mode: u16) void {
        const r = self.grid.row(self.cy);
        switch (mode) {
            0 => @memset(r[self.cx..], Cell.blank),
            1 => @memset(r[0 .. self.cx + 1], Cell.blank),
            2 => @memset(r, Cell.blank),
            else => {},
        }
    }

    pub fn eraseInDisplay(self: *Terminal, mode: u16) void {
        switch (mode) {
            0 => {
                self.eraseInLine(0);
                var r = self.cy + 1;
                while (r < self.grid.rows) : (r += 1) self.grid.clearRow(r);
            },
            1 => {
                var r: u16 = 0;
                while (r < self.cy) : (r += 1) self.grid.clearRow(r);
                self.eraseInLine(1);
            },
            2 => self.grid.clear(),
            else => {},
        }
    }

    pub fn sgr(self: *Terminal, params: []const u16) void {
        if (params.len == 0) {
            self.pen = .{};
            return;
        }
        var i: usize = 0;
        while (i < params.len) : (i += 1) {
            switch (params[i]) {
                0 => self.pen = .{},
                1 => self.pen.attrs.bold = true,
                4 => self.pen.attrs.underline = true,
                7 => self.pen.attrs.reverse = true,
                22 => self.pen.attrs.bold = false,
                24 => self.pen.attrs.underline = false,
                27 => self.pen.attrs.reverse = false,
                30...37 => self.pen.fg = .{ .indexed = @intCast(params[i] - 30) },
                39 => self.pen.fg = .default,
                40...47 => self.pen.bg = .{ .indexed = @intCast(params[i] - 40) },
                49 => self.pen.bg = .default,
                90...97 => self.pen.fg = .{ .indexed = @intCast(params[i] - 90 + 8) },
                100...107 => self.pen.bg = .{ .indexed = @intCast(params[i] - 100 + 8) },
                38 => i += self.extendedColor(params[i..], &self.pen.fg),
                48 => i += self.extendedColor(params[i..], &self.pen.bg),
                else => {},
            }
        }
    }

    fn extendedColor(_: *Terminal, p: []const u16, out: *Color) usize {
        if (p.len >= 3 and p[1] == 5) {
            out.* = .{ .indexed = @intCast(p[2] & 0xff) };
            return 2;
        }
        if (p.len >= 5 and p[1] == 2) {
            out.* = .{ .rgb = .{ .r = @intCast(p[2] & 0xff), .g = @intCast(p[3] & 0xff), .b = @intCast(p[4] & 0xff) } };
            return 4;
        }
        return 0;
    }
};

test "print wraps and advances cursor" {
    var t = try Terminal.init(std.testing.allocator, 2, 3);
    defer t.deinit();
    for ("abcd") |ch| t.print(ch);
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, 'c'), t.grid.at(0, 2).cp);
    try std.testing.expectEqual(@as(u21, 'd'), t.grid.at(1, 0).cp);
    try std.testing.expectEqual(@as(u16, 1), t.cx);
    try std.testing.expectEqual(@as(u16, 1), t.cy);
}

test "wide glyph occupies two columns" {
    var t = try Terminal.init(std.testing.allocator, 1, 5);
    defer t.deinit();
    t.print(0x4E00); // 一, width 2
    t.print('x');
    try std.testing.expectEqual(@as(u21, 0x4E00), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 1).cp); // spacer
    try std.testing.expectEqual(@as(u21, 'x'), t.grid.at(0, 2).cp);
    try std.testing.expectEqual(@as(u16, 3), t.cx);
}

test "wide glyph wraps when one column remains" {
    var t = try Terminal.init(std.testing.allocator, 2, 3);
    defer t.deinit();
    t.print('a');
    t.print('b'); // cols 0,1 filled; one column left
    t.print(0x4E00); // can't fit in col 2 → wraps to next row
    try std.testing.expectEqual(@as(u21, 0x4E00), t.grid.at(1, 0).cp);
    try std.testing.expectEqual(@as(u16, 1), t.cy);
}

test "lineFeed scrolls at bottom" {
    var t = try Terminal.init(std.testing.allocator, 2, 2);
    defer t.deinit();
    t.print('x');
    t.lineFeed();
    t.lineFeed();
    try std.testing.expectEqual(@as(u16, 1), t.cy);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 0).cp);
}

test "setCursor and eraseInLine" {
    var t = try Terminal.init(std.testing.allocator, 3, 5);
    defer t.deinit();
    for ("hello") |ch| t.print(ch);
    t.setCursor(1, 3);
    try std.testing.expectEqual(@as(u16, 0), t.cy);
    try std.testing.expectEqual(@as(u16, 2), t.cx);
    t.eraseInLine(0);
    try std.testing.expectEqual(@as(u21, 'e'), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 2).cp);
}

test "deleteChars shifts row left and blanks tail" {
    var t = try Terminal.init(std.testing.allocator, 1, 6);
    defer t.deinit();
    for ("abcdef") |ch| t.print(ch);
    t.cursorCol(2); // cx = 1 (on 'b')
    t.deleteChars(2); // remove 'b','c'
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, 'd'), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u21, 'f'), t.grid.at(0, 3).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 4).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 5).cp);
}

test "insertChars shifts row right and blanks gap" {
    var t = try Terminal.init(std.testing.allocator, 1, 5);
    defer t.deinit();
    for ("abc") |ch| t.print(ch);
    t.cursorCol(1); // cx = 0
    t.insertChars(2);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 2).cp);
    try std.testing.expectEqual(@as(u21, 'c'), t.grid.at(0, 4).cp);
}

test "eraseChars blanks n cells without shifting" {
    var t = try Terminal.init(std.testing.allocator, 1, 5);
    defer t.deinit();
    for ("abcde") |ch| t.print(ch);
    t.cursorCol(2); // cx = 1
    t.eraseChars(2);
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 2).cp);
    try std.testing.expectEqual(@as(u21, 'd'), t.grid.at(0, 3).cp);
}

test "save and restore cursor" {
    var t = try Terminal.init(std.testing.allocator, 3, 5);
    defer t.deinit();
    t.setCursor(2, 3);
    t.saveCursor();
    t.setCursor(1, 1);
    t.restoreCursor();
    try std.testing.expectEqual(@as(u16, 1), t.cy);
    try std.testing.expectEqual(@as(u16, 2), t.cx);
}

test "alt screen swaps to a blank grid and restores primary" {
    var t = try Terminal.init(std.testing.allocator, 2, 4);
    defer t.deinit();
    for ("hi") |ch| t.print(ch); // primary: row 0 = "hi", cursor at col 2

    t.setMode(1049, true);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 0).cp); // alt is blank
    try std.testing.expectEqual(@as(u16, 0), t.cx);
    t.print('X');
    try std.testing.expectEqual(@as(u21, 'X'), t.grid.at(0, 0).cp);

    t.setMode(1049, false);
    try std.testing.expectEqual(@as(u21, 'h'), t.grid.at(0, 0).cp); // primary restored
    try std.testing.expectEqual(@as(u21, 'i'), t.grid.at(0, 1).cp);
    try std.testing.expectEqual(@as(u16, 2), t.cx); // cursor restored
    try std.testing.expect(t.stash == null);
}

test "redundant alt enter is a no-op" {
    var t = try Terminal.init(std.testing.allocator, 2, 2);
    defer t.deinit();
    t.print('a');
    t.setMode(1049, true);
    t.setMode(47, true); // already in alt; must not lose the parked primary
    t.setMode(1049, false);
    try std.testing.expectEqual(@as(u21, 'a'), t.grid.at(0, 0).cp);
}

test "scrolled-off rows land in scrollback and viewRow reads them back" {
    var t = try Terminal.init(std.testing.allocator, 2, 2);
    defer t.deinit();
    t.print('a');
    t.carriageReturn();
    t.lineFeed();
    t.print('b');
    t.carriageReturn();
    t.lineFeed(); // 'a' row scrolls off into scrollback
    t.print('c');
    try std.testing.expectEqual(@as(usize, 1), t.scrollback.len());

    // live view: rows are 'b','c'
    try std.testing.expectEqual(@as(u21, 'b'), t.viewRow(0)[0].cp);
    try std.testing.expectEqual(@as(u21, 'c'), t.viewRow(1)[0].cp);

    // scroll back one: top row now the scrollback 'a'
    t.scrollView(1);
    try std.testing.expectEqual(@as(usize, 1), t.view_offset);
    try std.testing.expectEqual(@as(u21, 'a'), t.viewRow(0)[0].cp);
    try std.testing.expectEqual(@as(u21, 'b'), t.viewRow(1)[0].cp);

    // scrollView clamps at history depth and at live bottom
    t.scrollView(10);
    try std.testing.expectEqual(@as(usize, 1), t.view_offset);
    t.scrollView(-10);
    try std.testing.expectEqual(@as(usize, 0), t.view_offset);
}

test "resize shrink spills top rows into scrollback, bottom-anchored" {
    var t = try Terminal.init(std.testing.allocator, 3, 2);
    defer t.deinit();
    t.print('a');
    t.carriageReturn();
    t.lineFeed();
    t.print('b');
    t.carriageReturn();
    t.lineFeed();
    t.print('c'); // rows: a / b / c, cursor on row 2

    try t.resize(2, 2); // drop top row 'a' into scrollback, keep b/c
    try std.testing.expectEqual(@as(u16, 2), t.grid.rows);
    try std.testing.expectEqual(@as(usize, 1), t.scrollback.len());
    try std.testing.expectEqual(@as(u21, 'a'), t.scrollback.at(0)[0].cp);
    try std.testing.expectEqual(@as(u21, 'b'), t.viewRow(0)[0].cp);
    try std.testing.expectEqual(@as(u21, 'c'), t.viewRow(1)[0].cp);
    try std.testing.expectEqual(@as(u16, 1), t.cy); // followed bottom
}

test "resize grow pulls history back into the new top rows" {
    var t = try Terminal.init(std.testing.allocator, 2, 2);
    defer t.deinit();
    t.print('a');
    t.carriageReturn();
    t.lineFeed();
    t.print('b');
    t.carriageReturn();
    t.lineFeed(); // 'a' spills to scrollback, grid shows b / blank
    t.print('c');

    try std.testing.expectEqual(@as(usize, 1), t.scrollback.len());
    try t.resize(3, 2); // grow: pull 'a' back to the top
    try std.testing.expectEqual(@as(usize, 0), t.scrollback.len());
    try std.testing.expectEqual(@as(u21, 'a'), t.viewRow(0)[0].cp);
    try std.testing.expectEqual(@as(u21, 'b'), t.viewRow(1)[0].cp);
    try std.testing.expectEqual(@as(u21, 'c'), t.viewRow(2)[0].cp);
}

test "reflow splits a wrapped line when narrowing" {
    var t = try Terminal.init(std.testing.allocator, 3, 4);
    defer t.deinit();
    for ("abcdef") |ch| t.print(ch); // row0 "abcd" (wrapped), row1 "ef"
    try std.testing.expect(t.grid.wrapped[0]);

    try t.resize(3, 2); // "abcdef" rechunks to ab/cd/ef
    try std.testing.expectEqual(@as(u21, 'a'), t.viewRow(0)[0].cp);
    try std.testing.expectEqual(@as(u21, 'b'), t.viewRow(0)[1].cp);
    try std.testing.expectEqual(@as(u21, 'c'), t.viewRow(1)[0].cp);
    try std.testing.expectEqual(@as(u21, 'e'), t.viewRow(2)[0].cp);
    try std.testing.expect(t.grid.wrapped[0]);
    try std.testing.expect(t.grid.wrapped[1]);
    try std.testing.expect(!t.grid.wrapped[2]);
}

test "reflow rejoins a wrapped line when widening" {
    var t = try Terminal.init(std.testing.allocator, 3, 2);
    defer t.deinit();
    for ("abcdef") |ch| t.print(ch); // ab/cd/ef chain
    try std.testing.expect(t.grid.wrapped[0]);

    try t.resize(3, 6); // rejoins into one row "abcdef"
    try std.testing.expectEqual(@as(u21, 'a'), t.viewRow(2)[0].cp);
    try std.testing.expectEqual(@as(u21, 'f'), t.viewRow(2)[5].cp);
    try std.testing.expect(!t.grid.wrapped[2]);
}

test "selection extracts text, trims trailing blanks, joins rows" {
    var t = try Terminal.init(std.testing.allocator, 2, 5);
    defer t.deinit();
    for ("ab") |ch| t.print(ch); // row0: "ab   "
    t.carriageReturn();
    t.lineFeed();
    for ("cd") |ch| t.print(ch); // row1: "cd   "

    t.selectStart(0, 0);
    t.selectExtend(1, 4); // through end of row1
    try std.testing.expect(t.isSelected(0, 0));
    try std.testing.expect(t.isSelected(1, 4));
    var buf: [32]u8 = undefined;
    const n = t.selectionText(&buf);
    try std.testing.expectEqualStrings("ab\ncd", buf[0..n]);
}

test "selection ordered regardless of drag direction" {
    var t = try Terminal.init(std.testing.allocator, 1, 4);
    defer t.deinit();
    for ("wxyz") |ch| t.print(ch);
    t.selectStart(0, 3); // anchor at 'z'
    t.selectExtend(0, 1); // drag left to 'x'
    var buf: [16]u8 = undefined;
    const n = t.selectionText(&buf);
    try std.testing.expectEqualStrings("xyz", buf[0..n]);
}

test "sgr sets and resets pen" {
    var t = try Terminal.init(std.testing.allocator, 1, 4);
    defer t.deinit();
    t.sgr(&.{ 1, 31 });
    try std.testing.expect(t.pen.attrs.bold);
    try std.testing.expectEqual(Color{ .indexed = 1 }, t.pen.fg);
    t.sgr(&.{0});
    try std.testing.expect(!t.pen.attrs.bold);
    try std.testing.expectEqual(Color.default, t.pen.fg);
}
