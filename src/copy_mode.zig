const std = @import("std");
const Terminal = @import("vt/terminal.zig").Terminal;

pub const CopyMode = struct {
    open: bool = false,
    row: u16 = 0,
    col: u16 = 0,
    visual: bool = false,
    visual_row: u16 = 0,
    visual_col: u16 = 0,

    pub fn enter(self: *CopyMode, term: *const Terminal) void {
        self.open = true;
        self.visual = false;
        // Start the caret at the bottom-right of the visible area.
        self.row = if (term.grid.rows > 0) term.grid.rows - 1 else 0;
        self.col = 0;
    }

    pub fn exit(self: *CopyMode) void {
        self.open = false;
        self.visual = false;
    }

    pub fn startVisual(self: *CopyMode) void {
        self.visual = true;
        self.visual_row = self.row;
        self.visual_col = self.col;
    }

    /// Move the caret by (dr, dc), clamped to [0, rows) x [0, cols).
    /// Scrolls the terminal view to keep the caret visible.
    pub fn move(self: *CopyMode, term: *Terminal, dr: i32, dc: i32) void {
        const rows: i32 = @intCast(term.grid.rows);
        const cols: i32 = @intCast(term.grid.cols);

        var r: i32 = @intCast(self.row);
        var c: i32 = @intCast(self.col);
        r += dr;
        c += dc;

        // Vertical overflow: scroll the view.
        if (r < 0) {
            const scroll_by: i32 = -r;
            term.scrollView(scroll_by);
            r = 0;
        } else if (r >= rows) {
            const scroll_by: i32 = -(r - rows + 1);
            term.scrollView(scroll_by);
            r = rows - 1;
        }

        c = std.math.clamp(c, 0, cols - 1);
        self.row = @intCast(r);
        self.col = @intCast(c);

        if (self.visual) {
            term.selectStart(self.visual_row, self.visual_col);
            term.selectExtend(self.row, self.col);
        }
    }

    pub fn halfPage(self: *CopyMode, term: *Terminal, dir: i32) void {
        const half: i32 = @intCast(@divTrunc(term.grid.rows, 2));
        self.move(term, dir * half, 0);
    }

    pub fn gotoTop(self: *CopyMode, term: *Terminal) void {
        const sb: i32 = @intCast(term.scrollback.len());
        term.scrollView(sb);
        self.row = 0;
        self.col = 0;
        if (self.visual) {
            term.selectStart(self.visual_row, self.visual_col);
            term.selectExtend(self.row, self.col);
        }
    }

    pub fn gotoBottom(self: *CopyMode, term: *Terminal) void {
        term.scrollView(-@as(i32, @intCast(term.view_offset)));
        self.row = if (term.grid.rows > 0) term.grid.rows - 1 else 0;
        self.col = 0;
        if (self.visual) {
            term.selectStart(self.visual_row, self.visual_col);
            term.selectExtend(self.row, self.col);
        }
    }

    pub fn wordForward(self: *CopyMode, term: *Terminal) void {
        const cols: u16 = term.grid.cols;
        var r = self.row;
        var c = self.col;
        const cells = term.viewRow(r);
        // Skip non-space, then space, to land at start of next word.
        while (c < cols and cells[c].cp != ' ') c += 1;
        while (c < cols and cells[c].cp == ' ') c += 1;
        if (c >= cols) {
            self.move(term, 1, 0);
            r = self.row;
            c = 0;
        }
        self.col = c;
        self.row = r;
        if (self.visual) {
            term.selectStart(self.visual_row, self.visual_col);
            term.selectExtend(self.row, self.col);
        }
    }

    pub fn wordBack(self: *CopyMode, term: *Terminal) void {
        var c: i32 = @intCast(self.col);
        if (c == 0) {
            self.move(term, -1, 0);
            c = @intCast(term.grid.cols - 1);
        }
        const cells = term.viewRow(self.row);
        c -= 1;
        while (c > 0 and cells[@intCast(c)].cp == ' ') c -= 1;
        while (c > 0 and cells[@intCast(c - 1)].cp != ' ') c -= 1;
        self.col = @intCast(@max(c, 0));
        if (self.visual) {
            term.selectStart(self.visual_row, self.visual_col);
            term.selectExtend(self.row, self.col);
        }
    }
};

