const std = @import("std");
const Session = @import("session.zig").Session;
const SessionManager = @import("session_manager.zig").SessionManager;
const Renderer = @import("render/renderer.zig").Renderer;
const inst = @import("render/instance.zig");
const palette = @import("render/palette.zig");
const theme = @import("render/theme.zig");

const shader_src = @embedFile("platform/shaders.metal");
const max_instances = 60000;
const font_pt: f32 = 13.0;
const bar_h: f32 = 40; // compact title bar, device pixels (20pt @2x)

var mgr = SessionManager{ .alloc = std.heap.page_allocator };
var renderer = Renderer{ .cell_w = 16, .cell_h = 32, .pad_x = 8, .pad_y = bar_h + 6, .pad_bottom = 8 };
var instances: [max_instances]inst.CellInstance = undefined;
var ready = false;

fn focused() *Session {
    return mgr.focusedSession().?;
}

const ThemeMode = enum(c_int) { system = 0, light = 1, dark = 2 };
var theme_mode: ThemeMode = .system;
var os_dark: bool = true;

fn effectiveDark() bool {
    return switch (theme_mode) {
        .system => os_dark,
        .light => false,
        .dark => true,
    };
}

fn activeTheme() *const theme.Theme {
    return if (effectiveDark()) &theme.mineral_dark else &theme.mineral_light;
}

export fn anvil_set_theme_mode(m: c_int) callconv(.c) void {
    theme_mode = @enumFromInt(m);
}

export fn anvil_set_os_dark(d: c_int) callconv(.c) void {
    os_dark = d != 0;
}

export fn anvil_theme_is_dark() callconv(.c) c_int {
    return if (effectiveDark()) 1 else 0;
}

const AtlasParams = extern struct {
    first: u32,
    count: u32,
    cols: u32,
    rows: u32,
    pt_size: f32,
};

export fn anvil_shader_src(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = shader_src.len;
    return shader_src.ptr;
}

export fn anvil_atlas_params(out: *AtlasParams) callconv(.c) void {
    const a = renderer.atlas;
    out.* = .{ .first = a.first, .count = a.count, .cols = a.cols, .rows = a.rows(), .pt_size = font_pt };
}

export fn anvil_set_metrics(cell_w: f32, cell_h: f32) callconv(.c) void {
    renderer.cell_w = cell_w;
    renderer.cell_h = cell_h;
}

export fn anvil_resize(px_w: f32, px_h: f32) callconv(.c) void {
    const g = renderer.gridSize(px_w, px_h);
    if (ready) {
        const s = focused();
        if (g.cols == s.term.grid.cols and g.rows == s.term.grid.rows) return;
        s.resize(g.rows, g.cols) catch return;
    } else {
        mgr.spawn(g.rows, g.cols) catch return;
        ready = true;
    }
}

/// Drain pending shell output into the terminal. Returns 0 when the shell has
/// exited (EOF) so the front-end can quit.
export fn anvil_poll() callconv(.c) c_int {
    if (!ready) return 1;
    return if (focused().poll()) 1 else 0;
}

export fn anvil_input(ptr: [*]const u8, len: usize) callconv(.c) void {
    if (!ready) return;
    focused().write(ptr[0..len]);
}

export fn anvil_scroll(delta: c_int) callconv(.c) void {
    if (!ready) return;
    const s = focused();
    s.term.clearSelection();
    s.term.scrollView(@intCast(delta));
}

/// kind: 0 = press (start), 1 = drag, 2 = release (extend). x/y in device px.
export fn anvil_mouse(kind: c_int, x: f32, y: f32) callconv(.c) void {
    if (!ready) return;
    const cf = (x - renderer.pad_x) / renderer.cell_w;
    const rf = (y - renderer.pad_y) / renderer.cell_h;
    const s = focused();
    const col: u16 = @intFromFloat(std.math.clamp(cf, 0, @as(f32, @floatFromInt(s.term.grid.cols - 1))));
    const row: u16 = @intFromFloat(std.math.clamp(rf, 0, @as(f32, @floatFromInt(s.term.grid.rows - 1))));
    switch (kind) {
        0 => s.term.selectStart(row, col),
        else => s.term.selectExtend(row, col),
    }
}

var copy_buf: [1 << 20]u8 = undefined;

export fn anvil_copy(out_len: *usize) callconv(.c) [*]const u8 {
    if (!ready) {
        out_len.* = 0;
        return &copy_buf;
    }
    out_len.* = focused().term.selectionText(copy_buf[0..]);
    return &copy_buf;
}

export fn anvil_frame(out: *inst.FrameData) callconv(.c) void {
    const th = activeTheme();
    palette.setActive(th);
    if (!ready) {
        out.count = 0;
        out.bg = th.bg.f32x3();
        return;
    }
    const s = focused();
    var n = renderer.buildInstances(&s.term, instances[0..]);
    if (s.term.view_offset == 0) {
        instances[n] = renderer.cursorInstance(&s.term);
        n += 1;
    }
    out.* = .{
        .instances = &instances,
        .count = @intCast(n),
        .cell_w = renderer.cell_w,
        .cell_h = renderer.cell_h,
        .pad_x = renderer.pad_x,
        .pad_y = renderer.pad_y,
        .cell_uv = renderer.atlas.cellUV(),
        .bar_h = bar_h,
        .bg = th.bg.f32x3(),
        .bar_color = th.bar.f32x3(),
        .sep_color = th.separator.f32x3(),
    };
}
