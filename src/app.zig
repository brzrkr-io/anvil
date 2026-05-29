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
const keys = @import("keys.zig");
const persist = @import("session_persist.zig");
const chip_mod = @import("context_chip.zig");
const copy_mode_mod = @import("copy_mode.zig");

const shader_src = @embedFile("platform/shaders.metal");
const font_data = @embedFile("font_ttf");
const icon_data = @embedFile("app_icon_png");

/// Write UTF-8 text to the system pasteboard (OSC 52). Implemented in shim.m.
extern fn anvil_pasteboard_write(ptr: [*]const u8, len: usize) void;
/// Post a macOS user notification. Implemented in shim.m; no-op when unbundled
/// or when the app is frontmost. Title and body are null-terminated UTF-8.
extern fn anvil_notify(title: [*:0]const u8, body: [*:0]const u8) void;
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
var help_open: bool = false;
var copy_mode = copy_mode_mod.CopyMode{};
var overlay: [128 * 7]f32 = undefined; // colored rects (x,y,w,h,r,g,b)
var ctx_chip: chip_mod.Chip = .{};

var cfg: config.Config = .{};
var cfg_loaded = false;
var cfg_path_buf: [std.fs.max_path_bytes]u8 = undefined;
var cfg_path: ?[:0]const u8 = null;
var cfg_mtime: ?i128 = null;
var cfg_error_buf: [128]u8 = undefined;
var cfg_error_len: usize = 0;

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
/// Captures any parse error into cfg_error_buf; clears it on clean load.
fn loadConfig() void {
    const path = configPath() orelse return;
    const result = config.loadFull(path);
    cfg = result.cfg;
    cfg_error_len = result.err_len;
    if (result.err_len > 0) @memcpy(cfg_error_buf[0..result.err_len], result.err[0..result.err_len]);
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

/// Push the active theme's fg/bg/ANSI into every terminal so the parser can
/// answer OSC 10/11/4 color queries — this is how nvim detects our background
/// and selects the matching light/dark colorscheme.
fn pushThemeColors() void {
    const th = activeTheme();
    var ansi: [16][3]u8 = undefined;
    for (th.ansi, 0..) |c, i| ansi[i] = .{ c.r, c.g, c.b };
    const fg = [3]u8{ th.fg.r, th.fg.g, th.fg.b };
    const bg = [3]u8{ th.bg.r, th.bg.g, th.bg.b };
    for (mgr.sessions.items) |*s| s.term.setThemeColors(fg, bg, ansi);
    updateThemeEnv();
}

/// Export ANVIL_THEME so newly spawned shells (and the bundled nvim colorscheme)
/// know our active variant without a query round-trip. Set on change only.
var last_theme_dark: ?bool = null;
extern "c" fn setenv(name: [*:0]const u8, value: [*:0]const u8, overwrite: c_int) c_int;
fn updateThemeEnv() void {
    const dark = effectiveDark();
    if (last_theme_dark == dark) return;
    last_theme_dark = dark;
    _ = setenv("ANVIL_THEME", if (dark) "mineral-dark" else "mineral-light", 1);
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

export fn anvil_font_data(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = font_data.len;
    return font_data.ptr;
}

export fn anvil_icon_data(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = icon_data.len;
    return icon_data.ptr;
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
        loadConfig();
        const ws = workspaceRect();
        const g = renderer.paneGrid(ws.w, ws.h);
        var restored = false;
        if (persist.loadFromFile(std.heap.page_allocator)) |state| {
            mgr.spawnFromState(state, g.rows, g.cols) catch {};
            restored = mgr.tabs.items.len > 0;
        }
        if (!restored) mgr.spawnFirst(g.rows, g.cols) catch return;
        ready = true;
        applyCursorDefault();
        return;
    }
    relayout();
}

export fn anvil_save_session() callconv(.c) void {
    if (!ready) return;
    persist.saveToFile(std.heap.page_allocator, &mgr);
}

/// Drain pending shell output into the terminal. Returns 0 only when every
/// session has exited; individual dead panes show an in-pane indicator.
export fn anvil_poll() callconv(.c) c_int {
    if (!ready) return 1;
    reloadConfigIfChanged();
    pushThemeColors();
    var any_alive: bool = false;
    for (mgr.sessions.items) |*s| {
        if (!s.exited) {
            if (!s.poll()) {
                s.exited = true;
            } else {
                any_alive = true;
            }
        }
        if (s.term.takeClipboard()) |data| anvil_pasteboard_write(data.ptr, data.len);
        if (s.term.takeNotify()) |n| {
            var title_buf: [64]u8 = undefined;
            var body_buf: [64]u8 = undefined;
            const title_s = std.fmt.bufPrintZ(&title_buf, "Command finished", .{}) catch continue;
            const body_s = if (n.exit == 0)
                std.fmt.bufPrintZ(&body_buf, "exit 0 after {d}s", .{n.elapsed_s}) catch continue
            else
                std.fmt.bufPrintZ(&body_buf, "exit {d} after {d}s", .{ n.exit, n.elapsed_s }) catch continue;
            anvil_notify(title_s, body_s);
        }
    }
    return if (any_alive) 1 else 0;
}

/// Respawn the shell in the focused pane if it has exited. No-op if still alive.
export fn anvil_respawn() callconv(.c) void {
    if (!ready) return;
    const s = focused();
    if (!s.exited) return;
    s.respawn() catch {};
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

/// key: 0 esc, 1 enter (next match), 2 prev match, 4 backspace, 5 toggle regex.
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
        5 => {
            srch.toggleRegex(&focused().term);
            if (srch.current()) |m| jumpToMatch(m);
        },
        else => {},
    }
}

