//! Multi-pane render coordinator.
//!
//! `drawWorkspace` lays out a PaneTree onto an inner content rect, then calls
//! `drawViewport` once per leaf with that leaf's pixel origin set on the Raster.
//! The single-leaf case is behaviour-identical to the old single `drawViewport`
//! call in renderFrame: origin_x/origin_y encode the top-bar and panel offsets
//! that were previously handled via top_bar_rows and x_offset.
//!
//! Bleed guard: the smooth-scroll path draws row y=0..rows (inclusive — one
//! extra partially visible row). With vertical splits, any bleed into the
//! divider gutter is overdrawn by the divider fill, which is drawn LAST over
//! all panes. This is the "divider overdraw" approach: no clip added to
//! drawViewport (avoids adding branches to the hot path); correctness relies on
//! the divider being drawn after all pane content.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const Theme = @import("../config/theme.zig").Theme;
const draw_mod = @import("draw.zig");
const Search = @import("../terminal/search.zig").Search;
pub const layout_mod = @import("../workspace/layout.zig");
const PaneTree = layout_mod.PaneTree;
const PaneRegistry = @import("../workspace/pane.zig").PaneRegistry;
const PaneId = layout_mod.PaneId;
const cfg_mod = @import("../config/config.zig");

/// Divider gutter width in device pixels. Wide enough to overdraw scroll bleed.
pub const divider_px: f64 = 8.0;

/// Maximum number of leaves supported with zero heap allocation.
/// A FixedBufferAllocator backed by a stack array of this size is used for
/// the layout call, so no heap allocation occurs in steady state.
const max_layout_entries = 64;

/// Draw all panes in `tree` into `raster`, then draw divider hairlines over them.
///
/// Parameters:
///   raster       — full-window raster bitmap.
///   tree         — the current tab's pane tree (layout and focused id).
///   registry     — the pane registry for the current tab.
///   inner        — device-pixel content area (window minus top-bar and panels).
///                  y=0 is the top of the raster. Layout is done in this space.
///   div_px       — divider gutter width in device pixels (use `divider_px`).
///   font         — shared font for all panes.
///   theme        — shared theme for all panes.
///   search       — active search state, or null.
///   focused_id   — the pane that receives cursor rendering.
///   blink_phase  — cursor blink phase [0, 1).
///   cursor_cfg   — cursor style + blink preference from config.
///
/// After this function returns, raster.origin_x and raster.origin_y are both 0.
pub fn drawWorkspace(
    raster: *Raster,
    tree: *const PaneTree,
    registry: *const PaneRegistry,
    inner: layout_mod.Rect,
    div_px: f64,
    font: Font,
    theme: Theme,
    search: ?*const Search,
    focused_id: PaneId,
    blink_phase: f32,
    cursor_cfg: cfg_mod.Config.CursorCfg,
) void {
    // Layout into a stack-backed allocator — zero heap allocations.
    var entry_buf: [max_layout_entries]PaneTree.LayoutEntry = undefined;
    var fba = std.heap.FixedBufferAllocator.init(std.mem.sliceAsBytes(&entry_buf));
    const fba_alloc = fba.allocator();

    var entries = std.ArrayListUnmanaged(PaneTree.LayoutEntry).empty;
    tree.layout(inner, div_px, &entries, fba_alloc);

    // Draw each leaf.
    for (entries.items) |e| {
        const pane = registry.get(e.id) orelse continue;

        // Set the pane's pixel origin on the raster. origin_y is top-down
        // (raster space); cellRect converts to CG space internally.
        raster.origin_x = e.rect.x;
        raster.origin_y = e.rect.y;

        const cursor_params: ?draw_mod.CursorParams = if (e.id == focused_id)
            draw_mod.CursorParams{
                .ax = pane.cursor_ax,
                .ay = pane.cursor_ay,
                .blink_phase = blink_phase,
                .cfg = cursor_cfg,
            }
        else
            null;

        // rule_x bounds: horizontal span of this pane in device pixels.
        const rule_x_start = e.rect.x;
        const rule_x_end = e.rect.x + e.rect.w;

        draw_mod.drawViewport(
            raster,
            &pane.terminal,
            font,
            theme,
            pane.scroll_pos,
            pane.overscroll,
            pane.selection,
            search,
            0, // top_bar_rows: already encoded in origin_y
            cursor_params,
            rule_x_start,
            rule_x_end,
        );
    }

    // Reset origin before chrome draws in absolute space.
    raster.origin_x = 0;
    raster.origin_y = 0;

    // Draw divider hairlines over all pane content (bleed guard: see module doc).
    // A divider gutter sits between adjacent leaves in layout order.
    // We compute gutter positions by finding pairs of adjacent leaf rects.
    drawDividers(raster, entries.items, div_px, theme);
}

