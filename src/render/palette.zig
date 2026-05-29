const std = @import("std");
const Color = @import("../vt/cell.zig").Color;
const theme = @import("theme.zig");

pub const Rgb = theme.Rgb;

var active: *const theme.Theme = &theme.mineral_dark;

pub fn setActive(t: *const theme.Theme) void {
    active = t;
}

pub fn defaultFg() Rgb {
    return active.fg;
}

pub fn defaultBg() Rgb {
    return active.bg;
}

pub fn selectionBg() Rgb {
    return active.sel_bg;
}

pub fn selectionFg() Rgb {
    return active.sel_fg;
}

pub fn resolve(c: Color, is_fg: bool) Rgb {
    return switch (c) {
        .default => if (is_fg) active.fg else active.bg,
        .indexed => |i| indexed(i),
        .rgb => |v| .{ .r = v.r, .g = v.g, .b = v.b },
    };
}

pub fn indexed(i: u8) Rgb {
    if (i < 16) return active.ansi[i];
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
    try std.testing.expectEqual(theme.mineral_dark.fg, resolve(.default, true));
    try std.testing.expectEqual(theme.mineral_dark.bg, resolve(.default, false));
}

test "indexed 0-15 hit the active ansi table" {
    try std.testing.expectEqual(theme.mineral_dark.ansi[1], resolve(.{ .indexed = 1 }, true));
    try std.testing.expectEqual(theme.mineral_dark.ansi[15], indexed(15));
}

test "setActive switches the resolved palette" {
    setActive(&theme.mineral_light);
    defer setActive(&theme.mineral_dark);
    try std.testing.expectEqual(theme.mineral_light.bg, resolve(.default, false));
    try std.testing.expectEqual(theme.mineral_light.ansi[2], indexed(2));
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
