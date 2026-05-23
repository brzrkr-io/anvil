//! Translates key presses into the byte sequences a terminal expects.
//! Pure logic — no platform code — so it is fully unit-testable.

/// Keyboard modifier state.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub shift: bool,
    pub control: bool,
    pub option: bool, // Alt / Meta
    pub command: bool,
}

/// A key event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// An already-resolved typed Unicode codepoint.
    Text(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Delete, // forward delete
    Insert,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

/// Modifier parameter for XTerm modifier sequences: `1 + shift + alt*2 + ctrl*4`.
/// Returns 0 when no modifier is active (caller uses unmodified form instead).
fn mod_param(mods: Mods) -> u8 {
    let mut m: u8 = 1;
    if mods.shift {
        m += 1;
    }
    if mods.option {
        m += 2;
    }
    if mods.control {
        m += 4;
    }
    m
}

fn any_mod(mods: Mods) -> bool {
    mods.shift || mods.option || mods.control
}

/// Encode a key press into terminal input bytes written into `out` (16 bytes is
/// always sufficient).  Returns the used sub-slice.  `app_cursor` selects DECCKM
/// application cursor-key mode (ESC O x instead of ESC \[ x).
pub fn encode(key: Key, mods: Mods, app_cursor: bool, out: &mut [u8]) -> &[u8] {
    match key {
        Key::Text(ch) => encode_text(ch, mods, out),
        Key::Enter => set(out, b"\r"),
        Key::Tab => {
            if mods.shift {
                set(out, b"\x1b[Z")
            } else {
                set(out, b"\t")
            }
        }
        Key::Backspace => set(out, b"\x7f"),
        Key::Escape => set(out, b"\x1b"),
        Key::Up => cursor_key(out, mods, app_cursor, b'A'),
        Key::Down => cursor_key(out, mods, app_cursor, b'B'),
        Key::Right => cursor_key(out, mods, app_cursor, b'C'),
        Key::Left => cursor_key(out, mods, app_cursor, b'D'),
        Key::Home => cursor_key(out, mods, app_cursor, b'H'),
        Key::End => cursor_key(out, mods, app_cursor, b'F'),
        Key::PageUp => edit_key(out, mods, 5),
        Key::PageDown => edit_key(out, mods, 6),
        Key::Delete => edit_key(out, mods, 3),
        Key::Insert => edit_key(out, mods, 2),
        Key::F1 => fn_key(out, mods, 1),
        Key::F2 => fn_key(out, mods, 2),
        Key::F3 => fn_key(out, mods, 3),
        Key::F4 => fn_key(out, mods, 4),
        Key::F5 => fn_key(out, mods, 5),
        Key::F6 => fn_key(out, mods, 6),
        Key::F7 => fn_key(out, mods, 7),
        Key::F8 => fn_key(out, mods, 8),
        Key::F9 => fn_key(out, mods, 9),
        Key::F10 => fn_key(out, mods, 10),
        Key::F11 => fn_key(out, mods, 11),
        Key::F12 => fn_key(out, mods, 12),
    }
}

fn set<'a>(out: &'a mut [u8], bytes: &[u8]) -> &'a [u8] {
    out[..bytes.len()].copy_from_slice(bytes);
    &out[..bytes.len()]
}

fn cursor_key(out: &mut [u8], mods: Mods, app_cursor: bool, final_byte: u8) -> &[u8] {
    if !any_mod(mods) {
        out[0] = 0x1b;
        out[1] = if app_cursor { b'O' } else { b'[' };
        out[2] = final_byte;
        return &out[..3];
    }
    let m = mod_param(mods);
    let n = write_fmt(out, format_args!("\x1b[1;{m}{}", final_byte as char));
    &out[..n]
}

fn edit_key(out: &mut [u8], mods: Mods, n_code: u8) -> &[u8] {
    if !any_mod(mods) {
        let n = write_fmt(out, format_args!("\x1b[{n_code}~"));
        return &out[..n];
    }
    let m = mod_param(mods);
    let n = write_fmt(out, format_args!("\x1b[{n_code};{m}~"));
    &out[..n]
}

