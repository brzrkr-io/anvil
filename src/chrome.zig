const theme = @import("render/theme.zig");
pub const Rgb = theme.Rgb;

// Mineral palette tokens (BRAND.md exact hex).
pub const graphite = Rgb{ .r = 0x0b, .g = 0x0d, .b = 0x0e };
pub const charcoal = Rgb{ .r = 0x16, .g = 0x1a, .b = 0x1c };
pub const ash = Rgb{ .r = 0x37, .g = 0x40, .b = 0x46 };
pub const alloy = Rgb{ .r = 0x86, .g = 0x91, .b = 0x9a };
pub const mist = Rgb{ .r = 0xd2, .g = 0xd8, .b = 0xdb };
pub const bone = Rgb{ .r = 0xee, .g = 0xf1, .b = 0xf2 };
pub const mineral = Rgb{ .r = 0x2f, .g = 0x7f, .b = 0x86 };
pub const ember = Rgb{ .r = 0xc5, .g = 0x46, .b = 0x2a };
pub const verified = Rgb{ .r = 0x3f, .g = 0x8a, .b = 0x5b };
pub const attention = Rgb{ .r = 0xb0, .g = 0x7a, .b = 0x14 };
pub const agent = Rgb{ .r = 0x6a, .g = 0x5f, .b = 0xa3 };

// Snug-recess panel border: a touch lighter than charcoal, darker than ash.
pub const ash_soft = Rgb{ .r = 0x20, .g = 0x28, .b = 0x2b };

// Structural frame line: crisp enough to read as a boxed operator-console
// panel edge against charcoal/graphite — the Hermes/Honcho "every module is a
// box" look — without the harshness of full ash. Used for region frames,
// section-header rules, and panel edges.
pub const line = Rgb{ .r = 0x3a, .g = 0x45, .b = 0x4d };

// Row hover tint: between charcoal and ash_soft — a subtle pointer affordance.
pub const hover = Rgb{ .r = 0x1b, .g = 0x21, .b = 0x24 };

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

// Light mode: BRAND.md light surfaces. bone canvas, white raised panels, ink
// emphasis + slate primary text, alloy-darkened muted text, soft mist borders.
pub const surface_light = Surface{
    .graphite = bone, // #eef1f2 primary light canvas
    .charcoal = Rgb{ .r = 0xff, .g = 0xff, .b = 0xff }, // white raised panels
    .ash = Rgb{ .r = 0xa8, .g = 0xb0, .b = 0xb6 }, // dim muted gray
    .ash_soft = Rgb{ .r = 0xd8, .g = 0xde, .b = 0xe1 }, // recessed row inset
    .alloy = Rgb{ .r = 0x5b, .g = 0x65, .b = 0x6c }, // muted text, legible on light
    .mist = Rgb{ .r = 0x2b, .g = 0x33, .b = 0x38 }, // primary text (slate)
    .bone = Rgb{ .r = 0x0c, .g = 0x0d, .b = 0x0e }, // ink emphasis text
    .line = Rgb{ .r = 0xbc, .g = 0xc4, .b = 0xc8 }, // soft hairline on bone/white/mist
    .hover = Rgb{ .r = 0xdf, .g = 0xe4, .b = 0xe7 }, // hover tint
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