test "copy mode enter places caret at bottom" {
    var t = try Terminal.init(std.testing.allocator, 4, 10);
    defer t.deinit();
    var cm = CopyMode{};
    cm.enter(&t);
    try std.testing.expect(cm.open);
    try std.testing.expectEqual(@as(u16, 3), cm.row); // rows - 1
    try std.testing.expectEqual(@as(u16, 0), cm.col);
    try std.testing.expect(!cm.visual);
}

test "copy mode exit closes" {
    var cm = CopyMode{ .open = true, .visual = true };
    cm.exit();
    try std.testing.expect(!cm.open);
    try std.testing.expect(!cm.visual);
}

test "copy mode move clamps col to grid bounds" {
    var t = try Terminal.init(std.testing.allocator, 4, 10);
    defer t.deinit();
    var cm = CopyMode{ .open = true, .row = 2, .col = 5 };
    cm.move(&t, 0, 100);
    try std.testing.expectEqual(@as(u16, 9), cm.col);
    cm.move(&t, 0, -100);
    try std.testing.expectEqual(@as(u16, 0), cm.col);
}

test "copy mode move scrolls view when caret goes above row 0" {
    var t = try Terminal.init(std.testing.allocator, 4, 10);
    defer t.deinit();
    // Push some scrollback.
    for (0..6) |_| {
        for ("hello") |ch| t.print(ch);
        t.carriageReturn();
        t.lineFeed();
    }
    var cm = CopyMode{ .open = true, .row = 0, .col = 0 };
    const prev_offset = t.view_offset;
    cm.move(&t, -1, 0);
    try std.testing.expect(t.view_offset > prev_offset);
}

test "copy mode move scrolls view when caret goes below last row" {
    var t = try Terminal.init(std.testing.allocator, 4, 10);
    defer t.deinit();
    // Scroll the view into history.
    for (0..6) |_| {
        for ("hello") |ch| t.print(ch);
        t.carriageReturn();
        t.lineFeed();
    }
    t.scrollView(4);
    var cm = CopyMode{ .open = true, .row = 3, .col = 0 };
    const prev_offset = t.view_offset;
    cm.move(&t, 1, 0);
    try std.testing.expect(t.view_offset < prev_offset);
}

test "copy mode visual selection extends with caret" {
    var t = try Terminal.init(std.testing.allocator, 4, 10);
    defer t.deinit();
    for ("hello     ") |ch| t.print(ch);
    var cm = CopyMode{ .open = true, .row = 0, .col = 0 };
    cm.startVisual();
    cm.move(&t, 0, 4);
    try std.testing.expect(t.selection != null);
    const sel = t.selection.?;
    const o = sel.ordered();
    try std.testing.expectEqual(@as(u16, 0), o.start.col);
    try std.testing.expectEqual(@as(u16, 4), o.end.col);
}

test "gotoTop scrolls to max offset" {
    var t = try Terminal.init(std.testing.allocator, 4, 10);
    defer t.deinit();
    for (0..8) |_| {
        for ("hi") |ch| t.print(ch);
        t.carriageReturn();
        t.lineFeed();
    }
    var cm = CopyMode{ .open = true, .row = 3, .col = 0 };
    cm.gotoTop(&t);
    try std.testing.expectEqual(t.scrollback.len(), t.view_offset);
    try std.testing.expectEqual(@as(u16, 0), cm.row);
}

test "gotoBottom returns view to live bottom" {
    var t = try Terminal.init(std.testing.allocator, 4, 10);
    defer t.deinit();
    for (0..8) |_| {
        for ("hi") |ch| t.print(ch);
        t.carriageReturn();
        t.lineFeed();
    }
    t.scrollView(4);
    var cm = CopyMode{ .open = true, .row = 0, .col = 0 };
    cm.gotoBottom(&t);
    try std.testing.expectEqual(@as(usize, 0), t.view_offset);
    try std.testing.expectEqual(@as(u16, 3), cm.row);
}
