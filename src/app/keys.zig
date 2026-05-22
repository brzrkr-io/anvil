//! Translates key presses into the byte sequences a terminal expects.
//! Pure logic — no AppKit — so it is fully unit-testable. The AppKit
//! NSEvent -> Key extraction lives at the call site.

const std = @import("std");

pub const Mods = struct {
    shift: bool = false,
    control: bool = false,
    option: bool = false, // the Alt / Meta key
    command: bool = false,
};

pub const Key = union(enum) {
    /// An already-resolved typed Unicode codepoint.
    text: u21,
    enter,
    tab,
    backspace,
    escape,
    up,
    down,
    right,
    left,
    home,
    end,
    page_up,
    page_down,
    delete, // forward delete
    insert,
    f1,
    f2,
    f3,
    f4,
    f5,
    f6,
    f7,
    f8,
    f9,
    f10,
    f11,
    f12,
};

/// Modifier parameter for XTerm modifier sequences: 1 + shift + alt*2 + ctrl*4.
/// Returns 0 when no modifier is active (caller uses unmodified form instead).
fn modParam(mods: Mods) u8 {
    var m: u8 = 1;
    if (mods.shift) m += 1;
    if (mods.option) m += 2;
    if (mods.control) m += 4;
    return m;
}

/// True when any of shift/option/control is held (the three modifier bits).
fn anyMod(mods: Mods) bool {
    return mods.shift or mods.option or mods.control;
}

/// Encode a key press into terminal input bytes written into `out` (16 bytes
/// is always enough). Returns the used slice. `app_cursor` selects DECCKM
/// application cursor-key mode (ESC O x instead of ESC [ x).
pub fn encode(key: Key, mods: Mods, app_cursor: bool, out: []u8) []const u8 {
    switch (key) {
        .text => |cp| return encodeText(cp, mods, out),
        .enter => return set(out, "\r"),
        .tab => return if (mods.shift) set(out, "\x1b[Z") else set(out, "\t"),
        .backspace => return set(out, "\x7f"),
        .escape => return set(out, "\x1b"),
        .up => return cursorKey(out, mods, app_cursor, 'A'),
        .down => return cursorKey(out, mods, app_cursor, 'B'),
        .right => return cursorKey(out, mods, app_cursor, 'C'),
        .left => return cursorKey(out, mods, app_cursor, 'D'),
        .home => return cursorKey(out, mods, app_cursor, 'H'),
        .end => return cursorKey(out, mods, app_cursor, 'F'),
        .page_up => return editKey(out, mods, 5),
        .page_down => return editKey(out, mods, 6),
        .delete => return editKey(out, mods, 3),
        .insert => return editKey(out, mods, 2),
        .f1 => return fnKey(out, mods, 1),
        .f2 => return fnKey(out, mods, 2),
        .f3 => return fnKey(out, mods, 3),
        .f4 => return fnKey(out, mods, 4),
        .f5 => return fnKey(out, mods, 5),
        .f6 => return fnKey(out, mods, 6),
        .f7 => return fnKey(out, mods, 7),
        .f8 => return fnKey(out, mods, 8),
        .f9 => return fnKey(out, mods, 9),
        .f10 => return fnKey(out, mods, 10),
        .f11 => return fnKey(out, mods, 11),
        .f12 => return fnKey(out, mods, 12),
    }
}

fn set(out: []u8, bytes: []const u8) []const u8 {
    @memcpy(out[0..bytes.len], bytes);
    return out[0..bytes.len];
}

/// Cursor keys: up/down/right/left/home/end.
/// With no modifier: \x1b[{final} or \x1bO{final} (DECCKM).
/// With modifier:    \x1b[1;{m}{final} (always CSI form regardless of DECCKM).
fn cursorKey(out: []u8, mods: Mods, app_cursor: bool, final: u8) []const u8 {
    if (!anyMod(mods)) {
        out[0] = 0x1b;
        out[1] = if (app_cursor) 'O' else '[';
        out[2] = final;
        return out[0..3];
    }
    const m = modParam(mods);
    return std.fmt.bufPrint(out, "\x1b[1;{d}{c}", .{ m, final }) catch out[0..0];
}

