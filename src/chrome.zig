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

// Spacing tokens. Base values are logical points; applyScale multiplies the
// base table by s = ui_scale * backingScaleFactor to get device pixels.
// The pub vars are initialized to the 2x defaults so the unscaled state
// matches today exactly even before applyScale runs.
const base_sp4: f32 = 2;
const base_sp8: f32 = 4;
const base_sp12: f32 = 6;
const base_sp16: f32 = 8;
const base_sp24: f32 = 12;
const base_sp32: f32 = 16;
pub var sp4: f32 = 4;
pub var sp8: f32 = 8;
pub var sp12: f32 = 12;
pub var sp16: f32 = 16;
pub var sp24: f32 = 24;
pub var sp32: f32 = 32;

// Heights and widths (device pixels at the current scale).
const base_top_bar_h: f32 = 22;
const base_status_bar_h: f32 = 15;
// SNUG recess: a tight gutter frames the terminal panel — compact, not floating.
const base_panel_pad: f32 = 4; // inset gutter: left/right/top of body
const base_panel_pad_bottom: f32 = 4; // inset gutter: bottom (before status bar)
const base_header_strip_h: f32 = 17; // slim charcoal strip atop the panel (fits one Mono line)
pub var top_bar_h: f32 = 44;
pub var status_bar_h: f32 = 30;
pub var panel_pad: f32 = 8; // inset gutter: left/right/top of body
pub var panel_pad_bottom: f32 = 8; // inset gutter: bottom (before status bar)
pub var header_strip_h: f32 = 34; // slim charcoal strip atop the panel (fits one Mono line)

// Option A chrome (left side): activity rail + collapsible sidebar (device px).
const base_rail_w: f32 = 28;
const base_sidebar_w: f32 = 150;
const base_sidebar_header_h: f32 = 13;
const base_row_h: f32 = 14;
pub var rail_w: f32 = 56; // vertical activity rail
pub var sidebar_w: f32 = 300; // SESSIONS / EXPLORER sidebar
pub var sidebar_header_h: f32 = 26; // section header row height
pub var row_h: f32 = 28; // list row height (sessions, explorer entries)

// Option C chrome (right side): context drawer — RUNS / TRACE / AGENT.
const base_drawer_w: f32 = 156;
pub var drawer_w: f32 = 312;

/// Rescale the whole chrome size table by s = ui_scale * backingScaleFactor.
/// s=2 reproduces the original 2x defaults.
pub fn applyScale(s: f32) void {
    sp4 = base_sp4 * s;
    sp8 = base_sp8 * s;
    sp12 = base_sp12 * s;
    sp16 = base_sp16 * s;
    sp24 = base_sp24 * s;
    sp32 = base_sp32 * s;
    top_bar_h = base_top_bar_h * s;
    status_bar_h = base_status_bar_h * s;
    panel_pad = base_panel_pad * s;
    panel_pad_bottom = base_panel_pad_bottom * s;
    header_strip_h = base_header_strip_h * s;
    rail_w = base_rail_w * s;
    sidebar_w = base_sidebar_w * s;
    sidebar_header_h = base_sidebar_header_h * s;
    row_h = base_row_h * s;
    drawer_w = base_drawer_w * s;
}

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

test "applyScale scales sizes from the 2x base; s=2 reproduces defaults" {
    const std = @import("std");
    applyScale(2.0);
    try std.testing.expectEqual(@as(f32, 44), top_bar_h);
    try std.testing.expectEqual(@as(f32, 30), status_bar_h);
    try std.testing.expectEqual(@as(f32, 34), header_strip_h);
    applyScale(1.0); // half size
    try std.testing.expectEqual(@as(f32, 22), top_bar_h);
    try std.testing.expectEqual(@as(f32, 17), header_strip_h);
    applyScale(2.0); // restore default for other tests
}
