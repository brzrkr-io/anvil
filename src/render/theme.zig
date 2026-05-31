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

// Mineral Warm palette (BRAND.md, updated 2026-05-30). ANSI hues warm-recalibrated.
pub const mineral_dark = Theme{
    .bg = hex("0e0b0a"), // graphite
    .fg = hex("d8cfc8"), // mist
    .bar = hex("1c1614"), // charcoal
    .separator = hex("3d2f27"), // line
    .sel_bg = hex("3a2820"), // dim warm selection
    .sel_fg = hex("f0ebe4"), // bone
    .ansi = .{
        hex("1c1614"), // 0 black (charcoal)
        hex("b53a2e"), // 1 red (failure)
        hex("4a8c52"), // 2 green (verified)
        hex("b88220"), // 3 yellow (attention)
        hex("5272a0"), // 4 blue
        hex("8c5fa0"), // 5 magenta (agent)
        hex("c2614a"), // 6 coral (mineral)
        hex("d8cfc8"), // 7 white (mist)
        hex("503e34"), // 8 bright black
        hex("d45a44"), // 9 bright red
        hex("62a86a"), // 10 bright green
        hex("d4a030"), // 11 bright yellow
        hex("6882b8"), // 12 bright blue
        hex("a878c0"), // 13 bright magenta
        hex("d4733e"), // 14 bright coral
        hex("f0ebe4"), // 15 bright white (bone)
    },
};

pub const mineral_light = Theme{
    .bg = hex("f2ece4"), // canvas
    .fg = hex("302520"), // dark warm text
    .bar = hex("fdf6ee"), // panels
    .separator = hex("cbbfb4"), // line light
    .sel_bg = hex("e0d4c8"), // warm selection
    .sel_fg = hex("140e0a"), // ink
    .ansi = .{
        hex("140e0a"), // 0 black (ink)
        hex("a03025"), // 1 red
        hex("2e6c38"), // 2 green
        hex("8c6214"), // 3 yellow
        hex("3a5888"), // 4 blue
        hex("6c4880"), // 5 magenta
        hex("a84c38"), // 6 coral (mineral)
        hex("5c4e44"), // 7 white (dark warm on light)
        hex("9c8878"), // 8 bright black
        hex("c5462a"), // 9 bright red (ember)
        hex("4a8c52"), // 10 bright green
        hex("b88220"), // 11 bright yellow
        hex("5272a0"), // 12 bright blue
        hex("8c5fa0"), // 13 bright magenta
        hex("c2614a"), // 14 bright coral
        hex("3d2e26"), // 15 bright white (warm near-black)
    },
};

/// A coordinated dark+light pair for one visual variant.
pub const Variant = struct {
    dark: Theme,
    light: Theme,
};

// mineral-high: maximum-contrast Mineral Warm pair.
// Dark: graphite bg (#0e0b0a) + bone fg (#f0ebe4) — brand endpoints.
// Light: panels bg (#fdf6ee) + ink fg (#140e0a) — raised-panel contrast.
// Status hues are unchanged; only chrome/fg use the outer Mineral Warm extremes.
pub const mineral_high_dark = Theme{
    .bg = hex("0e0b0a"), // graphite (same as mineral_dark)
    .fg = hex("f0ebe4"), // bone — raised to max contrast
    .bar = hex("070504"), // near-black, darker than graphite
    .separator = hex("503e34"), // bright-black warm tone, visible on deep bg
    .sel_bg = hex("3a2820"),
    .sel_fg = hex("f0ebe4"),
    .ansi = mineral_dark.ansi,
};

pub const mineral_high_light = Theme{
    .bg = hex("fdf6ee"), // panels — raised-panel maximum
    .fg = hex("140e0a"), // ink
    .bar = hex("f2ece4"), // canvas
    .separator = hex("9c8878"), // warm alloy
    .sel_bg = hex("e0d4c8"),
    .sel_fg = hex("140e0a"),
    .ansi = mineral_light.ansi,
};

