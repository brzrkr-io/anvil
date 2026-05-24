//! VT/ANSI escape-sequence parser.
//!
//! Implements Paul Williams' VT500-series parser DFA. The parser is
//! byte-oriented and stateful: feed it arbitrary chunks and it drives a
//! caller-supplied [`Handler`]. UTF-8 is decoded in the ground state so
//! the handler always receives whole Unicode scalars via [`Handler::print`].

/// Replacement scalar emitted for malformed UTF-8.
const REPLACEMENT: char = '\u{FFFD}';

/// Upper bound on collected CSI/DCS numeric parameters.
const MAX_PARAMS: usize = 32;

/// Upper bound on collected intermediate / private-marker bytes.
const MAX_INTERMEDIATES: usize = 4;

/// Upper bound on buffered OSC payload. Longer strings are truncated.
const MAX_OSC: usize = 1024;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    OscString,
    SosPmApcString,
}

// ---------------------------------------------------------------------------
// Handler trait
// ---------------------------------------------------------------------------

/// Receiver for actions emitted by the parser.
///
/// `dcs_put` and `dcs_unhook` are optional; the default implementations are
/// no-ops, matching the Zig duck-typed shims.
pub trait Handler {
    fn print(&mut self, cp: char);
    fn execute(&mut self, byte: u8);
    fn csi_dispatch(&mut self, intermediates: &[u8], params: &[u16], final_byte: u8);
    fn esc_dispatch(&mut self, intermediates: &[u8], final_byte: u8);
    fn osc_dispatch(&mut self, data: &[u8]);

    /// Called for each byte of a DCS string payload (optional).
    fn dcs_put(&mut self, _byte: u8) {}
    /// Called when a DCS string is terminated (optional).
    fn dcs_unhook(&mut self) {}
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

pub struct Parser {
    state: State,

    params: [u16; MAX_PARAMS],
    param_count: usize,
    /// True once at least one digit or `;` of the current param run is seen,
    /// so a bare `CSI m` still dispatches a single (default) param.
    param_started: bool,

    intermediates: [u8; MAX_INTERMEDIATES],
    intermediate_count: usize,

    osc_buf: [u8; MAX_OSC],
    osc_len: usize,

