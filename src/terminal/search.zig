//! Incremental substring search over a Terminal's content rows (scrollback +
//! grid). Pure logic — the renderer and input layer drive it.

const std = @import("std");
const Terminal = @import("terminal.zig").Terminal;
const Cell = @import("cell.zig").Cell;

/// A run of matched cells on one content row.
pub const Match = struct {
    row: usize, // content_row index
    col: usize, // start column
    len: usize, // length in cells (query codepoint count)
};

pub const MatchKind = enum { none, other, current };

/// Upper bound on stored matches — a 1-character query over deep scrollback
/// could otherwise match millions of times.
pub const max_matches = 2048;

pub const Search = struct {
    alloc: std.mem.Allocator,
    query_buf: [256]u8 = undefined,
    query_len: usize = 0,
    matches: std.ArrayList(Match),
    current: usize = 0, // index into matches; 0 when empty

    pub fn init(alloc: std.mem.Allocator) Search {
        return .{ .alloc = alloc, .matches = std.ArrayList(Match).empty };
    }

    pub fn deinit(self: *Search) void {
        self.matches.deinit(self.alloc);
    }

    pub fn query(self: *const Search) []const u8 {
        return self.query_buf[0..self.query_len];
    }

    pub fn count(self: *const Search) usize {
        return self.matches.items.len;
    }

    pub fn currentMatch(self: *const Search) ?Match {
        if (self.matches.items.len == 0) return null;
        return self.matches.items[self.current];
    }

    /// Replace the query text and re-scan `term`. Truncates text past 256
    /// bytes. `current` resets to 0.
    pub fn setQuery(self: *Search, term: *const Terminal, text: []const u8) void {
        const n = @min(text.len, self.query_buf.len);
        @memcpy(self.query_buf[0..n], text[0..n]);
        self.query_len = n;
        self.rescan(term);
    }

    /// Re-run the scan with the existing query (e.g. after new shell output).
    pub fn rescan(self: *Search, term: *const Terminal) void {
        self.matches.clearRetainingCapacity();
        self.current = 0;

        // Decode the query to codepoints; detect smart-case.
        var q: [256]u21 = undefined;
        var qn: usize = 0;
        var case_sensitive = false;
        var view = std.unicode.Utf8View.init(self.query()) catch return;
        var it = view.iterator();
        while (it.nextCodepoint()) |cp| {
            if (qn >= q.len) break;
            if (cp >= 'A' and cp <= 'Z') case_sensitive = true;
            q[qn] = cp;
            qn += 1;
        }
        if (qn == 0) return; // empty query -> no matches

        var r: usize = 0;
        const total = term.lineCount();
        while (r < total) : (r += 1) {
            const row = term.line(r);
            if (row.len < qn) continue;
            var c: usize = 0;
            while (c + qn <= row.len) : (c += 1) {
                if (rowMatchesAt(row, c, q[0..qn], case_sensitive)) {
                    self.matches.append(self.alloc, .{ .row = r, .col = c, .len = qn }) catch return;
                    if (self.matches.items.len >= max_matches) return;
                }
            }
        }
    }

    /// Advance the current match (wraps). No-op when there are no matches.
    pub fn next(self: *Search) void {
        if (self.matches.items.len == 0) return;
        self.current = (self.current + 1) % self.matches.items.len;
    }

    /// Step the current match back (wraps). No-op when there are no matches.
    pub fn prev(self: *Search) void {
        if (self.matches.items.len == 0) return;
        self.current = (self.current + self.matches.items.len - 1) % self.matches.items.len;
    }

    /// How the cell at content (`row`,`col`) should be tinted.
    /// `.current` always wins over `.other` when a cell is covered by both
    /// the current match and another match (e.g. overlapping matches from a
    /// repeated-character query such as "aa" in "aaa").
    pub fn classify(self: *const Search, row: usize, col: usize) MatchKind {
        if (self.matches.items.len == 0) return .none;
        // Pass 1: check the current match first so it always wins.
        const cur = self.matches.items[self.current];
        if (cur.row == row and col >= cur.col and col < cur.col + cur.len) {
            return .current;
        }
        // Pass 2: check all other matches.
        for (self.matches.items, 0..) |m, i| {
            if (i == self.current) continue;
            if (m.row != row) continue;
            if (col >= m.col and col < m.col + m.len) return .other;
        }
        return .none;
    }
};

