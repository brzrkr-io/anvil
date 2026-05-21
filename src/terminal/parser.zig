//! A VT/ANSI escape-sequence parser implementing Paul Williams' VT500-series
//! parser DFA (the well-known public-domain state diagram, implemented fresh).
//!
//! The parser is byte-oriented and stateful: feed it arbitrary chunks and it
//! drives a caller-supplied handler. UTF-8 is decoded in the ground state so
//! the handler always receives whole Unicode scalars via `print`.
//!
//! The handler (passed as `anytype`) must provide:
//!   print(cp: u21)
//!   execute(byte: u8)
//!   csiDispatch(intermediates: []const u8, params: []const u16, final: u8)
//!   escDispatch(intermediates: []const u8, final: u8)
//!   oscDispatch(data: []const u8)
//! DCS hook/put/unhook are optional; missing methods are treated as no-ops.

const std = @import("std");

/// Replacement scalar emitted for malformed UTF-8.
const replacement: u21 = 0xFFFD;

/// Upper bound on collected CSI numeric parameters. Sequences with more are
/// still dispatched; the excess params are simply dropped.
const max_params = 32;

/// Upper bound on collected intermediate / private-marker bytes.
const max_intermediates = 4;

/// Upper bound on buffered OSC payload. Longer strings are truncated.
const max_osc = 1024;

/// The DFA states from the Williams diagram.
const State = enum {
    ground,
    escape,
    escape_intermediate,
    csi_entry,
    csi_param,
    csi_intermediate,
    csi_ignore,
    dcs_entry,
    dcs_param,
    dcs_intermediate,
    dcs_passthrough,
    dcs_ignore,
    osc_string,
    sos_pm_apc_string,
};