    /// Partial UTF-8 sequence carried across `feed` calls.
    utf8_buf: [u8; 4],
    utf8_len: usize,
    utf8_needed: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Ground,
            params: [0; MAX_PARAMS],
            param_count: 0,
            param_started: false,
            intermediates: [0; MAX_INTERMEDIATES],
            intermediate_count: 0,
            osc_buf: [0; MAX_OSC],
            osc_len: 0,
            utf8_buf: [0; 4],
            utf8_len: 0,
            utf8_needed: 0,
        }
    }

    /// Parse `bytes`, driving `handler`. Safe to call repeatedly with
    /// arbitrary chunk boundaries — escape and UTF-8 state is retained.
    pub fn feed(&mut self, handler: &mut dyn Handler, bytes: &[u8]) {
        for &byte in bytes {
            self.feed_byte(handler, byte);
        }
    }

    fn feed_byte(&mut self, handler: &mut dyn Handler, byte: u8) {
        // A multi-byte UTF-8 scalar in progress short-circuits the DFA: its
        // continuation bytes are data, never controls.
        if self.utf8_needed != 0 {
            self.continue_utf8(handler, byte);
            return;
        }

        // C0 controls and a handful of C1 bytes can interrupt most states.
        if self.handle_anywhere(handler, byte) {
            return;
        }

        match self.state {
            State::Ground => self.ground_byte(handler, byte),
            State::Escape => self.escape_byte(handler, byte),
            State::EscapeIntermediate => self.escape_intermediate_byte(handler, byte),
            State::CsiEntry => self.csi_entry_byte(handler, byte),
            State::CsiParam => self.csi_param_byte(handler, byte),
            State::CsiIntermediate => self.csi_intermediate_byte(handler, byte),
            State::CsiIgnore => self.csi_ignore_byte(byte),
            State::DcsEntry => self.dcs_entry_byte(byte),
            State::DcsParam => self.dcs_param_byte(byte),
            State::DcsIntermediate => self.dcs_intermediate_byte(byte),
            State::DcsPassthrough => self.dcs_passthrough_byte(handler, byte),
            State::DcsIgnore => {}
            State::OscString => self.osc_string_byte(byte),
            State::SosPmApcString => {}
        }
    }

    /// Transitions valid from (almost) any state. Returns `true` when the byte
    /// was consumed here.
    fn handle_anywhere(&mut self, handler: &mut dyn Handler, byte: u8) -> bool {
        match byte {
            0x1B => {
                // ESC inside an OSC string is the first half of an ST (`ESC \`)
                // terminator: finalize the pending OSC now.
                if self.state == State::OscString {
                    self.dispatch_osc(handler);
                }
                if self.state == State::DcsPassthrough {
                    handler.dcs_unhook();
                }
                self.clear();
                self.state = State::Escape;
                true
            }
            0x18 | 0x1A => {
                // CAN aborts; SUB also executes.
                if byte == 0x1A {
                    handler.execute(byte);
                }
                self.state = State::Ground;
                true
            }
            // C0 controls execute immediately, except inside string states.
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => {
                match self.state {
                    State::OscString => {
                        if byte == 0x07 {
                            // BEL terminates OSC
                            self.dispatch_osc(handler);
                            self.state = State::Ground;
                        }
                        true
                    }
                    State::DcsPassthrough
                    | State::DcsEntry
                    | State::DcsParam
                    | State::DcsIntermediate
                    | State::DcsIgnore
                    | State::SosPmApcString => true,
                    _ => {
                        handler.execute(byte);
                        true
                    }
                }
            }
            _ => false,
        }
    }

    // --- ground -------------------------------------------------------------

    fn ground_byte(&mut self, handler: &mut dyn Handler, byte: u8) {
        if byte < 0x80 {
            handler.print(byte as char);
        } else {
            self.begin_utf8(handler, byte);
        }
    }

    // --- escape -------------------------------------------------------------

    fn escape_byte(&mut self, handler: &mut dyn Handler, byte: u8) {
        match byte {
            0x20..=0x2F => {
                self.collect_intermediate(byte);
                self.state = State::EscapeIntermediate;
            }
            0x30..=0x4F | 0x51..=0x57 | 0x59 | 0x5A | 0x5C | 0x60..=0x7E => {
                handler.esc_dispatch(self.intermediate_slice(), byte);
                self.state = State::Ground;
            }
            0x50 => {
                // ESC P -> DCS
                self.clear();
                self.state = State::DcsEntry;
            }
            0x58 | 0x5E | 0x5F => {
                // SOS, PM, APC
                self.state = State::SosPmApcString;
            }
            0x5B => self.state = State::CsiEntry, // ESC [ -> CSI
            0x5D => {
                // ESC ] -> OSC
                self.osc_len = 0;
                self.state = State::OscString;
            }
            _ => self.state = State::Ground,
        }
    }

    fn escape_intermediate_byte(&mut self, handler: &mut dyn Handler, byte: u8) {
        match byte {
            0x20..=0x2F => self.collect_intermediate(byte),
            0x30..=0x7E => {
                handler.esc_dispatch(self.intermediate_slice(), byte);
                self.state = State::Ground;
            }
            _ => self.state = State::Ground,
        }
    }

    // --- CSI ----------------------------------------------------------------

    fn csi_entry_byte(&mut self, handler: &mut dyn Handler, byte: u8) {
        match byte {
            0x30..=0x39 | 0x3B => {
                // digit or ';' — delegate to csi_param
                self.csi_param_byte(handler, byte);
            }
            0x3A => self.state = State::CsiIgnore, // ':' — sub-params unsupported
            0x3C..=0x3F => {
                // private markers < = > ?
                self.collect_intermediate(byte);
                self.state = State::CsiParam;
            }
            0x20..=0x2F => {
                self.collect_intermediate(byte);
                self.state = State::CsiIntermediate;
            }
            0x40..=0x7E => {
                self.dispatch_csi(handler, byte);
            }
            _ => self.state = State::CsiIgnore,
        }
    }

    fn csi_param_byte(&mut self, handler: &mut dyn Handler, byte: u8) {
        match byte {
            0x30..=0x39 => {
                // digit
                self.param_started = true;
                self.state = State::CsiParam;
                if self.param_count == 0 {
                    self.param_count = 1;
                }
                self.append_digit(byte);
            }
            0x3B => {
                // ';' — next param slot
                self.param_started = true;
                self.state = State::CsiParam;
                self.next_param();
            }
            0x3A => self.state = State::CsiIgnore,
            0x20..=0x2F => {
                self.collect_intermediate(byte);
                self.state = State::CsiIntermediate;
            }
            0x3C..=0x3F => self.state = State::CsiIgnore, // private marker mid-param
            0x40..=0x7E => self.dispatch_csi(handler, byte),
            _ => self.state = State::CsiIgnore,
        }
    }

    fn csi_intermediate_byte(&mut self, handler: &mut dyn Handler, byte: u8) {
        match byte {
            0x20..=0x2F => self.collect_intermediate(byte),
            0x30..=0x3F => self.state = State::CsiIgnore,
            0x40..=0x7E => self.dispatch_csi(handler, byte),
            _ => self.state = State::CsiIgnore,
        }
    }

    fn csi_ignore_byte(&mut self, byte: u8) {
        if (0x40..=0x7E).contains(&byte) {
            self.state = State::Ground;
        }
    }

    fn dispatch_csi(&mut self, handler: &mut dyn Handler, final_byte: u8) {
        let count = if self.param_started && self.param_count == 0 {
            1
        } else {
            self.param_count
        };
        let valid = count.min(MAX_PARAMS);
        handler.csi_dispatch(self.intermediate_slice(), &self.params[..valid], final_byte);
        self.state = State::Ground;
    }

    // --- DCS ----------------------------------------------------------------

    fn dcs_entry_byte(&mut self, byte: u8) {
        match byte {
            0x30..=0x39 | 0x3B => self.dcs_param_byte(byte),
            0x3C..=0x3F => {
                self.collect_intermediate(byte);
                self.state = State::DcsParam;
            }
            0x20..=0x2F => {
                self.collect_intermediate(byte);
                self.state = State::DcsIntermediate;
            }
            0x40..=0x7E => self.state = State::DcsPassthrough,
            0x3A => self.state = State::DcsIgnore,
            _ => self.state = State::DcsIgnore,
        }
    }

    fn dcs_param_byte(&mut self, byte: u8) {
        match byte {
            0x30..=0x39 => {
                self.state = State::DcsParam;
                if self.param_count == 0 {
                    self.param_count = 1;
                }
                self.append_digit(byte);
            }
            0x3B => {
                self.state = State::DcsParam;
                self.next_param();
            }
            0x20..=0x2F => {
                self.collect_intermediate(byte);
                self.state = State::DcsIntermediate;
            }
            0x40..=0x7E => self.state = State::DcsPassthrough,
            _ => self.state = State::DcsIgnore,
        }
    }

    fn dcs_intermediate_byte(&mut self, byte: u8) {
        match byte {
            0x20..=0x2F => self.collect_intermediate(byte),
            0x40..=0x7E => self.state = State::DcsPassthrough,
            _ => self.state = State::DcsIgnore,
        }
    }

    fn dcs_passthrough_byte(&mut self, handler: &mut dyn Handler, byte: u8) {
        if byte == 0x9C {
            // ST (8-bit)
            handler.dcs_unhook();
            self.state = State::Ground;
            return;
        }
        handler.dcs_put(byte);
    }

    // --- OSC ----------------------------------------------------------------

    fn osc_string_byte(&mut self, byte: u8) {
        if self.osc_len < MAX_OSC {
            self.osc_buf[self.osc_len] = byte;
            self.osc_len += 1;
        }
    }

    fn dispatch_osc(&mut self, handler: &mut dyn Handler) {
        handler.osc_dispatch(&self.osc_buf[..self.osc_len]);
        self.osc_len = 0;
    }

    // --- UTF-8 decoding -----------------------------------------------------

    fn begin_utf8(&mut self, handler: &mut dyn Handler, byte: u8) {
        let needed: usize = match byte {
            0xC0..=0xDF => 1,
            0xE0..=0xEF => 2,
            0xF0..=0xF4 => 3,
            _ => {
                // Stray continuation (0x80..=0xBF) or invalid lead (0xF5+).
                handler.print(REPLACEMENT);
                return;
            }
        };
        self.utf8_buf[0] = byte;
        self.utf8_len = 1;
        self.utf8_needed = needed;
    }

    fn continue_utf8(&mut self, handler: &mut dyn Handler, byte: u8) {
        if !(0x80..=0xBF).contains(&byte) {
            // Not a continuation byte — the in-progress scalar is malformed.
            handler.print(REPLACEMENT);
            self.utf8_needed = 0;
            self.utf8_len = 0;
            // Reprocess this byte from a clean slate.
            self.feed_byte(handler, byte);
            return;
        }
        self.utf8_buf[self.utf8_len] = byte;
        self.utf8_len += 1;
        self.utf8_needed -= 1;
        if self.utf8_needed != 0 {
            return;
        }

        let buf = &self.utf8_buf[..self.utf8_len];
        let cp = std::str::from_utf8(buf)
            .ok()
            .and_then(|s| s.chars().next())
            .unwrap_or(REPLACEMENT);
        handler.print(cp);
        self.utf8_len = 0;
    }

    // --- collection helpers -------------------------------------------------

    fn clear(&mut self) {
        self.param_count = 0;
        self.param_started = false;
        self.intermediate_count = 0;
        self.osc_len = 0;
        self.params[0] = 0;
    }

    fn collect_intermediate(&mut self, byte: u8) {
        if self.intermediate_count < MAX_INTERMEDIATES {
            self.intermediates[self.intermediate_count] = byte;
            self.intermediate_count += 1;
        }
    }

    fn intermediate_slice(&self) -> &[u8] {
        &self.intermediates[..self.intermediate_count]
    }

    fn append_digit(&mut self, byte: u8) {
        if self.param_count == 0 {
            self.param_count = 1;
        }
        let idx = self.param_count - 1;
        if idx >= MAX_PARAMS {
            return;
        }
        let digit = (byte - b'0') as u16;
        let (scaled, ov1) = self.params[idx].overflowing_mul(10);
        let (value, ov2) = scaled.overflowing_add(digit);
        self.params[idx] = if ov1 || ov2 { u16::MAX } else { value };
    }

    fn next_param(&mut self) {
        if self.param_count == 0 {
            // A leading ';' means an omitted first param plus a second slot.
            self.param_count = 2;
            if MAX_PARAMS >= 2 {
                self.params[0] = 0;
                self.params[1] = 0;
            }
            return;
        }
        if self.param_count < MAX_PARAMS {
            self.params[self.param_count] = 0;
        }
        self.param_count += 1;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Test handler -------------------------------------------------------

    #[derive(Debug)]
    enum Event {
        Print(char),
        Execute(u8),
        Csi {
            inter: [u8; 4],
            inter_len: usize,
            params: [u16; 8],
            param_len: usize,
            final_byte: u8,
        },
        Esc {
            inter: [u8; 4],
            inter_len: usize,
            final_byte: u8,
        },
        Osc {
            buf: [u8; 256],
            len: usize,
        },
    }

    struct TestHandler {
        events: Vec<Event>,
    }

    impl TestHandler {
        fn new() -> Self {
            Self { events: Vec::new() }
        }

        fn prints_as_string(&self) -> String {
            self.events
                .iter()
                .filter_map(|e| {
                    if let Event::Print(c) = e {
                        Some(*c)
                    } else {
                        None
                    }
                })
                .collect()
        }
    }

    impl Handler for TestHandler {
        fn print(&mut self, cp: char) {
            self.events.push(Event::Print(cp));
        }

        fn execute(&mut self, byte: u8) {
            self.events.push(Event::Execute(byte));
        }

        fn csi_dispatch(&mut self, intermediates: &[u8], params: &[u16], final_byte: u8) {
            let mut inter = [0u8; 4];
            let inter_len = intermediates.len().min(4);
            inter[..inter_len].copy_from_slice(&intermediates[..inter_len]);

            let mut ps = [0u16; 8];
            let param_len = params.len().min(8);
            ps[..param_len].copy_from_slice(&params[..param_len]);

            self.events.push(Event::Csi {
                inter,
                inter_len,
                params: ps,
                param_len,
                final_byte,
            });
        }

        fn esc_dispatch(&mut self, intermediates: &[u8], final_byte: u8) {
            let mut inter = [0u8; 4];
            let inter_len = intermediates.len().min(4);
            inter[..inter_len].copy_from_slice(&intermediates[..inter_len]);
            self.events.push(Event::Esc {
                inter,
                inter_len,
                final_byte,
            });
        }

        fn osc_dispatch(&mut self, data: &[u8]) {
            let mut buf = [0u8; 256];
            let len = data.len().min(256);
            buf[..len].copy_from_slice(&data[..len]);
            self.events.push(Event::Osc { buf, len });
        }
    }

    // --- DCS test handler ---------------------------------------------------

    struct DcsHandler {
        put: Vec<u8>,
        unhooked: bool,
    }

    impl DcsHandler {
        fn new() -> Self {
            Self {
                put: Vec::new(),
                unhooked: false,
            }
        }
    }

    impl Handler for DcsHandler {
        fn print(&mut self, _: char) {}
        fn execute(&mut self, _: u8) {}
        fn csi_dispatch(&mut self, _: &[u8], _: &[u16], _: u8) {}
        fn esc_dispatch(&mut self, _: &[u8], _: u8) {}
        fn osc_dispatch(&mut self, _: &[u8]) {}

        fn dcs_put(&mut self, byte: u8) {
            self.put.push(byte);
        }

        fn dcs_unhook(&mut self) {
            self.unhooked = true;
        }
    }

    // --- helpers ------------------------------------------------------------

    fn feed(p: &mut Parser, h: &mut dyn Handler, s: &[u8]) {
        p.feed(h, s);
    }

    // ========================================================================
    // Ported tests (39 total, matching the Zig test suite)
    // ========================================================================

    #[test]
    fn prints_plain_ascii_as_scalars() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"Hi!");
        assert_eq!(h.events.len(), 3);
        assert!(matches!(h.events[0], Event::Print('H')));
        assert!(matches!(h.events[1], Event::Print('i')));
        assert!(matches!(h.events[2], Event::Print('!')));
    }

    #[test]
    fn executes_c0_control_bytes() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"a\nb\r");
        assert!(matches!(h.events[0], Event::Print('a')));
        assert!(matches!(h.events[1], Event::Execute(b'\n')));
        assert!(matches!(h.events[2], Event::Print('b')));
        assert!(matches!(h.events[3], Event::Execute(b'\r')));
    }

    #[test]
    fn decodes_multibyte_utf8() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        // "é" (2 bytes), "€" (3 bytes), "𝄞" (4 bytes)
        feed(&mut p, &mut h, b"\xC3\xA9\xE2\x82\xAC\xF0\x9D\x84\x9E");
        assert_eq!(h.events.len(), 3);
        assert!(matches!(h.events[0], Event::Print('\u{00E9}')));
        assert!(matches!(h.events[1], Event::Print('\u{20AC}')));
        assert!(matches!(h.events[2], Event::Print('\u{1D11E}')));
    }

    #[test]
    fn utf8_split_across_feed_calls_is_reassembled() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\xE2\x82"); // first two bytes of "€"
        assert_eq!(h.events.len(), 0);
        feed(&mut p, &mut h, b"\xAC"); // final byte
        assert_eq!(h.events.len(), 1);
        assert!(matches!(h.events[0], Event::Print('\u{20AC}')));
    }

    #[test]
    fn invalid_utf8_yields_replacement() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\xFF"); // invalid lead byte
        assert!(matches!(h.events[0], Event::Print(REPLACEMENT)));

        let mut h2 = TestHandler::new();
        let mut p2 = Parser::new();
        // Lead byte expecting a continuation, then ASCII instead.
        feed(&mut p2, &mut h2, b"\xE2A");
        assert!(matches!(h2.events[0], Event::Print(REPLACEMENT)));
        assert!(matches!(h2.events[1], Event::Print('A')));
    }

    #[test]
    fn csi_cursor_position_carries_two_params() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[12;40H");
        assert_eq!(h.events.len(), 1);
        match &h.events[0] {
            Event::Csi {
                final_byte,
                param_len,
                params,
                inter_len,
                ..
            } => {
                assert_eq!(*final_byte, b'H');
                assert_eq!(*param_len, 2);
                assert_eq!(params[0], 12);
                assert_eq!(params[1], 40);
                assert_eq!(*inter_len, 0);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn csi_with_no_params_dispatches_zero_params() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[H");
        match &h.events[0] {
            Event::Csi { param_len, .. } => assert_eq!(*param_len, 0),
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn csi_private_marker_is_surfaced_as_intermediate() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[?25h");
        match &h.events[0] {
            Event::Csi {
                final_byte,
                inter_len,
                inter,
                params,
                param_len,
                ..
            } => {
                assert_eq!(*final_byte, b'h');
                assert_eq!(*inter_len, 1);
                assert_eq!(inter[0], b'?');
                assert_eq!(*param_len, 1);
                assert_eq!(params[0], 25);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn csi_sgr_with_many_params() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[38;2;255;128;0m");
        match &h.events[0] {
            Event::Csi {
                final_byte,
                param_len,
                params,
                ..
            } => {
                assert_eq!(*final_byte, b'm');
                assert_eq!(*param_len, 5);
                assert_eq!(params[0], 38);
                assert_eq!(params[1], 2);
                assert_eq!(params[2], 255);
                assert_eq!(params[3], 128);
                assert_eq!(params[4], 0);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn leading_semicolon_yields_omitted_first_param() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[;5H");
        match &h.events[0] {
            Event::Csi {
                param_len, params, ..
            } => {
                assert_eq!(*param_len, 2);
                assert_eq!(params[0], 0);
                assert_eq!(params[1], 5);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn esc_dispatch_with_intermediate_charset_select() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B(0");
        assert_eq!(h.events.len(), 1);
        match &h.events[0] {
            Event::Esc {
                final_byte,
                inter_len,
                inter,
            } => {
                assert_eq!(*final_byte, b'0');
                assert_eq!(*inter_len, 1);
                assert_eq!(inter[0], b'(');
            }
            _ => panic!("expected ESC"),
        }
    }

    #[test]
    fn esc_dispatch_without_intermediate_ris() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1Bc");
        match &h.events[0] {
            Event::Esc {
                final_byte,
                inter_len,
                ..
            } => {
                assert_eq!(*final_byte, b'c');
                assert_eq!(*inter_len, 0);
            }
            _ => panic!("expected ESC"),
        }
    }

    #[test]
    fn osc_terminated_by_bel() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B]0;my title\x07");
        assert_eq!(h.events.len(), 1);
        match &h.events[0] {
            Event::Osc { buf, len } => {
                assert_eq!(&buf[..*len], b"0;my title");
            }
            _ => panic!("expected OSC"),
        }
    }

    #[test]
    fn osc_terminated_by_st_esc_backslash() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B]2;hi\x1B\\");
        // The ST's ESC finalizes the OSC; its trailing `\` is a benign ESC dispatch.
        assert_eq!(h.events.len(), 2);
        match &h.events[0] {
            Event::Osc { buf, len } => {
                assert_eq!(&buf[..*len], b"2;hi");
            }
            _ => panic!("expected OSC"),
        }
        match &h.events[1] {
            Event::Esc { final_byte, .. } => assert_eq!(*final_byte, b'\\'),
            _ => panic!("expected ESC"),
        }
    }

    #[test]
    fn printing_resumes_cleanly_after_csi_sequence() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"ab\x1B[1mcd");
        assert_eq!(h.prints_as_string(), "abcd");
    }

    #[test]
    fn c0_control_inside_csi_sequence_executes_then_parsing_continues() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        // A BS in the middle of the CSI params should execute immediately.
        feed(&mut p, &mut h, b"\x1B[1\x083m");
        assert!(matches!(h.events[0], Event::Execute(0x08)));
        match &h.events[1] {
            Event::Csi {
                final_byte, params, ..
            } => {
                assert_eq!(*final_byte, b'm');
                // The interrupted "1" then "3" still form param 13.
                assert_eq!(params[0], 13);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn feed_handles_escape_split_across_calls() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B");
        feed(&mut p, &mut h, b"[");
        feed(&mut p, &mut h, b"5");
        feed(&mut p, &mut h, b"A");
        assert_eq!(h.events.len(), 1);
        match &h.events[0] {
            Event::Csi {
                final_byte, params, ..
            } => {
                assert_eq!(*final_byte, b'A');
                assert_eq!(params[0], 5);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn consecutive_csi_sequences_do_not_leak_param_state() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[1;31m\x1B[0m");
        assert_eq!(h.events.len(), 2);
        match &h.events[0] {
            Event::Csi { param_len, .. } => assert_eq!(*param_len, 2),
            _ => panic!("expected CSI"),
        }
        match &h.events[1] {
            Event::Csi {
                param_len, params, ..
            } => {
                assert_eq!(*param_len, 1);
                assert_eq!(params[0], 0);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn csi_with_colon_sub_params_is_ignored() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[4:3mX");
        // The sub-param sequence is dropped; only the trailing X prints.
        assert_eq!(h.events.len(), 1);
        assert!(matches!(h.events[0], Event::Print('X')));
    }

    #[test]
    fn can_aborts_escape_sequence_to_ground() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        // CAN (0x18) mid-CSI drops the sequence; the following text still prints.
        feed(&mut p, &mut h, b"\x1B[1;2\x18X");
        assert_eq!(h.events.len(), 1);
        assert!(matches!(h.events[0], Event::Print('X')));
    }

    #[test]
    fn sub_aborts_to_ground_and_executes() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        // SUB (0x1A) executes, then aborts the in-progress sequence to ground.
        feed(&mut p, &mut h, b"\x1B[3\x1AY");
        assert!(matches!(h.events[0], Event::Execute(0x1A)));
        assert!(matches!(h.events[1], Event::Print('Y')));
    }

    #[test]
    fn esc_with_unhandled_final_byte_returns_to_ground() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B\x7FZ"); // DEL is not a valid ESC final
        assert_eq!(h.events.len(), 1);
        assert!(matches!(h.events[0], Event::Print('Z')));
    }

    #[test]
    fn esc_dispatch_collects_multiple_intermediates() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B #8"); // ESC, intermediates SP and '#', final '8'
        match &h.events[0] {
            Event::Esc {
                final_byte,
                inter_len,
                inter,
            } => {
                assert_eq!(*final_byte, b'8');
                assert_eq!(*inter_len, 2);
                assert_eq!(inter[0], b' ');
                assert_eq!(inter[1], b'#');
            }
            _ => panic!("expected ESC"),
        }
    }

    #[test]
    fn esc_intermediate_then_unhandled_byte_returns_to_ground() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B \x7FW"); // ESC, intermediate SP, DEL (unhandled)
        assert_eq!(h.events.len(), 1);
        assert!(matches!(h.events[0], Event::Print('W')));
    }

    #[test]
    fn csi_cursor_style_sequence_carries_param_and_intermediate() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[2 q"); // DECSCUSR: param 2, intermediate SP, final 'q'
        match &h.events[0] {
            Event::Csi {
                final_byte,
                params,
                inter_len,
                inter,
                ..
            } => {
                assert_eq!(*final_byte, b'q');
                assert_eq!(params[0], 2);
                assert_eq!(*inter_len, 1);
                assert_eq!(inter[0], b' ');
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn csi_intermediate_immediately_after_csi_dispatches() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[ q"); // intermediate with no preceding parameter
        match &h.events[0] {
            Event::Csi {
                final_byte,
                inter_len,
                ..
            } => {
                assert_eq!(*final_byte, b'q');
                assert_eq!(*inter_len, 1);
            }
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn csi_collects_multiple_intermediates() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[ !q"); // intermediates SP and '!'
        match &h.events[0] {
            Event::Csi { inter_len, .. } => assert_eq!(*inter_len, 2),
            _ => panic!("expected CSI"),
        }
    }

    #[test]
    fn csi_intermediate_followed_by_digit_abandons_sequence() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[ 1qX"); // a digit after an intermediate is invalid
        assert_eq!(h.events.len(), 1);
        assert!(matches!(h.events[0], Event::Print('X')));
    }

    #[test]
    fn csi_intermediate_followed_by_unhandled_byte_abandons_sequence() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[ \x7FqX"); // DEL after an intermediate
        assert_eq!(h.events.len(), 1);
        assert!(matches!(h.events[0], Event::Print('X')));
    }

    #[test]
    fn csi_colon_immediately_after_csi_is_ignored() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[:5mX"); // ':' at csi_entry abandons the sequence
        assert_eq!(h.events.len(), 1);
        assert!(matches!(h.events[0], Event::Print('X')));
    }

    #[test]
    fn csi_private_marker_after_param_is_ignored() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[1?hX"); // '?' mid-param abandons the sequence
        assert_eq!(h.events.len(), 1);
        assert!(matches!(h.events[0], Event::Print('X')));
    }

    #[test]
    fn csi_unhandled_byte_at_entry_and_mid_param_is_ignored() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1B[\x7FmA"); // DEL right after CSI
        feed(&mut p, &mut h, b"\x1B[2\x7FmB"); // DEL after a parameter digit
        assert_eq!(h.events.len(), 2);
        assert!(matches!(h.events[0], Event::Print('A')));
        assert!(matches!(h.events[1], Event::Print('B')));
    }

    #[test]
    fn dcs_delivers_payload_and_unhooks_on_st() {
        let mut h = DcsHandler::new();
        let mut p = Parser::new();
        // ESC P, params 1;2, intermediate '!', final 'q', payload, then ST.
        feed(&mut p, &mut h, b"\x1BP1;2 !qDATA\x1B\\");
        assert_eq!(h.put, b"DATA");
        assert!(h.unhooked);
    }

    #[test]
    fn dcs_entered_via_private_marker_and_ended_by_8bit_st() {
        let mut h = DcsHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1BP<qPAYLOAD\x9C");
        assert_eq!(h.put, b"PAYLOAD");
        assert!(h.unhooked);
    }

    #[test]
    fn dcs_with_leading_intermediate_reaches_passthrough() {
        let mut h = DcsHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1BP qX\x9C");
        assert_eq!(h.put, b"X");
    }

    #[test]
    fn c0_control_inside_dcs_is_swallowed_not_delivered_as_payload() {
        let mut h = DcsHandler::new();
        let mut p = Parser::new();
        // BS (0x08) arrives in dcs_passthrough; DCS string states absorb C0.
        feed(&mut p, &mut h, b"\x1BPqAB\x08CD\x9C");
        assert_eq!(h.put, b"ABCD");
    }

    #[test]
    fn dcs_aborted_by_colon_ignores_rest_of_sequence() {
        let mut h = DcsHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"\x1BP:junk\x1B\\");
        assert_eq!(h.put.len(), 0);
        assert!(!h.unhooked);
    }

    #[test]
    fn dcs_rejects_unhandled_bytes_in_entry_param_and_intermediate_states() {
        let mut p = Parser::new();
        let mut h1 = DcsHandler::new();
        feed(&mut p, &mut h1, b"\x1BP\x7Fjunk\x1B\\"); // DEL at dcs_entry
        assert_eq!(h1.put.len(), 0);

        let mut p = Parser::new();
        let mut h2 = DcsHandler::new();
        feed(&mut p, &mut h2, b"\x1BP1\x7Fjunk\x1B\\"); // DEL after a param digit
        assert_eq!(h2.put.len(), 0);

        let mut p = Parser::new();
        let mut h3 = DcsHandler::new();
        feed(&mut p, &mut h3, b"\x1BP 1junk\x1B\\"); // digit after a leading intermediate
        assert_eq!(h3.put.len(), 0);
    }

    #[test]
    fn pm_string_is_consumed_and_discarded() {
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        // ESC ^ ... ST (PM). Content is swallowed; the trailing text prints.
        feed(&mut p, &mut h, b"\x1B^private\x07message\x1B\\X");
        assert!(matches!(h.events[h.events.len() - 1], Event::Print('X')));
    }

    // ── dcs_put / dcs_unhook default impls via TestHandler (lines 58, 60) ────

    #[test]
    fn dcs_with_test_handler_calls_dcs_put_and_dcs_unhook_defaults() {
        // TestHandler does not override dcs_put or dcs_unhook — the default
        // no-op impls (lines 58, 60) are exercised here.
        let mut h = TestHandler::new();
        let mut p = Parser::new();
        // ESC P <final> PAYLOAD ST — DCS with no params
        feed(&mut p, &mut h, b"\x1BPqHI\x9C");
        // No event is recorded (TestHandler's DCS stubs do nothing),
        // but parsing must complete without panic.
        // Feed a printable after to verify parser state recovered.
        feed(&mut p, &mut h, b"Z");
        assert!(h.events.iter().any(|e| matches!(e, Event::Print('Z'))));
    }

    // ── DcsHandler stubs called when text precedes DCS (lines 620-624) ────────

    #[test]
    fn dcs_handler_stubs_invoked_by_text_and_execute_before_dcs() {
        // Feed printable text and a control char to DcsHandler before the DCS.
        // This causes the parser to invoke print(), execute(), etc.
        // on DcsHandler, exercising the no-op stubs.
        let mut h = DcsHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, b"A\x08\x1B[1mX");
        // After those, send a DCS so the actual payload is captured.
        feed(&mut p, &mut h, b"\x1BPqDATA\x9C");
        assert_eq!(h.put, b"DATA");
    }

    // ── append_digit: idx >= MAX_PARAMS early return (line 477) ──────────────

    #[test]
    fn csi_with_more_than_max_params_is_truncated_without_panic() {
        // Build a CSI sequence with 40 semicolons (41 params) — well above MAX_PARAMS=32.
        let mut seq: Vec<u8> = b"\x1B[".to_vec();
        for _ in 0..40 {
            seq.extend_from_slice(b"1;");
        }
        seq.push(b'm');

        let mut h = TestHandler::new();
        let mut p = Parser::new();
        feed(&mut p, &mut h, &seq);
        // The sequence dispatches successfully (no panic), truncated to MAX_PARAMS.
        assert_eq!(1, h.events.len());
        assert!(matches!(h.events[0], Event::Csi { .. }));
    }
}
