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
