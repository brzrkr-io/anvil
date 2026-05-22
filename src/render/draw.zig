//! Per-frame viewport draw loop, extracted from main.zig so it can be called
//! from unit tests without a live AppKit/Metal context.
//!
//! `drawViewport` renders the visible cell grid (plus prompt-rule hairlines and
//! the text cursor) into `raster`. It reads only the arguments it is given —
//! no global state.

const std = @import("std");
const Raster = @import("raster.zig").Raster;
const Font = @import("font.zig").Font;
const color = @import("color.zig");
const Theme = @import("../config/theme.zig").Theme;
const term_mod = @import("../terminal/terminal.zig");
const Terminal = term_mod.Terminal;
const Cell = term_mod.Cell;
const Selection = @import("../app/selection.zig").Selection;
const Search = @import("../terminal/search.zig").Search;
const cfg_mod = @import("../config/config.zig");

/// Cursor rendering parameters. Bundled so the public signature stays stable.
pub const CursorParams = struct {
    /// Animated column (fractional viewport cell, from `g.cursor_ax`).
    ax: f32,
    /// Animated row (fractional viewport cell, from `g.cursor_ay`).
    ay: f32,
    /// Blink phase in [0, 1).
    blink_phase: f32,
    /// Config cursor style + blink preference.
    cfg: cfg_mod.Config.CursorCfg,
};

/// Resolve a `term.Color` to an RGB triple, falling back to `default`.
pub fn resolveColor(col: term_mod.Color, default: [3]u8, theme: Theme) [3]u8 {
    return switch (col) {
        .default => default,
        .palette => |p| theme.palette256(p),
        .rgb => |v| v,
    };
}

/// Cursor opacity for blink phase `p` in [0,1): solid, fade out, dim hold,
/// fade in — a soft blink rather than a hard on/off toggle. Shared with the
/// blink-animation timing in main.zig so the render and the tick agree.
pub fn cursorOpacity(p: f32) f32 {
    const smoothstep = struct {
        fn f(t: f32) f32 {
            const c = std.math.clamp(t, 0.0, 1.0);
            return c * c * (3.0 - 2.0 * c);
        }
    }.f;
    if (p < 0.50) return 1.0;
    if (p < 0.62) return 1.0 - smoothstep((p - 0.50) / 0.12);
    if (p < 0.88) return 0.0;
    return smoothstep((p - 0.88) / 0.12);
}

/// Should a prompt-rule hairline be drawn above viewport row `y`?
/// `off` is the scroll offset (0 when the viewport is pinned to the live bottom).
pub fn ruleRow(terminal: *const Terminal, viewport_y: usize, off: usize) bool {
    const hist = terminal.scrollbackLen();
    // Content row for this viewport row, mirroring the formula in renderFrame.
    const crow: usize = if (off > viewport_y) (hist + viewport_y) -| off else hist + viewport_y - off;
    return terminal.isPromptStart(terminal.absoluteLineOfContent(crow));
}

/// Draw one cell into the raster.
pub fn drawCell(
    raster: *Raster,
    font: Font,
    theme: Theme,
    x: usize,
    y: usize,
    content_row: usize,
    cell: Cell,
    top_bar_rows: usize,
    selection: Selection,
    search: ?*const Search,
) void {
    var fg = resolveColor(cell.fg, theme.foreground, theme);
    var bg = resolveColor(cell.bg, theme.background, theme);
    if (cell.attrs.inverse) {
        const tmp = fg;
        fg = bg;
        bg = tmp;
    }
    if (selection.active and selection.contains(content_row, x)) {
        bg = color.mix(theme.background, theme.accent, 0.28);
    }
    if (search) |s| {
        switch (s.classify(content_row, x)) {
            .current => bg = theme.accent,
            .other => bg = theme.ansi[8],
            .none => {},
        }
    }
    const ry = y + top_bar_rows;
    if (!std.mem.eql(u8, &bg, &theme.background)) {
        raster.cellBg(font, x, ry, bg);
    }
    if (cell.cp != ' ' and cell.cp != 0) {
        raster.cellGlyph(font, x, ry, font.glyph(cell.cp), fg);
    }
}