/// True when `row[col..col+q.len]` equals `q` (codepoint-wise, case-folded
/// for ASCII letters unless `case_sensitive`).
fn rowMatchesAt(row: []const Cell, col: usize, q: []const u21, case_sensitive: bool) bool {
    for (q, 0..) |qc, i| {
        const cc = row[col + i].cp;
        if (case_sensitive) {
            if (cc != qc) return false;
        } else {
            if (lowerCp(cc) != lowerCp(qc)) return false;
        }
    }
    return true;
}

/// Lowercase an ASCII letter codepoint; everything else unchanged.
fn lowerCp(cp: u21) u21 {
    return if (cp >= 'A' and cp <= 'Z') cp + 32 else cp;
}

const testing = std.testing;

test "finds a substring in the grid" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("hello world");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "world");
    try testing.expectEqual(@as(usize, 1), s.count());
    const m = s.currentMatch().?;
    try testing.expectEqual(@as(usize, 6), m.col);
    try testing.expectEqual(@as(usize, 5), m.len);
}

test "smart-case: lowercase query is case-insensitive" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("The Cat SAT");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "cat");
    try testing.expectEqual(@as(usize, 1), s.count()); // matches "Cat"
}

test "smart-case: an uppercase letter forces case-sensitive" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("the cat Cat");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "Cat");
    try testing.expectEqual(@as(usize, 1), s.count()); // only the capitalized one
}

test "empty query yields no matches" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("anything");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "");
    try testing.expectEqual(@as(usize, 0), s.count());
    try testing.expect(s.currentMatch() == null);
}

test "finds multiple matches left-to-right" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("aa aa aa");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "aa");
    try testing.expectEqual(@as(usize, 3), s.count());
}

test "finds a match in scrollback" {
    // 20-wide, 3 visible rows, 1000-row scrollback capacity.
    // Feed "findme" then 5 more newline-terminated lines so "findme"
    // scrolls out of the grid into history.
    var t = try Terminal.init(testing.allocator, 20, 3, 1000);
    defer t.deinit();
    t.feed("findme\r\n");
    t.feed("x\r\n");
    t.feed("x\r\n");
    t.feed("x\r\n");
    t.feed("x\r\n");
    t.feed("x\r\n");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "findme");
    try testing.expectEqual(@as(usize, 1), s.count());
    const m = s.currentMatch().?;
    try testing.expect(m.row < t.history.len());
}

test "next and prev wrap around" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("x x x");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "x");
    try testing.expectEqual(@as(usize, 3), s.count());
    try testing.expectEqual(@as(usize, 0), s.current);
    s.next();
    try testing.expectEqual(@as(usize, 1), s.current);
    s.next();
    s.next();
    try testing.expectEqual(@as(usize, 0), s.current); // wrapped
    s.prev();
    try testing.expectEqual(@as(usize, 2), s.current); // wrapped back
}

test "next/prev are no-ops with no matches" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("abc");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "zzz");
    s.next();
    s.prev();
    try testing.expectEqual(@as(usize, 0), s.current);
}

test "classify tags current, other, and none" {
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("ab ab");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "ab");
    // current match is index 0 at (grid row 0, col 0..2)
    const r0 = t.contentRowOfViewport(0);
    try testing.expectEqual(MatchKind.current, s.classify(r0, 0));
    try testing.expectEqual(MatchKind.current, s.classify(r0, 1));
    try testing.expectEqual(MatchKind.other, s.classify(r0, 3));
    try testing.expectEqual(MatchKind.none, s.classify(r0, 2)); // the space
}

test "classify: current match wins on overlap" {
    // "aaa" with query "aa" produces two overlapping matches: col 0 and col 1.
    // After next(), current == 1 (col 1). Col 1 is covered by both match 0
    // (col 0, len 2) and match 1 (col 1, len 2). classify must return .current.
    var t = try Terminal.init(testing.allocator, 40, 5, 1000);
    defer t.deinit();
    t.feed("aaa");
    var s = Search.init(testing.allocator);
    defer s.deinit();
    s.setQuery(&t, "aa");
    try testing.expectEqual(@as(usize, 2), s.count()); // matches at col 0 and col 1
    s.next(); // current is now index 1 (col 1)
    try testing.expectEqual(@as(usize, 1), s.current);
    const r0 = t.contentRowOfViewport(0);
    // Col 1 is covered by both match 0 (col 0..1) and match 1 (col 1..2).
    // current (match 1) must win.
    try testing.expectEqual(MatchKind.current, s.classify(r0, 1));
}
