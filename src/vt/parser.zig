const std = @import("std");
const Terminal = @import("terminal.zig").Terminal;

const State = enum { ground, escape, csi, osc, osc_esc };

pub const Parser = struct {
    state: State = .ground,
    params: [16]u16 = undefined,
    nparams: usize = 0,
    cur: u16 = 0,
    has_param: bool = false,
    private: bool = false,
    intermediate: u8 = 0, // CSI intermediate byte (0x20–0x2f), e.g. SP for DECSCUSR

    // OSC accumulator (terminated by BEL or ST = ESC \)
    osc_buf: [2048]u8 = undefined,
    osc_len: usize = 0,

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
            .osc => self.osc(term, b),
            .osc_esc => self.oscEsc(term, b),
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
                self.private = false;
                self.intermediate = 0;
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
            ']' => {
                self.osc_len = 0;
                self.state = .osc;
            },
            else => {
                self.state = .ground;
                self.byte(term, b);
            },
        }
    }

    fn osc(self: *Parser, term: *Terminal, b: u8) void {
        switch (b) {
            0x07 => { // BEL terminates
                self.oscDispatch(term);
                self.state = .ground;
            },
            0x1b => self.state = .osc_esc, // maybe ST (ESC \)
            else => {
                if (self.osc_len < self.osc_buf.len) {
                    self.osc_buf[self.osc_len] = b;
                    self.osc_len += 1;
                }
            },
        }
    }

    fn oscEsc(self: *Parser, term: *Terminal, b: u8) void {
        self.oscDispatch(term);
        self.state = .ground;
        // ESC \ is the full ST; a lone ESC means a new sequence starts here.
        if (b != '\\') self.byte(term, b);
    }

    fn oscDispatch(self: *Parser, term: *Terminal) void {
        const buf = self.osc_buf[0..self.osc_len];
        const semi = std.mem.indexOfScalar(u8, buf, ';') orelse return;
        const ps = std.fmt.parseInt(u16, buf[0..semi], 10) catch return;
        const pt = buf[semi + 1 ..];
        switch (ps) {
            0, 2 => term.setTitle(pt),
            7 => term.setCwd(pt),
            52 => { // set clipboard: Pc ; <base64>. Query ("?") is ignored.
                const sc = std.mem.indexOfScalar(u8, pt, ';') orelse return;
                const data = pt[sc + 1 ..];
                if (data.len == 0 or (data.len == 1 and data[0] == '?')) return;
                var dec: [2048]u8 = undefined;
                const dlen = std.base64.standard.Decoder.calcSizeForSlice(data) catch return;
                if (dlen > dec.len) return;
                std.base64.standard.Decoder.decode(dec[0..dlen], data) catch return;
                term.setClipboard(dec[0..dlen]);
            },
            else => {},
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
            '?' => self.private = true,
            0x20...0x2f => self.intermediate = b,
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
            'h' => if (self.private) term.setMode(self.arg(0, 0), true),
            'l' => if (self.private) term.setMode(self.arg(0, 0), false),
            'm' => term.sgr(self.params[0..self.nparams]),
            'q' => if (self.intermediate == ' ') term.setCursorStyle(self.arg(0, 1)),
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

test "CSI ?1049h/l enters and exits the alt screen" {
    var t = try Terminal.init(std.testing.allocator, 2, 4);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "A\x1b[?1049hB");
    try std.testing.expectEqual(@as(u21, 'B'), t.grid.at(0, 0).cp); // on alt
    p.feed(&t, "\x1b[?1049l");
    try std.testing.expectEqual(@as(u21, 'A'), t.grid.at(0, 0).cp); // primary back
}

test "private prefix does not leak into next sequence" {
    var t = try Terminal.init(std.testing.allocator, 1, 4);
    defer t.deinit();
    var p = Parser{};
    // a private set, then a plain CSI K must not be treated as private
    p.feed(&t, "\x1b[?25lab\x1b[1G\x1b[K");
    try std.testing.expectEqual(@as(u21, ' '), t.grid.at(0, 0).cp);
}

test "OSC 2 sets window title (BEL terminated)" {
    var t = try Terminal.init(std.testing.allocator, 1, 10);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "\x1b]2;hello\x07X");
    try std.testing.expectEqualStrings("hello", t.title());
    try std.testing.expectEqual(@as(u21, 'X'), t.grid.at(0, 0).cp); // parsing resumed
}