pub const Parser = struct {
    state: State = .ground,

    /// Collected CSI/DCS numeric parameters. `param_count` may exceed
    /// `max_params`; only the first `max_params` slots are valid.
    params: [max_params]u16 = undefined,
    param_count: usize = 0,
    /// True once at least one digit or `;` of the current param run is seen,
    /// so a bare `CSI m` still dispatches a single (default) param.
    param_started: bool = false,

    intermediates: [max_intermediates]u8 = undefined,
    intermediate_count: usize = 0,

    osc_buf: [max_osc]u8 = undefined,
    osc_len: usize = 0,

    /// Partial UTF-8 sequence carried across `feed` calls.
    utf8_buf: [4]u8 = undefined,
    utf8_len: usize = 0,
    utf8_needed: usize = 0,

    pub fn init() Parser {
        return .{};
    }

    /// Parse `bytes`, driving `handler`. Safe to call repeatedly with
    /// arbitrary chunk boundaries — escape and UTF-8 state is retained.
    pub fn feed(self: *Parser, handler: anytype, bytes: []const u8) void {
        for (bytes) |byte| self.feedByte(handler, byte);
    }

    fn feedByte(self: *Parser, handler: anytype, byte: u8) void {
        // A multi-byte UTF-8 scalar in progress short-circuits the DFA: its
        // continuation bytes are data, never controls.
        if (self.utf8_needed != 0) {
            self.continueUtf8(handler, byte);
            return;
        }

        // C0 controls and a handful of C1 bytes can interrupt most states;
        // the Williams diagram routes these from the "anywhere" pseudo-state.
        if (self.handleAnywhere(handler, byte)) return;

        switch (self.state) {
            .ground => self.groundByte(handler, byte),
            .escape => self.escapeByte(handler, byte),
            .escape_intermediate => self.escapeIntermediateByte(handler, byte),
            .csi_entry => self.csiEntryByte(handler, byte),
            .csi_param => self.csiParamByte(handler, byte),
            .csi_intermediate => self.csiIntermediateByte(handler, byte),
            .csi_ignore => self.csiIgnoreByte(byte),
            .dcs_entry => self.dcsEntryByte(byte),
            .dcs_param => self.dcsParamByte(byte),
            .dcs_intermediate => self.dcsIntermediateByte(byte),
            .dcs_passthrough => self.dcsPassthroughByte(handler, byte),
            .dcs_ignore => {},
            .osc_string => self.oscStringByte(byte),
            .sos_pm_apc_string => {},
        }
    }

    /// Transitions valid from (almost) any state: ESC restarts a sequence,
    /// C0 controls execute, and CAN/SUB abort to ground. Returns true when
    /// the byte was consumed here.
    fn handleAnywhere(self: *Parser, handler: anytype, byte: u8) bool {
        switch (byte) {
            0x1B => { // ESC
                // An ESC inside an OSC string is the first half of an ST
                // (`ESC \`) terminator: finalize the pending OSC now. The
                // trailing `\` then dispatches as a harmless ESC.
                if (self.state == .osc_string) self.dispatchOsc(handler);
                if (self.state == .dcs_passthrough) callDcsUnhook(handler);
                self.clear();
                self.state = .escape;
                return true;
            },
            0x18, 0x1A => { // CAN, SUB — abort to ground
                if (byte == 0x1A) callExecute(handler, byte);
                self.state = .ground;
                return true;
            },
            // C0 controls execute immediately, except inside string states
            // where they are part of (or terminate) the payload.
            0x00...0x17, 0x19, 0x1C...0x1F => {
                switch (self.state) {
                    .osc_string => {
                        if (byte == 0x07) { // BEL terminates OSC
                            self.dispatchOsc(handler);
                            self.state = .ground;
                        }
                        return true;
                    },
                    .dcs_passthrough, .dcs_entry, .dcs_param, .dcs_intermediate, .dcs_ignore, .sos_pm_apc_string => return true,
                    else => {
                        callExecute(handler, byte);
                        return true;
                    },
                }
            },
            else => return false,
        }
    }

    // --- ground ------------------------------------------------------------

    fn groundByte(self: *Parser, handler: anytype, byte: u8) void {
        if (byte < 0x80) {
            callPrint(handler, byte);
            return;
        }
        self.beginUtf8(handler, byte);
    }

    // --- escape ------------------------------------------------------------

    fn escapeByte(self: *Parser, handler: anytype, byte: u8) void {
        switch (byte) {
            0x20...0x2F => { // intermediate
                self.collectIntermediate(byte);
                self.state = .escape_intermediate;
            },
            0x30...0x4F, 0x51...0x57, 0x59, 0x5A, 0x5C, 0x60...0x7E => {
                callEscDispatch(handler, self.intermediateSlice(), byte);
                self.state = .ground;
            },
            0x50 => { // ESC P -> DCS
                self.clear();
                self.state = .dcs_entry;
            },
            0x58, 0x5E, 0x5F => { // SOS, PM, APC
                self.state = .sos_pm_apc_string;
            },
            0x5B => self.state = .csi_entry, // ESC [ -> CSI
            0x5D => { // ESC ] -> OSC
                self.osc_len = 0;
                self.state = .osc_string;
            },
            else => self.state = .ground,
        }
    }

    fn escapeIntermediateByte(self: *Parser, handler: anytype, byte: u8) void {
        switch (byte) {
            0x20...0x2F => self.collectIntermediate(byte),
            0x30...0x7E => {
                callEscDispatch(handler, self.intermediateSlice(), byte);
                self.state = .ground;
            },
            else => self.state = .ground,
        }
    }

    // --- CSI ---------------------------------------------------------------

    fn csiEntryByte(self: *Parser, handler: anytype, byte: u8) void {
        switch (byte) {
            0x30...0x39, 0x3B => { // digit or ';'
                self.csiParamByte(handler, byte);
            },
            0x3A => self.state = .csi_ignore, // ':' — sub-params unsupported
            0x3C...0x3F => { // private markers < = > ?
                self.collectIntermediate(byte);
                self.state = .csi_param;
            },
            0x20...0x2F => {
                self.collectIntermediate(byte);
                self.state = .csi_intermediate;
            },
            0x40...0x7E => {
                self.dispatchCsi(handler, byte);
            },
            else => self.state = .csi_ignore,
        }
    }

    fn csiParamByte(self: *Parser, handler: anytype, byte: u8) void {
        switch (byte) {
            0x30...0x39 => { // digit
                self.param_started = true;
                self.state = .csi_param;
                if (self.param_count == 0) self.param_count = 1;
                self.appendDigit(byte);
            },
            0x3B => { // ';' — next param slot
                self.param_started = true;
                self.state = .csi_param;
                self.nextParam();
            },
            0x3A => self.state = .csi_ignore,
            0x20...0x2F => {
                self.collectIntermediate(byte);
                self.state = .csi_intermediate;
            },
            0x3C...0x3F => self.state = .csi_ignore, // private marker mid-param
            0x40...0x7E => self.dispatchCsi(handler, byte),
            else => self.state = .csi_ignore,
        }
    }

    fn csiIntermediateByte(self: *Parser, handler: anytype, byte: u8) void {
        switch (byte) {
            0x20...0x2F => self.collectIntermediate(byte),
            0x30...0x3F => self.state = .csi_ignore,
            0x40...0x7E => self.dispatchCsi(handler, byte),
            else => self.state = .csi_ignore,
        }
    }

    fn csiIgnoreByte(self: *Parser, byte: u8) void {
        if (byte >= 0x40 and byte <= 0x7E) self.state = .ground;
    }

    fn dispatchCsi(self: *Parser, handler: anytype, final: u8) void {
        const count = if (self.param_started and self.param_count == 0) 1 else self.param_count;
        const valid = @min(count, max_params);
        callCsiDispatch(handler, self.intermediateSlice(), self.params[0..valid], final);
        self.state = .ground;
    }

    // --- DCS ---------------------------------------------------------------
    // DCS is parsed structurally but its payload is delivered via optional
    // hook/put/unhook methods; missing methods make these no-ops.

    fn dcsEntryByte(self: *Parser, byte: u8) void {
        switch (byte) {
            0x30...0x39, 0x3B => self.dcsParamByte(byte),
            0x3C...0x3F => {
                self.collectIntermediate(byte);
                self.state = .dcs_param;
            },
            0x20...0x2F => {
                self.collectIntermediate(byte);
                self.state = .dcs_intermediate;
            },
            0x40...0x7E => self.state = .dcs_passthrough,
            0x3A => self.state = .dcs_ignore,
            else => self.state = .dcs_ignore,
        }
    }

    fn dcsParamByte(self: *Parser, byte: u8) void {
        switch (byte) {
            0x30...0x39 => {
                self.state = .dcs_param;
                if (self.param_count == 0) self.param_count = 1;
                self.appendDigit(byte);
            },
            0x3B => {
                self.state = .dcs_param;
                self.nextParam();
            },
            0x20...0x2F => {
                self.collectIntermediate(byte);
                self.state = .dcs_intermediate;
            },
            0x40...0x7E => self.state = .dcs_passthrough,
            else => self.state = .dcs_ignore,
        }
    }

    fn dcsIntermediateByte(self: *Parser, byte: u8) void {
        switch (byte) {
            0x20...0x2F => self.collectIntermediate(byte),
            0x40...0x7E => self.state = .dcs_passthrough,
            else => self.state = .dcs_ignore,
        }
    }

    fn dcsPassthroughByte(self: *Parser, handler: anytype, byte: u8) void {
        if (byte == 0x9C) { // ST (8-bit)
            callDcsUnhook(handler);
            self.state = .ground;
            return;
        }
        callDcsPut(handler, byte);
    }

    // --- OSC ---------------------------------------------------------------

    fn oscStringByte(self: *Parser, byte: u8) void {
        // ST is `ESC \`; the ESC is caught by handleAnywhere, so a `\` while
        // still in osc_string only arrives as ordinary data. The escape path
        // below finishes the OSC when ESC then `\` is seen.
        if (self.osc_len < max_osc) {
            self.osc_buf[self.osc_len] = byte;
            self.osc_len += 1;
        }
    }

    /// Finalize a pending OSC string. Called on BEL or ST.
    fn dispatchOsc(self: *Parser, handler: anytype) void {
        callOscDispatch(handler, self.osc_buf[0..self.osc_len]);
        self.osc_len = 0;
    }

    // --- UTF-8 decoding ----------------------------------------------------

    fn beginUtf8(self: *Parser, handler: anytype, byte: u8) void {
        const needed: usize = switch (byte) {
            0xC0...0xDF => 1,
            0xE0...0xEF => 2,
            0xF0...0xF4 => 3,
            else => {
                // 0x80..0xBF stray continuation or 0xF5+ — invalid lead.
                callPrint(handler, replacement);
                return;
            },
        };
        self.utf8_buf[0] = byte;
        self.utf8_len = 1;
        self.utf8_needed = needed;
    }

    fn continueUtf8(self: *Parser, handler: anytype, byte: u8) void {
        if (byte < 0x80 or byte > 0xBF) {
            // Not a continuation byte — the in-progress scalar is malformed.
            callPrint(handler, replacement);
            self.utf8_needed = 0;
            self.utf8_len = 0;
            // Reprocess this byte from a clean slate.
            self.feedByte(handler, byte);
            return;
        }
        self.utf8_buf[self.utf8_len] = byte;
        self.utf8_len += 1;
        self.utf8_needed -= 1;
        if (self.utf8_needed != 0) return;

        const decoded = std.unicode.utf8Decode(self.utf8_buf[0..self.utf8_len]) catch replacement;
        callPrint(handler, decoded);
        self.utf8_len = 0;
    }

    // --- collection helpers ------------------------------------------------

    fn clear(self: *Parser) void {
        self.param_count = 0;
        self.param_started = false;
        self.intermediate_count = 0;
        self.osc_len = 0;
        // Zero the first slot so `appendDigit` accumulates from a clean base
        // even before the slot is formally "activated".
        self.params[0] = 0;
    }

    fn collectIntermediate(self: *Parser, byte: u8) void {
        if (self.intermediate_count < max_intermediates) {
            self.intermediates[self.intermediate_count] = byte;
            self.intermediate_count += 1;
        }
    }

    fn intermediateSlice(self: *const Parser) []const u8 {
        return self.intermediates[0..self.intermediate_count];
    }

    fn appendDigit(self: *Parser, byte: u8) void {
        if (self.param_count == 0) self.param_count = 1;
        const idx = self.param_count - 1;
        if (idx >= max_params) return;
        const digit: u16 = byte - '0';
        const scaled = @mulWithOverflow(self.params[idx], 10);
        const value = @addWithOverflow(scaled[0], digit);
        // Saturate rather than overflow on absurdly long numbers.
        self.params[idx] = if (scaled[1] != 0 or value[1] != 0) std.math.maxInt(u16) else value[0];
    }

    fn nextParam(self: *Parser) void {
        if (self.param_count == 0) {
            // A leading ';' means an omitted first param plus a second slot.
            self.param_count = 2;
            if (max_params >= 2) {
                self.params[0] = 0;
                self.params[1] = 0;
            }
            return;
        }
        if (self.param_count < max_params) {
            self.params[self.param_count] = 0;
        }
        self.param_count += 1;
    }
};

