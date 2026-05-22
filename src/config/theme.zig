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
    .background = .{ 0x1a, 0x1c, 0x24 },
    .foreground = .{ 0xdf, 0xe1, 0xe8 },
    .accent = .{ 0x46, 0xa5, 0xad },
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
    .accent = .{ 0x2f, 0x7f, 0x86 }, // mineral
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

/// Parse a `#rrggbb` (or bare `rrggbb`) string into RGB bytes.
pub fn hexToRgb(hex: []const u8) error{InvalidHex}![3]u8 {
    var s = hex;
    if (s.len > 0 and s[0] == '#') s = s[1..];
    if (s.len != 6) return error.InvalidHex;
    return .{
        std.fmt.parseInt(u8, s[0..2], 16) catch return error.InvalidHex,
        std.fmt.parseInt(u8, s[2..4], 16) catch return error.InvalidHex,
        std.fmt.parseInt(u8, s[4..6], 16) catch return error.InvalidHex,
    };
}

/// Apply one optional override onto `slot`. A bad hex string is logged and
/// leaves `slot` unchanged.
fn applyOverride(slot: *[3]u8, maybe_hex: ?[]const u8) void {
    const hex = maybe_hex orelse return;
    slot.* = hexToRgb(hex) catch {
        std.debug.print("caldera-console: invalid theme color \"{s}\", ignored\n", .{hex});
        return;
    };
}

/// Build the active theme: base theme `name` with `ov` applied on top.
pub fn resolve(name: []const u8, ov: config.Overrides) Theme {
    var t = byName(name);
    applyOverride(&t.background, ov.background);
    applyOverride(&t.foreground, ov.foreground);
    applyOverride(&t.accent, ov.accent);
    applyOverride(&t.ansi[0], ov.ansi.black);
    applyOverride(&t.ansi[1], ov.ansi.red);
    applyOverride(&t.ansi[2], ov.ansi.green);
    applyOverride(&t.ansi[3], ov.ansi.yellow);
    applyOverride(&t.ansi[4], ov.ansi.blue);
    applyOverride(&t.ansi[5], ov.ansi.magenta);
    applyOverride(&t.ansi[6], ov.ansi.cyan);
    applyOverride(&t.ansi[7], ov.ansi.white);
    applyOverride(&t.ansi[8], ov.ansi.bright_black);
    applyOverride(&t.ansi[9], ov.ansi.bright_red);
    applyOverride(&t.ansi[10], ov.ansi.bright_green);
    applyOverride(&t.ansi[11], ov.ansi.bright_yellow);
    applyOverride(&t.ansi[12], ov.ansi.bright_blue);
    applyOverride(&t.ansi[13], ov.ansi.bright_magenta);
    applyOverride(&t.ansi[14], ov.ansi.bright_cyan);
    applyOverride(&t.ansi[15], ov.ansi.bright_white);
    return t;
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

test "resolve with no overrides equals the base theme" {
    const t = resolve("mineral-dark", .{});
    try testing.expectEqual(mineral_dark.background, t.background);
    try testing.expectEqual(mineral_dark.ansi[2], t.ansi[2]);
}

test "resolve applies a valid override" {
    const t = resolve("mineral-dark", .{
        .background = "#101316",
        .ansi = .{ .green = "#52b070" },
    });
    try testing.expectEqual([3]u8{ 0x10, 0x13, 0x16 }, t.background);
    try testing.expectEqual([3]u8{ 0x52, 0xb0, 0x70 }, t.ansi[2]);
    try testing.expectEqual(mineral_dark.foreground, t.foreground); // untouched
}

test "resolve keeps the base value for an invalid-hex override" {
    const t = resolve("mineral-dark", .{ .accent = "not-a-color" });
    try testing.expectEqual(mineral_dark.accent, t.accent);
}

test "hexToRgb parses and rejects" {
    try testing.expectEqual([3]u8{ 0x0b, 0x0d, 0x0e }, try hexToRgb("#0b0d0e"));
    try testing.expectEqual([3]u8{ 0x0b, 0x0d, 0x0e }, try hexToRgb("0b0d0e"));
    try testing.expectError(error.InvalidHex, hexToRgb("#fff"));
    try testing.expectError(error.InvalidHex, hexToRgb("#zzzzzz"));
}
