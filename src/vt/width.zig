const std = @import("std");

/// Display columns for a codepoint: 0 (combining), 1 (normal), or 2 (wide).
/// Range-based approximation of East Asian Width + emoji — not the full
/// Unicode table, but covers the common cases that break alignment.
pub fn charWidth(cp: u21) u2 {
    if (cp == 0) return 0;
    if (cp < 0x0300) return 1; // fast path: ASCII/Latin-1/Latin-Ext
    if (inAny(cp, &combining)) return 0;
    if (inAny(cp, &wide)) return 2;
    return 1;
}

const Range = struct { lo: u21, hi: u21 };

fn inAny(cp: u21, ranges: []const Range) bool {
    for (ranges) |r| {
        if (cp < r.lo) return false; // ranges are sorted ascending
        if (cp <= r.hi) return true;
    }
    return false;
}

// Zero-width combining marks (common blocks).
const combining = [_]Range{
    .{ .lo = 0x0300, .hi = 0x036F }, // combining diacritical marks
    .{ .lo = 0x1AB0, .hi = 0x1AFF },
    .{ .lo = 0x1DC0, .hi = 0x1DFF },
    .{ .lo = 0x20D0, .hi = 0x20FF }, // combining marks for symbols
    .{ .lo = 0xFE20, .hi = 0xFE2F }, // combining half marks
};

// East Asian Wide / Fullwidth + emoji ranges (sorted ascending).
const wide = [_]Range{
    .{ .lo = 0x1100, .hi = 0x115F }, // Hangul Jamo
    .{ .lo = 0x2329, .hi = 0x232A }, // angle brackets
    .{ .lo = 0x2E80, .hi = 0x303E }, // CJK radicals, Kangxi, symbols
    .{ .lo = 0x3041, .hi = 0x33FF }, // Hiragana..CJK compat
    .{ .lo = 0x3400, .hi = 0x4DBF }, // CJK Ext A
    .{ .lo = 0x4E00, .hi = 0x9FFF }, // CJK Unified
    .{ .lo = 0xA000, .hi = 0xA4CF }, // Yi
    .{ .lo = 0xAC00, .hi = 0xD7A3 }, // Hangul syllables
    .{ .lo = 0xF900, .hi = 0xFAFF }, // CJK compat ideographs
    .{ .lo = 0xFE30, .hi = 0xFE4F }, // CJK compat forms
    .{ .lo = 0xFF00, .hi = 0xFF60 }, // fullwidth forms
    .{ .lo = 0xFFE0, .hi = 0xFFE6 }, // fullwidth signs
    .{ .lo = 0x1F300, .hi = 0x1F64F }, // emoji + pictographs
    .{ .lo = 0x1F900, .hi = 0x1F9FF }, // supplemental symbols
    .{ .lo = 0x20000, .hi = 0x3FFFD }, // CJK Ext B+
};

test "ascii and latin are width 1" {
    try std.testing.expectEqual(@as(u2, 1), charWidth('A'));
    try std.testing.expectEqual(@as(u2, 1), charWidth(0xE9)); // é
}

test "CJK and emoji are width 2" {
    try std.testing.expectEqual(@as(u2, 2), charWidth(0x4E00)); // 一
    try std.testing.expectEqual(@as(u2, 2), charWidth(0x1F600)); // 😀
    try std.testing.expectEqual(@as(u2, 2), charWidth(0xFF21)); // fullwidth A
}

test "combining marks are width 0" {
    try std.testing.expectEqual(@as(u2, 0), charWidth(0x0301)); // combining acute
}