fn fn_key(out: &mut [u8], mods: Mods, n: u8) -> &[u8] {
    if n <= 4 {
        let letters = [b'P', b'Q', b'R', b'S'];
        let letter = letters[(n - 1) as usize];
        if !any_mod(mods) {
            out[0] = 0x1b;
            out[1] = b'O';
            out[2] = letter;
            return &out[..3];
        }
        let m = mod_param(mods);
        let len = write_fmt(out, format_args!("\x1b[1;{m}{}", letter as char));
        return &out[..len];
    }
    let vt_codes: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24]; // F5–F12
    let vt = vt_codes[(n - 5) as usize];
    if !any_mod(mods) {
        let len = write_fmt(out, format_args!("\x1b[{vt}~"));
        return &out[..len];
    }
    let m = mod_param(mods);
    let len = write_fmt(out, format_args!("\x1b[{vt};{m}~"));
    &out[..len]
}

/// Mouse event encoding.  Returns the bytes to write to the PTY.
///
/// - `button`: 0 left, 1 middle, 2 right; add 32 for drag; 64/65 for scroll.
/// - `col`, `row`: 1-based terminal cell coordinates.
/// - `press`: `true` for button-down / motion with button held; `false` for
///   release.
/// - `sgr`: `true` = SGR encoding (`ESC[<...`); `false` = legacy X10
///   (`ESC[M...`).
pub fn encode_mouse(
    button: u8,
    col: usize,
    row: usize,
    press: bool,
    sgr: bool,
    out: &mut [u8],
) -> &[u8] {
    if sgr {
        let suffix = if press { b'M' } else { b'm' };
        let n = write_fmt(
            out,
            format_args!("\x1b[<{button};{col};{row}{}", suffix as char),
        );
        &out[..n]
    } else {
        // Legacy X10: only encodes press (release uses button 3).
        let b_byte = (32u8).wrapping_add(button);
        let c_byte = (32u8).wrapping_add(col.min(223) as u8);
        let r_byte = (32u8).wrapping_add(row.min(223) as u8);
        if out.len() < 6 {
            return &out[..0];
        }
        out[0] = 0x1b;
        out[1] = b'[';
        out[2] = b'M';
        out[3] = b_byte;
        out[4] = c_byte;
        out[5] = r_byte;
        &out[..6]
    }
}

fn encode_text(ch: char, mods: Mods, out: &mut [u8]) -> &[u8] {
    if mods.control {
        if let Some(b) = control_byte(ch) {
            if mods.option {
                out[0] = 0x1b;
                out[1] = b;
                return &out[..2];
            }
            out[0] = b;
            return &out[..1];
        }
    }
    let mut n = 0;
    if mods.option {
        out[0] = 0x1b;
        n = 1;
    }
    let mut buf = [0u8; 4];
    let s = ch.encode_utf8(&mut buf);
    let len = s.len();
    out[n..n + len].copy_from_slice(s.as_bytes());
    &out[..n + len]
}

fn control_byte(ch: char) -> Option<u8> {
    match ch {
        ' ' => Some(0),
        'a'..='z' => Some(ch as u8 - b'a' + 1),
        'A'..='Z' => Some(ch as u8 - b'A' + 1),
        '@' => Some(0),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

/// Write a `format_args!` string into `out`, return the number of bytes
/// written.  Uses a stack-based `Write` impl to avoid heap allocation.
fn write_fmt(out: &mut [u8], args: std::fmt::Arguments<'_>) -> usize {
    use std::fmt::Write;
    struct Cursor<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl Write for Cursor<'_> {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            let src = s.as_bytes();
            let avail = self.buf.len().saturating_sub(self.pos);
            let n = src.len().min(avail);
            self.buf[self.pos..self.pos + n].copy_from_slice(&src[..n]);
            self.pos += n;
            Ok(())
        }
    }
    let mut c = Cursor { buf: out, pos: 0 };
    let _ = c.write_fmt(args);
    c.pos
}