// --- handler call shims ----------------------------------------------------
// Each shim tolerates a handler that omits the optional DCS methods.

fn callPrint(handler: anytype, cp: u21) void {
    handler.print(cp);
}

fn callExecute(handler: anytype, byte: u8) void {
    handler.execute(byte);
}

fn callCsiDispatch(handler: anytype, inter: []const u8, params: []const u16, final: u8) void {
    handler.csiDispatch(inter, params, final);
}

fn callEscDispatch(handler: anytype, inter: []const u8, final: u8) void {
    handler.escDispatch(inter, final);
}

fn callOscDispatch(handler: anytype, data: []const u8) void {
    handler.oscDispatch(data);
}

fn callDcsPut(handler: anytype, byte: u8) void {
    if (@hasDecl(@TypeOf(handler.*), "dcsPut")) handler.dcsPut(byte);
}

fn callDcsUnhook(handler: anytype) void {
    if (@hasDecl(@TypeOf(handler.*), "dcsUnhook")) handler.dcsUnhook();
}

// --- tests -----------------------------------------------------------------

/// A recording handler that captures every dispatched event for assertions.
const TestHandler = struct {
    const Event = union(enum) {
        print: u21,
        execute: u8,
        csi: struct { inter: [4]u8, inter_len: usize, params: [8]u16, param_len: usize, final: u8 },
        esc: struct { inter: [4]u8, inter_len: usize, final: u8 },
        osc: struct { buf: [256]u8, len: usize },
    };

    events: [256]Event = undefined,
    count: usize = 0,

    fn record(self: *TestHandler, event: Event) void {
        if (self.count < self.events.len) {
            self.events[self.count] = event;
            self.count += 1;
        }
    }

    pub fn print(self: *TestHandler, cp: u21) void {
        self.record(.{ .print = cp });
    }

    pub fn execute(self: *TestHandler, byte: u8) void {
        self.record(.{ .execute = byte });
    }

    pub fn csiDispatch(self: *TestHandler, inter: []const u8, params: []const u16, final: u8) void {
        var ev = Event{ .csi = .{ .inter = undefined, .inter_len = inter.len, .params = undefined, .param_len = params.len, .final = final } };
        for (inter, 0..) |b, i| ev.csi.inter[i] = b;
        for (params, 0..) |p, i| ev.csi.params[i] = p;
        self.record(ev);
    }

    pub fn escDispatch(self: *TestHandler, inter: []const u8, final: u8) void {
        var ev = Event{ .esc = .{ .inter = undefined, .inter_len = inter.len, .final = final } };
        for (inter, 0..) |b, i| ev.esc.inter[i] = b;
        self.record(ev);
    }

    pub fn oscDispatch(self: *TestHandler, data: []const u8) void {
        var ev = Event{ .osc = .{ .buf = undefined, .len = data.len } };
        for (data, 0..) |b, i| ev.osc.buf[i] = b;
        self.record(ev);
    }

    fn printsAsString(self: *const TestHandler, out: []u8) []const u8 {
        var n: usize = 0;
        for (self.events[0..self.count]) |ev| {
            if (ev == .print) {
                n += std.unicode.utf8Encode(ev.print, out[n..]) catch 0;
            }
        }
        return out[0..n];
    }
};