export fn anvil_help_toggle() callconv(.c) void {
    if (help_open) {
        help_open = false;
    } else {
        cpal.hide();
        srch.hide();
        help_open = true;
    }
}

export fn anvil_help_open() callconv(.c) c_int {
    return if (help_open) 1 else 0;
}

/// key: 0 = esc/close.
export fn anvil_help_key(key: c_int) callconv(.c) void {
    switch (key) {
        0 => help_open = false,
        else => {},
    }
}

export fn anvil_copy_mode_toggle() callconv(.c) void {
    if (!ready) return;
    if (copy_mode.open) {
        copy_mode.exit();
        focused().term.clearSelection();
    } else {
        cpal.hide();
        srch.hide();
        help_open = false;
        copy_mode.enter(&focused().term);
    }
}

export fn anvil_copy_mode_open() callconv(.c) c_int {
    return if (copy_mode.open) 1 else 0;
}

/// key: 0 esc/q exit, 1 v visual, 2 y/enter copy+exit, 3 up/k, 4 down/j,
/// 5 left/h, 6 right/l, 7 g (top), 8 G (bottom), 9 ctrl-u half up,
/// 10 ctrl-d half down, 11 w word forward, 12 b word back.
export fn anvil_copy_mode_key(key: c_int) callconv(.c) void {
    if (!ready) return;
    const t = &focused().term;
    switch (key) {
        0 => { // esc / q
            copy_mode.exit();
            t.clearSelection();
        },
        1 => copy_mode.startVisual(), // v
        2 => { // y / enter — copy and exit
            if (copy_mode.visual) {
                var len: usize = 0;
                const txt = anvil_copy(&len);
                if (len > 0) anvil_pasteboard_write(txt, len);
            }
            copy_mode.exit();
            t.clearSelection();
        },
        3 => copy_mode.move(t, -1, 0), // up / k
        4 => copy_mode.move(t, 1, 0), // down / j
        5 => copy_mode.move(t, 0, -1), // left / h
        6 => copy_mode.move(t, 0, 1), // right / l
        7 => copy_mode.gotoTop(t), // g
        8 => copy_mode.gotoBottom(t), // G
        9 => copy_mode.halfPage(t, -1), // ctrl-u
        10 => copy_mode.halfPage(t, 1), // ctrl-d
        11 => copy_mode.wordForward(t), // w
        12 => copy_mode.wordBack(t), // b
        else => {},
    }
}

