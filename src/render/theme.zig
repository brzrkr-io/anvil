const std = @import("std");

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

    pub fn f32x3(self: Rgb) [3]f32 {
        return .{
            @as(f32, @floatFromInt(self.r)) / 255.0,
            @as(f32, @floatFromInt(self.g)) / 255.0,
            @as(f32, @floatFromInt(self.b)) / 255.0,
        };
    }
};

fn nib(comptime c: u8) u8 {
    return switch (c) {
        '0'...'9' => c - '0',
        'a'...'f' => c - 'a' + 10,
        'A'...'F' => c - 'A' + 10,
        else => 0,
    };
}

fn hex(comptime s: []const u8) Rgb {
    return .{
        .r = nib(s[0]) * 16 + nib(s[1]),
        .g = nib(s[2]) * 16 + nib(s[3]),
        .b = nib(s[4]) * 16 + nib(s[5]),
    };
}

/// Resolved colors for one surface mode. ANSI 0-15 plus chrome and selection.
pub const Theme = struct {
    bg: Rgb,
    fg: Rgb,
    bar: Rgb,
    separator: Rgb,
    sel_bg: Rgb,
    sel_fg: Rgb,
    ansi: [16]Rgb,
};

// Mineral palette (BRAND.md). ANSI hues are brand-flavored but kept distinct.
pub const mineral_dark = Theme{
    .bg = hex("0b0d0e"), // graphite
    .fg = hex("d2d8db"), // mist
    .bar = hex("161a1c"), // charcoal
    .separator = hex("374046"), // ash
    .sel_bg = hex("2f4a4e"), // dim mineral
    .sel_fg = hex("eef1f2"), // bone
    .ansi = .{
        hex("161a1c"), // 0 black (charcoal, visible on graphite)
        hex("b13a30"), // 1 red (failure)
        hex("3f8a5b"), // 2 green (verified)
        hex("b07a14"), // 3 yellow (attention)
        hex("4a6f8a"), // 4 blue (steel)
        hex("6a5fa3"), // 5 magenta (agent)
        hex("2f7f86"), // 6 cyan (mineral)
        hex("d2d8db"), // 7 white (mist)
        hex("4a555c"), // 8 bright black
        hex("cf5346"), // 9 bright red
        hex("57a673"), // 10 bright green
        hex("cf962b"), // 11 bright yellow
        hex("5d86a3"), // 12 bright blue
        hex("8377c0"), // 13 bright magenta
        hex("3f9aa1"), // 14 bright cyan
        hex("eef1f2"), // 15 bright white (bone)
    },
};

pub const mineral_light = Theme{
    .bg = hex("eef1f2"), // bone
    .fg = hex("0c0d0e"), // ink
    .bar = hex("d2d8db"), // mist
    .separator = hex("86919a"), // alloy
    .sel_bg = hex("b8d4d6"), // light mineral
    .sel_fg = hex("0c0d0e"), // ink
    .ansi = .{
        hex("0c0d0e"), // 0 black (ink)
        hex("a8322a"), // 1 red
        hex("2f7048"), // 2 green
        hex("8f6210"), // 3 yellow
        hex("3c5e78"), // 4 blue
        hex("574d8c"), // 5 magenta
        hex("266a70"), // 6 cyan (mineral)
        hex("565f66"), // 7 white (dark gray on light)
        hex("86919a"), // 8 bright black (alloy)
        hex("c5462a"), // 9 bright red (ember)
        hex("3f8a5b"), // 10 bright green
        hex("b07a14"), // 11 bright yellow
        hex("4a6f8a"), // 12 bright blue
        hex("6a5fa3"), // 13 bright magenta
        hex("2f7f86"), // 14 bright cyan
        hex("374046"), // 15 bright white (ash)
    },
};

test "hex parses brand tokens" {
    try std.testing.expectEqual(Rgb{ .r = 0x0b, .g = 0x0d, .b = 0x0e }, mineral_dark.bg);
    try std.testing.expectEqual(Rgb{ .r = 0xee, .g = 0xf1, .b = 0xf2 }, mineral_light.bg);
}