/// Edit keys: page_up/page_down/delete/insert — CSI {n} ~ form.
/// With no modifier: \x1b[{n}~.
/// With modifier:    \x1b[{n};{m}~.
fn editKey(out: []u8, mods: Mods, n: u8) []const u8 {
    if (!anyMod(mods)) {
        return std.fmt.bufPrint(out, "\x1b[{d}~", .{n}) catch out[0..0];
    }
    const m = modParam(mods);
    return std.fmt.bufPrint(out, "\x1b[{d};{d}~", .{ n, m }) catch out[0..0];
}

/// Function keys F1–F12.
/// F1-F4 unmodified:  \x1bOP/OQ/OR/OS.
/// F1-F4 modified:    \x1b[1;{m}P/Q/R/S.
/// F5-F12 unmodified: \x1b[{vt}~  (vt codes: 15,17,18,19,20,21,23,24).
/// F5-F12 modified:   \x1b[{vt};{m}~.
fn fnKey(out: []u8, mods: Mods, n: u8) []const u8 {
    // F1-F4 use SS3 letters without modifiers.
    if (n <= 4) {
        const letters = [_]u8{ 'P', 'Q', 'R', 'S' };
        const letter = letters[n - 1];
        if (!anyMod(mods)) {
            out[0] = 0x1b;
            out[1] = 'O';
            out[2] = letter;
            return out[0..3];
        }
        const m = modParam(mods);
        return std.fmt.bufPrint(out, "\x1b[1;{d}{c}", .{ m, letter }) catch out[0..0];
    }
    // F5-F12 use tilde sequences.
    const vt_codes = [_]u8{ 15, 17, 18, 19, 20, 21, 23, 24 }; // F5-F12
    const vt = vt_codes[n - 5];
    if (!anyMod(mods)) {
        return std.fmt.bufPrint(out, "\x1b[{d}~", .{vt}) catch out[0..0];
    }
    const m = modParam(mods);
    return std.fmt.bufPrint(out, "\x1b[{d};{d}~", .{ vt, m }) catch out[0..0];
}

/// Mouse event encoding. Returns the bytes to write to the PTY.
/// `button`: 0 left, 1 middle, 2 right; for drag add 32; for scroll 64/65.
/// `col`, `row`: 1-based terminal cell coordinates.
/// `press`: true for button-down / motion with button held, false for release.
/// `sgr`: true = SGR encoding (\x1b[<...); false = legacy X10 (\x1b[M...).
pub fn encodeMouse(
    button: u8,
    col: usize,
    row: usize,
    press: bool,
    sgr: bool,
    out: []u8,
) []const u8 {
    if (sgr) {
        const suffix: u8 = if (press) 'M' else 'm';
        return std.fmt.bufPrint(out, "\x1b[<{d};{d};{d}{c}", .{ button, col, row, suffix }) catch out[0..0];
    } else {
        // Legacy X10: only encodes press (release uses button 3).
        const b_byte: u8 = @truncate(32 + button);
        const c_byte: u8 = @truncate(32 + @min(col, 223));
        const r_byte: u8 = @truncate(32 + @min(row, 223));
        if (out.len < 6) return out[0..0];
        out[0] = 0x1b;
        out[1] = '[';
        out[2] = 'M';
        out[3] = b_byte;
        out[4] = c_byte;
        out[5] = r_byte;
        return out[0..6];
    }
}

