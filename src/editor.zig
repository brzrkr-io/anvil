const std = @import("std");
const Terminal = @import("vt/terminal.zig").Terminal;
const syntax = @import("syntax.zig");
const fileview = @import("fileview.zig");
const Color = @import("vt/cell.zig").Color;

const Line = std.ArrayListUnmanaged(u8);
const tab_width: usize = 4;

/// Map a syntax role to a palette color (mirrors session.zig's viewer mapping).
fn roleColor(role: syntax.Role) Color {
    return switch (role) {
        .keyword => .{ .indexed = 13 },
        .string => .{ .indexed = 2 },
        .number => .{ .indexed = 3 },
        .comment => .{ .indexed = 8 },
        .type => .{ .indexed = 6 },
        .punct => .default,
        .text => .default,
    };
}

/// A native, mouse-driven, editable text buffer. Lines are owned byte buffers;
/// the buffer always holds at least one line. Coordinates are byte offsets
/// (ASCII-correct for B1). Renders into a Terminal grid for the existing GPU
/// path to draw, so no renderer changes are needed.
pub const Editor = struct {
    alloc: std.mem.Allocator,
    lines: std.ArrayListUnmanaged(Line) = .empty,
    cur_row: usize = 0,
    cur_col: usize = 0,
    top: usize = 0,
    goal_col: usize = 0,
    // When true, render scrolls the viewport to keep the cursor visible (set on
    // any keyboard input). A manual wheel scroll clears it so the view can move
    // independently of the cursor, like every other editor.
    follow: bool = true,
    lang: syntax.Lang = .unknown,
    path: std.ArrayListUnmanaged(u8) = .empty,
    dirty: bool = false,

    pub fn initEmpty(alloc: std.mem.Allocator) !Editor {
        var ed = Editor{ .alloc = alloc };
        try ed.lines.append(alloc, .empty);
        return ed;
    }

    pub fn deinit(self: *Editor) void {
        for (self.lines.items) |*line| line.deinit(self.alloc);
        self.lines.deinit(self.alloc);
        self.path.deinit(self.alloc);
    }

    /// Load a file into the buffer. Refuses files over the 2 MiB cap and binary
    /// files so saving can never truncate or corrupt the source.
    pub fn loadFile(self: *Editor, path: []const u8) !void {
        var res = try fileview.load(self.alloc, path);
        defer res.deinit(self.alloc);
        if (res.truncated) return error.FileTooLarge;
        if (res.is_binary) return error.BinaryFile;

        self.clearLines();
        var start: usize = 0;
        for (res.bytes, 0..) |b, i| {
            if (b == '\n') {
                try self.pushLineCopy(res.bytes[start..i]);
                start = i + 1;
            }
        }
        if (start < res.bytes.len) try self.pushLineCopy(res.bytes[start..]);
        if (self.lines.items.len == 0) try self.pushLineCopy("");

        try self.setPath(path);
        self.lang = syntax.detect(path);
        self.cur_row = 0;
        self.cur_col = 0;
        self.top = 0;
        self.goal_col = 0;
        self.dirty = false;
    }

    /// Join lines with '\n' (trailing newline) and write to the buffer's path.
    pub fn save(self: *Editor) !void {
        if (self.path.items.len == 0) return error.NoPath;
        var buf: std.ArrayListUnmanaged(u8) = .empty;
        defer buf.deinit(self.alloc);
        for (self.lines.items) |line| {
            try buf.appendSlice(self.alloc, line.items);
            try buf.append(self.alloc, '\n');
        }
        try fileview.save(self.path.items, buf.items);
        self.dirty = false;
    }

    // --- editing ---------------------------------------------------------

    pub fn insertByte(self: *Editor, b: u8) !void {
        try self.lines.items[self.cur_row].insert(self.alloc, self.cur_col, b);
        self.cur_col += 1;
        self.goal_col = self.cur_col;
        self.dirty = true;
    }

    pub fn insertNewline(self: *Editor) !void {
        var tail: Line = .empty;
        errdefer tail.deinit(self.alloc);
        try tail.appendSlice(self.alloc, self.lines.items[self.cur_row].items[self.cur_col..]);
        try self.lines.insert(self.alloc, self.cur_row + 1, tail);
        self.lines.items[self.cur_row].shrinkRetainingCapacity(self.cur_col);
        self.cur_row += 1;
        self.cur_col = 0;
        self.goal_col = 0;
        self.dirty = true;
    }

    pub fn backspace(self: *Editor) !void {
        if (self.cur_col > 0) {
            _ = self.lines.items[self.cur_row].orderedRemove(self.cur_col - 1);
            self.cur_col -= 1;
        } else if (self.cur_row > 0) {
            var cur = self.lines.orderedRemove(self.cur_row);
            const prev = &self.lines.items[self.cur_row - 1];
            self.cur_col = prev.items.len;
            try prev.appendSlice(self.alloc, cur.items);
            cur.deinit(self.alloc);
            self.cur_row -= 1;
        } else return;
        self.goal_col = self.cur_col;
        self.dirty = true;
    }

    pub fn deleteForward(self: *Editor) !void {
        const len = self.lines.items[self.cur_row].items.len;
        if (self.cur_col < len) {
            _ = self.lines.items[self.cur_row].orderedRemove(self.cur_col);
        } else if (self.cur_row + 1 < self.lines.items.len) {
            var next = self.lines.orderedRemove(self.cur_row + 1);
            const cur = &self.lines.items[self.cur_row];
            try cur.appendSlice(self.alloc, next.items);
            next.deinit(self.alloc);
        } else return;
        self.dirty = true;
    }

    // --- cursor movement -------------------------------------------------

    /// Width of the line-number gutter: digit count of the last line number plus
    /// one trailing pad space. Depends only on `lines.len`, so the content column
    /// stays fixed while scrolling. Callers clamp to the grid width.
    fn gutterWidth(self: *const Editor) usize {
        var n = self.lines.items.len; // always >= 1
        var digits: usize = 1;
        while (n >= 10) : (n /= 10) digits += 1;
        return digits + 1;
    }

    fn lineLen(self: *Editor, row: usize) usize {
        return self.lines.items[row].items.len;
    }

    pub fn left(self: *Editor) void {
        if (self.cur_col > 0) {
            self.cur_col -= 1;
        } else if (self.cur_row > 0) {
            self.cur_row -= 1;
            self.cur_col = self.lineLen(self.cur_row);
        }
        self.goal_col = self.cur_col;
    }

    pub fn right(self: *Editor) void {
        if (self.cur_col < self.lineLen(self.cur_row)) {
            self.cur_col += 1;
        } else if (self.cur_row + 1 < self.lines.items.len) {
            self.cur_row += 1;
            self.cur_col = 0;
        }
        self.goal_col = self.cur_col;
    }

    pub fn up(self: *Editor) void {
        if (self.cur_row == 0) return;
        self.cur_row -= 1;
        self.cur_col = @min(self.goal_col, self.lineLen(self.cur_row));
    }

    pub fn down(self: *Editor) void {
        if (self.cur_row + 1 >= self.lines.items.len) return;
        self.cur_row += 1;
        self.cur_col = @min(self.goal_col, self.lineLen(self.cur_row));
    }

    pub fn home(self: *Editor) void {
        self.cur_col = 0;
        self.goal_col = 0;
    }

    pub fn end(self: *Editor) void {
        self.cur_col = self.lineLen(self.cur_row);
        self.goal_col = self.cur_col;
    }

    // --- input -----------------------------------------------------------

    /// Feed raw key bytes. Parses the CSI escapes the shim emits for arrows,
    /// delete, home, and end; everything else is treated as text/control.
    pub fn input(self: *Editor, bytes: []const u8) !void {
        self.follow = true;
        var i: usize = 0;
        while (i < bytes.len) {
            const b = bytes[i];
            if (b == 0x1b and i + 2 < bytes.len and bytes[i + 1] == '[') {
                switch (bytes[i + 2]) {
                    'A' => {
                        self.up();
                        i += 3;
                        continue;
                    },
                    'B' => {
                        self.down();
                        i += 3;
                        continue;
                    },
                    'C' => {
                        self.right();
                        i += 3;
                        continue;
                    },
                    'D' => {
                        self.left();
                        i += 3;
                        continue;
                    },
                    'H' => {
                        self.home();
                        i += 3;
                        continue;
                    },
                    'F' => {
                        self.end();
                        i += 3;
                        continue;
                    },
                    '3' => {
                        if (i + 3 < bytes.len and bytes[i + 3] == '~') {
                            try self.deleteForward();
                            i += 4;
                            continue;
                        }
                        i += 1;
                        continue;
                    },
                    else => {
                        i += 1;
                        continue;
                    },
                }
            }
            switch (b) {
                0x0d, 0x0a => try self.insertNewline(),
                0x7f, 0x08 => try self.backspace(),
                0x09 => {
                    var k: usize = 0;
                    while (k < tab_width) : (k += 1) try self.insertByte(' ');
                },
                else => {
                    if (b >= 0x20 and b < 0x7f) try self.insertByte(b);
                },
            }
            i += 1;
        }
    }

    // --- viewport + mouse ------------------------------------------------

    /// Scroll the viewport by `delta` lines (negative = up), clamped.
    pub fn scroll(self: *Editor, delta: i32, rows: u16) void {
        self.follow = false;
        const max_top: usize = if (self.lines.items.len > rows) self.lines.items.len - rows else 0;
        var t: i64 = @as(i64, @intCast(self.top)) + delta;
        if (t < 0) t = 0;
        if (t > @as(i64, @intCast(max_top))) t = @intCast(max_top);
        self.top = @intCast(t);
    }

    /// Scroll just enough to keep the cursor row inside the viewport.
    pub fn ensureVisible(self: *Editor, rows: u16) void {
        if (self.cur_row < self.top) {
            self.top = self.cur_row;
        } else if (rows > 0 and self.cur_row >= self.top + rows) {
            self.top = self.cur_row - rows + 1;
        }
    }

    /// Place the cursor from a click at the given on-screen cell.
    pub fn click(self: *Editor, screen_row: usize, screen_col: usize, rows: u16) void {
        _ = rows;
        const target = self.top + screen_row;
        self.cur_row = if (target >= self.lines.items.len)
            (if (self.lines.items.len > 0) self.lines.items.len - 1 else 0)
        else
            target;
        const buf_col = screen_col -| self.gutterWidth();
        self.cur_col = @min(buf_col, self.lineLen(self.cur_row));
        self.goal_col = self.cur_col;
    }

    // --- render ----------------------------------------------------------

    /// Paint a right-aligned 1-based line number into columns [0, gw) of screen
    /// row `sr`. The current line's number is bright; others are dim. `term.reset`
    /// has already blanked the row, so unwritten leading columns stay spaces,
    /// giving right-alignment for free.
    fn drawGutter(self: *Editor, term: *Terminal, sr: u16, gw: u16, li: usize) void {
        if (gw < 2) return; // no room for a digit plus the pad space
        const fg: Color = if (li == self.cur_row) .default else .{ .indexed = 8 };
        var num = li + 1;
        var c: u16 = gw - 2; // rightmost digit column (gw-1 is the pad space)
        while (true) {
            term.grid.at(sr, c).* = .{ .cp = @intCast('0' + (num % 10)), .fg = fg };
            num /= 10;
            if (num == 0 or c == 0) break;
            c -= 1;
        }
    }

    /// Paint the visible window into the terminal grid and place the cursor.
    /// Resets the terminal first (clears grid, view_offset=0 so the app draws
    /// the editor cursor at term.cx/cy).
    pub fn render(self: *Editor, term: *Terminal, rows: u16, cols: u16) void {
        term.reset();
        if (self.follow) self.ensureVisible(rows);

        // Gutter occupies the leftmost columns; clamp so content keeps >= 1 col.
        const gw_cap: usize = if (cols > 0) cols - 1 else 0;
        const gw: u16 = @intCast(@min(self.gutterWidth(), gw_cap));

        var tok_buf: [512]syntax.Token = undefined;
        var sr: u16 = 0;
        while (sr < rows) : (sr += 1) {
            const li = self.top + sr;
            if (li >= self.lines.items.len) break;

            self.drawGutter(term, sr, gw, li);

            const line = self.lines.items[li].items;
            const n_toks = syntax.tokenizeLine(self.lang, line, &tok_buf);

            var col: u16 = gw;
            var ti: usize = 0;
            while (ti < n_toks and col < cols) : (ti += 1) {
                const tok = tok_buf[ti];
                const fg = roleColor(tok.role);
                var bi: usize = tok.start;
                while (bi < tok.start + tok.len and col < cols) : (bi += 1) {
                    const byte = line[bi];
                    const cp: u21 = if (byte >= 0x20 and byte < 0x7f) byte else ' ';
                    term.grid.at(sr, col).* = .{ .cp = cp, .fg = fg };
                    col += 1;
                }
            }
        }

        // After a manual scroll the cursor may sit outside the viewport; clamp
        // its on-screen row to the visible edge so it never underflows or draws
        // off-grid.
        const max_row: usize = if (rows > 0) rows - 1 else 0;
        const scr_row: usize = if (self.cur_row < self.top)
            0
        else
            @min(self.cur_row - self.top, max_row);
        const max_col: usize = if (cols > 0) cols - 1 else 0;
        const scr_col = @min(self.cur_col + gw, max_col);
        term.setCursor(@intCast(scr_row + 1), @intCast(scr_col + 1));
    }

    // --- internals -------------------------------------------------------

    fn clearLines(self: *Editor) void {
        for (self.lines.items) |*line| line.deinit(self.alloc);
        self.lines.clearRetainingCapacity();
    }

    fn pushLineCopy(self: *Editor, bytes: []const u8) !void {
        var line: Line = .empty;
        errdefer line.deinit(self.alloc);
        try line.appendSlice(self.alloc, bytes);
        try self.lines.append(self.alloc, line);
    }

    fn setPath(self: *Editor, path: []const u8) !void {
        self.path.clearRetainingCapacity();
        try self.path.appendSlice(self.alloc, path);
    }
};