test "prints plain ASCII as scalars" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "Hi!");
    try std.testing.expectEqual(@as(usize, 3), h.count);
    try std.testing.expectEqual(@as(u21, 'H'), h.events[0].print);
    try std.testing.expectEqual(@as(u21, 'i'), h.events[1].print);
    try std.testing.expectEqual(@as(u21, '!'), h.events[2].print);
}

test "executes C0 control bytes" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "a\nb\r");
    try std.testing.expectEqual(@as(u21, 'a'), h.events[0].print);
    try std.testing.expectEqual(@as(u8, '\n'), h.events[1].execute);
    try std.testing.expectEqual(@as(u21, 'b'), h.events[2].print);
    try std.testing.expectEqual(@as(u8, '\r'), h.events[3].execute);
}

test "decodes multibyte UTF-8" {
    var h = TestHandler{};
    var p = Parser.init();
    // "é" (2 bytes), "€" (3 bytes), "𝄞" (4 bytes).
    p.feed(&h, "\xC3\xA9\xE2\x82\xAC\xF0\x9D\x84\x9E");
    try std.testing.expectEqual(@as(usize, 3), h.count);
    try std.testing.expectEqual(@as(u21, 0x00E9), h.events[0].print);
    try std.testing.expectEqual(@as(u21, 0x20AC), h.events[1].print);
    try std.testing.expectEqual(@as(u21, 0x1D11E), h.events[2].print);
}

