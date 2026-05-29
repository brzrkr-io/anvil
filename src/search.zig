const std = @import("std");
const Terminal = @import("vt/terminal.zig").Terminal;
const regex = @import("regex.zig");

const max_query = 128;
const max_matches = 512;

/// One scrollback hit: logical line, starting column, and length in cells.
pub const Match = struct { line: usize, col: u16, len: u16 };

/// Incremental scrollback search. Holds the query, the matches found across
/// the whole logical buffer (scrollback + grid), and the current selection.
pub const Search = struct {
    open: bool = false,
    query: [max_query]u8 = undefined,
    qlen: usize = 0,
    matches: [max_matches]Match = undefined,
    count: usize = 0,
    cur: usize = 0,
    regex_mode: bool = false,
    bad_pattern: bool = false,

    pub fn show(self: *Search) void {
        self.open = true;
        self.qlen = 0;
        self.count = 0;
        self.cur = 0;
    }

    pub fn hide(self: *Search) void {
        self.open = false;
    }

    pub fn queryStr(self: *const Search) []const u8 {
        return self.query[0..self.qlen];
    }

    pub fn typeChar(self: *Search, c: u8, term: *Terminal) void {
        if (self.qlen >= max_query) return;
        self.query[self.qlen] = c;
        self.qlen += 1;
        self.run(term);
    }

    pub fn backspace(self: *Search, term: *Terminal) void {
        if (self.qlen == 0) return;
        self.qlen -= 1;
        self.run(term);
    }

    pub fn toggleRegex(self: *Search, term: *Terminal) void {
        self.regex_mode = !self.regex_mode;
        self.run(term);
    }

    pub fn current(self: *const Search) ?Match {
        if (self.count == 0) return null;
        return self.matches[self.cur];
    }

    /// Advance to the next match (wraps). Newest matches sort last, so "next"
    /// moves toward the bottom of the buffer.
    pub fn next(self: *Search) ?Match {
        if (self.count == 0) return null;
        self.cur = (self.cur + 1) % self.count;
        return self.matches[self.cur];
    }

    pub fn prev(self: *Search) ?Match {
        if (self.count == 0) return null;
        self.cur = (self.cur + self.count - 1) % self.count;
        return self.matches[self.cur];
    }

    /// Rescan the whole buffer. Uses regex matching when regex_mode is on;
    /// falls back to case-insensitive substring on compile failure.
    pub fn run(self: *Search, term: *Terminal) void {
        self.count = 0;
        self.bad_pattern = false;
        const q = self.queryStr();
        if (q.len == 0) return;

        if (self.regex_mode) {
            if (regex.compile(q)) |pat| {
                var line: usize = 0;
                const total = term.totalLines();
                while (line < total and self.count < max_matches) : (line += 1) {
                    const cells = term.logicalRow(line);
                    findInRowRegex(self, cells, line, &pat);
                }
            } else {
                self.bad_pattern = true;
                self.runSubstring(term, q);
            }
        } else {
            self.runSubstring(term, q);
        }

        self.cur = if (self.count == 0) 0 else self.count - 1;
    }

    fn runSubstring(self: *Search, term: *Terminal, q: []const u8) void {
        var line: usize = 0;
        const total = term.totalLines();
        while (line < total and self.count < max_matches) : (line += 1) {
            const cells = term.logicalRow(line);
            findInRow(self, cells, line, q);
        }
    }

    fn findInRowRegex(self: *Search, cells: []const @import("vt/cell.zig").Cell, line: usize, pat: *const regex.Pattern) void {
        // Extract ASCII text from cells into a scratch buffer.
        var buf: [512]u8 = undefined;
        var blen: usize = 0;
        for (cells) |cell| {
            if (blen >= buf.len) break;
            const cp = cell.cp;
            buf[blen] = if (cp > 0 and cp <= 0x7e) @intCast(cp) else ' ';
            blen += 1;
        }
        const text = buf[0..blen];
        var start: usize = 0;
        while (start <= text.len) {
            const r = regex.search(pat, text[start..]) orelse break;
            if (self.count >= max_matches) break;
            self.matches[self.count] = .{
                .line = line,
                .col = @intCast(start + r.start),
                .len = @intCast(@max(1, r.len)),
            };
            self.count += 1;
            const advance = r.start + @max(1, r.len);
            if (start + advance <= start) break;
            start += advance;
        }
    }

    fn findInRow(self: *Search, cells: []const @import("vt/cell.zig").Cell, line: usize, q: []const u8) void {
        if (cells.len < q.len) return;
        var start: usize = 0;
        while (start + q.len <= cells.len) : (start += 1) {
            var k: usize = 0;
            while (k < q.len) : (k += 1) {
                const ch = cells[start + k].cp;
                if (ch > 0x7f or lower(@intCast(ch)) != lower(q[k])) break;
            }
            if (k == q.len) {
                if (self.count >= max_matches) return;
                self.matches[self.count] = .{ .line = line, .col = @intCast(start), .len = @intCast(q.len) };
                self.count += 1;
                start += q.len - 1; // non-overlapping
            }
        }
    }
};