// --- tests ---------------------------------------------------------------

test "initEmpty has one empty line, cursor at origin" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    try std.testing.expectEqual(@as(usize, 1), ed.lines.items.len);
    try std.testing.expectEqual(@as(usize, 0), ed.cur_row);
    try std.testing.expectEqual(@as(usize, 0), ed.cur_col);
}

test "insertByte builds a line and advances the cursor" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    try ed.input("hello");
    try std.testing.expectEqualStrings("hello", ed.lines.items[0].items);
    try std.testing.expectEqual(@as(usize, 5), ed.cur_col);
    try std.testing.expect(ed.dirty);
}

test "insertNewline splits the current line" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    try ed.input("abcd");
    ed.cur_col = 2;
    try ed.insertNewline();
    try std.testing.expectEqual(@as(usize, 2), ed.lines.items.len);
    try std.testing.expectEqualStrings("ab", ed.lines.items[0].items);
    try std.testing.expectEqualStrings("cd", ed.lines.items[1].items);
    try std.testing.expectEqual(@as(usize, 1), ed.cur_row);
    try std.testing.expectEqual(@as(usize, 0), ed.cur_col);
}

test "backspace within a line and across a line boundary" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    try ed.input("ab\ncd");
    // cursor at end of "cd"
    try ed.backspace();
    try std.testing.expectEqualStrings("c", ed.lines.items[1].items);
    ed.home();
    try ed.backspace(); // merge line 1 into line 0
    try std.testing.expectEqual(@as(usize, 1), ed.lines.items.len);
    try std.testing.expectEqualStrings("abc", ed.lines.items[0].items);
    try std.testing.expectEqual(@as(usize, 2), ed.cur_col);
}

