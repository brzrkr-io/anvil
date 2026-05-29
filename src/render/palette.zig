const std = @import("std");
const Color = @import("../vt/cell.zig").Color;

pub const Rgb = struct {
    r: u8,
    g: u8,
    b: u8,

    pub fn f32x4(self: Rgb) [4]f32 {
        return .{
            @as(f32, @floatFromInt(self.r)) / 255.0,
            @as(f32, @floatFromInt(self.g)) / 255.0,
            @as(f32, @floatFromInt(self.b)) / 255.0,
            1.0,
        };
    }
};

pub const default_fg = Rgb{ .r = 229, .g = 229, .b = 229 };
pub const default_bg = Rgb{ .r = 13, .g = 15, .b = 20 };

const ansi16 = [16]Rgb{
    .{ .r = 0, .g = 0, .b = 0 },
    .{ .r = 205, .g = 0, .b = 0 },
    .{ .r = 0, .g = 205, .b = 0 },
    .{ .r = 205, .g = 205, .b = 0 },
    .{ .r = 0, .g = 0, .b = 238 },
    .{ .r = 205, .g = 0, .b = 205 },
    .{ .r = 0, .g = 205, .b = 205 },
    .{ .r = 229, .g = 229, .b = 229 },
    .{ .r = 127, .g = 127, .b = 127 },
    .{ .r = 255, .g = 0, .b = 0 },
    .{ .r = 0, .g = 255, .b = 0 },
    .{ .r = 255, .g = 255, .b = 0 },
    .{ .r = 92, .g = 92, .b = 255 },
    .{ .r = 255, .g = 0, .b = 255 },
    .{ .r = 0, .g = 255, .b = 255 },
    .{ .r = 255, .g = 255, .b = 255 },
};

pub fn resolve(c: Color, is_fg: bool) Rgb {
    return switch (c) {
        .default => if (is_fg) default_fg else default_bg,
        .indexed => |i| indexed(i),
        .rgb => |v| .{ .r = v.r, .g = v.g, .b = v.b },
    };
}

pub fn indexed(i: u8) Rgb {
    if (i < 16) return ansi16[i];
    if (i < 232) {
        const n = i - 16;
        return .{ .r = cube(n / 36), .g = cube(n / 6 % 6), .b = cube(n % 6) };
    }
    const v: u8 = 8 + (i - 232) * 10;
    return .{ .r = v, .g = v, .b = v };
}

fn cube(n: u8) u8 {
    return if (n == 0) 0 else 55 + n * 40;
}

test "default fg/bg differ by role" {
    try std.testing.expectEqual(default_fg, resolve(.default, true));
    try std.testing.expectEqual(default_bg, resolve(.default, false));
}

test "indexed 0-15 hit the ansi table" {
    try std.testing.expectEqual(ansi16[1], resolve(.{ .indexed = 1 }, true));
    try std.testing.expectEqual(ansi16[15], indexed(15));
}

test "256 color cube" {
    // 16 = cube(0,0,0) = black
    try std.testing.expectEqual(Rgb{ .r = 0, .g = 0, .b = 0 }, indexed(16));
    // 196 = cube(5,0,0) = bright red
    try std.testing.expectEqual(Rgb{ .r = 255, .g = 0, .b = 0 }, indexed(196));
    // 231 = cube(5,5,5) = white
    try std.testing.expectEqual(Rgb{ .r = 255, .g = 255, .b = 255 }, indexed(231));
}

test "grayscale ramp" {
    try std.testing.expectEqual(Rgb{ .r = 8, .g = 8, .b = 8 }, indexed(232));
    try std.testing.expectEqual(Rgb{ .r = 238, .g = 238, .b = 238 }, indexed(255));
}

test "rgb passthrough" {
    try std.testing.expectEqual(Rgb{ .r = 10, .g = 20, .b = 30 }, resolve(.{ .rgb = .{ .r = 10, .g = 20, .b = 30 } }, true));
}

test "f32x4 normalizes" {
    const v = (Rgb{ .r = 255, .g = 0, .b = 51 }).f32x4();
    try std.testing.expectApproxEqAbs(@as(f32, 1.0), v[0], 0.001);
    try std.testing.expectApproxEqAbs(@as(f32, 0.0), v[1], 0.001);
    try std.testing.expectApproxEqAbs(@as(f32, 0.2), v[2], 0.001);
    try std.testing.expectEqual(@as(f32, 1.0), v[3]);
}