/// Fill divider gutters between all adjacent leaf pairs. Called after all pane
/// content is drawn so the dividers overdraw any scroll bleed.
fn drawDividers(
    raster: *Raster,
    entries: []const PaneTree.LayoutEntry,
    div_px: f64,
    theme: Theme,
) void {
    // For each pair of leaves, if they share a boundary (with a gutter between
    // them), fill the gutter rectangle.
    for (entries, 0..) |a, ai| {
        for (entries[ai + 1 ..]) |b| {
            // Horizontal split: b is to the right of a.
            // Gutter: x in [a.rect.x + a.rect.w, b.rect.x], full height of overlap.
            {
                const gap_x = a.rect.x + a.rect.w;
                const gap_end = b.rect.x;
                if (gap_end > gap_x and gap_end - gap_x <= div_px + 1.0) {
                    // Vertical overlap.
                    const oy = @max(a.rect.y, b.rect.y);
                    const oy_end = @min(a.rect.y + a.rect.h, b.rect.y + b.rect.h);
                    if (oy_end > oy) {
                        raster.fillPixelRect(gap_x, oy, gap_end - gap_x, oy_end - oy, theme.border);
                    }
                }
            }
            // Vertical split: b is below a.
            // Gutter: y in [a.rect.y + a.rect.h, b.rect.y], full width of overlap.
            {
                const gap_y = a.rect.y + a.rect.h;
                const gap_end = b.rect.y;
                if (gap_end > gap_y and gap_end - gap_y <= div_px + 1.0) {
                    // Horizontal overlap.
                    const ox = @max(a.rect.x, b.rect.x);
                    const ox_end = @min(a.rect.x + a.rect.w, b.rect.x + b.rect.w);
                    if (ox_end > ox) {
                        raster.fillPixelRect(ox, gap_y, ox_end - ox, gap_end - gap_y, theme.border);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test "drawWorkspace single-leaf: leaf rect equals inner rect" {
    // Verifies behaviour preservation: a single-leaf tree gives the same
    // geometry as the old single drawViewport call.
    const testing = std.testing;
    const scrollback_mod = @import("../terminal/scrollback.zig");
    const Raster_t = @import("raster.zig").Raster;
    const Font_t = @import("font.zig").Font;
    const theme_mod = @import("../config/theme.zig");
    const pane_mod = @import("../workspace/pane.zig");
    const Terminal = @import("../terminal/terminal.zig").Terminal;
    const Selection = @import("../app/selection.zig").Selection;

    const font = try Font_t.init("Menlo", 26.0);
    defer font.deinit();

    var raster = try Raster_t.init(testing.allocator, 400, 300);
    defer raster.deinit();

    // Single-leaf tree.
    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();

    // Build a minimal pane on the stack (no PTY, no reader thread).
    var terminal = try Terminal.init(testing.allocator, 20, 6, scrollback_mod.default_capacity);
    defer terminal.deinit();
    terminal.feed("hello world\r\n");
    terminal.feed("second line");

    var pane = pane_mod.Pane{
        .alloc = testing.allocator,
        .id = 1,
        .terminal = terminal,
        .pty = undefined,
    };

    var registry = pane_mod.PaneRegistry{};
    defer registry.map.deinit(testing.allocator);
    try registry.map.put(testing.allocator, 1, &pane);

    const pad: f64 = 24;
    const inner: layout_mod.Rect = .{
        .x = pad,
        .y = pad,
        .w = @as(f64, @floatFromInt(raster.width)) - 2 * pad,
        .h = @as(f64, @floatFromInt(raster.height)) - 2 * pad,
    };

    // Draw. Should not panic or produce wrong geometry.
    var entry_buf: [max_layout_entries]PaneTree.LayoutEntry = undefined;
    var fba = std.heap.FixedBufferAllocator.init(std.mem.sliceAsBytes(&entry_buf));
    var entries = std.ArrayListUnmanaged(PaneTree.LayoutEntry).empty;
    tree.layout(inner, divider_px, &entries, fba.allocator());

    try testing.expectEqual(@as(usize, 1), entries.items.len);
    // The single leaf rect must equal the inner rect.
    try testing.expectApproxEqAbs(inner.x, entries.items[0].rect.x, 1e-9);
    try testing.expectApproxEqAbs(inner.y, entries.items[0].rect.y, 1e-9);
    try testing.expectApproxEqAbs(inner.w, entries.items[0].rect.w, 1e-9);
    try testing.expectApproxEqAbs(inner.h, entries.items[0].rect.h, 1e-9);

    // Full drawWorkspace call must not panic.
    const theme = theme_mod.mineral_dark;
    const cursor_cfg: cfg_mod.Config.CursorCfg = .{};
    drawWorkspace(&raster, &tree, &registry, inner, divider_px, font, theme, null, 1, 0.0, cursor_cfg);

    // origin must be reset after the call.
    try testing.expectEqual(@as(f64, 0), raster.origin_x);
    try testing.expectEqual(@as(f64, 0), raster.origin_y);

    _ = Selection{};
}

test "drawWorkspace two-leaf: zero heap allocations per frame" {
    // Extends the draw.zig zero-alloc test to a 2-leaf tree through drawWorkspace.
    const testing = std.testing;
    const CountingAllocator = @import("../testing/counting_allocator.zig").CountingAllocator;
    const scrollback_mod = @import("../terminal/scrollback.zig");
    const Raster_t = @import("raster.zig").Raster;
    const Font_t = @import("font.zig").Font;
    const theme_mod = @import("../config/theme.zig");
    const pane_mod = @import("../workspace/pane.zig");
    const Terminal = @import("../terminal/terminal.zig").Terminal;

    var ca = CountingAllocator.init(testing.allocator);
    const alloc = ca.allocator();

    // Build two terminals with some content.
    var t1 = try Terminal.init(alloc, 20, 6, scrollback_mod.default_capacity);
    defer t1.deinit();
    t1.feed("hello world\r\n");
    t1.feed("second line");

    var t2 = try Terminal.init(alloc, 20, 6, scrollback_mod.default_capacity);
    defer t2.deinit();
    t2.feed("pane two\r\n");

    // Build two panes on the stack (no PTY, no reader thread).
    var pane1 = pane_mod.Pane{
        .alloc = alloc,
        .id = 1,
        .terminal = t1,
        .pty = undefined,
    };
    var pane2 = pane_mod.Pane{
        .alloc = alloc,
        .id = 2,
        .terminal = t2,
        .pty = undefined,
    };

    // Build a 2-leaf tree (vertical split).
    var tree = try PaneTree.initSingle(alloc, 1);
    defer tree.deinit();
    try tree.split(.vertical, 2);

    var registry = pane_mod.PaneRegistry{};
    defer registry.map.deinit(alloc);
    try registry.map.put(alloc, 1, &pane1);
    try registry.map.put(alloc, 2, &pane2);

    const font = try Font_t.init("Menlo", 26.0);
    defer font.deinit();

    var raster = try Raster_t.init(testing.allocator, 400, 300);
    defer raster.deinit();

    const theme = theme_mod.mineral_dark;
    const cursor_cfg: cfg_mod.Config.CursorCfg = .{};
    const pad: f64 = 24;
    const inner: layout_mod.Rect = .{
        .x = pad,
        .y = pad,
        .w = @as(f64, @floatFromInt(raster.width)) - 2 * pad,
        .h = @as(f64, @floatFromInt(raster.height)) - 2 * pad,
    };

    // Reset counters — everything from here is steady-state.
    ca.reset();

    drawWorkspace(&raster, &tree, &registry, inner, divider_px, font, theme, null, 1, 0.0, cursor_cfg);

    try testing.expectEqual(@as(usize, 0), ca.alloc_count);
    try testing.expectEqual(@as(usize, 0), ca.resize_count);
}

test "drawWorkspace two vertical panes: no cross-divider bleed" {
    // Lay out two vertically-stacked panes. Scroll the top pane mid-line.
    // Assert that the first row of the bottom pane's raster region is drawn
    // with the bottom pane's background (not the top pane's bled row).
    //
    // Bleed guard: divider overdraw. The divider (theme.border) is drawn after
    // all pane content so any partial row that bleeds into the gutter is covered.
    // This test verifies that divider pixels carry theme.border, not the top
    // pane's cell color.
    const testing = std.testing;
    const scrollback_mod = @import("../terminal/scrollback.zig");
    const Raster_t = @import("raster.zig").Raster;
    const Font_t = @import("font.zig").Font;
    const theme_mod = @import("../config/theme.zig");
    const pane_mod = @import("../workspace/pane.zig");
    const Terminal = @import("../terminal/terminal.zig").Terminal;

    const font = try Font_t.init("Menlo", 26.0);
    defer font.deinit();

    const ch = font.metrics.cell_h;
    const cw = font.metrics.cell_w;

    // Raster large enough for 8 rows and 20 cols.
    const w: usize = @intFromFloat(cw * 20 + 48);
    const h: usize = @intFromFloat(ch * 8 + 48);
    var raster = try Raster_t.init(testing.allocator, w, h);
    defer raster.deinit();

    const pad: f64 = 24;

    // Two terminals: top gets a bright cell, bottom gets background only.
    var t_top = try Terminal.init(testing.allocator, 20, 4, scrollback_mod.default_capacity);
    defer t_top.deinit();
    // Feed enough lines to enable mid-line scrollback.
    var li: usize = 0;
    while (li < 8) : (li += 1) {
        t_top.feed("\x1b[41m    \x1b[m\r\n"); // red background on 4 cells, then newline
    }

    var t_bot = try Terminal.init(testing.allocator, 20, 4, scrollback_mod.default_capacity);
    defer t_bot.deinit();
    // Bottom terminal is blank (background only).

    var pane_top = pane_mod.Pane{
        .alloc = testing.allocator,
        .id = 1,
        .terminal = t_top,
        .pty = undefined,
        .scroll_pos = 1.5, // mid-scroll → y_shift_px will be set
    };
    var pane_bot = pane_mod.Pane{
        .alloc = testing.allocator,
        .id = 2,
        .terminal = t_bot,
        .pty = undefined,
    };

    var tree = try PaneTree.initSingle(testing.allocator, 1);
    defer tree.deinit();
    try tree.split(.vertical, 2);

    var registry = pane_mod.PaneRegistry{};
    defer registry.map.deinit(testing.allocator);
    try registry.map.put(testing.allocator, 1, &pane_top);
    try registry.map.put(testing.allocator, 2, &pane_bot);

    const theme = theme_mod.mineral_dark;
    const cursor_cfg: cfg_mod.Config.CursorCfg = .{};

    const inner: layout_mod.Rect = .{
        .x = pad,
        .y = pad,
        .w = @as(f64, @floatFromInt(w)) - 2 * pad,
        .h = @as(f64, @floatFromInt(h)) - 2 * pad,
    };

    raster.clear(theme.background);
    drawWorkspace(&raster, &tree, &registry, inner, divider_px, font, theme, null, 1, 0.0, cursor_cfg);

    // Compute the divider gutter position in raster space.
    // layout() gives each child: available = inner.h - divider_px; each child
    // gets available/2. So:
    //   top pane h = (inner.h - divider_px) * 0.5
    //   gutter starts at: inner.y + (inner.h - divider_px) * 0.5
    //   gutter ends at:   inner.y + (inner.h - divider_px) * 0.5 + divider_px
    const top_h = (inner.h - divider_px) * 0.5;
    const gutter_y = inner.y + top_h;
    const gutter_mid_y: usize = @intFromFloat(gutter_y + divider_px / 2.0);
    const mid_x: usize = @intFromFloat(inner.x + inner.w / 2.0);

    // The gutter pixel must carry theme.border, not a bled cell color.
    const gutter_px = pixelAt(&raster, mid_x, gutter_mid_y);
    try testing.expectEqual(theme.border, gutter_px);
}

fn pixelAt(r: *Raster, x: usize, y: usize) [3]u8 {
    const i = (y * r.width + x) * 4;
    return .{ r.pixels[i + 2], r.pixels[i + 1], r.pixels[i + 0] }; // BGRA -> RGB
}