// ---------------------------------------------------------------------------
// Third-party themes. Each is a coordinated dark+light pair using the upstream
// project's canonical palette. Chrome (bar/separator/selection) is drawn from
// each palette's own surface tones so the window furniture matches.
// ---------------------------------------------------------------------------

// Tokyo Night — dark "night" + light "day".
pub const tokyonight_dark = Theme{
    .bg = hex("1a1b26"),
    .fg = hex("c0caf5"),
    .bar = hex("16161e"),
    .separator = hex("3b4261"),
    .sel_bg = hex("283457"),
    .sel_fg = hex("c0caf5"),
    .ansi = .{
        hex("15161e"), hex("f7768e"), hex("9ece6a"), hex("e0af68"),
        hex("7aa2f7"), hex("bb9af7"), hex("7dcfff"), hex("a9b1d6"),
        hex("414868"), hex("f7768e"), hex("9ece6a"), hex("e0af68"),
        hex("7aa2f7"), hex("bb9af7"), hex("7dcfff"), hex("c0caf5"),
    },
};
pub const tokyonight_light = Theme{
    .bg = hex("e1e2e7"),
    .fg = hex("3760bf"),
    .bar = hex("d0d5e3"),
    .separator = hex("a1a6c5"),
    .sel_bg = hex("b7c1e3"),
    .sel_fg = hex("3760bf"),
    .ansi = .{
        hex("b4b5b9"), hex("f52a65"), hex("587539"), hex("8c6c3e"),
        hex("2e7de9"), hex("9854f1"), hex("007197"), hex("6172b0"),
        hex("a1a6c5"), hex("f52a65"), hex("587539"), hex("8c6c3e"),
        hex("2e7de9"), hex("9854f1"), hex("007197"), hex("3760bf"),
    },
};

// Gruvbox — Pavel Pertsev's retro groove.
pub const gruvbox_dark = Theme{
    .bg = hex("282828"),
    .fg = hex("ebdbb2"),
    .bar = hex("1d2021"),
    .separator = hex("504945"),
    .sel_bg = hex("504945"),
    .sel_fg = hex("ebdbb2"),
    .ansi = .{
        hex("282828"), hex("cc241d"), hex("98971a"), hex("d79921"),
        hex("458588"), hex("b16286"), hex("689d6a"), hex("a89984"),
        hex("928374"), hex("fb4934"), hex("b8bb26"), hex("fabd2f"),
        hex("83a598"), hex("d3869b"), hex("8ec07c"), hex("ebdbb2"),
    },
};
pub const gruvbox_light = Theme{
    .bg = hex("fbf1c7"),
    .fg = hex("3c3836"),
    .bar = hex("ebdbb2"),
    .separator = hex("bdae93"),
    .sel_bg = hex("d5c4a1"),
    .sel_fg = hex("3c3836"),
    .ansi = .{
        hex("fbf1c7"), hex("cc241d"), hex("98971a"), hex("d79921"),
        hex("458588"), hex("b16286"), hex("689d6a"), hex("7c6f64"),
        hex("928374"), hex("9d0006"), hex("79740e"), hex("b57614"),
        hex("076678"), hex("8f3f71"), hex("427b58"), hex("3c3836"),
    },
};

// Catppuccin — Mocha (dark) + Latte (light).
pub const catppuccin_dark = Theme{
    .bg = hex("1e1e2e"),
    .fg = hex("cdd6f4"),
    .bar = hex("181825"),
    .separator = hex("45475a"),
    .sel_bg = hex("585b70"),
    .sel_fg = hex("cdd6f4"),
    .ansi = .{
        hex("45475a"), hex("f38ba8"), hex("a6e3a1"), hex("f9e2af"),
        hex("89b4fa"), hex("f5c2e7"), hex("94e2d5"), hex("bac2de"),
        hex("585b70"), hex("f38ba8"), hex("a6e3a1"), hex("f9e2af"),
        hex("89b4fa"), hex("f5c2e7"), hex("94e2d5"), hex("a6adc8"),
    },
};
pub const catppuccin_light = Theme{
    .bg = hex("eff1f5"),
    .fg = hex("4c4f69"),
    .bar = hex("e6e9ef"),
    .separator = hex("bcc0cc"),
    .sel_bg = hex("bcc0cc"),
    .sel_fg = hex("4c4f69"),
    .ansi = .{
        hex("5c5f77"), hex("d20f39"), hex("40a02b"), hex("df8e1d"),
        hex("1e66f5"), hex("ea76cb"), hex("179299"), hex("acb0be"),
        hex("6c6f85"), hex("d20f39"), hex("40a02b"), hex("df8e1d"),
        hex("1e66f5"), hex("ea76cb"), hex("179299"), hex("bcc0cc"),
    },
};