fn lower(c: u8) u8 {
    return if (c >= 'A' and c <= 'Z') c + 32 else c;
}

test "finds a substring in the live grid" {
    var t = try Terminal.init(std.testing.allocator, 2, 20);
    defer t.deinit();
    for ("hello world") |ch| t.print(ch);
    var s = Search{};
    s.show();
    for ("world") |ch| s.typeChar(ch, &t);
    try std.testing.expectEqual(@as(usize, 1), s.count);
    const m = s.current().?;
    try std.testing.expectEqual(@as(u16, 6), m.col);
    try std.testing.expectEqual(@as(u16, 5), m.len);
}

test "case-insensitive, multiple matches, cur starts at last" {
    var t = try Terminal.init(std.testing.allocator, 1, 20);
    defer t.deinit();
    for ("Foo foo FOO") |ch| t.print(ch);
    var s = Search{};
    s.show();
    for ("foo") |ch| s.typeChar(ch, &t);
    try std.testing.expectEqual(@as(usize, 3), s.count);
    try std.testing.expectEqual(@as(u16, 8), s.current().?.col); // last hit
    _ = s.next(); // wraps to first
    try std.testing.expectEqual(@as(u16, 0), s.current().?.col);
}

test "backspace re-runs and empty query clears matches" {
    var t = try Terminal.init(std.testing.allocator, 1, 10);
    defer t.deinit();
    for ("abcabc") |ch| t.print(ch);
    var s = Search{};
    s.show();
    for ("ab") |ch| s.typeChar(ch, &t);
    try std.testing.expectEqual(@as(usize, 2), s.count);
    s.backspace(&t); // "a"
    try std.testing.expectEqual(@as(usize, 2), s.count);
    s.backspace(&t); // ""
    try std.testing.expectEqual(@as(usize, 0), s.count);
}

test "matches found in scrollback history" {
    var t = try Terminal.init(std.testing.allocator, 1, 8);
    defer t.deinit();
    for ("needle") |ch| t.print(ch);
    t.carriageReturn();
    t.lineFeed(); // pushes "needle" into scrollback
    for ("xyz") |ch| t.print(ch);
    var s = Search{};
    s.show();
    for ("needle") |ch| s.typeChar(ch, &t);
    try std.testing.expectEqual(@as(usize, 1), s.count);
    try std.testing.expectEqual(@as(usize, 0), s.current().?.line); // oldest line
}

test "regex mode: dot and plus match" {
    var t = try Terminal.init(std.testing.allocator, 1, 20);
    defer t.deinit();
    for ("foo123bar") |ch| t.print(ch);
    var s = Search{};
    s.show();
    s.regex_mode = true;
    for ("[0-9]+") |ch| s.typeChar(ch, &t);
    try std.testing.expectEqual(@as(usize, 1), s.count);
    const m = s.current().?;
    try std.testing.expectEqual(@as(u16, 3), m.col);
    try std.testing.expectEqual(@as(u16, 3), m.len);
}

test "regex mode: bad pattern falls back to substring, sets bad_pattern" {
    var t = try Terminal.init(std.testing.allocator, 1, 20);
    defer t.deinit();
    for ("*star*") |ch| t.print(ch);
    var s = Search{};
    s.show();
    s.regex_mode = true;
    for ("*star*") |ch| s.typeChar(ch, &t); // leading * is invalid regex
    try std.testing.expect(s.bad_pattern);
    // Fell back to substring; "*star*" is a literal match.
    try std.testing.expectEqual(@as(usize, 1), s.count);
}

test "toggleRegex reruns search" {
    var t = try Terminal.init(std.testing.allocator, 1, 20);
    defer t.deinit();
    for ("abc123") |ch| t.print(ch);
    var s = Search{};
    s.show();
    for ("abc") |ch| s.typeChar(ch, &t);
    try std.testing.expectEqual(@as(usize, 1), s.count);
    // Enable regex mode via toggle — same result for literal query.
    s.toggleRegex(&t);
    try std.testing.expect(s.regex_mode);
    try std.testing.expectEqual(@as(usize, 1), s.count);
    // Toggle off.
    s.toggleRegex(&t);
    try std.testing.expect(!s.regex_mode);
}

test "match count for known buffer" {
    var t = try Terminal.init(std.testing.allocator, 1, 30);
    defer t.deinit();
    for ("error: bad error: very bad") |ch| t.print(ch);
    var s = Search{};
    s.show();
    for ("error") |ch| s.typeChar(ch, &t);
    try std.testing.expectEqual(@as(usize, 2), s.count);
    // cur starts at last match (index 1).
    try std.testing.expectEqual(@as(usize, 1), s.cur);
    _ = s.next(); // wraps to index 0
    try std.testing.expectEqual(@as(usize, 0), s.cur);
}