test "deleteForward within a line and pulling the next line up" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    try ed.input("ab\ncd");
    ed.cur_row = 0;
    ed.cur_col = 0;
    try ed.deleteForward(); // removes 'a'
    try std.testing.expectEqualStrings("b", ed.lines.items[0].items);
    ed.end();
    try ed.deleteForward(); // pulls "cd" up
    try std.testing.expectEqual(@as(usize, 1), ed.lines.items.len);
    try std.testing.expectEqualStrings("bcd", ed.lines.items[0].items);
}

test "up/down preserve the goal column" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    try ed.input("longline\nx\nanother");
    ed.cur_row = 0;
    ed.end(); // goal_col = 8
    ed.down(); // line "x" len 1 -> col clamps to 1
    try std.testing.expectEqual(@as(usize, 1), ed.cur_col);
    ed.down(); // line "another" -> col returns toward goal 8 -> len 7
    try std.testing.expectEqual(@as(usize, 7), ed.cur_col);
}

test "input parses arrow-key CSI sequences" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    try ed.input("abc");
    try ed.input("\x1b[D\x1b[D"); // left, left
    try std.testing.expectEqual(@as(usize, 1), ed.cur_col);
    try ed.input("\x1b[C"); // right
    try std.testing.expectEqual(@as(usize, 2), ed.cur_col);
}