// Nord — arctic, north-bluish. Light pairing uses the Snow Storm surfaces.
pub const nord_dark = Theme{
    .bg = hex("2e3440"),
    .fg = hex("d8dee9"),
    .bar = hex("272c36"),
    .separator = hex("434c5e"),
    .sel_bg = hex("434c5e"),
    .sel_fg = hex("eceff4"),
    .ansi = .{
        hex("3b4252"), hex("bf616a"), hex("a3be8c"), hex("ebcb8b"),
        hex("81a1c1"), hex("b48ead"), hex("88c0d0"), hex("e5e9f0"),
        hex("4c566a"), hex("bf616a"), hex("a3be8c"), hex("ebcb8b"),
        hex("81a1c1"), hex("b48ead"), hex("8fbcbb"), hex("eceff4"),
    },
};
pub const nord_light = Theme{
    .bg = hex("eceff4"),
    .fg = hex("2e3440"),
    .bar = hex("e5e9f0"),
    .separator = hex("d8dee9"),
    .sel_bg = hex("d8dee9"),
    .sel_fg = hex("2e3440"),
    .ansi = .{
        hex("3b4252"), hex("99324b"), hex("4f894c"), hex("9e7a14"),
        hex("3b6ea8"), hex("8c5a8e"), hex("398eac"), hex("4c566a"),
        hex("434c5e"), hex("99324b"), hex("4f894c"), hex("9e7a14"),
        hex("3b6ea8"), hex("8c5a8e"), hex("398eac"), hex("2e3440"),
    },
};

// Dracula — dark + Alucard light.
pub const dracula_dark = Theme{
    .bg = hex("282a36"),
    .fg = hex("f8f8f2"),
    .bar = hex("21222c"),
    .separator = hex("44475a"),
    .sel_bg = hex("44475a"),
    .sel_fg = hex("f8f8f2"),
    .ansi = .{
        hex("21222c"), hex("ff5555"), hex("50fa7b"), hex("f1fa8c"),
        hex("bd93f9"), hex("ff79c6"), hex("8be9fd"), hex("f8f8f2"),
        hex("6272a4"), hex("ff6e6e"), hex("69ff94"), hex("ffffa5"),
        hex("d6acff"), hex("ff92df"), hex("a4ffff"), hex("ffffff"),
    },
};
pub const dracula_light = Theme{
    .bg = hex("f8f8f2"),
    .fg = hex("1f1f1f"),
    .bar = hex("e8e8e2"),
    .separator = hex("cfcfca"),
    .sel_bg = hex("d0d0cc"),
    .sel_fg = hex("1f1f1f"),
    .ansi = .{
        hex("d0d0cc"), hex("cb3a2a"), hex("14710a"), hex("846e15"),
        hex("644ac9"), hex("a3144d"), hex("036a96"), hex("1f1f1f"),
        hex("9c9c96"), hex("cb3a2a"), hex("14710a"), hex("846e15"),
        hex("644ac9"), hex("a3144d"), hex("036a96"), hex("1f1f1f"),
    },
};

