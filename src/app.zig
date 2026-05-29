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
const search = @import("search.zig");
const config = @import("config.zig");

const shader_src = @embedFile("platform/shaders.metal");

/// Write UTF-8 text to the system pasteboard (OSC 52). Implemented in shim.m.
extern fn anvil_pasteboard_write(ptr: [*]const u8, len: usize) void;
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
var srch = search.Search{};
var overlay: [64 * 7]f32 = undefined; // colored rects (x,y,w,h,r,g,b)

var cfg: config.Config = .{};
var cfg_loaded = false;
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
    cfg = config.load(path);
    cfg_loaded = true;
    theme_mode = switch (cfg.theme) {
        .system => .system,
        .light => .light,
        .dark => .dark,
    };
    renderer.pad_x = cfg.padding_x;
    renderer.pad_y = bar_h + cfg.padding_y;
    cfg_mtime = config.mtime(path);
}

/// Whether the cursor is in its visible blink phase. Steady (non-blink)
/// cursors are always visible. ~530 ms half-period, free-running off the
/// wall clock since the render loop ticks at 60 fps.
fn cursorVisible(t: *const @import("vt/terminal.zig").Terminal) bool {
    if (!t.cursor_blink) return true;
    var ts: std.c.timespec = undefined;
    _ = std.c.clock_gettime(.MONOTONIC, &ts);
    const ms = @as(i64, ts.sec) * 1000 + @divTrunc(ts.nsec, std.time.ns_per_ms);
    const period_ms = 530;
    return @mod(ms, period_ms * 2) < period_ms;
}

/// Stamp the configured default cursor onto the focused session. Programs may
/// still override it at runtime via DECSCUSR.
fn applyCursorDefault() void {
    const t = &focused().term;
    t.cursor_style = switch (cfg.cursor_style) {
        .block => .block,
        .underline => .underline,
        .bar => .bar,
    };
    t.cursor_blink = cfg.cursor_blink;
}

/// Reload config if the file changed on disk. Cheap stat, called each poll.
fn reloadConfigIfChanged() void {
    const path = configPath() orelse return;
    const m = config.mtime(path) orelse return;
    if (cfg_mtime) |prev| {
        if (m == prev) return;
    }
    loadConfig();
    if (ready) relayout(); // padding may have changed the usable grid
}

fn focused() *Session {
    return mgr.focusedSession().?;
}

/// The pane area: the window minus the title bar.
fn workspaceRect() pane.Rect {
    return .{ .x = 0, .y = bar_h, .w = win_w, .h = win_h - bar_h };
}

var zoomed = false; // focused pane temporarily fills the workspace

/// Lay out the active tab's panes into `out`. When zoomed, the focused pane
/// alone fills the workspace (one entry); otherwise the normal split layout.
fn layoutPanes(out: []pane.PaneRect) usize {
    const tree = mgr.activeTree() orelse return 0;
    if (zoomed) {
        out[0] = .{ .id = mgr.focused, .rect = workspaceRect() };
        return 1;
    }
    return tree.layout(workspaceRect(), divider_px, out);
}

/// Resize every session's grid + PTY to match its current pane rect.
fn relayout() void {
    const n = layoutPanes(&pane_buf);
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
    if (!cfg_loaded) loadConfig(); // font size must be known before the atlas builds
    out.* = .{ .cols = atlasmod.cols, .rows = atlasmod.rows_n, .pt_size = cfg.font_size };
}

export fn anvil_set_metrics(cell_w: f32, cell_h: f32) callconv(.c) void {
    renderer.cell_w = cell_w;
    renderer.cell_h = cell_h;
}

export fn anvil_resize(px_w: f32, px_h: f32) callconv(.c) void {
    win_w = px_w;
    win_h = px_h;
    if (!ready) {
        loadConfig(); // padding affects the grid; load before sizing
        const ws = workspaceRect();
        const g = renderer.paneGrid(ws.w, ws.h);
        mgr.spawnFirst(g.rows, g.cols) catch return;
        ready = true;
        applyCursorDefault();
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
        if (s.term.takeClipboard()) |data| anvil_pasteboard_write(data.ptr, data.len);
    }
    return alive;
}

export fn anvil_input(ptr: [*]const u8, len: usize) callconv(.c) void {
    if (!ready) return;
    focused().write(ptr[0..len]);
}

/// Paste clipboard text. Wraps in bracketed-paste markers when the program
/// enabled mode 2004 so editors can tell a paste from typing.
export fn anvil_paste(ptr: [*]const u8, len: usize) callconv(.c) void {
    if (!ready) return;
    const s = focused();
    if (s.term.bracketed_paste) {
        s.write("\x1b[200~");
        s.write(ptr[0..len]);
        s.write("\x1b[201~");
    } else {
        s.write(ptr[0..len]);
    }
}