test "tab expands to spaces" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    try ed.input("\t");
    try std.testing.expectEqualStrings("    ", ed.lines.items[0].items);
}

test "gutterWidth counts digits of the last line plus a pad space" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    // One line -> "1 " -> width 2.
    try std.testing.expectEqual(@as(usize, 2), ed.gutterWidth());
    // Ten lines -> "10 " -> width 3.
    try ed.input("a\nb\nc\nd\ne\nf\ng\nh\ni\nj");
    try std.testing.expectEqual(@as(usize, 3), ed.gutterWidth());
}

test "scroll clamps to the document bounds" {
    var ed = try Editor.initEmpty(std.testing.allocator);
    defer ed.deinit();
    try ed.input("0\n1\n2\n3\n4\n5"); // 6 lines
    ed.scroll(100, 4); // max_top = 6 - 4 = 2
    try std.testing.expectEqual(@as(usize, 2), ed.top);
    ed.scroll(-100, 4);
    try std.testing.expectEqual(@as(usize, 0), ed.top);
}

test "render keeps a manual scroll instead of snapping back to the cursor" {
    const alloc = std.testing.allocator;
    var term = try Terminal.init(alloc, 2, 10);
    defer term.deinit();
    var ed = try Editor.initEmpty(alloc);
    defer ed.deinit();
    try ed.input("0\n1\n2\n3\n4"); // 5 lines; input leaves follow=true
    ed.cur_row = 0; // cursor at the top of the document
    ed.cur_col = 0;
    ed.scroll(2, 2); // wheel down two lines -> top=2, cursor now off-screen
    ed.render(&term, 2, 10);
    // The viewport stayed where the wheel put it (line "2" at the top), not
    // snapped back to the cursor's line.
    try std.testing.expectEqual(@as(usize, 2), ed.top);
    // 5 lines -> gw = 2; content shifts to col 2.
    try std.testing.expectEqual(@as(u21, '2'), term.grid.at(0, 2).cp);
    // Cursor is clamped on-screen, no underflow.
    try std.testing.expectEqual(@as(u16, 0), term.cy);
    // Typing re-enables follow and brings the cursor back into view.
    try ed.input("x");
    ed.render(&term, 2, 10);
    try std.testing.expectEqual(@as(usize, 0), ed.top);
}

