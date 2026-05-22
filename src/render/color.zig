const std = @import("std");

pub const ClearColor = struct { r: f64, g: f64, b: f64, a: f64 };

/// Linearly blend two RGB colors. `t=0` → `a`, `t=1` → `b`.
pub fn mix(a: [3]u8, b: [3]u8, t: f32) [3]u8 {
    const c = std.math.clamp(t, 0.0, 1.0);
    var out: [3]u8 = undefined;
    for (0..3) |i| {
        const av: f32 = @floatFromInt(a[i]);
        const bv: f32 = @floatFromInt(b[i]);
        out[i] = @intFromFloat(@round(av + (bv - av) * c));
    }
    return out;
}

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

test "mix at t=0 returns a" {
    const a = [3]u8{ 10, 20, 30 };
    const b = [3]u8{ 200, 150, 100 };
    try std.testing.expectEqual(a, mix(a, b, 0.0));
}

test "mix at t=1 returns b" {
    const a = [3]u8{ 10, 20, 30 };
    const b = [3]u8{ 200, 150, 100 };
    try std.testing.expectEqual(b, mix(a, b, 1.0));
}

test "mix at t=0.5 returns midpoint" {
    const a = [3]u8{ 0, 0, 0 };
    const b = [3]u8{ 200, 100, 50 };
    const m = mix(a, b, 0.5);
    // Each channel should be within 1 of the exact midpoint.
    try std.testing.expect(@abs(@as(i16, m[0]) - 100) <= 1);
    try std.testing.expect(@abs(@as(i16, m[1]) - 50) <= 1);
    try std.testing.expect(@abs(@as(i16, m[2]) - 25) <= 1);
}