test "UTF-8 split across feed calls is reassembled" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\xE2\x82"); // first two bytes of "€"
    try std.testing.expectEqual(@as(usize, 0), h.count);
    p.feed(&h, "\xAC"); // final byte
    try std.testing.expectEqual(@as(usize, 1), h.count);
    try std.testing.expectEqual(@as(u21, 0x20AC), h.events[0].print);
}

test "invalid UTF-8 yields U+FFFD" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\xFF"); // invalid lead byte
    try std.testing.expectEqual(@as(u21, replacement), h.events[0].print);

    var h2 = TestHandler{};
    var p2 = Parser.init();
    // Lead byte expecting a continuation, then an ASCII byte instead.
    p2.feed(&h2, "\xE2A");
    try std.testing.expectEqual(@as(u21, replacement), h2.events[0].print);
    try std.testing.expectEqual(@as(u21, 'A'), h2.events[1].print);
}

test "CSI cursor position carries two params" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B[12;40H");
    try std.testing.expectEqual(@as(usize, 1), h.count);
    const csi = h.events[0].csi;
    try std.testing.expectEqual(@as(u8, 'H'), csi.final);
    try std.testing.expectEqual(@as(usize, 2), csi.param_len);
    try std.testing.expectEqual(@as(u16, 12), csi.params[0]);
    try std.testing.expectEqual(@as(u16, 40), csi.params[1]);
    try std.testing.expectEqual(@as(usize, 0), csi.inter_len);
}

test "CSI with no params dispatches zero params" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B[H");
    try std.testing.expectEqual(@as(usize, 0), h.events[0].csi.param_len);
}

test "CSI private marker is surfaced as an intermediate" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B[?25h");
    const csi = h.events[0].csi;
    try std.testing.expectEqual(@as(u8, 'h'), csi.final);
    try std.testing.expectEqual(@as(usize, 1), csi.inter_len);
    try std.testing.expectEqual(@as(u8, '?'), csi.inter[0]);
    try std.testing.expectEqual(@as(u16, 25), csi.params[0]);
}

