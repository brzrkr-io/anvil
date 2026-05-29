const std = @import("std");
const Session = @import("session.zig").Session;
const SessionManager = @import("session_manager.zig").SessionManager;
const pane = @import("workspace/pane_tree.zig");
const Renderer = @import("render/renderer.zig").Renderer;
const inst = @import("render/instance.zig");
const atlasmod = @import("render/atlas.zig");
const palette = @import("render/palette.zig");
const theme = @import("render/theme.zig");
const cmd = @import("palette.zig");
const config = @import("config.zig");

const shader_src = @embedFile("platform/shaders.metal");
const max_instances = 60000;
const max_panes = 64;
const divider_px: f32 = 2;
const font_pt: f32 = 13.0;
const bar_h: f32 = 40; // compact title bar, device pixels (20pt @2x)
const tab_inset_x: f32 = 152; // clear the macOS traffic-light buttons (device px)

var mgr = SessionManager{ .alloc = std.heap.page_allocator };
var renderer = Renderer{ .cell_w = 16, .cell_h = 32, .pad_x = 8, .pad_y = bar_h + 6, .pad_bottom = 8 };
var instances: [max_instances]inst.CellInstance = undefined;
var pane_buf: [max_panes]pane.PaneRect = undefined;
var divider_rects: [max_panes]pane.Rect = undefined;
var win_w: f32 = 0;
var win_h: f32 = 0;
var ready = false;
var cpal = cmd.Palette{};
var overlay: [8 * 7]f32 = undefined; // up to 8 colored rects (x,y,w,h,r,g,b)

var cfg_path_buf: [std.fs.max_path_bytes]u8 = undefined;
var cfg_path: ?[:0]const u8 = null;
var cfg_mtime: ?i128 = null;

/// `$HOME/.config/anvil/config.toml`, or null if HOME is unset.
fn configPath() ?[:0]const u8 {
    if (cfg_path) |p| return p;
    const home = std.c.getenv("HOME") orelse return null;
    const h = std.mem.span(home);
    const p = std.fmt.bufPrintZ(&cfg_path_buf, "{s}/.config/anvil/config.toml", .{h}) catch return null;
    cfg_path = p;
    return p;
}

/// Load config and apply it. Records the file mtime for change detection.
fn loadConfig() void {
    const path = configPath() orelse return;
    const cfg = config.load(path);
    theme_mode = switch (cfg.theme) {
        .system => .system,
        .light => .light,
        .dark => .dark,
    };
    cfg_mtime = config.mtime(path);
}

/// Reload config if the file changed on disk. Cheap stat, called each poll.
fn reloadConfigIfChanged() void {
    const path = configPath() orelse return;
    const m = config.mtime(path) orelse return;
    if (cfg_mtime) |prev| {
        if (m == prev) return;
    }
    loadConfig();
}

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
    cols: u32,
    rows: u32,
    pt_size: f32,
};

export fn anvil_shader_src(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = shader_src.len;
    return shader_src.ptr;
}

export fn anvil_atlas_params(out: *AtlasParams) callconv(.c) void {
    out.* = .{ .cols = atlasmod.cols, .rows = atlasmod.rows_n, .pt_size = font_pt };
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
        loadConfig();
        ready = true;
        return;
    }
    relayout();
}

