//! Terminal color themes. A `Theme` is plain data; `resolve` produces an
//! active theme from a base name plus optional per-color overrides.

const std = @import("std");
const config = @import("config.zig");

pub const Theme = struct {
    background: [3]u8,
    foreground: [3]u8,
    accent: [3]u8, // cursor color
    surface: [3]u8, // raised panel/card surfaces (HUD, tree, cheatsheet, active tab)
    border: [3]u8, // panel edges / separators
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

// Mineral Dark — a soft, slightly-pastel palette on a calm (not pure-black)
// canvas. Brand hue families are kept: mineral teal, semantic red/green/amber,
// agent violet, steel blue. Normal ANSI colors are lifted to luminous pastels
// so they read on the dark canvas; `bright-black` is a genuinely visible dim
// grey (the old #374046 was ~1.4:1 — invisible).
pub const mineral_dark: Theme = .{
    .background = .{ 0x18, 0x1a, 0x21 },
    .foreground = .{ 0xd8, 0xdb, 0xe2 },
    .accent = .{ 0x54, 0xb7, 0xc0 }, // luminous mineral
    .surface = .{ 0x22, 0x26, 0x2f }, // #22262f — clear lift above canvas
    .border = .{ 0x36, 0x3c, 0x49 }, // #363c49 — panel edges
    .ansi = .{
        .{ 0x22, 0x25, 0x30 }, .{ 0xe0, 0x8b, 0x82 }, .{ 0x8e, 0xc9, 0x9b }, .{ 0xe2, 0xc0, 0x89 },
        .{ 0x8b, 0xb0, 0xd4 }, .{ 0xbb, 0xa6, 0xdd }, .{ 0x7e, 0xca, 0xce }, .{ 0xc3, 0xc8, 0xd2 },
        .{ 0x6d, 0x74, 0x88 }, .{ 0xee, 0x9f, 0x96 }, .{ 0xa6, 0xd8, 0xb1 }, .{ 0xef, 0xce, 0x9a },
        .{ 0xa3, 0xc4, 0xe4 }, .{ 0xcb, 0xb8, 0xe9 }, .{ 0x95, 0xd9, 0xde }, .{ 0xee, 0xf1, 0xf6 },
    },
};

// Mineral Light — a refined reader-mode palette on the brand bone canvas.
// ANSI colors are mid-deep and gently muted for ~4.5:1+ contrast on bone;
// `bright-black` is a readable dim grey (the old #86919a was ~2.4:1 — too pale).
pub const mineral_light: Theme = .{
    .background = .{ 0xee, 0xf1, 0xf2 }, // bone
    .foreground = .{ 0x1b, 0x1f, 0x24 }, // ink
    .accent = .{ 0x2c, 0x7a, 0x82 }, // mineral
    .surface = .{ 0xff, 0xff, 0xff }, // #ffffff — raised light panels (BRAND.md: "white only")
    .border = .{ 0xd4, 0xd9, 0xdc }, // #d4d9dc — panel edges on bone
    .ansi = .{
        .{ 0x1b, 0x1f, 0x24 }, .{ 0xb5, 0x44, 0x3a }, .{ 0x32, 0x79, 0x52 }, .{ 0x94, 0x64, 0x10 },
        .{ 0x3f, 0x6c, 0x95 }, .{ 0x62, 0x55, 0x8f }, .{ 0x2c, 0x7a, 0x82 }, .{ 0x7a, 0x82, 0x8b },
        .{ 0x5d, 0x66, 0x71 }, .{ 0xc4, 0x4a, 0x3c }, .{ 0x35, 0x78, 0x50 }, .{ 0x86, 0x59, 0x0e },
        .{ 0x37, 0x60, 0x8a }, .{ 0x56, 0x4a, 0x83 }, .{ 0x25, 0x6a, 0x70 }, .{ 0xf6, 0xf8, 0xf9 },
    },
};

/// Resolve a base theme by name. An unknown name falls back to `mineral_dark`.
pub fn byName(name: []const u8) Theme {
    if (std.mem.eql(u8, name, "mineral-light")) return mineral_light;
    if (std.mem.eql(u8, name, "mineral-dark")) return mineral_dark;
    std.debug.print("anvil: unknown theme \"{s}\", using mineral-dark\n", .{name});
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
        std.debug.print("anvil: invalid theme color \"{s}\", ignored\n", .{hex});
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
    try testing.expectEqual([3]u8{ 0x7e, 0xca, 0xce }, mineral_dark.palette256(6));
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
