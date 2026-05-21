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
};

/// Encode a key press into terminal input bytes written into `out` (8 bytes
/// is always enough). Returns the used slice. `app_cursor` selects DECCKM
/// application cursor-key mode (ESC O x instead of ESC [ x).
pub fn encode(key: Key, mods: Mods, app_cursor: bool, out: []u8) []const u8 {
    switch (key) {
        .text => |cp| return encodeText(cp, mods, out),
        .enter => return set(out, "\r"),
        .tab => return if (mods.shift) set(out, "\x1b[Z") else set(out, "\t"),
        .backspace => return set(out, "\x7f"),
        .escape => return set(out, "\x1b"),
        .up => return cursorKey(out, app_cursor, 'A'),
        .down => return cursorKey(out, app_cursor, 'B'),
        .right => return cursorKey(out, app_cursor, 'C'),
        .left => return cursorKey(out, app_cursor, 'D'),
        .home => return cursorKey(out, app_cursor, 'H'),
        .end => return cursorKey(out, app_cursor, 'F'),
        .page_up => return set(out, "\x1b[5~"),
        .page_down => return set(out, "\x1b[6~"),
        .delete => return set(out, "\x1b[3~"),
        .insert => return set(out, "\x1b[2~"),
    }
}

fn set(out: []u8, bytes: []const u8) []const u8 {
    @memcpy(out[0..bytes.len], bytes);
    return out[0..bytes.len];
}

fn cursorKey(out: []u8, app_cursor: bool, final: u8) []const u8 {
    out[0] = 0x1b;
    out[1] = if (app_cursor) 'O' else '[';
    out[2] = final;
    return out[0..3];
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
    var b: [8]u8 = undefined;
    try std.testing.expectEqualStrings("a", encode(.{ .text = 'a' }, .{}, false, &b));
}

test "unicode codepoint encodes as utf-8" {
    var b: [8]u8 = undefined;
    try std.testing.expectEqualStrings("\u{00e9}", encode(.{ .text = 0xe9 }, .{}, false, &b));
}

test "control letters map to C0 codes" {
    var b: [8]u8 = undefined;
    try std.testing.expectEqualSlices(u8, &.{0x03}, encode(.{ .text = 'c' }, .{ .control = true }, false, &b));
    try std.testing.expectEqualSlices(u8, &.{0x03}, encode(.{ .text = 'C' }, .{ .control = true }, false, &b));
    try std.testing.expectEqualSlices(u8, &.{0x01}, encode(.{ .text = 'a' }, .{ .control = true }, false, &b));
    try std.testing.expectEqualSlices(u8, &.{0x00}, encode(.{ .text = ' ' }, .{ .control = true }, false, &b));
}

test "named keys" {
    var b: [8]u8 = undefined;
    try std.testing.expectEqualStrings("\r", encode(.enter, .{}, false, &b));
    try std.testing.expectEqualStrings("\x7f", encode(.backspace, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b", encode(.escape, .{}, false, &b));
    try std.testing.expectEqualStrings("\t", encode(.tab, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[Z", encode(.tab, .{ .shift = true }, false, &b));
}

test "cursor keys honor DECCKM" {
    var b: [8]u8 = undefined;
    try std.testing.expectEqualStrings("\x1b[A", encode(.up, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1bOA", encode(.up, .{}, true, &b));
    try std.testing.expectEqualStrings("\x1b[D", encode(.left, .{}, false, &b));
}

test "option prefixes ESC (meta)" {
    var b: [8]u8 = undefined;
    try std.testing.expectEqualStrings("\x1bx", encode(.{ .text = 'x' }, .{ .option = true }, false, &b));
}

test "page and edit keys" {
    var b: [8]u8 = undefined;
    try std.testing.expectEqualStrings("\x1b[5~", encode(.page_up, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[6~", encode(.page_down, .{}, false, &b));
    try std.testing.expectEqualStrings("\x1b[3~", encode(.delete, .{}, false, &b));
}
