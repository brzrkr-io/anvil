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

/// The Caldera graphite canvas (brand token: caldera.graphite #0b0d0e).
pub const mineral_dark_bg = "#0b0d0e";

/// Default terminal foreground (Mineral dark `--text`) and background.
pub const default_fg = [3]u8{ 0xe8, 0xea, 0xee };
pub const default_bg = [3]u8{ 0x0b, 0x0d, 0x0e };

/// The 16 ANSI base colors aligned to the Caldera brand semantic palette.
/// Normal slots map to brand status colors; bright slots are lightened variants.
/// Neutral/black/white use Mineral core materials.
const ansi16 = [16][3]u8{
    .{ 0x0b, 0x0d, 0x0e }, // 0  black        — graphite #0b0d0e (canvas)
    .{ 0xb1, 0x3a, 0x30 }, // 1  red          — status.failure #b13a30
    .{ 0x3f, 0x8a, 0x5b }, // 2  green        — status.verified #3f8a5b
    .{ 0xb0, 0x7a, 0x14 }, // 3  yellow       — status.attention #b07a14
    .{ 0x4a, 0x6f, 0x8a }, // 4  blue         — muted steel (no distinct brand blue)
    .{ 0x6a, 0x5f, 0xa3 }, // 5  magenta      — status.agent #6a5fa3
    .{ 0x2f, 0x7f, 0x86 }, // 6  cyan         — accent.mineral / status.info #2f7f86
    .{ 0x86, 0x91, 0x9a }, // 7  white        — alloy #86919a (muted text)
    .{ 0x37, 0x40, 0x46 }, // 8  bright black — ash #374046 (dark surface)
    .{ 0xd4, 0x4a, 0x3f }, // 9  bright red   — failure lightened
    .{ 0x52, 0xb0, 0x70 }, // 10 bright green — verified lightened
    .{ 0xd4, 0x9a, 0x28 }, // 11 bright yellow — attention lightened
    .{ 0x6a, 0x9a, 0xb8 }, // 12 bright blue  — steel blue lightened
    .{ 0x8f, 0x84, 0xc8 }, // 13 bright magenta — agent lightened
    .{ 0x4a, 0xa8, 0xb0 }, // 14 bright cyan  — mineral accent lightened
    .{ 0xe8, 0xea, 0xee }, // 15 bright white  — foreground text
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
    try std.testing.expectEqual([3]u8{ 0x2f, 0x7f, 0x86 }, palette256(6)); // ANSI cyan — accent.mineral
    try std.testing.expectEqual([3]u8{ 0, 0, 0 }, palette256(16)); // cube origin
    try std.testing.expectEqual([3]u8{ 255, 255, 255 }, palette256(231)); // cube max
    try std.testing.expectEqual([3]u8{ 8, 8, 8 }, palette256(232)); // gray start
    try std.testing.expectEqual([3]u8{ 238, 238, 238 }, palette256(255)); // gray end
}

test "parses #rrggbb" {
    const c = try hexToClearColor("#0b0d0e");
    try std.testing.expectApproxEqAbs(@as(f64, 0x0b) / 255.0, c.r, 1e-9);
    try std.testing.expectApproxEqAbs(@as(f64, 0x0d) / 255.0, c.g, 1e-9);
    try std.testing.expectApproxEqAbs(@as(f64, 0x0e) / 255.0, c.b, 1e-9);
    try std.testing.expectEqual(@as(f64, 1.0), c.a);
}

test "accepts hex without leading #" {
    const c = try hexToClearColor("0b0d0e");
    try std.testing.expectApproxEqAbs(@as(f64, 0x0b) / 255.0, c.r, 1e-9);
}

test "rejects wrong length" {
    try std.testing.expectError(error.InvalidHex, hexToClearColor("#fff"));
    try std.testing.expectError(error.InvalidHex, hexToClearColor(""));
}

test "rejects non-hex digits" {
    try std.testing.expectError(error.InvalidHex, hexToClearColor("#zzzzzz"));
}
