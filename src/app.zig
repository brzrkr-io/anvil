const std = @import("std");
const Session = @import("session.zig").Session;
const SessionManager = @import("session_manager.zig").SessionManager;
const pane = @import("workspace/pane_tree.zig");
const Renderer = @import("render/renderer.zig").Renderer;
const inst = @import("render/instance.zig");
const palette = @import("render/palette.zig");
const theme = @import("render/theme.zig");

const shader_src = @embedFile("platform/shaders.metal");
const max_instances = 60000;
const max_panes = 64;
const divider_px: f32 = 2;
const font_pt: f32 = 13.0;
const bar_h: f32 = 40; // compact title bar, device pixels (20pt @2x)

var mgr = SessionManager{ .alloc = std.heap.page_allocator };
var renderer = Renderer{ .cell_w = 16, .cell_h = 32, .pad_x = 8, .pad_y = bar_h + 6, .pad_bottom = 8 };
var instances: [max_instances]inst.CellInstance = undefined;
var pane_buf: [max_panes]pane.PaneRect = undefined;
var divider_rects: [max_panes]pane.Rect = undefined;
var win_w: f32 = 0;
var win_h: f32 = 0;
var ready = false;

fn focused() *Session {
    return mgr.focusedSession().?;
}

/// The pane area: the window minus the title bar.
fn workspaceRect() pane.Rect {
    return .{ .x = 0, .y = bar_h, .w = win_w, .h = win_h - bar_h };
}

/// Resize every session's grid + PTY to match its current pane rect.
fn relayout() void {
    const tree = mgr.activeTree() orelse return;
    const n = tree.layout(workspaceRect(), divider_px, &pane_buf);
    for (pane_buf[0..n]) |p| {
        const s = mgr.byId(p.id) orelse continue;
        const g = renderer.paneGrid(p.rect.w, p.rect.h);
        if (g.cols != s.term.grid.cols or g.rows != s.term.grid.rows) {
            s.resize(g.rows, g.cols) catch {};
        }
    }
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
    win_w = px_w;
    win_h = px_h;
    if (!ready) {
        const ws = workspaceRect();
        const g = renderer.paneGrid(ws.w, ws.h);
        mgr.spawnFirst(g.rows, g.cols) catch return;
        ready = true;
        return;
    }
    relayout();
}

