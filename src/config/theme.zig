//! Terminal color themes. A `Theme` is plain data; `resolve` produces an
//! active theme from a base name plus optional per-color overrides.

const std = @import("std");
const config = @import("config.zig");

pub const Theme = struct {
    background: [3]u8,
    foreground: [3]u8,
    accent: [3]u8, // cursor color
    ansi: [16][3]u8,

    /// xterm-style 256-color lookup. Slots 0-15 come from `ansi`; 16-231 are
    /// the 6x6x6 cube; 232-255 are the grayscale ramp.
    pub fn palette256(self: Theme, index: u8) [3]u8 {
        if (index < 16) return self.ansi[index];
        if (index < 232) {
            const i: usize = @as(usize, index) - 16;
            const levels = [6]u8{ 0, 95, 135, 175, 215, 255 };
            return .{ levels[(i / 36) % 6], levels[(i / 6) % 6], levels[i % 6] };
        }
        const v: u8 = @intCast(8 + 10 * (@as(u16, index) - 232));
        return .{ v, v, v };
    }
};

pub const mineral_dark: Theme = .{
    .background = .{ 0x0b, 0x0d, 0x0e },
    .foreground = .{ 0xe8, 0xea, 0xee },
    .accent = .{ 0x2f, 0x7f, 0x86 },
    .ansi = .{
        .{ 0x0b, 0x0d, 0x0e }, .{ 0xb1, 0x3a, 0x30 }, .{ 0x3f, 0x8a, 0x5b }, .{ 0xb0, 0x7a, 0x14 },
        .{ 0x4a, 0x6f, 0x8a }, .{ 0x6a, 0x5f, 0xa3 }, .{ 0x2f, 0x7f, 0x86 }, .{ 0x86, 0x91, 0x9a },
        .{ 0x37, 0x40, 0x46 }, .{ 0xd4, 0x4a, 0x3f }, .{ 0x52, 0xb0, 0x70 }, .{ 0xd4, 0x9a, 0x28 },
        .{ 0x6a, 0x9a, 0xb8 }, .{ 0x8f, 0x84, 0xc8 }, .{ 0x4a, 0xa8, 0xb0 }, .{ 0xe8, 0xea, 0xee },
    },
};

pub const mineral_light: Theme = .{
    .background = .{ 0xee, 0xf1, 0xf2 }, // bone
    .foreground = .{ 0x16, 0x1a, 0x1c }, // charcoal
    .accent = .{ 0x2f, 0x7f, 0x86 },     // mineral
    .ansi = .{
        .{ 0x16, 0x1a, 0x1c }, .{ 0xb1, 0x3a, 0x30 }, .{ 0x3f, 0x8a, 0x5b }, .{ 0x8a, 0x5f, 0x10 },
        .{ 0x4a, 0x6f, 0x8a }, .{ 0x6a, 0x5f, 0xa3 }, .{ 0x2f, 0x7f, 0x86 }, .{ 0xd2, 0xd8, 0xdb },
        .{ 0x86, 0x91, 0x9a }, .{ 0x8f, 0x2e, 0x26 }, .{ 0x2f, 0x6b, 0x45 }, .{ 0xb0, 0x7a, 0x14 },
        .{ 0x3a, 0x58, 0x6e }, .{ 0x53, 0x4a, 0x82 }, .{ 0x25, 0x64, 0x6a }, .{ 0xee, 0xf1, 0xf2 },
    },
};

/// Resolve a base theme by name. An unknown name falls back to `mineral_dark`.
pub fn byName(name: []const u8) Theme {
    if (std.mem.eql(u8, name, "mineral-light")) return mineral_light;
    if (std.mem.eql(u8, name, "mineral-dark")) return mineral_dark;
    std.debug.print("caldera-console: unknown theme \"{s}\", using mineral-dark\n", .{name});
    return mineral_dark;
}

const testing = std.testing;

test "byName resolves built-in themes" {
    try testing.expectEqual(mineral_dark.background, byName("mineral-dark").background);
    try testing.expectEqual(mineral_light.background, byName("mineral-light").background);
}

test "byName falls back to dark for an unknown name" {
    try testing.expectEqual(mineral_dark.background, byName("nope").background);
}

test "palette256 covers the three ranges" {
    try testing.expectEqual([3]u8{ 0x2f, 0x7f, 0x86 }, mineral_dark.palette256(6));
    try testing.expectEqual([3]u8{ 0, 0, 0 }, mineral_dark.palette256(16));
    try testing.expectEqual([3]u8{ 255, 255, 255 }, mineral_dark.palette256(231));
    try testing.expectEqual([3]u8{ 8, 8, 8 }, mineral_dark.palette256(232));
    try testing.expectEqual([3]u8{ 238, 238, 238 }, mineral_dark.palette256(255));
}