/// Drain pending shell output into the terminal. Returns 0 when the shell has
/// exited (EOF) so the front-end can quit.
export fn anvil_poll() callconv(.c) c_int {
    if (!ready) return 1;
    reloadConfigIfChanged();
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

export fn anvil_palette_toggle() callconv(.c) void {
    if (!ready) return;
    if (cpal.open) cpal.hide() else cpal.show();
}

export fn anvil_palette_open() callconv(.c) c_int {
    return if (cpal.open) 1 else 0;
}

export fn anvil_palette_char(c: u8) callconv(.c) void {
    cpal.typeChar(c);
}

/// key: 0 esc, 1 enter, 2 up, 3 down, 4 backspace.
export fn anvil_palette_key(key: c_int) callconv(.c) void {
    switch (key) {
        0 => cpal.hide(),
        1 => {
            if (cpal.selected()) |id| runAction(id);
            cpal.hide();
        },
        2 => cpal.moveUp(),
        3 => cpal.moveDown(),
        4 => cpal.backspace(),
        else => {},
    }
}

fn runAction(id: cmd.ActionId) void {
    switch (id) {
        .split_side => anvil_split(0),
        .split_stacked => anvil_split(1),
        .close_pane => anvil_close_pane(),
        .new_tab => anvil_new_tab(),
        .next_tab => anvil_cycle_tab(1),
        .prev_tab => anvil_cycle_tab(-1),
        .focus_left => anvil_focus_dir(0),
        .focus_right => anvil_focus_dir(1),
        .focus_up => anvil_focus_dir(2),
        .focus_down => anvil_focus_dir(3),
        .theme_system => anvil_set_theme_mode(0),
        .theme_light => anvil_set_theme_mode(1),
        .theme_dark => anvil_set_theme_mode(2),
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
    renderer.atlas.resetPending(); // before any glyph lookup this frame
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
        .overlay = &overlay,
        .overlay_count = 0,
        .palette_text_count = 0,
        .pending = &renderer.atlas.pending,
        .pending_count = 0,
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
                .x = tab_inset_x + @as(f32, @floatFromInt(ti)) * renderer.cell_w * 2,
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

    if (cpal.open) {
        const r = emitPalette(th, n);
        out.palette_text_count = @intCast(r.text);
        out.overlay_count = @intCast(r.rects);
    }

    out.pending_count = renderer.atlas.pending_n;
}

fn putRect(ri: usize, x: f32, y: f32, w: f32, h: f32, c: theme.Rgb) void {
    const o = overlay[ri * 7 ..];
    o[0] = x;
    o[1] = y;
    o[2] = w;
    o[3] = h;
    const f = c.f32x3();
    o[4] = f[0];
    o[5] = f[1];
    o[6] = f[2];
}

fn putGlyph(idx: usize, x: f32, y: f32, fg: theme.Rgb, bg: theme.Rgb, ch: u8) void {
    instances[idx] = .{
        .x = x,
        .y = y,
        .fg = fg.f32x4(),
        .bg = bg.f32x4(),
        .uv = renderer.atlas.uvOrigin(@intCast(ch)),
    };
}

/// Lay out the command palette overlay: colored rects into `overlay` and
/// glyph instances into `instances[start..]`. Returns the counts written.
fn emitPalette(th: *const theme.Theme, start: usize) struct { text: usize, rects: usize } {
    const cw = renderer.cell_w;
    const ch = renderer.cell_h;
    const pad = renderer.pad_x;

    var pw: f32 = 60 * cw;
    const max_pw = win_w * 0.9;
    if (pw > max_pw) pw = max_pw;
    const visible: usize = @min(cpal.count, 8);
    const ph = (1 + @as(f32, @floatFromInt(visible))) * ch;
    const px = @floor((win_w - pw) / 2);
    const py = @floor(win_h * 0.25);
    const top: usize = if (cpal.sel >= 8) cpal.sel - 7 else 0;

    var ri: usize = 0;
    putRect(ri, px - 1, py - 1, pw + 2, ph + 2, th.separator); // border
    ri += 1;
    putRect(ri, px, py, pw, ph, th.bar); // panel
    ri += 1;
    putRect(ri, px, py, pw, ch, th.bg); // input line
    ri += 1;
    putRect(ri, px, py + ch, pw, 1, th.separator); // input/result divider
    ri += 1;
    if (cpal.count > 0) {
        const hy = py + ch + @as(f32, @floatFromInt(cpal.sel - top)) * ch;
        putRect(ri, px, hy, pw, ch, th.sel_bg); // selection highlight
        ri += 1;
    }

    var n = start;
    const tx = px + pad;
    // Query text on the input line.
    for (cpal.query[0..cpal.qlen], 0..) |c, i| {
        putGlyph(n, tx + @as(f32, @floatFromInt(i)) * cw, py, th.fg, th.bg, c);
        n += 1;
    }
    // Result rows.
    var r: usize = 0;
    while (r < visible) : (r += 1) {
        const idx = cpal.results[top + r];
        const label = cmd.registry[idx].label;
        const selected = (top + r) == cpal.sel;
        const fg = if (selected) th.sel_fg else th.fg;
        const bg = if (selected) th.sel_bg else th.bar;
        const ry = py + ch * (1 + @as(f32, @floatFromInt(r)));
        for (label, 0..) |c, j| {
            putGlyph(n, tx + @as(f32, @floatFromInt(j)) * cw, ry, fg, bg, c);
            n += 1;
        }
    }
    return .{ .text = n - start, .rects = ri };
}