export fn anvil_scroll(delta: c_int) callconv(.c) void {
    if (!ready) return;
    const s = focused();
    // Program tracking the mouse? Wheel becomes button 64 (up) / 65 (down).
    if (s.term.mouse != .off) {
        const cb: u8 = if (delta > 0) 64 else 65;
        var n: c_int = if (delta > 0) delta else -delta;
        while (n > 0) : (n -= 1) sendMouseReport(s, cb, 0, 0, false);
        return;
    }
    s.term.clearSelection();
    s.term.scrollView(@intCast(delta));
}

/// Jump the focused pane's view to the previous (dir < 0) or next (dir > 0)
/// shell prompt mark (OSC 133). No-op without marks in that direction.
export fn anvil_jump_prompt(dir: c_int) callconv(.c) void {
    if (!ready) return;
    const s = focused();
    s.term.clearSelection();
    s.term.jumpPrompt(@intCast(dir));
}

fn contains(r: pane.Rect, x: f32, y: f32) bool {
    return x >= r.x and x < r.x + r.w and y >= r.y and y < r.y + r.h;
}

/// kind: 0 = press (start), 1 = drag, 2 = release (extend). x/y in device px.
/// Press hit-tests the pane under the cursor and focuses it; drag/release
/// stay in the focused pane.
export fn anvil_mouse(kind: c_int, x: f32, y: f32) callconv(.c) void {
    if (!ready) return;
    const np = layoutPanes(&pane_buf);
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

    // Program tracking the mouse? Forward the event to the PTY instead of
    // driving local selection.
    if (s.term.mouse != .off) {
        // Suppress drag reports the program didn't ask for.
        if (kind == 1 and s.term.mouse == .normal) return;
        const motion = kind == 1;
        // Button code: left = 0, +32 motion bit. Release uses code 3 (legacy)
        // or the original button (SGR).
        const cb: u8 = if (motion) 0 + 32 else 0;
        sendMouseReport(s, cb, col, row, kind == 2);
        return;
    }

    switch (kind) {
        0 => s.term.selectStart(row, col),
        else => s.term.selectExtend(row, col),
    }
}

/// Encode one mouse event for the PTY. SGR (1006) when the program enabled it,
/// else the legacy X10 byte encoding. `release` picks the report's final byte.
fn sendMouseReport(s: *Session, cb: u8, col: u16, row: u16, release: bool) void {
    var buf: [32]u8 = undefined;
    if (s.term.mouse_sgr) {
        const final: u8 = if (release) 'm' else 'M';
        const out = std.fmt.bufPrint(&buf, "\x1b[<{d};{d};{d}{c}", .{ cb, col + 1, row + 1, final }) catch return;
        s.write(out);
    } else {
        // Legacy: ESC [ M  <cb+32> <col+33> <row+33>; release reports button 3.
        const b: u8 = if (release) 3 + 32 else cb + 32;
        const cx: u8 = @intCast(@min(@as(u16, 223), col) + 33);
        const cy: u8 = @intCast(@min(@as(u16, 223), row) + 33);
        s.write(&[_]u8{ 0x1b, '[', 'M', b, cx, cy });
    }
}

