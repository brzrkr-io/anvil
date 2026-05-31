const theme = @import("render/theme.zig");
pub const Rgb = theme.Rgb;

// Mineral Warm palette tokens (BRAND.md exact hex, updated 2026-05-30).
pub const graphite = Rgb{ .r = 0x0e, .g = 0x0b, .b = 0x0a };
pub const charcoal = Rgb{ .r = 0x1c, .g = 0x16, .b = 0x14 };
pub const ash = Rgb{ .r = 0x3e, .g = 0x30, .b = 0x28 };
pub const alloy = Rgb{ .r = 0x8a, .g = 0x80, .b = 0x76 };
pub const mist = Rgb{ .r = 0xd8, .g = 0xcf, .b = 0xc8 };
pub const bone = Rgb{ .r = 0xf0, .g = 0xeb, .b = 0xe4 };
pub const mineral = Rgb{ .r = 0xc2, .g = 0x61, .b = 0x4a };
pub const ember = Rgb{ .r = 0xd4, .g = 0x60, .b = 0x1e };
pub const verified = Rgb{ .r = 0x5a, .g = 0x8c, .b = 0x45 };
pub const attention = Rgb{ .r = 0xb8, .g = 0x82, .b = 0x1a };
pub const agent = Rgb{ .r = 0x8c, .g = 0x5f, .b = 0xa0 };

// Snug-recess panel border: a touch lighter than charcoal, darker than ash.
pub const ash_soft = Rgb{ .r = 0x26, .g = 0x1e, .b = 0x1a };

// Structural frame line.
pub const line = Rgb{ .r = 0x3d, .g = 0x2f, .b = 0x27 };

// Row hover tint.
pub const hover = Rgb{ .r = 0x22, .g = 0x19, .b = 0x14 };

// Chrome surface palette — the window-furniture tones, selected by the active
// light/dark mode (see app.zig effectiveDark). Dark keeps the deep
// graphite/charcoal operator console; light maps each ROLE onto the BRAND.md
// light tokens: bone canvas, white raised panels, ink/slate text, mist borders.
// Semantic accents (mineral, ember, verified, attention, agent) communicate
// state, not surface, so they are mode-independent and stay as the consts above.
//
// Field names match the dark tokens for continuity. In `surface_light` each
// field carries the light-mode value for that role, not the literal hue of the
// same name (e.g. `graphite` = bone on light).
pub const Surface = struct {
    graphite: Rgb, // primary canvas: window + activity-rail fill
    charcoal: Rgb, // raised panel: sidebar, drawer, status bar, header strip
    ash: Rgb, // dim glyphs: separators, file icons, "none" placeholders
    ash_soft: Rgb, // recessed selected-row fill
    alloy: Rgb, // muted label text: section headers, metadata, inactive tabs
    mist: Rgb, // primary chrome text
    bone: Rgb, // emphasis chrome text
    line: Rgb, // structural frame edges, rules, dividers
    hover: Rgb, // pointer hover tint
};

pub const surface_dark = Surface{
    .graphite = graphite,
    .charcoal = charcoal,
    .ash = ash,
    .ash_soft = ash_soft,
    .alloy = alloy,
    .mist = mist,
    .bone = bone,
    .line = line,
    .hover = hover,
};

// Light mode: Mineral Warm light surfaces.
pub const surface_light = Surface{
    .graphite = Rgb{ .r = 0xf2, .g = 0xec, .b = 0xe4 }, // #f2ece4 primary light canvas
    .charcoal = Rgb{ .r = 0xfd, .g = 0xf6, .b = 0xee }, // #fdf6ee raised panels
    .ash = Rgb{ .r = 0xa0, .g = 0x90, .b = 0x80 }, // dim muted warm gray
    .ash_soft = Rgb{ .r = 0xe8, .g = 0xdd, .b = 0xd2 }, // recessed row inset
    .alloy = Rgb{ .r = 0x6b, .g = 0x5f, .b = 0x54 }, // muted text, legible on light
    .mist = Rgb{ .r = 0x30, .g = 0x25, .b = 0x20 }, // primary text
    .bone = Rgb{ .r = 0x14, .g = 0x0e, .b = 0x0a }, // ink emphasis text
    .line = Rgb{ .r = 0xcb, .g = 0xbf, .b = 0xb4 }, // soft hairline
    .hover = Rgb{ .r = 0xed, .g = 0xe3, .b = 0xd8 }, // hover tint
};

// Spacing tokens (px).
pub const sp4: f32 = 4;
pub const sp8: f32 = 8;
pub const sp12: f32 = 12;
pub const sp16: f32 = 16;
pub const sp24: f32 = 24;
pub const sp32: f32 = 32;

// Heights (device pixels at 2x, i.e. logical pt * 2).
pub const top_bar_h: f32 = 44;
pub const status_bar_h: f32 = 30;
// SNUG recess: a tight 8px gutter frames the terminal panel — compact, not floating.
pub const panel_pad: f32 = 8; // inset gutter: left/right/top of body
pub const panel_pad_bottom: f32 = 8; // inset gutter: bottom (before status bar)
pub const header_strip_h: f32 = 34; // slim charcoal strip atop the panel (fits one Mono line)

// Option A chrome (left side): activity rail + collapsible sidebar (device px).
pub const rail_w: f32 = 56; // vertical activity rail
pub const sidebar_w: f32 = 300; // SESSIONS / EXPLORER sidebar
pub const sidebar_header_h: f32 = 26; // section header row height
pub const row_h: f32 = 28; // list row height (sessions, explorer entries)

// Option C chrome (right side): context drawer — RUNS / TRACE / AGENT.
pub const drawer_w: f32 = 312;

/// Geometry regions for one window frame (device pixels).
/// Left rail, sidebar, and right drawer are zero-width for now (Option B).
pub const LayoutRegions = struct {
    win_w: f32,
    win_h: f32,

    // Derived regions.
    top_bar: Rect,
    body: Rect, // the workspace area (between bars)
    status_bar: Rect,

    // Reserved for later phases (zero for now).
    left_rail_w: f32 = 0,
    sidebar_w: f32 = 0,
    right_drawer_w: f32 = 0,

    pub fn compute(win_w: f32, win_h: f32) LayoutRegions {
        return .{
            .win_w = win_w,
            .win_h = win_h,
            .top_bar = .{ .x = 0, .y = 0, .w = win_w, .h = top_bar_h },
            .body = .{
                .x = 0,
                .y = top_bar_h,
                .w = win_w,
                .h = win_h - top_bar_h - status_bar_h,
            },
            .status_bar = .{
                .x = 0,
                .y = win_h - status_bar_h,
                .w = win_w,
                .h = status_bar_h,
            },
        };
    }
};

pub const Rect = struct {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
};

test "LayoutRegions.compute geometry" {
    const std = @import("std");
    const r = LayoutRegions.compute(1480, 920);
    try std.testing.expectEqual(@as(f32, 0), r.top_bar.y);
    try std.testing.expectEqual(top_bar_h, r.top_bar.h);
    try std.testing.expectEqual(top_bar_h, r.body.y);
    try std.testing.expectEqual(920 - top_bar_h - status_bar_h, r.body.h);
    try std.testing.expectEqual(920 - status_bar_h, r.status_bar.y);
    try std.testing.expectEqual(status_bar_h, r.status_bar.h);
}