fn encodeText(cp: u21, mods: Mods, out: []u8) []const u8 {
    if (mods.control) {
        if (controlByte(cp)) |b| {
            if (mods.option) {
                out[0] = 0x1b;
                out[1] = b;
                return out[0..2];
            }
            out[0] = b;
            return out[0..1];
        }
    }
    // Plain (or Option-prefixed) text: UTF-8 encode the codepoint.
    var n: usize = 0;
    if (mods.option) {
        out[0] = 0x1b;
        n = 1;
    }
    const len = std.unicode.utf8Encode(cp, out[n..]) catch return out[0..n];
    return out[0 .. n + len];
}

/// Map a codepoint to its C0 control byte when Control is held, else null.
fn controlByte(cp: u21) ?u8 {
    return switch (cp) {
        ' ' => 0, // Ctrl-Space -> NUL
        'a'...'z' => @intCast(cp - 'a' + 1), // Ctrl-A..Z -> 1..26
        'A'...'Z' => @intCast(cp - 'A' + 1),
        '@' => 0,
        '[' => 0x1b,
        '\\' => 0x1c,
        ']' => 0x1d,
        '^' => 0x1e,
        '_' => 0x1f,
        '?' => 0x7f, // Ctrl-? -> DEL
        else => null,
    };
}

test "plain ascii encodes to one byte" {
    var b: [16]u8 = undefined;
    try std.testing.expectEqualStrings("a", encode(.{ .text = 'a' }, .{}, false, &b));
}

test "unicode codepoint encodes as utf-8" {
    var b: [16]u8 = undefined;
    try std.testing.expectEqualStrings("\u{00e9}", encode(.{ .text = 0xe9 }, .{}, false, &b));
}

test "control letters map to C0 codes" {
    var b: [16]u8 = undefined;
    try std.testing.expectEqualSlices(u8, &.{0x03}, encode(.{ .text = 'c' }, .{ .control = true }, false, &b));
    try std.testing.expectEqualSlices(u8, &.{0x03}, encode(.{ .text = 'C' }, .{ .control = true }, false, &b));
    try std.testing.expectEqualSlices(u8, &.{0x01}, encode(.{ .text = 'a' }, .{ .control = true }, false, &b));
    try std.testing.expectEqualSlices(u8, &.{0x00}, encode(.{ .text = ' ' }, .{ .control = true }, false, &b));
}