test "render writes cells and places the cursor" {
    const alloc = std.testing.allocator;
    var term = try Terminal.init(alloc, 4, 10);
    defer term.deinit();
    var ed = try Editor.initEmpty(alloc);
    defer ed.deinit();
    try ed.input("ab"); // one line -> gw = 2
    ed.render(&term, 4, 10);
    // Gutter digit '1' for the (current) first line at col gw-2 = 0.
    try std.testing.expectEqual(@as(u21, '1'), term.grid.at(0, 0).cp);
    // Content shifted right by gw = 2.
    try std.testing.expectEqual(@as(u21, 'a'), term.grid.at(0, 2).cp);
    try std.testing.expectEqual(@as(u21, 'b'), term.grid.at(0, 3).cp);
    try std.testing.expectEqual(@as(u16, 0), term.cy);
    try std.testing.expectEqual(@as(u16, 4), term.cx); // cur_col 2 + gw 2
}

test "gutter shows right-aligned line numbers, current line brighter" {
    const alloc = std.testing.allocator;
    var term = try Terminal.init(alloc, 4, 20);
    defer term.deinit();
    var ed = try Editor.initEmpty(alloc);
    defer ed.deinit();
    // 12 lines -> gw = digits(12)+1 = 3. Numbers right-aligned in [0,gw-1),
    // col gw-1 (=2) is the pad space.
    try ed.input("l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12");
    ed.cur_row = 0;
    ed.cur_col = 0;
    ed.top = 0;
    ed.render(&term, 4, 20);
    // Row 0 = line 1 -> " 1": col0 blank, col1 '1'.
    try std.testing.expectEqual(@as(u21, ' '), term.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, '1'), term.grid.at(0, 1).cp);
    // Row 3 = line 4 -> " 4".
    try std.testing.expectEqual(@as(u21, '4'), term.grid.at(3, 1).cp);
    // Content starts at col gw = 3.
    try std.testing.expectEqual(@as(u21, 'l'), term.grid.at(0, 3).cp);
    // Current line number is bright (.default); others dim (indexed 8).
    try std.testing.expectEqual(Color.default, term.grid.at(0, 1).fg);
    try std.testing.expectEqual(Color{ .indexed = 8 }, term.grid.at(3, 1).fg);
    // Multi-digit numbers fill both digit columns. Scroll so line 10 is visible.
    ed.top = 8; // viewport shows lines 9..12
    ed.follow = false; // keep the manual viewport; don't snap to the cursor
    ed.render(&term, 4, 20);
    // Row 1 = line 10 -> "10": col0 '1', col1 '0'.
    try std.testing.expectEqual(@as(u21, '1'), term.grid.at(1, 0).cp);
    try std.testing.expectEqual(@as(u21, '0'), term.grid.at(1, 1).cp);
}