export fn anvil_cfg_error_open() callconv(.c) c_int {
    return if (cfg_error_len > 0) 1 else 0;
}

export fn anvil_cfg_error_dismiss() callconv(.c) void {
    cfg_error_len = 0;
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

var link_buf: [512]u8 = undefined;

/// Return the hyperlink URI under device-pixel coordinate (x, y).
/// Writes the URI into an internal buffer and sets *out_ptr / *out_len.
/// Returns 1 if a link was found, 0 otherwise.
export fn anvil_link_at(x: f32, y: f32, out_ptr: *[*]const u8, out_len: *usize) callconv(.c) c_int {
    if (!ready) return 0;
    const np = layoutPanes(&pane_buf);
    for (pane_buf[0..np]) |p| {
        if (!contains(p.rect, x, y)) continue;
        const s = mgr.byId(p.id) orelse continue;
        const ox = p.rect.x + renderer.pad_x;
        const oy = p.rect.y + renderer.pad_x;
        const cf = (x - ox) / renderer.cell_w;
        const rf = (y - oy) / renderer.cell_h;
        const col: u16 = @intFromFloat(std.math.clamp(cf, 0, @as(f32, @floatFromInt(s.term.grid.cols - 1))));
        const row: u16 = @intFromFloat(std.math.clamp(rf, 0, @as(f32, @floatFromInt(s.term.grid.rows - 1))));
        const cell = s.term.viewRow(row)[col];
        if (cell.link == 0) return 0;
        const uri = s.term.linkUri(cell.link);
        if (uri.len == 0) return 0;
        const n = @min(uri.len, link_buf.len);
        @memcpy(link_buf[0..n], uri[0..n]);
        out_ptr.* = &link_buf;
        out_len.* = n;
        return 1;
    }
    return 0;
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
        const show_live_cursor = s.id == mgr.focused and !copy_mode.open and
            s.term.view_offset == 0 and cursorVisible(&s.term);
        if (show_live_cursor) {
            instances[n] = renderer.cursorInstance(&s.term, ox, oy);
            n += 1;
        }
        if (s.id == mgr.focused and copy_mode.open) {
            if (n < instances.len) {
                instances[n] = copyModeCaret(th, ox, oy);
                n += 1;
            }
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

    // Context chip: git branch + kube context in the right side of the title bar.
    n += emitContextChip(th, n);

    out.count = @intCast(n);
    out.divider_count = if (zoomed) 0 else @intCast(tree.dividers(ws, divider_px, &divider_rects));

    if (help_open) {
        const r = emitHelp(th, n);
        out.palette_text_count = @intCast(r.text);
        out.overlay_count = @intCast(r.rects);
    } else if (cpal.open) {
        const r = emitPalette(th, n);
        out.palette_text_count = @intCast(r.text);
        out.overlay_count = @intCast(r.rects);
    } else if (srch.open) {
        const r = emitSearch(th, n);
        out.palette_text_count = @intCast(r.text);
        out.overlay_count = @intCast(r.rects);
    } else {
        const rails = emitRunRails(th);
        const ex = emitExitedPanes(th, rails, n);
        const ce = emitCfgError(th, rails + ex.rects, n + ex.text);
        out.overlay_count = @intCast(rails + ex.rects + ce.rects);
        out.palette_text_count = @intCast(ex.text + ce.text);
    }

    out.pending_count = renderer.atlas.pending_n;
}

// Nerd Font glyph codepoints used in the context chip.
const glyph_git: u21 = 0xe0a0; // nf-pl-branch
const glyph_kube: u21 = 0xf10d6; // nf-md-kubernetes
const chip_max_branch: usize = 20;
const chip_max_kube: usize = 20;

/// Render the context chip (git branch + kube context) right-aligned in the
/// title bar. Returns the number of glyph instances written into `instances`.
fn emitContextChip(th: *const theme.Theme, start: usize) usize {
    // Update cache from the focused pane's cwd.
    if (mgr.focusedSession()) |s| {
        ctx_chip.update(s.term.cwd());
    }
    if (ctx_chip.isEmpty()) return 0;

    const cw = renderer.cell_w;
    const label_y: f32 = (bar_h - renderer.cell_h) / 2;
    const fg = th.ansi[6]; // mineral/cyan = status.info/trace
    const bg = th.bar;
    const pad: f32 = 8;

    var n = start;

    // Build the codepoint list for the chip: git icon + branch, spaces, kube icon + ctx.
    // Encode: icon + branch, space, icon + kube.
    // We'll write the text segments; icons go via atlas.uvOrigin for the u21 cp.
    // Instead of a scratch buf for Unicode glyphs we build a list of codepoints.
    var cps: [64]u21 = undefined;
    var cp_n: usize = 0;

    const branch = ctx_chip.branch();
    if (branch.len > 0) {
        cps[cp_n] = glyph_git;
        cp_n += 1;
        cps[cp_n] = ' ';
        cp_n += 1;
        var it = std.unicode.Utf8View.initUnchecked(branch[0..@min(branch.len, chip_max_branch)]).iterator();
        while (it.nextCodepoint()) |cp| {
            if (cp_n >= cps.len) break;
            cps[cp_n] = cp;
            cp_n += 1;
        }
    }

    const kube = ctx_chip.kube();
    if (kube.len > 0) {
        if (cp_n > 0) {
            if (cp_n + 3 < cps.len) {
                cps[cp_n] = ' ';
                cp_n += 1;
                cps[cp_n] = ' ';
                cp_n += 1;
            }
        }
        if (cp_n + 2 < cps.len) {
            cps[cp_n] = glyph_kube;
            cp_n += 1;
            cps[cp_n] = ':';
            cp_n += 1;
        }
        var it = std.unicode.Utf8View.initUnchecked(kube[0..@min(kube.len, chip_max_kube)]).iterator();
        while (it.nextCodepoint()) |cp| {
            if (cp_n >= cps.len) break;
            cps[cp_n] = cp;
            cp_n += 1;
        }
    }

    if (cp_n == 0) return 0;

    // Right-align: start x so the chip ends at win_w - pad.
    const total_w = @as(f32, @floatFromInt(cp_n)) * cw;
    var x = win_w - pad - total_w;
    if (x < tab_inset_x) x = tab_inset_x; // don't overlap traffic lights

    for (cps[0..cp_n]) |cp| {
        if (n >= instances.len) break;
        instances[n] = .{
            .x = x,
            .y = label_y,
            .fg = fg.f32x4(),
            .bg = bg.f32x4(),
            .uv = renderer.atlas.uvOrigin(cp),
        };
        n += 1;
        x += cw;
    }

    return n - start;
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

/// A block caret for copy mode, rendered in the status.trace color (mineral/cyan).
fn copyModeCaret(th: *const theme.Theme, ox: f32, oy: f32) inst.CellInstance {
    const s = focused();
    const row = @min(copy_mode.row, s.term.grid.rows - 1);
    const col = @min(copy_mode.col, s.term.grid.cols - 1);
    const cells = s.term.viewRow(row);
    const cp = if (col < cells.len) cells[col].cp else ' ';
    const caret_color = th.ansi[6]; // status.trace = mineral/cyan
    const cell_bg = palette.resolve(if (col < cells.len) cells[col].bg else .default, false);
    return .{
        .x = ox + @as(f32, @floatFromInt(col)) * renderer.cell_w,
        .y = oy + @as(f32, @floatFromInt(row)) * renderer.cell_h,
        .fg = cell_bg.f32x4(),
        .bg = caret_color.f32x4(),
        .uv = renderer.atlas.uvOrigin(cp),
    };
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

/// Render a one-line config error banner at the top of the workspace using
/// the semantic failure color. Returns rect and text instance counts written.
fn emitCfgError(th: *const theme.Theme, ri_start: usize, inst_start: usize) struct { rects: usize, text: usize } {
    if (cfg_error_len == 0) return .{ .rects = 0, .text = 0 };
    const cw = renderer.cell_w;
    const ch = renderer.cell_h;
    const bar_color = theme.Rgb{ .r = 0xb1, .g = 0x3a, .b = 0x30 }; // status.failure
    const text_color = theme.Rgb{ .r = 0xee, .g = 0xf1, .b = 0xf2 }; // bone
    putRect(ri_start, 0, bar_h, win_w, ch, bar_color);
    const msg = cfg_error_buf[0..cfg_error_len];
    var ni = inst_start;
    for (msg, 0..) |byte, i| {
        const gx = renderer.pad_x + @as(f32, @floatFromInt(i)) * cw;
        if (gx + cw > win_w) break;
        if (ni >= instances.len) break;
        instances[ni] = .{
            .x = gx,
            .y = bar_h,
            .fg = text_color.f32x4(),
            .bg = bar_color.f32x4(),
            .uv = renderer.atlas.uvOrigin(@intCast(byte)),
        };
        ni += 1;
    }
    _ = th;
    return .{ .rects = 1, .text = ni - inst_start };
}

const exited_msg = "[process exited — Cmd+R to restart]";

/// Render a "process exited" status bar at the bottom of each exited pane.
/// `ri_start` is the first free slot in `overlay`; `inst_start` is the first
/// free slot in `instances` beyond the already-written terminal cells.
/// Returns counts of new rects and text instances written.
fn emitExitedPanes(th: *const theme.Theme, ri_start: usize, inst_start: usize) struct { rects: usize, text: usize } {
    const max_rect = overlay.len / 7;
    var ri = ri_start;
    var ni = inst_start;
    const np = layoutPanes(&pane_buf);
    for (pane_buf[0..np]) |p| {
        const s = mgr.byId(p.id) orelse continue;
        if (!s.exited) continue;
        if (ri >= max_rect) break;
        const cw = renderer.cell_w;
        const ch = renderer.cell_h;
        const bar_y = p.rect.y + p.rect.h - ch;
        putRect(ri, p.rect.x, bar_y, p.rect.w, ch, th.ansi[1]);
        ri += 1;
        const tx = p.rect.x + renderer.pad_x;
        for (exited_msg, 0..) |c, i| {
            const gx = tx + @as(f32, @floatFromInt(i)) * cw;
            if (gx + cw > p.rect.x + p.rect.w) break;
            if (ni >= instances.len) break;
            instances[ni] = .{
                .x = gx,
                .y = bar_y,
                .fg = th.bg.f32x4(),
                .bg = th.ansi[1].f32x4(),
                .uv = renderer.atlas.uvOrigin(@intCast(c)),
            };
            ni += 1;
        }
    }
    return .{ .rects = ri - ri_start, .text = ni - inst_start };
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
    // Right-aligned status: optional "[R]"/"[R?]" regex indicator + "cur/total".
    var cbuf: [32]u8 = undefined;
    const cur = if (srch.count == 0) 0 else srch.cur + 1;
    const cnt = std.fmt.bufPrint(&cbuf, "{d}/{d}", .{ cur, srch.count }) catch "";
    const mode_label: []const u8 = if (srch.regex_mode) (if (srch.bad_pattern) "[R?] " else "[R] ") else "";
    const total_right_len = mode_label.len + cnt.len;
    var rx = bx + bw - pad - @as(f32, @floatFromInt(total_right_len)) * cw;
    for (mode_label, 0..) |c, i| {
        const fg = if (srch.bad_pattern) th.ansi[3] else th.ansi[6];
        putGlyph(n, rx + @as(f32, @floatFromInt(i)) * cw, by, fg, th.bar, c);
        n += 1;
    }
    rx += @as(f32, @floatFromInt(mode_label.len)) * cw;
    for (cnt, 0..) |c, i| {
        putGlyph(n, rx + @as(f32, @floatFromInt(i)) * cw, by, th.separator, th.bar, c);
        n += 1;
    }
    return .{ .text = n - start, .rects = ri };
}

/// Lay out the keyboard shortcut cheatsheet overlay: a centered panel listing
/// all key bindings grouped by section.
fn emitHelp(th: *const theme.Theme, start: usize) struct { text: usize, rects: usize } {
    const cw = renderer.cell_w;
    const ch = renderer.cell_h;
    const pad = renderer.pad_x;

    // Column layout: chord col is 10 cells wide, 2-cell gutter, then action.
    const chord_cols: usize = 10;
    const gutter: usize = 2;
    const action_cols: usize = 22;
    const inner_cols: usize = chord_cols + gutter + action_cols;
    const pw = @as(f32, @floatFromInt(inner_cols)) * cw + pad * 2;

    // Count total rows: title + blank + (section header + items) per section.
    var total_rows: usize = 2; // title row + blank
    for (keys.sections) |sec| {
        total_rows += 1 + sec.items.len; // header + bindings
    }
    // Clamp height to 85% of the window.
    const max_rows_f = @floor(win_h * 0.85 / ch);
    const max_rows: usize = @intFromFloat(max_rows_f);
    const visible_rows = @min(total_rows, max_rows);
    const ph = @as(f32, @floatFromInt(visible_rows)) * ch;

    const px = @floor((win_w - pw) / 2);
    const py = @floor(win_h * 0.12);

    var ri: usize = 0;
    putRect(ri, px - 1, py - 1, pw + 2, ph + 2, th.separator); // border
    ri += 1;
    putRect(ri, px, py, pw, ph, th.bar); // panel
    ri += 1;

    var n = start;
    const tx = px + pad;
    var row: usize = 0;

    // Title row.
    const title = "Keyboard Shortcuts";
    for (title, 0..) |c, i| {
        const ry = py + @as(f32, @floatFromInt(row)) * ch;
        putGlyph(n, tx + @as(f32, @floatFromInt(i)) * cw, ry, th.sel_fg, th.bar, c);
        n += 1;
    }
    row += 1;
    row += 1; // blank line

    for (keys.sections) |sec| {
        if (row >= visible_rows) break;
        // Section header in cyan.
        const ry = py + @as(f32, @floatFromInt(row)) * ch;
        for (sec.title, 0..) |c, i| {
            putGlyph(n, tx + @as(f32, @floatFromInt(i)) * cw, ry, th.ansi[6], th.bar, c);
            n += 1;
        }
        row += 1;

        for (sec.items) |b| {
            if (row >= visible_rows) break;
            const by2 = py + @as(f32, @floatFromInt(row)) * ch;

            // Chord column: iterate codepoints (chord strings contain multi-byte UTF-8).
            var col: usize = 0;
            var it = std.unicode.Utf8View.initUnchecked(b.chord).iterator();
            while (it.nextCodepoint()) |cp| {
                if (col >= chord_cols) break;
                instances[n] = .{
                    .x = tx + @as(f32, @floatFromInt(col)) * cw,
                    .y = by2,
                    .fg = th.ansi[14].f32x4(),
                    .bg = th.bar.f32x4(),
                    .uv = renderer.atlas.uvOrigin(cp),
                };
                n += 1;
                col += 1;
            }

            // Action column: plain ASCII, use putGlyph.
            const ax = tx + @as(f32, @floatFromInt(chord_cols + gutter)) * cw;
            for (b.action, 0..) |c, i| {
                if (i >= action_cols) break;
                putGlyph(n, ax + @as(f32, @floatFromInt(i)) * cw, by2, th.fg, th.bar, c);
                n += 1;
            }
            row += 1;
        }
    }

    return .{ .text = n - start, .rects = ri };
}