test "named keys" {
    var b: [16]u8 = undefined;
    try std.testing.expectEqualStrings("\r", encode(.enter, .{}, false, &b));
    try std.testing.expectEqualStrings("\x7f", encode(.backspace, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b", encode(.escape, .{}, false, &b));
    try std.testing.expectEqualStrings("\t", encode(.tab, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[Z", encode(.tab, .{ .shift = true }, false, &b));
}

test "cursor keys honor DECCKM" {
    var b: [16]u8 = undefined;
    try std.testing.expectEqualStrings("\x1b[A", encode(.up, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1bOA", encode(.up, .{}, true, &b));
    try std.testing.expectEqualStrings("\x1b[D", encode(.left, .{}, false, &b));
}

test "option prefixes ESC (meta)" {
    var b: [16]u8 = undefined;
    try std.testing.expectEqualStrings("\x1bx", encode(.{ .text = 'x' }, .{ .option = true }, false, &b));
}

test "page and edit keys" {
    var b: [16]u8 = undefined;
    try std.testing.expectEqualStrings("\x1b[5~", encode(.page_up, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[6~", encode(.page_down, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[3~", encode(.delete, .{}, false, &b));
}

test "modified cursor keys emit CSI 1;m final" {
    var b: [16]u8 = undefined;
    // Ctrl+Up -> \x1b[1;5A  (m = 1+4 = 5)
    try std.testing.expectEqualStrings("\x1b[1;5A", encode(.up, .{ .control = true }, false, &b));
    // Shift+Left -> \x1b[1;2D  (m = 1+1 = 2)
    try std.testing.expectEqualStrings("\x1b[1;2D", encode(.left, .{ .shift = true }, false, &b));
    // Alt+Down -> \x1b[1;3B  (m = 1+2 = 3)
    try std.testing.expectEqualStrings("\x1b[1;3B", encode(.down, .{ .option = true }, false, &b));
    // Shift+Alt+Right -> \x1b[1;4C  (m = 1+1+2 = 4)
    try std.testing.expectEqualStrings("\x1b[1;4C", encode(.right, .{ .shift = true, .option = true }, false, &b));
    // Modified cursor ignores DECCKM (always CSI form).
    try std.testing.expectEqualStrings("\x1b[1;5A", encode(.up, .{ .control = true }, true, &b));
}

test "modified edit keys emit CSI n;m~" {
    var b: [16]u8 = undefined;
    // Shift+PageUp -> \x1b[5;2~
    try std.testing.expectEqualStrings("\x1b[5;2~", encode(.page_up, .{ .shift = true }, false, &b));
    // Ctrl+Delete -> \x1b[3;5~
    try std.testing.expectEqualStrings("\x1b[3;5~", encode(.delete, .{ .control = true }, false, &b));
}

test "function keys F1-F4 unmodified use SS3" {
    var b: [16]u8 = undefined;
    try std.testing.expectEqualStrings("\x1bOP", encode(.f1, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1bOQ", encode(.f2, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1bOR", encode(.f3, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1bOS", encode(.f4, .{}, false, &b));
}

test "function keys F5-F12 unmodified use tilde sequences" {
    var b: [16]u8 = undefined;
    try std.testing.expectEqualStrings("\x1b[15~", encode(.f5, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[17~", encode(.f6, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[18~", encode(.f7, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[19~", encode(.f8, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[20~", encode(.f9, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[21~", encode(.f10, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[23~", encode(.f11, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[24~", encode(.f12, .{}, false, &b));
}

test "modified function keys" {
    var b: [16]u8 = undefined;
    // Shift+F1 -> \x1b[1;2P  (m = 1+1 = 2)
    try std.testing.expectEqualStrings("\x1b[1;2P", encode(.f1, .{ .shift = true }, false, &b));
    // Ctrl+F5 -> \x1b[15;5~  (m = 1+4 = 5)
    try std.testing.expectEqualStrings("\x1b[15;5~", encode(.f5, .{ .control = true }, false, &b));
}

test "mouse SGR encoding" {
    var b: [32]u8 = undefined;
    // Left press at col 5, row 3: \x1b[<0;5;3M
    try std.testing.expectEqualStrings("\x1b[<0;5;3M", encodeMouse(0, 5, 3, true, true, &b));
    // Left release: \x1b[<0;5;3m
    try std.testing.expectEqualStrings("\x1b[<0;5;3m", encodeMouse(0, 5, 3, false, true, &b));
    // Right press at col 10, row 2: \x1b[<2;10;2M
    try std.testing.expectEqualStrings("\x1b[<2;10;2M", encodeMouse(2, 10, 2, true, true, &b));
    // Scroll-up at col 1, row 1: button = 64
    try std.testing.expectEqualStrings("\x1b[<64;1;1M", encodeMouse(64, 1, 1, true, true, &b));
}

test "mouse legacy encoding" {
    var b: [16]u8 = undefined;
    // Left press at col 1, row 1: ESC [ M (32+0) (32+1) (32+1) = ESC [ M ' '!'!'
    const result = encodeMouse(0, 1, 1, true, false, &b);
    try std.testing.expectEqual(@as(usize, 6), result.len);
    try std.testing.expectEqual(@as(u8, 0x1b), result[0]);
    try std.testing.expectEqual(@as(u8, '['), result[1]);
    try std.testing.expectEqual(@as(u8, 'M'), result[2]);
    try std.testing.expectEqual(@as(u8, 32), result[3]); // 32+0
    try std.testing.expectEqual(@as(u8, 33), result[4]); // 32+1
    try std.testing.expectEqual(@as(u8, 33), result[5]); // 32+1
}
