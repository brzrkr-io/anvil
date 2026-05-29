const std = @import("std");
const Terminal = @import("terminal.zig").Terminal;

const State = enum { ground, escape, csi };

pub const Parser = struct {
    state: State = .ground,
    params: [16]u16 = undefined,
    nparams: usize = 0,
    cur: u16 = 0,
    has_param: bool = false,

    // UTF-8 accumulator
    cp: u21 = 0,
    pending: u8 = 0,

    pub fn feed(self: *Parser, term: *Terminal, bytes: []const u8) void {
        for (bytes) |b| self.byte(term, b);
    }

    fn byte(self: *Parser, term: *Terminal, b: u8) void {
        switch (self.state) {
            .ground => self.ground(term, b),
            .escape => self.escape(term, b),
            .csi => self.csi(term, b),
        }
    }

    fn ground(self: *Parser, term: *Terminal, b: u8) void {
        if (self.pending > 0) {
            if (b & 0xc0 == 0x80) {
                self.cp = (self.cp << 6) | (b & 0x3f);
                self.pending -= 1;
                if (self.pending == 0) term.print(self.cp);
            } else {
                self.pending = 0;
            }
            return;
        }
        switch (b) {
            0x1b => self.state = .escape,
            0x07 => {},
            0x08 => term.backspace(),
            0x09 => term.tab(),
            0x0a, 0x0b, 0x0c => term.lineFeed(),
            0x0d => term.carriageReturn(),
            else => {
                if (b < 0x80) {
                    term.print(b);
                } else if (b & 0xe0 == 0xc0) {
                    self.cp = b & 0x1f;
                    self.pending = 1;
                } else if (b & 0xf0 == 0xe0) {
                    self.cp = b & 0x0f;
                    self.pending = 2;
                } else if (b & 0xf8 == 0xf0) {
                    self.cp = b & 0x07;
                    self.pending = 3;
                }
            },
        }
    }

    fn escape(self: *Parser, term: *Terminal, b: u8) void {
        switch (b) {
            '[' => {
                self.params = undefined;
                self.nparams = 0;
                self.cur = 0;
                self.has_param = false;
                self.state = .csi;
            },
            '7' => {
                term.saveCursor();
                self.state = .ground;
            },
            '8' => {
                term.restoreCursor();
                self.state = .ground;
            },
            else => {
                self.state = .ground;
                self.byte(term, b);
            },
        }
    }

    fn csi(self: *Parser, term: *Terminal, b: u8) void {
        switch (b) {
            '0'...'9' => {
                self.cur = self.cur *% 10 +% (b - '0');
                self.has_param = true;
            },
            ';' => {
                self.pushParam();
            },
            0x40...0x7e => {
                self.pushParam();
                self.dispatch(term, b);
                self.state = .ground;
            },
            else => {},
        }
    }

    fn pushParam(self: *Parser) void {
        if (self.nparams < self.params.len) {
            self.params[self.nparams] = self.cur;
            self.nparams += 1;
        }
        self.cur = 0;
    }

    fn arg(self: *Parser, i: usize, default: u16) u16 {
        if (i >= self.nparams) return default;
        const v = self.params[i];
        return if (v == 0) default else v;
    }

    fn dispatch(self: *Parser, term: *Terminal, final: u8) void {
        switch (final) {
            'A' => term.cursorUp(self.arg(0, 1)),
            'B' => term.cursorDown(self.arg(0, 1)),
            'C' => term.cursorForward(self.arg(0, 1)),
            'D' => term.cursorBack(self.arg(0, 1)),
            'H', 'f' => term.setCursor(self.arg(0, 1), self.arg(1, 1)),
            'G' => term.cursorCol(self.arg(0, 1)),
            'd' => term.cursorRow(self.arg(0, 1)),
            'J' => term.eraseInDisplay(self.arg(0, 0)),
            'K' => term.eraseInLine(self.arg(0, 0)),
            'P' => term.deleteChars(self.arg(0, 1)),
            '@' => term.insertChars(self.arg(0, 1)),
            'X' => term.eraseChars(self.arg(0, 1)),
            's' => term.saveCursor(),
            'u' => term.restoreCursor(),
            'm' => term.sgr(self.params[0..self.nparams]),
            else => {},
        }
    }
};

test "feed prints text" {
    var t = try Terminal.init(std.testing.allocator, 2, 10);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "hi");
    try std.testing.expectEqual(@as(u21, 'h'), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, 'i'), t.grid.at(0, 1).cp);
}

test "CSI cursor position and erase" {
    var t = try Terminal.init(std.testing.allocator, 3, 10);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "abc\x1b[1;1Hxy");
    try std.testing.expectEqual(@as(u21, 'x'), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, 'y'), t.grid.at(0, 1).cp);
}

test "CSI SGR sets pen" {
    var t = try Terminal.init(std.testing.allocator, 1, 10);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "\x1b[1;31mA");
    try std.testing.expect(t.grid.at(0, 0).attrs.bold);
    try std.testing.expectEqual(@import("cell.zig").Color{ .indexed = 1 }, t.grid.at(0, 0).fg);
}

test "CHA repaint overwrites stale tail (clearaar bug)" {
    var t = try Terminal.init(std.testing.allocator, 1, 10);
    defer t.deinit();
    var p = Parser{};
    // stale "clearaar", then zsh repaints: col 1, write "clear", erase to EOL
    p.feed(&t, "clearaar\x1b[1G\x1b[0mclear\x1b[K");
    try std.testing.expectEqual(@as(u21, 'c'), t.grid.at(0, 0).cp);
    try std.testing.expectEqual(@as(u21, 'r'), t.grid.at(0, 4).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 5).cp);
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 7).cp);
}

test "DECSC/DECRC save and restore cursor" {
    var t = try Terminal.init(std.testing.allocator, 2, 5);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "ab\x1b7cd\x1b8X");
    try std.testing.expectEqual(@as(u21, 'X'), t.grid.at(0, 2).cp);
}

test "UTF-8 multibyte decode" {
    var t = try Terminal.init(std.testing.allocator, 1, 10);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "é");
    try std.testing.expectEqual(@as(u21, 0xe9), t.grid.at(0, 0).cp);
}