// ---------------------------------------------------------------------------
// Tests  (20 Zig tests → 20 Rust tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(key: Key, mods: Mods, app_cursor: bool) -> Vec<u8> {
        let mut buf = [0u8; 16];
        encode(key, mods, app_cursor, &mut buf).to_vec()
    }

    #[test]
    fn plain_ascii_encodes_to_one_byte() {
        assert_eq!(enc(Key::Text('a'), Mods::default(), false), b"a");
    }

    #[test]
    fn unicode_codepoint_encodes_as_utf8() {
        // U+00E9 = é
        let result = enc(Key::Text('\u{00e9}'), Mods::default(), false);
        assert_eq!(result, "\u{00e9}".as_bytes());
    }

    #[test]
    fn control_letters_map_to_c0_codes() {
        assert_eq!(
            enc(
                Key::Text('c'),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x03]
        );
        assert_eq!(
            enc(
                Key::Text('C'),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x03]
        );
        assert_eq!(
            enc(
                Key::Text('a'),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x01]
        );
        assert_eq!(
            enc(
                Key::Text(' '),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x00]
        );
    }

    #[test]
    fn named_keys() {
        assert_eq!(enc(Key::Enter, Mods::default(), false), b"\r");
        assert_eq!(enc(Key::Backspace, Mods::default(), false), b"\x7f");
        assert_eq!(enc(Key::Escape, Mods::default(), false), b"\x1b");
        assert_eq!(enc(Key::Tab, Mods::default(), false), b"\t");
        assert_eq!(
            enc(
                Key::Tab,
                Mods {
                    shift: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1b[Z"
        );
    }

    #[test]
    fn cursor_keys_honor_decckm() {
        assert_eq!(enc(Key::Up, Mods::default(), false), b"\x1b[A");
        assert_eq!(enc(Key::Up, Mods::default(), true), b"\x1bOA");
        assert_eq!(enc(Key::Left, Mods::default(), false), b"\x1b[D");
    }

    #[test]
    fn option_prefixes_esc_meta() {
        assert_eq!(
            enc(
                Key::Text('x'),
                Mods {
                    option: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1bx"
        );
    }

    #[test]
    fn page_and_edit_keys() {
        assert_eq!(enc(Key::PageUp, Mods::default(), false), b"\x1b[5~");
        assert_eq!(enc(Key::PageDown, Mods::default(), false), b"\x1b[6~");
        assert_eq!(enc(Key::Delete, Mods::default(), false), b"\x1b[3~");
    }

    #[test]
    fn modified_cursor_keys_emit_csi_1_m_final() {
        // Ctrl+Up → \x1b[1;5A (m = 1+4 = 5)
        assert_eq!(
            enc(
                Key::Up,
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1b[1;5A"
        );
        // Shift+Left → \x1b[1;2D (m = 1+1 = 2)
        assert_eq!(
            enc(
                Key::Left,
                Mods {
                    shift: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1b[1;2D"
        );
        // Alt+Down → \x1b[1;3B (m = 1+2 = 3)
        assert_eq!(
            enc(
                Key::Down,
                Mods {
                    option: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1b[1;3B"
        );
        // Shift+Alt+Right → \x1b[1;4C (m = 1+1+2 = 4)
        assert_eq!(
            enc(
                Key::Right,
                Mods {
                    shift: true,
                    option: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1b[1;4C"
        );
        // Modified cursor ignores DECCKM (always CSI form).
        assert_eq!(
            enc(
                Key::Up,
                Mods {
                    control: true,
                    ..Default::default()
                },
                true
            ),
            b"\x1b[1;5A"
        );
    }

    #[test]
    fn modified_edit_keys_emit_csi_n_m_tilde() {
        // Shift+PageUp → \x1b[5;2~
        assert_eq!(
            enc(
                Key::PageUp,
                Mods {
                    shift: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1b[5;2~"
        );
        // Ctrl+Delete → \x1b[3;5~
        assert_eq!(
            enc(
                Key::Delete,
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1b[3;5~"
        );
    }

    #[test]
    fn function_keys_f1_f4_unmodified_use_ss3() {
        assert_eq!(enc(Key::F1, Mods::default(), false), b"\x1bOP");
        assert_eq!(enc(Key::F2, Mods::default(), false), b"\x1bOQ");
        assert_eq!(enc(Key::F3, Mods::default(), false), b"\x1bOR");
        assert_eq!(enc(Key::F4, Mods::default(), false), b"\x1bOS");
    }

    #[test]
    fn function_keys_f5_f12_unmodified_use_tilde_sequences() {
        assert_eq!(enc(Key::F5, Mods::default(), false), b"\x1b[15~");
        assert_eq!(enc(Key::F6, Mods::default(), false), b"\x1b[17~");
        assert_eq!(enc(Key::F7, Mods::default(), false), b"\x1b[18~");
        assert_eq!(enc(Key::F8, Mods::default(), false), b"\x1b[19~");
        assert_eq!(enc(Key::F9, Mods::default(), false), b"\x1b[20~");
        assert_eq!(enc(Key::F10, Mods::default(), false), b"\x1b[21~");
        assert_eq!(enc(Key::F11, Mods::default(), false), b"\x1b[23~");
        assert_eq!(enc(Key::F12, Mods::default(), false), b"\x1b[24~");
    }

    #[test]
    fn modified_function_keys() {
        // Shift+F1 → \x1b[1;2P
        assert_eq!(
            enc(
                Key::F1,
                Mods {
                    shift: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1b[1;2P"
        );
        // Ctrl+F5 → \x1b[15;5~
        assert_eq!(
            enc(
                Key::F5,
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            b"\x1b[15;5~"
        );
    }

    #[test]
    fn mouse_sgr_encoding() {
        let mut b = [0u8; 32];
        // Left press at col 5, row 3: \x1b[<0;5;3M
        assert_eq!(encode_mouse(0, 5, 3, true, true, &mut b), b"\x1b[<0;5;3M");
        // Left release: \x1b[<0;5;3m
        assert_eq!(encode_mouse(0, 5, 3, false, true, &mut b), b"\x1b[<0;5;3m");
        // Right press at col 10, row 2: \x1b[<2;10;2M
        assert_eq!(encode_mouse(2, 10, 2, true, true, &mut b), b"\x1b[<2;10;2M");
        // Scroll-up: button=64
        assert_eq!(encode_mouse(64, 1, 1, true, true, &mut b), b"\x1b[<64;1;1M");
    }

    #[test]
    fn mouse_legacy_encoding() {
        let mut b = [0u8; 16];
        let result = encode_mouse(0, 1, 1, true, false, &mut b);
        assert_eq!(result.len(), 6);
        assert_eq!(result[0], 0x1b);
        assert_eq!(result[1], b'[');
        assert_eq!(result[2], b'M');
        assert_eq!(result[3], 32); // 32+0
        assert_eq!(result[4], 33); // 32+1
        assert_eq!(result[5], 33); // 32+1
    }

    #[test]
    fn mouse_legacy_buffer_too_small_returns_empty() {
        // Buffer < 6 bytes → returns &out[..0].
        let mut b = [0u8; 4];
        let result = encode_mouse(0, 1, 1, true, false, &mut b);
        assert_eq!(result.len(), 0);
    }

    // ── Home / End / Insert ─────────────────────────────────────────────────

    #[test]
    fn home_end_insert_plain() {
        // Home and End are cursor-key variants; Insert is an edit key.
        assert_eq!(enc(Key::Home, Mods::default(), false), b"\x1b[H");
        assert_eq!(enc(Key::End, Mods::default(), false), b"\x1b[F");
        assert_eq!(enc(Key::Insert, Mods::default(), false), b"\x1b[2~");
    }

    #[test]
    fn home_end_app_cursor_mode() {
        // Home/End honor DECCKM like Up/Down/Left/Right.
        assert_eq!(enc(Key::Home, Mods::default(), true), b"\x1bOH");
        assert_eq!(enc(Key::End, Mods::default(), true), b"\x1bOF");
    }

    // ── ctrl+option text encoding ───────────────────────────────────────────

    #[test]
    fn ctrl_option_control_char_produces_esc_byte() {
        // Ctrl+Opt+A: ESC + 0x01
        let result = enc(
            Key::Text('a'),
            Mods {
                control: true,
                option: true,
                ..Default::default()
            },
            false,
        );
        assert_eq!(result, &[0x1b, 0x01]);
    }

    #[test]
    fn ctrl_non_control_char_passes_through_without_control_byte() {
        // '1' has no control_byte mapping; ctrl+1 → just '1' (no control prefix).
        let result = enc(
            Key::Text('1'),
            Mods {
                control: true,
                ..Default::default()
            },
            false,
        );
        assert_eq!(result, b"1");
    }

    // ── control_byte special characters ────────────────────────────────────

    #[test]
    fn control_byte_special_chars() {
        // '@', '[', '\', ']', '^', '_', '?' all have control_byte mappings.
        assert_eq!(
            enc(
                Key::Text('@'),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x00]
        );
        assert_eq!(
            enc(
                Key::Text('['),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x1b]
        );
        assert_eq!(
            enc(
                Key::Text('\\'),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x1c]
        );
        assert_eq!(
            enc(
                Key::Text(']'),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x1d]
        );
        assert_eq!(
            enc(
                Key::Text('^'),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x1e]
        );
        assert_eq!(
            enc(
                Key::Text('_'),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x1f]
        );
        assert_eq!(
            enc(
                Key::Text('?'),
                Mods {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            &[0x7f]
        );
    }
}