test "CSI SGR with many params" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B[38;2;255;128;0m");
    const csi = h.events[0].csi;
    try std.testing.expectEqual(@as(u8, 'm'), csi.final);
    try std.testing.expectEqual(@as(usize, 5), csi.param_len);
    try std.testing.expectEqual(@as(u16, 38), csi.params[0]);
    try std.testing.expectEqual(@as(u16, 2), csi.params[1]);
    try std.testing.expectEqual(@as(u16, 255), csi.params[2]);
    try std.testing.expectEqual(@as(u16, 128), csi.params[3]);
    try std.testing.expectEqual(@as(u16, 0), csi.params[4]);
}

test "leading semicolon yields an omitted first param" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B[;5H");
    const csi = h.events[0].csi;
    try std.testing.expectEqual(@as(usize, 2), csi.param_len);
    try std.testing.expectEqual(@as(u16, 0), csi.params[0]);
    try std.testing.expectEqual(@as(u16, 5), csi.params[1]);
}

test "ESC dispatch with intermediate (charset select)" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B(0");
    try std.testing.expectEqual(@as(usize, 1), h.count);
    const esc = h.events[0].esc;
    try std.testing.expectEqual(@as(u8, '0'), esc.final);
    try std.testing.expectEqual(@as(usize, 1), esc.inter_len);
    try std.testing.expectEqual(@as(u8, '('), esc.inter[0]);
}

test "ESC dispatch without intermediate (RIS)" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1Bc");
    try std.testing.expectEqual(@as(u8, 'c'), h.events[0].esc.final);
    try std.testing.expectEqual(@as(usize, 0), h.events[0].esc.inter_len);
}

test "OSC terminated by BEL" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B]0;my title\x07");
    try std.testing.expectEqual(@as(usize, 1), h.count);
    var buf: [256]u8 = undefined;
    const osc = h.events[0].osc;
    @memcpy(buf[0..osc.len], osc.buf[0..osc.len]);
    try std.testing.expectEqualStrings("0;my title", buf[0..osc.len]);
}

test "OSC terminated by ST (ESC backslash)" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B]2;hi\x1B\\");
    // The ST's ESC finalizes the OSC; its trailing `\` is a benign ESC.
    try std.testing.expectEqual(@as(usize, 2), h.count);
    const osc = h.events[0].osc;
    try std.testing.expectEqualStrings("2;hi", osc.buf[0..osc.len]);
    try std.testing.expectEqual(@as(u8, '\\'), h.events[1].esc.final);
}

test "printing resumes cleanly after a CSI sequence" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "ab\x1B[1mcd");
    var buf: [16]u8 = undefined;
    try std.testing.expectEqualStrings("abcd", h.printsAsString(&buf));
}

test "C0 control inside a CSI sequence executes then parsing continues" {
    var h = TestHandler{};
    var p = Parser.init();
    // A BS in the middle of the CSI params should execute immediately.
    p.feed(&h, "\x1B[1\x083m");
    try std.testing.expectEqual(@as(u8, 0x08), h.events[0].execute);
    const csi = h.events[1].csi;
    try std.testing.expectEqual(@as(u8, 'm'), csi.final);
    // The interrupted "1" then "3" still form param 13.
    try std.testing.expectEqual(@as(u16, 13), csi.params[0]);
}

test "feed handles escape split across calls" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B");
    p.feed(&h, "[");
    p.feed(&h, "5");
    p.feed(&h, "A");
    try std.testing.expectEqual(@as(usize, 1), h.count);
    try std.testing.expectEqual(@as(u8, 'A'), h.events[0].csi.final);
    try std.testing.expectEqual(@as(u16, 5), h.events[0].csi.params[0]);
}

test "consecutive CSI sequences do not leak param state" {
    var h = TestHandler{};
    var p = Parser.init();
    // Regression: a `CSI 0 m` after `CSI 1;31 m` must dispatch param 0, not
    // a digit accumulated onto the stale first slot.
    p.feed(&h, "\x1B[1;31m\x1B[0m");
    try std.testing.expectEqual(@as(usize, 2), h.count);
    try std.testing.expectEqual(@as(usize, 2), h.events[0].csi.param_len);
    try std.testing.expectEqual(@as(usize, 1), h.events[1].csi.param_len);
    try std.testing.expectEqual(@as(u16, 0), h.events[1].csi.params[0]);
}

test "CSI with colon sub-params is ignored" {
    var h = TestHandler{};
    var p = Parser.init();
    p.feed(&h, "\x1B[4:3mX");
    // The sub-param sequence is dropped; only the trailing X prints.
    try std.testing.expectEqual(@as(usize, 1), h.count);
    try std.testing.expectEqual(@as(u21, 'X'), h.events[0].print);
}