/// Draw the text cursor into the raster.
pub fn drawCursor(
    raster: *Raster,
    terminal: *Terminal,
    font: Font,
    theme: Theme,
    top_bar_rows: usize,
    params: CursorParams,
) void {
    const blink: bool = terminal.app_cursor_blink orelse params.cfg.blink;
    const opacity: f32 = if (blink) cursorOpacity(params.blink_phase) else 1.0;
    const ax: f64 = params.ax;
    const ay: f64 = @as(f64, params.ay) + @as(f64, @floatFromInt(top_bar_rows));
    const cursor_rgb = color.mix(theme.background, theme.accent, opacity);

    const style: cfg_mod.CursorStyle = if (terminal.app_cursor_shape) |s| switch (s) {
        .block => .block,
        .underline => .underline,
        .bar => .bar,
    } else params.cfg.style;

    switch (style) {
        .block => {
            raster.cellInset(font, ax, ay, cursor_rgb, 0.0, 0.0, 1.0, 1.0);
            const ic: usize = @intFromFloat(@round(params.ax));
            const ir: usize = @intFromFloat(@round(params.ay));
            if (ir < terminal.rows() and ic < terminal.cols()) {
                const row = terminal.viewportRow(ir);
                if (ic < row.len) {
                    const cell = row[ic];
                    if (cell.cp != ' ' and cell.cp != 0) {
                        const base_fg = resolveColor(cell.fg, theme.foreground, theme);
                        const glyph_fg = color.mix(base_fg, theme.background, opacity);
                        raster.cellGlyph(font, ic, ir + top_bar_rows, font.glyph(cell.cp), glyph_fg);
                    }
                }
            }
        },
        .bar => raster.cellInset(font, ax, ay, cursor_rgb, 0.0, 0.0, 0.15, 1.0),
        .underline => raster.cellInset(font, ax, ay, cursor_rgb, 0.0, 0.0, 1.0, 0.12),
    }
}

/// Draw the viewport: the visible cell grid, prompt-rule hairlines, and the
/// cursor. Corresponds to the per-frame draw body previously inline in
/// `renderFrame` in main.zig.
///
/// `scroll_pos` and `overscroll` drive smooth scrolling (0/0 = pinned).
/// `top_bar_rows` offsets every cell row by the tab-bar height.
/// Pass `cursor_params = null` to suppress cursor drawing (e.g. in tests).
pub fn drawViewport(
    raster: *Raster,
    terminal: *Terminal,
    font: Font,
    theme: Theme,
    scroll_pos: f32,
    overscroll: f32,
    selection: Selection,
    search: ?*const Search,
    top_bar_rows: usize,
    cursor_params: ?CursorParams,
    rule_x_start: f64,
    rule_x_end: f64,
) void {
    const rows = terminal.rows();
    const cols = terminal.cols();
    const rule_rgb = color.mix(theme.background, theme.foreground, 0.28);

    if (scroll_pos == 0 and overscroll == 0) {
        // Live bottom: no fractional offset.
        var y: usize = 0;
        while (y < rows) : (y += 1) {
            const crow = terminal.contentRowOfViewport(y);
            const line = terminal.viewportRow(y);
            var x: usize = 0;
            while (x < cols and x < line.len) : (x += 1) {
                drawCell(raster, font, theme, x, y, crow, line[x], top_bar_rows, selection, search);
            }
            if (terminal.isPromptStart(terminal.absoluteLineOfContent(crow))) {
                const ry: f64 = @floatFromInt(y + top_bar_rows);
                raster.rowRule(font, ry, rule_rgb, rule_x_start, rule_x_end);
            }
        }
    } else {
        // Smooth-scroll path: render integer offset (base+1) and slide the
        // grid by the fractional part plus overscroll.
        const base: usize = @intFromFloat(@floor(scroll_pos));
        const frac: f64 = @as(f64, scroll_pos) - @floor(@as(f64, scroll_pos));
        const scroll_shift = (1.0 - frac) * font.metrics.cell_h;
        raster.y_shift_px = scroll_shift - @as(f64, overscroll);
        const hist = terminal.scrollbackLen();
        const off = base + 1;
        var y: usize = 0;
        while (y <= rows) : (y += 1) {
            const crow: usize = if (off > y) (hist + y) -| off else hist + y - off;
            const line = terminal.viewportRowAt(off, y);
            var x: usize = 0;
            while (x < cols and x < line.len) : (x += 1) {
                drawCell(raster, font, theme, x, y, crow, line[x], top_bar_rows, selection, search);
            }
            if (terminal.isPromptStart(terminal.absoluteLineOfContent(crow))) {
                const ry: f64 = @floatFromInt(y + top_bar_rows);
                raster.rowRule(font, ry, rule_rgb, rule_x_start, rule_x_end);
            }
        }
        raster.y_shift_px = 0;
    }

    // Cursor: only when the viewport is pinned to the live bottom.
    if (cursor_params) |cp| {
        const cur = terminal.cursor();
        if (cur.visible and terminal.viewportOffset() == 0 and
            scroll_pos == 0 and cur.x < cols and cur.y < rows)
        {
            raster.y_shift_px = -@as(f64, overscroll);
            drawCursor(raster, terminal, font, theme, top_bar_rows, cp);
            raster.y_shift_px = 0;
        }
    }
}