test "OSC 0 title terminated by ST (ESC backslash)" {
    var t = try Terminal.init(std.testing.allocator, 1, 10);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "\x1b]0;abc\x1b\\Y");
    try std.testing.expectEqualStrings("abc", t.title());
    try std.testing.expectEqual(@as(u21, 'Y'), t.grid.at(0, 0).cp);
}

test "OSC 7 stores path from file URI" {
    var t = try Terminal.init(std.testing.allocator, 1, 10);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "\x1b]7;file://host/Users/me/proj\x07");
    try std.testing.expectEqualStrings("/Users/me/proj", t.cwd());
}

test "mouse modes set and clear via DEC private modes" {
    var t = try Terminal.init(std.testing.allocator, 1, 4);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "\x1b[?1002h\x1b[?1006h");
    try std.testing.expectEqual(@import("terminal.zig").MouseMode.button, t.mouse);
    try std.testing.expect(t.mouse_sgr);
    p.feed(&t, "\x1b[?1002l");
    try std.testing.expectEqual(@import("terminal.zig").MouseMode.off, t.mouse);
}

test "SGR italic/dim/strike/blink set and reset" {
    var t = try Terminal.init(std.testing.allocator, 1, 4);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "\x1b[2;3;5;9mA");
    const a = t.grid.at(0, 0).attrs;
    try std.testing.expect(a.dim and a.italic and a.blink and a.strike);
    p.feed(&t, "\x1b[23;25;29m\x1b[1G\x1b[0mB");
    try std.testing.expect(!t.pen.attrs.italic and !t.pen.attrs.blink and !t.pen.attrs.strike);
}

test "bracketed paste mode toggles via 2004" {
    var t = try Terminal.init(std.testing.allocator, 1, 4);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "\x1b[?2004h");
    try std.testing.expect(t.bracketed_paste);
    p.feed(&t, "\x1b[?2004l");
    try std.testing.expect(!t.bracketed_paste);
}

test "OSC 52 sets clipboard from base64, ignores query" {
    var t = try Terminal.init(std.testing.allocator, 1, 4);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "\x1b]52;c;aGVsbG8=\x07"); // base64("hello")
    try std.testing.expectEqualStrings("hello", t.takeClipboard().?);
    try std.testing.expect(t.takeClipboard() == null); // drained
    p.feed(&t, "\x1b]52;c;?\x07"); // query: must not set anything
    try std.testing.expect(t.takeClipboard() == null);
}

test "DECSCUSR sets cursor shape and blink" {
    var t = try Terminal.init(std.testing.allocator, 1, 4);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "\x1b[5 q"); // blinking bar
    try std.testing.expectEqual(@import("terminal.zig").CursorStyle.bar, t.cursor_style);
    try std.testing.expect(t.cursor_blink);
    p.feed(&t, "\x1b[4 q"); // steady underline
    try std.testing.expectEqual(@import("terminal.zig").CursorStyle.underline, t.cursor_style);
    try std.testing.expect(!t.cursor_blink);
    p.feed(&t, "\x1b[0 q"); // back to default block
    try std.testing.expectEqual(@import("terminal.zig").CursorStyle.block, t.cursor_style);
}

test "UTF-8 multibyte decode" {
    var t = try Terminal.init(std.testing.allocator, 1, 10);
    defer t.deinit();
    var p = Parser{};
    p.feed(&t, "é");
    try std.testing.expectEqual(@as(u21, 0xe9), t.grid.at(0, 0).cp);
}