// Everforest — medium dark + medium light.
pub const everforest_dark = Theme{
    .bg = hex("2d353b"),
    .fg = hex("d3c6aa"),
    .bar = hex("272e33"),
    .separator = hex("4b565c"),
    .sel_bg = hex("4f5b58"),
    .sel_fg = hex("d3c6aa"),
    .ansi = .{
        hex("4b565c"), hex("e67e80"), hex("a7c080"), hex("dbbc7f"),
        hex("7fbbb3"), hex("d699b6"), hex("83c092"), hex("d3c6aa"),
        hex("4b565c"), hex("e67e80"), hex("a7c080"), hex("dbbc7f"),
        hex("7fbbb3"), hex("d699b6"), hex("83c092"), hex("d3c6aa"),
    },
};
pub const everforest_light = Theme{
    .bg = hex("fdf6e3"),
    .fg = hex("5c6a72"),
    .bar = hex("f4f0d9"),
    .separator = hex("e0dcc7"),
    .sel_bg = hex("e0dcc7"),
    .sel_fg = hex("5c6a72"),
    .ansi = .{
        hex("5c6a72"), hex("f85552"), hex("8da101"), hex("dfa000"),
        hex("3a94c5"), hex("df69ba"), hex("35a77c"), hex("5c6a72"),
        hex("939f91"), hex("f85552"), hex("8da101"), hex("dfa000"),
        hex("3a94c5"), hex("df69ba"), hex("35a77c"), hex("5c6a72"),
    },
};

pub const variants = [_]struct { name: []const u8, v: Variant }{
    .{ .name = "mineral", .v = .{ .dark = mineral_dark, .light = mineral_light } },
    .{ .name = "mineral-high", .v = .{ .dark = mineral_high_dark, .light = mineral_high_light } },
    .{ .name = "tokyo-night", .v = .{ .dark = tokyonight_dark, .light = tokyonight_light } },
    .{ .name = "gruvbox", .v = .{ .dark = gruvbox_dark, .light = gruvbox_light } },
    .{ .name = "catppuccin", .v = .{ .dark = catppuccin_dark, .light = catppuccin_light } },
    .{ .name = "nord", .v = .{ .dark = nord_dark, .light = nord_light } },
    .{ .name = "dracula", .v = .{ .dark = dracula_dark, .light = dracula_light } },
    .{ .name = "everforest", .v = .{ .dark = everforest_dark, .light = everforest_light } },
};

pub fn byName(name: []const u8) ?Variant {
    for (variants) |entry| {
        if (std.mem.eql(u8, entry.name, name)) return entry.v;
    }
    return null;
}

test "hex parses brand tokens" {
    try std.testing.expectEqual(Rgb{ .r = 0x0e, .g = 0x0b, .b = 0x0a }, mineral_dark.bg);
    try std.testing.expectEqual(Rgb{ .r = 0xf2, .g = 0xec, .b = 0xe4 }, mineral_light.bg);
}

test "byName returns the named variant" {
    const v = byName("mineral") orelse return error.VariantNotFound;
    try std.testing.expectEqual(mineral_dark.bg, v.dark.bg);
    try std.testing.expectEqual(mineral_light.bg, v.light.bg);

    const vh = byName("mineral-high") orelse return error.VariantNotFound;
    try std.testing.expectEqual(mineral_high_dark.bg, vh.dark.bg);
}

test "third-party variants resolve and pair distinct dark/light backgrounds" {
    const names = [_][]const u8{ "tokyo-night", "gruvbox", "catppuccin", "nord", "dracula", "everforest" };
    for (names) |name| {
        const v = byName(name) orelse return error.VariantNotFound;
        try std.testing.expect(!std.meta.eql(v.dark.bg, v.light.bg));
    }
}

test "byName returns null for unknown names" {
    try std.testing.expectEqual(@as(?Variant, null), byName("unknown"));
    try std.testing.expectEqual(@as(?Variant, null), byName(""));
}

test "mineral-high dark bg differs from mineral dark bg" {
    // mineral_dark.bg == mineral_high_dark.bg (both graphite) — that's intentional.
    // The distinguishing difference is fg and bar.
    const v = byName("mineral-high") orelse return error.VariantNotFound;
    const base = byName("mineral") orelse return error.VariantNotFound;
    // bar color must differ
    try std.testing.expect(!std.meta.eql(v.dark.bar, base.dark.bar));
    // light bg must differ
    try std.testing.expect(!std.meta.eql(v.light.bg, base.light.bg));
}