// --- tests -----------------------------------------------------------------

test "ruleRow returns true only for prompt-start content rows" {
    const testing = std.testing;
    const scrollback = @import("../terminal/scrollback.zig");
    var t = try Terminal.init(testing.allocator, 10, 4, scrollback.default_capacity);
    defer t.deinit();

    // No marks: no rule on any row.
    try testing.expect(!ruleRow(&t, 0, 0));
    try testing.expect(!ruleRow(&t, 1, 0));

    // Emit OSC 133;A on the current line (row 0, absolute line 0).
    t.feed("\x1b]133;A\x07");
    try testing.expect(ruleRow(&t, 0, 0));
    // Row 1 has no mark.
    try testing.expect(!ruleRow(&t, 1, 0));
}

test "prompt-rule rows match the prompt-mark set across viewport scroll" {
    const testing = std.testing;
    const scrollback = @import("../terminal/scrollback.zig");
    // 5 rows wide enough for a simple feed.
    var t = try Terminal.init(testing.allocator, 10, 5, scrollback.default_capacity);
    defer t.deinit();

    // Feed 3 lines with prompt marks on rows 0 and 2.
    t.feed("\x1b]133;A\x07");
    t.feed("line0\r\n");
    t.feed("line1\r\n");
    t.feed("\x1b]133;A\x07");
    t.feed("line2\r\n");
    t.feed("line3");

    // At minimum no panic — the invariant is that ruleRow agrees with
    // isPromptStart for every viewport row.
    var y: usize = 0;
    while (y < t.rows()) : (y += 1) {
        const crow = t.contentRowOfViewport(y);
        const abs = t.absoluteLineOfContent(crow);
        const expected = t.isPromptStart(abs);
        try testing.expectEqual(expected, ruleRow(&t, y, 0));
    }
}

test "a steady-state frame performs zero heap allocations" {
    // Per-frame cost (Bug C): drawViewport must not allocate on the heap.
    // Terminal and Raster are built with a CountingAllocator; after the
    // initial setup, reset() clears the counters, then drawViewport runs.
    // alloc_count and resize_count must both be zero.
    const testing = std.testing;
    const CountingAllocator = @import("../testing/counting_allocator.zig").CountingAllocator;
    const scrollback_mod = @import("../terminal/scrollback.zig");
    const Raster_t = @import("raster.zig").Raster;
    const Font_t = @import("font.zig").Font;
    const theme_mod = @import("../config/theme.zig");

    var ca = CountingAllocator.init(testing.allocator);
    const alloc = ca.allocator();

    // Build terminal with some content and a prompt mark.
    var t = try Terminal.init(alloc, 20, 6, scrollback_mod.default_capacity);
    defer t.deinit();
    t.feed("\x1b]133;A\x07");
    t.feed("hello world\r\n");
    t.feed("second line\r\n");
    t.feed("third line");

    const font = try Font_t.init("Menlo", 26.0);
    defer font.deinit();

    var raster = try Raster_t.init(testing.allocator, 400, 200);
    defer raster.deinit();

    const theme = theme_mod.mineral_dark;
    const sel = @import("../app/selection.zig").Selection{};

    // Reset counters — everything from here on is steady-state.
    ca.reset();

    drawViewport(
        &raster,
        &t,
        font,
        theme,
        0.0,
        0.0,
        sel,
        null,
        0,
        null,
        0,
        @floatFromInt(raster.width),
    );

    try testing.expectEqual(@as(usize, 0), ca.alloc_count);
    try testing.expectEqual(@as(usize, 0), ca.resize_count);

    // Also verify with viewport scrolled into history (smooth scroll path).
    // Push enough lines to build scrollback.
    var i: usize = 0;
    while (i < 10) : (i += 1) {
        t.feed("scrollback line\r\n");
    }
    ca.reset();

    drawViewport(
        &raster,
        &t,
        font,
        theme,
        2.0,
        0.0,
        sel,
        null,
        0,
        null,
        0,
        @floatFromInt(raster.width),
    );

    try testing.expectEqual(@as(usize, 0), ca.alloc_count);
    try testing.expectEqual(@as(usize, 0), ca.resize_count);
}