/// axis: 0 = side by side (vertical divider), 1 = stacked (horizontal divider).
export fn anvil_split(axis: c_int) callconv(.c) void {
    if (!ready) return;
    const s = focused();
    const a: pane.Axis = if (axis == 0) .x else .y;
    mgr.splitFocused(a, s.term.grid.rows, s.term.grid.cols) catch return;
    applyCursorDefault();
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

/// Grow the focused pane toward `dir` (0 left, 1 right, 2 up, 3 down).
export fn anvil_resize_pane(dir: c_int) callconv(.c) void {
    if (!ready or zoomed) return;
    mgr.resizeFocused(@enumFromInt(dir), 0.05);
    relayout();
}

/// Reset the active tab's splits to even 50/50.
export fn anvil_balance_panes() callconv(.c) void {
    if (!ready or zoomed) return;
    mgr.balanceActive();
    relayout();
}

/// Toggle zoom: the focused pane fills the workspace, hiding its siblings.
export fn anvil_zoom_toggle() callconv(.c) void {
    if (!ready) return;
    zoomed = !zoomed;
    relayout();
}

export fn anvil_new_tab() callconv(.c) void {
    if (!ready) return;
    const ws = workspaceRect();
    const g = renderer.paneGrid(ws.w, ws.h);
    mgr.newTab(g.rows, g.cols) catch return;
    applyCursorDefault();
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

export fn anvil_search_toggle() callconv(.c) void {
    if (!ready) return;
    if (srch.open) {
        srch.hide();
        focused().term.clearSelection();
    } else {
        cpal.hide();
        srch.show();
    }
}

export fn anvil_search_open() callconv(.c) c_int {
    return if (srch.open) 1 else 0;
}

export fn anvil_search_char(c: u8) callconv(.c) void {
    if (!ready) return;
    srch.typeChar(c, &focused().term);
    if (srch.current()) |m| jumpToMatch(m);
}

/// key: 0 esc, 1 enter (next match), 2 prev match, 4 backspace.
export fn anvil_search_key(key: c_int) callconv(.c) void {
    if (!ready) return;
    switch (key) {
        0 => {
            srch.hide();
            focused().term.clearSelection();
        },
        1 => {
            if (srch.next()) |m| jumpToMatch(m);
        },
        2 => {
            if (srch.prev()) |m| jumpToMatch(m);
        },
        4 => {
            srch.backspace(&focused().term);
            if (srch.current()) |m| jumpToMatch(m);
        },
        else => {},
    }
}

/// Scroll the focused terminal so `m` is visible and select its span, reusing
/// the normal selection highlight. Centers the match line when possible.
fn jumpToMatch(m: search.Match) void {
    const t = &focused().term;
    const sb: i64 = @intCast(t.scrollback.len());
    const rows: i64 = @intCast(t.grid.rows);
    const center = @divTrunc(rows, 2);
    var off = sb - @as(i64, @intCast(m.line)) + center;
    off = std.math.clamp(off, 0, sb);
    t.view_offset = @intCast(off);
    const top_logical = sb - off; // logical line shown at visible row 0
    const r = @as(i64, @intCast(m.line)) - top_logical;
    if (r >= 0 and r < rows) {
        const rr: u16 = @intCast(r);
        t.selection = .{
            .anchor = .{ .row = rr, .col = m.col },
            .head = .{ .row = rr, .col = m.col + m.len - 1 },
        };
    } else {
        t.selection = null;
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
    const np = layoutPanes(&pane_buf);
    var n: usize = 0;
    const multi = np > 1;
    for (pane_buf[0..np]) |p| {
        const s = mgr.byId(p.id) orelse continue;
        const ox = p.rect.x + renderer.pad_x;
        const oy = p.rect.y + renderer.pad_x;
        const start = n;
        n += renderer.buildInstances(&s.term, ox, oy, instances[n..]);
        // Dim unfocused panes (only meaningful when split).
        if (multi and s.id != mgr.focused) {
            for (instances[start..n]) |*ci| ci.flags |= inst.flag_dim;
        }
        if (s.id == mgr.focused and s.term.view_offset == 0 and cursorVisible(&s.term)) {
            instances[n] = renderer.cursorInstance(&s.term, ox, oy);
            n += 1;
        }
    }
    // Tab labels in the title bar: program title (or cwd basename, or number),
    // the active tab highlighted.
    if (mgr.tabs.items.len > 1) {
        const label_y: f32 = (bar_h - renderer.cell_h) / 2;
        var x = tab_inset_x;
        var ti: usize = 0;
        while (ti < mgr.tabs.items.len) : (ti += 1) {
            const active = ti == mgr.active_tab;
            const fg4 = if (active) palette.selectionFg().f32x4() else palette.defaultFg().f32x4();
            const bg4 = if (active) palette.selectionBg().f32x4() else th.bar.f32x4();
            var buf: [128]u8 = undefined;
            const label = tabLabel(ti, &buf);
            var it = std.unicode.Utf8View.initUnchecked(label).iterator();
            while (it.nextCodepoint()) |cp| {
                instances[n] = .{
                    .x = x,
                    .y = label_y,
                    .fg = fg4,
                    .bg = bg4,
                    .uv = renderer.atlas.uvOrigin(cp),
                };
                n += 1;
                x += renderer.cell_w;
            }
            x += renderer.cell_w; // gap between tabs
        }
    }

    out.count = @intCast(n);
    out.divider_count = if (zoomed) 0 else @intCast(tree.dividers(ws, divider_px, &divider_rects));

    if (cpal.open) {
        const r = emitPalette(th, n);
        out.palette_text_count = @intCast(r.text);
        out.overlay_count = @intCast(r.rects);
    } else if (srch.open) {
        const r = emitSearch(th, n);
        out.palette_text_count = @intCast(r.text);
        out.overlay_count = @intCast(r.rects);
    } else {
        out.overlay_count = @intCast(emitRunRails(th));
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

/// Inline run-block rails: a 3px vertical bar in each pane's left gutter,
/// one per visible OSC-133 command block, colored by exit state. Writes into
/// `overlay` and returns the rect count. Only called when no modal is open.
fn emitRunRails(th: *const theme.Theme) usize {
    const RunBlock = @import("vt/terminal.zig").RunBlock;
    const max_rail = overlay.len / 7;
    var blocks: [64]RunBlock = undefined;
    var ri: usize = 0;
    const np = layoutPanes(&pane_buf);
    for (pane_buf[0..np]) |p| {
        const s = mgr.byId(p.id) orelse continue;
        const oy = p.rect.y + renderer.pad_x;
        const nb = s.term.visibleRunBlocks(&blocks);
        for (blocks[0..nb]) |b| {
            if (ri >= max_rail) break;
            const c = switch (b.state) {
                .running => th.ansi[5],
                .ok => th.ansi[2],
                .fail => th.ansi[1],
            };
            const y = oy + @as(f32, @floatFromInt(b.row)) * renderer.cell_h;
            const h = @as(f32, @floatFromInt(b.rows)) * renderer.cell_h;
            putRect(ri, p.rect.x + 2, y, 3, h, c);
            ri += 1;
        }
    }
    return ri;
}

const tab_label_max = 16; // cells

/// Final path component of `p`, or the whole string when there is no slash.
fn basename(p: []const u8) []const u8 {
    var i = p.len;
    while (i > 0) : (i -= 1) {
        if (p[i - 1] == '/') return p[i..];
    }
    return p;
}

/// Human label for tab `ti`: program title, else cwd basename, else the tab
/// number. Writes UTF-8 into `buf`, capped to `tab_label_max` codepoints.
fn tabLabel(ti: usize, buf: []u8) []const u8 {
    const fallback = std.fmt.bufPrint(buf, "{d}", .{ti + 1}) catch "?";
    const s = mgr.byId(mgr.tabs.items[ti].anyLeaf()) orelse return fallback;
    const t = &s.term;
    var src: []const u8 = t.title();
    if (src.len == 0) src = basename(t.cwd());
    if (src.len == 0) return fallback;

    var it = std.unicode.Utf8View.initUnchecked(src).iterator();
    var n: usize = 0;
    var cells: usize = 0;
    while (it.nextCodepointSlice()) |bytes| {
        if (cells >= tab_label_max or n + bytes.len > buf.len) break;
        @memcpy(buf[n .. n + bytes.len], bytes);
        n += bytes.len;
        cells += 1;
    }
    return buf[0..n];
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

/// Lay out the search bar: a one-line input box at the top-right of the window
/// showing the query and the current/total match count.
fn emitSearch(th: *const theme.Theme, start: usize) struct { text: usize, rects: usize } {
    const cw = renderer.cell_w;
    const ch = renderer.cell_h;
    const pad = renderer.pad_x;

    var bw: f32 = 50 * cw;
    const max_bw = win_w * 0.9;
    if (bw > max_bw) bw = max_bw;
    const bx = @floor(win_w - bw - pad);
    const by = bar_h + pad;

    var ri: usize = 0;
    putRect(ri, bx - 1, by - 1, bw + 2, ch + 2, th.separator); // border
    ri += 1;
    putRect(ri, bx, by, bw, ch, th.bar); // panel
    ri += 1;

    var n = start;
    const tx = bx + pad;
    putGlyph(n, tx, by, th.fg, th.bar, '/'); // prompt prefix
    n += 1;
    for (srch.queryStr(), 0..) |c, i| {
        putGlyph(n, tx + @as(f32, @floatFromInt(i + 1)) * cw, by, th.fg, th.bar, c);
        n += 1;
    }
    // Match count, right-aligned: "cur/total" (1-based, 0/0 when no matches).
    var cbuf: [24]u8 = undefined;
    const cur = if (srch.count == 0) 0 else srch.cur + 1;
    const cnt = std.fmt.bufPrint(&cbuf, "{d}/{d}", .{ cur, srch.count }) catch "";
    const cnt_x = bx + bw - pad - @as(f32, @floatFromInt(cnt.len)) * cw;
    for (cnt, 0..) |c, i| {
        putGlyph(n, cnt_x + @as(f32, @floatFromInt(i)) * cw, by, th.separator, th.bar, c);
        n += 1;
    }
    return .{ .text = n - start, .rects = ri };
}