test "click maps past the gutter to the buffer column" {
    const alloc = std.testing.allocator;
    var ed = try Editor.initEmpty(alloc);
    defer ed.deinit();
    try ed.input("hello world"); // one line -> gw = 2
    ed.click(0, 2 + 3, 4); // screen col gw+3 -> buffer col 3
    try std.testing.expectEqual(@as(usize, 0), ed.cur_row);
    try std.testing.expectEqual(@as(usize, 3), ed.cur_col);
    // A click inside the gutter saturates to buffer column 0.
    ed.click(0, 1, 4);
    try std.testing.expectEqual(@as(usize, 0), ed.cur_col);
}

test "loadFile and save round-trip through disk" {
    const alloc = std.testing.allocator;
    const path = "/tmp/anvil_editor_roundtrip_test.txt";
    try fileview.save(path, "hello\nworld\n");
    defer _ = std.c.unlink(path);

    var ed = try Editor.initEmpty(alloc);
    defer ed.deinit();
    try ed.loadFile(path);
    try std.testing.expectEqual(@as(usize, 2), ed.lines.items.len);
    try std.testing.expectEqualStrings("hello", ed.lines.items[0].items);

    ed.cur_row = 0;
    ed.end();
    try ed.insertByte('!');
    try ed.save();

    var ed2 = try Editor.initEmpty(alloc);
    defer ed2.deinit();
    try ed2.loadFile(path);
    try std.testing.expectEqualStrings("hello!", ed2.lines.items[0].items);
    try std.testing.expectEqualStrings("world", ed2.lines.items[1].items);
}
