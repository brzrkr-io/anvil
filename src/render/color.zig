const std = @import("std");

pub const ClearColor = struct { r: f64, g: f64, b: f64, a: f64 };

/// Parse a `#rrggbb` (or bare `rrggbb`) hex string into a normalized,
/// fully-opaque ClearColor. Returns error.InvalidHex on bad length or digits.
pub fn hexToClearColor(hex: []const u8) !ClearColor {
    var s = hex;
    if (s.len > 0 and s[0] == '#') s = s[1..];
    if (s.len != 6) return error.InvalidHex;
    const r = std.fmt.parseInt(u8, s[0..2], 16) catch return error.InvalidHex;
    const g = std.fmt.parseInt(u8, s[2..4], 16) catch return error.InvalidHex;
    const b = std.fmt.parseInt(u8, s[4..6], 16) catch return error.InvalidHex;
    return .{
        .r = @as(f64, @floatFromInt(r)) / 255.0,
        .g = @as(f64, @floatFromInt(g)) / 255.0,
        .b = @as(f64, @floatFromInt(b)) / 255.0,
        .a = 1.0,
    };
}

/// The Mineral dark-mode background, carried over from anvil-console/src/app.css.
pub const mineral_dark_bg = "#0c0d10";

/// Default terminal foreground (Mineral dark `--text`) and background.
pub const default_fg = [3]u8{ 0xe8, 0xea, 0xee };
pub const default_bg = [3]u8{ 0x0c, 0x0d, 0x10 };

/// The 16 ANSI base colors, tuned to the Mineral palette.
const ansi16 = [16][3]u8{
    .{ 0x0c, 0x0d, 0x10 }, // 0  black
    .{ 0xe5, 0x48, 0x4d }, // 1  red
    .{ 0x46, 0xa7, 0x58 }, // 2  green
    .{ 0xe2, 0xa3, 0x36 }, // 3  yellow
    .{ 0x4f, 0x7c, 0xc9 }, // 4  blue
    .{ 0xb0, 0x5c, 0xe6 }, // 5  magenta
    .{ 0x2b, 0xb8, 0xb0 }, // 6  cyan (Mineral accent)
    .{ 0x98, 0x9b, 0xa6 }, // 7  white (muted)
    .{ 0x5a, 0x5e, 0x68 }, // 8  bright black
    .{ 0xff, 0x63, 0x69 }, // 9  bright red
    .{ 0x5d, 0xc8, 0x73 }, // 10 bright green
    .{ 0xff, 0xc7, 0x4a }, // 11 bright yellow
    .{ 0x6e, 0x9b, 0xe8 }, // 12 bright blue
    .{ 0xc9, 0x8a, 0xf0 }, // 13 bright magenta
    .{ 0x57, 0xd6, 0xcd }, // 14 bright cyan
    .{ 0xe8, 0xea, 0xee }, // 15 bright white (text)
};

/// xterm-style 256-color palette lookup -> RGB.
pub fn palette256(index: u8) [3]u8 {
    if (index < 16) return ansi16[index];
    if (index < 232) {
        const i: usize = @as(usize, index) - 16;
        const levels = [6]u8{ 0, 95, 135, 175, 215, 255 };
        return .{ levels[(i / 36) % 6], levels[(i / 6) % 6], levels[i % 6] };
    }
    const v: u8 = @intCast(8 + 10 * (@as(u16, index) - 232));
    return .{ v, v, v };
}

test "palette256 covers the three ranges" {
    try std.testing.expectEqual([3]u8{ 0x2b, 0xb8, 0xb0 }, palette256(6)); // ANSI
    try std.testing.expectEqual([3]u8{ 0, 0, 0 }, palette256(16)); // cube origin
    try std.testing.expectEqual([3]u8{ 255, 255, 255 }, palette256(231)); // cube max
    try std.testing.expectEqual([3]u8{ 8, 8, 8 }, palette256(232)); // gray start
    try std.testing.expectEqual([3]u8{ 238, 238, 238 }, palette256(255)); // gray end
}

test "parses #rrggbb" {
    const c = try hexToClearColor("#0c0d10");
    try std.testing.expectApproxEqAbs(@as(f64, 0x0c) / 255.0, c.r, 1e-9);
    try std.testing.expectApproxEqAbs(@as(f64, 0x0d) / 255.0, c.g, 1e-9);
    try std.testing.expectApproxEqAbs(@as(f64, 0x10) / 255.0, c.b, 1e-9);
    try std.testing.expectEqual(@as(f64, 1.0), c.a);
}

test "accepts hex without leading #" {
    const c = try hexToClearColor("0c0d10");
    try std.testing.expectApproxEqAbs(@as(f64, 0x0c) / 255.0, c.r, 1e-9);
}

test "rejects wrong length" {
    try std.testing.expectError(error.InvalidHex, hexToClearColor("#fff"));
    try std.testing.expectError(error.InvalidHex, hexToClearColor(""));
}

test "rejects non-hex digits" {
    try std.testing.expectError(error.InvalidHex, hexToClearColor("#zzzzzz"));
}
