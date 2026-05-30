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
pub const sidebar_header_h: f32 = 30; // section header row height
pub const row_h: f32 = 34; // list row height (sessions, explorer entries)

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