/// Drain pending shell output into the terminal. Returns 0 when the shell has
/// exited (EOF) so the front-end can quit.
export fn anvil_poll() callconv(.c) c_int {
    if (!ready) return 1;
    var alive: c_int = 1;
    for (mgr.sessions.items) |*s| {
        if (!s.poll() and s.id == mgr.focused) alive = 0;
    }
    return alive;
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

fn contains(r: pane.Rect, x: f32, y: f32) bool {
    return x >= r.x and x < r.x + r.w and y >= r.y and y < r.y + r.h;
}

/// kind: 0 = press (start), 1 = drag, 2 = release (extend). x/y in device px.
/// Press hit-tests the pane under the cursor and focuses it; drag/release
/// stay in the focused pane.
export fn anvil_mouse(kind: c_int, x: f32, y: f32) callconv(.c) void {
    if (!ready) return;
    const tree = mgr.activeTree() orelse return;
    const np = tree.layout(workspaceRect(), divider_px, &pane_buf);
    if (kind == 0) {
        for (pane_buf[0..np]) |p| {
            if (contains(p.rect, x, y)) {
                mgr.focused = p.id;
                break;
            }
        }
    }
    var fr: ?pane.Rect = null;
    for (pane_buf[0..np]) |p| {
        if (p.id == mgr.focused) fr = p.rect;
    }
    const r = fr orelse return;
    const s = mgr.byId(mgr.focused) orelse return;
    const ox = r.x + renderer.pad_x;
    const oy = r.y + renderer.pad_x;
    const cf = (x - ox) / renderer.cell_w;
    const rf = (y - oy) / renderer.cell_h;
    const col: u16 = @intFromFloat(std.math.clamp(cf, 0, @as(f32, @floatFromInt(s.term.grid.cols - 1))));
    const row: u16 = @intFromFloat(std.math.clamp(rf, 0, @as(f32, @floatFromInt(s.term.grid.rows - 1))));
    switch (kind) {
        0 => s.term.selectStart(row, col),
        else => s.term.selectExtend(row, col),
    }
}

/// axis: 0 = side by side (vertical divider), 1 = stacked (horizontal divider).
export fn anvil_split(axis: c_int) callconv(.c) void {
    if (!ready) return;
    const s = focused();
    const a: pane.Axis = if (axis == 0) .x else .y;
    mgr.splitFocused(a, s.term.grid.rows, s.term.grid.cols) catch return;
    relayout();
}

export fn anvil_close_pane() callconv(.c) void {
    if (!ready) return;
    mgr.closeFocused();
    relayout();
}

/// dir: 0 left, 1 right, 2 up, 3 down.
export fn anvil_focus_dir(dir: c_int) callconv(.c) void {
    if (!ready) return;
    mgr.focusNeighbor(workspaceRect(), @enumFromInt(dir), &pane_buf);
}

export fn anvil_new_tab() callconv(.c) void {
    if (!ready) return;
    const ws = workspaceRect();
    const g = renderer.paneGrid(ws.w, ws.h);
    mgr.newTab(g.rows, g.cols) catch return;
    relayout();
}

/// delta: signed tab offset, wraps.
export fn anvil_cycle_tab(delta: c_int) callconv(.c) void {
    if (!ready) return;
    mgr.cycleTab(@intCast(delta));
    relayout();
}

export fn anvil_close_tab() callconv(.c) void {
    if (!ready) return;
    mgr.closeTab();
    relayout();
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
    out.* = .{
        .instances = &instances,
        .count = 0,
        .cell_w = renderer.cell_w,
        .cell_h = renderer.cell_h,
        .pad_x = renderer.pad_x,
        .pad_y = renderer.pad_y,
        .cell_uv = renderer.atlas.cellUV(),
        .bar_h = bar_h,
        .bg = th.bg.f32x3(),
        .bar_color = th.bar.f32x3(),
        .sep_color = th.separator.f32x3(),
        .dividers = @ptrCast(&divider_rects),
        .divider_count = 0,
    };
    if (!ready) return;

    const ws = workspaceRect();
    const tree = mgr.activeTree() orelse return;
    const np = tree.layout(ws, divider_px, &pane_buf);
    var n: usize = 0;
    for (pane_buf[0..np]) |p| {
        const s = mgr.byId(p.id) orelse continue;
        const ox = p.rect.x + renderer.pad_x;
        const oy = p.rect.y + renderer.pad_x;
        n += renderer.buildInstances(&s.term, ox, oy, instances[n..]);
        if (s.id == mgr.focused and s.term.view_offset == 0) {
            instances[n] = renderer.cursorInstance(&s.term, ox, oy);
            n += 1;
        }
    }
    // Tab labels in the title bar: a digit per tab, active one highlighted.
    if (mgr.tabs.items.len > 1) {
        const label_y: f32 = (bar_h - renderer.cell_h) / 2;
        var ti: usize = 0;
        while (ti < mgr.tabs.items.len) : (ti += 1) {
            const active = ti == mgr.active_tab;
            const digit: u21 = '1' + @as(u21, @intCast(@min(ti, 8)));
            const fg4 = if (active) palette.selectionFg().f32x4() else palette.defaultFg().f32x4();
            const bg4 = if (active) palette.selectionBg().f32x4() else th.bar.f32x4();
            instances[n] = .{
                .x = renderer.pad_x + @as(f32, @floatFromInt(ti)) * renderer.cell_w * 2,
                .y = label_y,
                .fg = fg4,
                .bg = bg4,
                .uv = renderer.atlas.uvOrigin(digit),
            };
            n += 1;
        }
    }

    out.count = @intCast(n);
    out.divider_count = @intCast(tree.dividers(ws, divider_px, &divider_rects));
}
