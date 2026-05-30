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
const caldera = @import("caldera.zig");
const ipc = @import("ipc.zig");
const chrome = @import("chrome.zig");

// libc directory listing for the EXPLORER sidebar (std.fs is mid-migration to
// the Io interface in this Zig; libc @cImport matches the pty/socket pattern).
const cdir = @cImport({
    @cInclude("dirent.h");
    @cInclude("unistd.h");
});

const shader_src = @embedFile("platform/shaders.metal");
const font_data = @embedFile("font_ttf");
const font_bold_data = @embedFile("font_ttf_bold");
const sans_data = @embedFile("sans_ttf");
const icon_data = @embedFile("app_icon_png");

/// Write UTF-8 text to the system pasteboard (OSC 52). Implemented in shim.m.
extern fn anvil_pasteboard_write(ptr: [*]const u8, len: usize) void;
/// Post a macOS user notification. Implemented in shim.m; no-op when unbundled
/// or when the app is frontmost. Title and body are null-terminated UTF-8.
extern fn anvil_notify(title: [*:0]const u8, body: [*:0]const u8) void;
/// Horizontal advance (device px) of `cp` in the IBM Plex Sans chrome face, at
/// the atlas point size. Implemented in shim.m; lazily loads the Sans CTFont.
extern fn anvil_sans_advance(cp: u32) f32;
const max_instances = 60000;
const max_panes = 64;
const divider_px: f32 = 2; // layout gap + mouse hit zone (device px)
const divider_draw_px: f32 = 2; // drawn hairline width (1 logical pt @2x)
const font_pt: f32 = 13.0;
const bar_h: f32 = chrome.top_bar_h; // command bar, device pixels (22pt @2x)
const tab_inset_x: f32 = 152; // clear the macOS traffic-light buttons (device px)
// Advance from an active tab's status dot to its label, as a fraction of the
// chrome icon cell: the ● ink is centred in a full cell, so a snug-but-clear
// fraction pulls the label close without the numeral clipping the dot. Shared
// by emitCommandBar (drawing) and tabsWidth (right-alignment) so the strip
// stays aligned to its right margin.
const tab_dot_adv: f32 = 0.9;
const tab_strip_margin: f32 = chrome.sp16; // right margin of the tab strip

var mgr = SessionManager{ .alloc = std.heap.page_allocator };
var renderer = Renderer{ .cell_w = 16, .cell_h = 32, .pad_x = 8, .pad_y = bar_h + 6, .pad_bottom = 8 };
var instances: [max_instances]inst.CellInstance = undefined;
var pane_buf: [max_panes]pane.PaneRect = undefined;
var pane_range_buf: [max_panes + 1]inst.PaneRange = undefined;
var divider_rects: [max_panes]pane.Rect = undefined;
var win_w: f32 = 0;
var win_h: f32 = 0;
var ready = false;
var cpal = cmd.Palette{};
var srch = search.Search{};
var help_open: bool = false;
var copy_mode = copy_mode_mod.CopyMode{};
var overlay: [256 * 7]f32 = undefined; // colored rects (x,y,w,h,r,g,b)
var ctx_chip: chip_mod.Chip = .{};

var caldera_snap: caldera.Snapshot = .{};
var caldera_sel: usize = 0;
var caldera_drawer: bool = false;

var frame_dirty: bool = true;
inline fn markDirty() void {
    frame_dirty = true;
}

/// Set by main() before window.run() to open the first shell in a chosen dir.
pub var start_cwd: []const u8 = "";

/// Set by main() when --new was passed. Skips session restore on init and
/// session save on quit, preventing the persist race with the primary window.
pub var suppress_persist: bool = false;

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
    active_variant = theme.byName(cfg.themeVariant()) orelse
        .{ .dark = theme.mineral_dark, .light = theme.mineral_light };
    renderer.pad_x = cfg.padding_x;
    renderer.pad_y = bar_h + cfg.padding_y;
    cfg_mtime = config.mtime(path);
}

const blink_period_ms = 530;

fn currentBlinkPhase() bool {
    var ts: std.c.timespec = undefined;
    _ = std.c.clock_gettime(.MONOTONIC, &ts);
    const ms = @as(i64, ts.sec) * 1000 + @divTrunc(ts.nsec, std.time.ns_per_ms);
    return @mod(ms, blink_period_ms * 2) < blink_period_ms;
}

fn cursorVisible(t: *const @import("vt/terminal.zig").Terminal) bool {
    if (!t.cursor_blink) return true;
    return currentBlinkPhase();
}

var last_blink_phase: bool = false;

fn blinkActive() bool {
    if (cpal.open or srch.open or help_open or copy_mode.open or caldera_drawer) return false;
    const s = focused();
    return s.id == mgr.focused and !s.exited and s.term.cursor_blink and s.term.view_offset == 0;
}

fn pollBlink() void {
    if (!blinkActive()) {
        last_blink_phase = false;
        return;
    }
    const phase = currentBlinkPhase();
    if (phase != last_blink_phase) {
        last_blink_phase = phase;
        markDirty();
    }
}

// Cursor animation state. cur_anim_init=false causes the next call to snap
// instead of glide, which is the right behavior on first use and after
// cursor_smooth is toggled off then on.
var cur_anim_x: f32 = 0;
var cur_anim_y: f32 = 0;
var cur_anim_id: usize = 0;
var cur_anim_init: bool = false;
var cur_anim_last_ms: i64 = 0;

/// Read the MONOTONIC clock and return milliseconds.
fn nowMs() i64 {
    var ts: std.c.timespec = undefined;
    _ = std.c.clock_gettime(.MONOTONIC, &ts);
    return @as(i64, ts.sec) * 1000 + @divTrunc(ts.nsec, std.time.ns_per_ms);
}

/// Exponential-decay animation for the live cursor. Takes the snapped target
/// position (from cursorInstance), the focused session id, and returns the
/// current animated position. Calls markDirty() while the cursor is in motion;
/// does NOT call markDirty() once settled (distance < 0.5px), which lets the
/// terminal go idle.
///
/// Snap conditions (immediate teleport, no glide):
///   - first call (cur_anim_init == false)
///   - session id changed (tab/pane switch)
///   - large jump (> 6 cells) — avoids long cross-screen swooshes in editors
fn animateCursor(target_x: f32, target_y: f32, id: usize) struct { x: f32, y: f32 } {
    const tau: f32 = 0.028; // seconds; tunes glide speed
    const snap_cells: f32 = 6;
    const settle_px: f32 = 0.5;
    const max_dt_ms: i64 = 64; // clamp stalled frame delta

    const now = nowMs();
    const snap_threshold_x = snap_cells * renderer.cell_w;
    const snap_threshold_y = snap_cells * renderer.cell_h;

    const should_snap = !cur_anim_init or
        id != cur_anim_id or
        @abs(target_x - cur_anim_x) > snap_threshold_x or
        @abs(target_y - cur_anim_y) > snap_threshold_y;

    if (should_snap) {
        cur_anim_x = target_x;
        cur_anim_y = target_y;
        cur_anim_id = id;
        cur_anim_init = true;
        cur_anim_last_ms = now;
        return .{ .x = target_x, .y = target_y };
    }

    const raw_dt = now - cur_anim_last_ms;
    const dt_ms = if (raw_dt > max_dt_ms) max_dt_ms else raw_dt;
    cur_anim_last_ms = now;

    const dt_s: f32 = @as(f32, @floatFromInt(dt_ms)) / 1000.0;
    const alpha = 1.0 - std.math.exp(-dt_s / tau);

    cur_anim_x += (target_x - cur_anim_x) * alpha;
    cur_anim_y += (target_y - cur_anim_y) * alpha;

    const dx = @abs(target_x - cur_anim_x);
    const dy = @abs(target_y - cur_anim_y);
    if (dx < settle_px and dy < settle_px) {
        cur_anim_x = target_x;
        cur_anim_y = target_y;
        return .{ .x = target_x, .y = target_y };
    }

    markDirty();
    return .{ .x = cur_anim_x, .y = cur_anim_y };
}

// Scroll animation state. scr_anim_init=false causes the next call to snap.
var scr_anim_off: f32 = 0;
var scr_anim_id: usize = 0;
var scr_anim_init: bool = false;
var scr_anim_last_ms: i64 = 0;

/// Exponential-decay animation for scrollback scrolling. `target_lines` is the
/// integer view_offset (lines from live); `id` is the session id. Returns
/// `off_f`, the fractional line offset for rendering.
///
/// Snap conditions (no glide):
///   - first call (scr_anim_init == false)
///   - session id changed (tab/pane switch)
///   - large jump (> grid.rows lines)
fn animateScroll(target_lines: f32, id: usize, rows: u16) f32 {
    const tau: f32 = 0.045; // short glide: smooths per-frame steps but tracks input fast
    const max_dt_ms: i64 = 64;

    const now = nowMs();
    const snap_lines: f32 = @floatFromInt(rows);

    const should_snap = !scr_anim_init or
        id != scr_anim_id or
        @abs(target_lines - scr_anim_off) > snap_lines;

    if (should_snap) {
        scr_anim_off = target_lines;
        scr_anim_id = id;
        scr_anim_init = true;
        scr_anim_last_ms = now;
        return target_lines;
    }

    const raw_dt = now - scr_anim_last_ms;
    const dt_ms = if (raw_dt > max_dt_ms) max_dt_ms else raw_dt;
    scr_anim_last_ms = now;

    const dt_s: f32 = @as(f32, @floatFromInt(dt_ms)) / 1000.0;
    const alpha = 1.0 - std.math.exp(-dt_s / tau);
    scr_anim_off += (target_lines - scr_anim_off) * alpha;

    // settle threshold: 0.5 / cell_h lines (sub-pixel residue)
    const settle: f32 = 0.5 / renderer.cell_h;
    if (@abs(target_lines - scr_anim_off) < settle) {
        scr_anim_off = target_lines;
        return target_lines; // idle: do NOT markDirty
    }

    markDirty();
    return scr_anim_off;
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
    markDirty();
    if (ready) relayout();
}

fn focused() *Session {
    return mgr.focusedSession().?;
}

/// Zen / terminal-only mode: hides all chrome (top bar, status bar, rail,
/// sidebar, drawer, panel inset) so the terminal fills the window. Toggled by
/// Cmd+Return or the rail's terminal icon.
var zen: bool = false;

/// Top command-bar height, collapsed to zero in zen mode.
fn barH() f32 {
    return if (zen) 0 else bar_h;
}

/// True when the SESSIONS/EXPLORER sidebar is shown (Option A chrome).
var sidebar_open: bool = true;

/// Current sidebar width (device px); user-draggable within sane bounds.
var sidebar_w: f32 = chrome.sidebar_w;
var sidebar_dragging: bool = false;
const sidebar_w_min: f32 = 180;
const sidebar_w_max: f32 = 520;

/// Last hover position in device px (-1 = outside the window / unknown).
var hover_x: f32 = -1;
var hover_y: f32 = -1;

/// Left chrome width: activity rail plus the sidebar when open. Zero in zen.
fn leftChromeW() f32 {
    if (zen) return 0;
    return chrome.rail_w + (if (sidebar_open) sidebar_w else 0);
}

/// True when the right context drawer (RUNS / TRACE / AGENT) is shown (Option C).
var drawer_open: bool = true;

/// Right chrome width: the context drawer when open, else zero. Zero in zen.
fn rightChromeW() f32 {
    return if (drawer_open and !zen) chrome.drawer_w else 0;
}

/// The pane area: the window minus command bar, status bar, left chrome
/// (rail + sidebar), panel inset, and the per-pane header strip. Panes lay
/// out inside the inset panel body.
fn workspaceRect() pane.Rect {
    if (zen) return .{ .x = 0, .y = 0, .w = win_w, .h = win_h };
    const pp = chrome.panel_pad;
    const hs = chrome.header_strip_h;
    const sb = chrome.status_bar_h;
    const pb = chrome.panel_pad_bottom;
    const lc = leftChromeW();
    const rc = rightChromeW();
    return .{
        .x = lc + pp,
        .y = bar_h + pp + hs,
        .w = win_w - lc - rc - 2 * pp,
        .h = win_h - bar_h - pp - hs - sb - pb,
    };
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
var active_variant: theme.Variant = .{ .dark = theme.mineral_dark, .light = theme.mineral_light };

fn effectiveDark() bool {
    return switch (theme_mode) {
        .system => os_dark,
        .light => false,
        .dark => true,
    };
}

fn activeTheme() *const theme.Theme {
    return if (effectiveDark()) &active_variant.dark else &active_variant.light;
}

fn effectiveBackgroundOpacity() f32 {
    if (!effectiveDark()) return 1.0; // legibility floor: light variants always opaque
    const v = cfg.background_opacity;
    if (v >= 1.0) return 1.0;
    return @max(v, 0.75); // floor at 0.75
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
    if (m < 0 or m > 2) return;
    theme_mode = @enumFromInt(m);
    markDirty();
}

export fn anvil_set_os_dark(d: c_int) callconv(.c) void {
    os_dark = d != 0;
    markDirty();
}

export fn anvil_theme_is_dark() callconv(.c) c_int {
    return if (effectiveDark()) 1 else 0;
}

const AtlasParams = extern struct {
    cols: u32,
    rows: u32,
    pt_size: f32,
    weight: f32,
};

export fn anvil_shader_src(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = shader_src.len;
    return shader_src.ptr;
}

export fn anvil_font_data(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = font_data.len;
    return font_data.ptr;
}

export fn anvil_font_bold_data(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = font_bold_data.len;
    return font_bold_data.ptr;
}

export fn anvil_sans_data(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = sans_data.len;
    return sans_data.ptr;
}

export fn anvil_icon_data(out_len: *usize) callconv(.c) [*]const u8 {
    out_len.* = icon_data.len;
    return icon_data.ptr;
}

export fn anvil_atlas_params(out: *AtlasParams) callconv(.c) void {
    if (!cfg_loaded) loadConfig(); // font size must be known before the atlas builds
    out.* = .{ .cols = atlasmod.cols, .rows = atlasmod.rows_n, .pt_size = cfg.font_size, .weight = cfg.font_weight };
}

/// True only when the user actually opted into a translucent background. The
/// window setup uses this to keep the window/layer fully opaque (and skip the
/// vibrancy view) by default, so the standard look is a solid, crisp surface.
export fn anvil_translucent() callconv(.c) bool {
    if (!cfg_loaded) loadConfig();
    return effectiveBackgroundOpacity() < 1.0;
}

/// Queue the common TUI glyph set into the atlas so drainPending can upload
/// them before the first frame. out_ptr receives a pointer to the pending
/// PendingGlyph array; out_count receives the number of entries queued.
export fn anvil_prewarm_atlas(out_ptr: **const anyopaque, out_count: *u32) callconv(.c) void {
    var cp: u21 = 0x21;
    while (cp <= 0x7e) : (cp += 1) _ = renderer.atlas.slotFor(cp);
    cp = 0x2500;
    while (cp <= 0x259f) : (cp += 1) _ = renderer.atlas.slotFor(cp);
    out_ptr.* = &renderer.atlas.pending;
    out_count.* = renderer.atlas.pending_n;
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
        if (!suppress_persist) {
            if (persist.loadFromFile(std.heap.page_allocator)) |state| {
                mgr.spawnFromState(state, g.rows, g.cols) catch {};
                restored = mgr.tabs.items.len > 0;
            }
        }
        if (!restored) mgr.spawnFirstWithCwd(g.rows, g.cols, start_cwd) catch return;
        caldera.start(std.heap.page_allocator);
        ipc.start();
        ready = true;
        applyCursorDefault();
        markDirty();
        return;
    }
    markDirty();
    relayout();
}

export fn anvil_save_session() callconv(.c) void {
    if (!ready or suppress_persist) return;
    persist.saveToFile(std.heap.page_allocator, &mgr);
}

/// Called when this window becomes the active app, so the CLI can target the
/// front window by socket mtime. One window per process → app-active == focus.
export fn anvil_ipc_focus() callconv(.c) void {
    ipc.touchFocus();
}

fn drainIpc() void {
    var cmds: [32]ipc.Command = undefined;
    const n = ipc.takeCommands(&cmds);
    if (n > 0) markDirty();
    for (cmds[0..n]) |icmd| {
        switch (icmd) {
            .split => |axis| {
                const ws = workspaceRect();
                const g = renderer.paneGrid(ws.w, ws.h);
                mgr.splitFocused(axis, g.rows, g.cols) catch {};
                applyCursorDefault();
                relayout();
            },
            .tab => |iarg| {
                const ws = workspaceRect();
                const g = renderer.paneGrid(ws.w, ws.h);
                const path: []const u8 = if (iarg.has_path) iarg.path[0..iarg.len] else "";
                mgr.newTabCwd(g.rows, g.cols, path) catch {};
                applyCursorDefault();
                relayout();
            },
            .run => |rarg| {
                const ws = workspaceRect();
                const g = renderer.paneGrid(ws.w, ws.h);
                mgr.newTabCwd(g.rows, g.cols, "") catch {};
                applyCursorDefault();
                relayout();
                if (mgr.focusedSession()) |s| {
                    s.write(rarg.cmd[0..rarg.len]);
                    s.write("\n");
                }
            },
        }
    }
}

var last_forced_ms: i64 = 0;
const force_interval_ms: i64 = 2000;

export fn anvil_needs_render() callconv(.c) bool {
    const d = frame_dirty;
    frame_dirty = false;
    return d;
}

export fn anvil_force_render() callconv(.c) void {
    markDirty();
}

export fn anvil_poll() callconv(.c) c_int {
    if (!ready) return 1;
    drainIpc();
    reloadConfigIfChanged();
    pushThemeColors();
    if (caldera_drawer) markDirty();
    var any_alive: bool = false;
    for (mgr.sessions.items) |*s| {
        if (!s.exited) {
            const r = s.poll();
            if (!r.alive) {
                s.exited = true;
                markDirty();
            } else {
                any_alive = true;
                if (r.consumed) markDirty();
            }
        }
        if (s.term.takeClipboard()) |data| anvil_pasteboard_write(data.ptr, data.len);
        if (s.term.takeNotify()) |n| {
            markDirty();
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
    // Periodic safety net: force a full redraw at most every ~2s.
    {
        var ts: std.c.timespec = undefined;
        _ = std.c.clock_gettime(.MONOTONIC, &ts);
        const now_ms = @as(i64, ts.sec) * 1000 + @divTrunc(ts.nsec, std.time.ns_per_ms);
        if (now_ms - last_forced_ms >= force_interval_ms) {
            last_forced_ms = now_ms;
            markDirty();
        }
    }
    pollBlink();
    return if (any_alive) 1 else 0;
}

/// Respawn the shell in the focused pane if it has exited. No-op if still alive.
export fn anvil_respawn() callconv(.c) void {
    if (!ready) return;
    const s = focused();
    if (!s.exited) return;
    s.respawn() catch {};
    markDirty();
}

export fn anvil_input(ptr: [*]const u8, len: usize) callconv(.c) void {
    if (!ready) return;
    focused().write(ptr[0..len]);
    markDirty();
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
    markDirty();
}

export fn anvil_scroll(delta: c_int) callconv(.c) void {
    if (!ready) return;
    const s = focused();
    // Program tracking the mouse? Wheel becomes button 64 (up) / 65 (down).
    if (s.term.mouse != .off) {
        const cb: u8 = if (delta > 0) 64 else 65;
        var n: c_int = if (delta > 0) delta else -delta;
        while (n > 0) : (n -= 1) sendMouseReport(s, cb, 0, 0, false);
        markDirty();
        return;
    }
    s.term.clearSelection();
    s.term.scrollView(@intCast(delta));
    markDirty();
}

/// Jump the focused pane's view to the previous (dir < 0) or next (dir > 0)
/// shell prompt mark (OSC 133). No-op without marks in that direction.
export fn anvil_jump_prompt(dir: c_int) callconv(.c) void {
    if (!ready) return;
    const s = focused();
    s.term.clearSelection();
    s.term.jumpPrompt(@intCast(dir));
    markDirty();
}

fn contains(r: pane.Rect, x: f32, y: f32) bool {
    return x >= r.x and x < r.x + r.w and y >= r.y and y < r.y + r.h;
}

/// kind: 0 = press (start), 1 = drag, 2 = release (extend). x/y in device px.
/// Press hit-tests the pane under the cursor and focuses it; drag/release
/// stay in the focused pane.
/// Passive mouse-move: record the hover position and repaint if it moved into
/// a different sidebar row (so the hover highlight can follow the cursor).
export fn anvil_hover(x: f32, y: f32) callconv(.c) void {
    if (!ready) return;
    if (x == hover_x and y == hover_y) return;
    const was_row = hoverSidebarRow(hover_x, hover_y);
    const was_rail = hoverRailSlot(hover_x, hover_y);
    hover_x = x;
    hover_y = y;
    if (!sameHoverRow(hoverSidebarRow(x, y), was_row) or
        !sameSlot(hoverRailSlot(x, y), was_rail)) markDirty();
}

fn sameHoverRow(a: ?HoverRow, b: ?HoverRow) bool {
    if (a == null and b == null) return true;
    if (a == null or b == null) return false;
    return a.?.kind == b.?.kind and a.?.idx == b.?.idx;
}

fn sameSlot(a: ?usize, b: ?usize) bool {
    if (a == null and b == null) return true;
    if (a == null or b == null) return false;
    return a.? == b.?;
}

/// Clear hover state when the cursor leaves the view.
export fn anvil_hover_exit() callconv(.c) void {
    if (!ready) return;
    if (hover_x < 0 and hover_y < 0) return;
    const had = hoverSidebarRow(hover_x, hover_y) != null or
        hoverRailSlot(hover_x, hover_y) != null;
    hover_x = -1;
    hover_y = -1;
    if (had) markDirty();
}

/// The (kind, index) of the sidebar row under (x,y), or null. kind: 0 = session
/// row, 1 = explorer row. Geometry mirrors emitSidebar; exp_row_y0 is the
/// explorer list top recorded on the previous frame.
const HoverRow = struct { kind: u8, idx: usize };

fn hoverSidebarRow(x: f32, y: f32) ?HoverRow {
    if (!sidebar_open) return null;
    if (x < chrome.rail_w or x >= leftChromeW()) return null;
    const sess_y0 = bar_h + chrome.sidebar_header_h + 8;
    if (y >= sess_y0 and mgr.tabs.items.len > 0) {
        const i_f = (y - sess_y0) / chrome.row_h;
        if (i_f >= 0) {
            const i: usize = @intFromFloat(i_f);
            if (i < mgr.tabs.items.len) return .{ .kind = 0, .idx = i };
        }
    }
    if (exp_row_y0 > 0 and y >= exp_row_y0) {
        const i_f = (y - exp_row_y0) / chrome.row_h;
        if (i_f >= 0) {
            const i: usize = @intFromFloat(i_f);
            if (i < exp_n) return .{ .kind = 1, .idx = i };
        }
    }
    return null;
}

export fn anvil_mouse(kind: c_int, x: f32, y: f32) callconv(.c) void {
    if (!ready) return;
    // Sidebar resize: grab the right edge and drag. A 6px band around the edge
    // starts the drag; subsequent motion sets the width; release ends it.
    if (sidebar_open and !zen) {
        const edge = leftChromeW();
        const in_body = y > bar_h and y < win_h - chrome.status_bar_h;
        if (kind == 0 and in_body and @abs(x - edge) <= 6) {
            sidebar_dragging = true;
            return;
        }
        if (sidebar_dragging) {
            if (kind == 2) {
                sidebar_dragging = false;
                return;
            }
            sidebar_w = std.math.clamp(x - chrome.rail_w, sidebar_w_min, sidebar_w_max);
            relayout();
            markDirty();
            return;
        }
    }
    // Left chrome (activity rail + sidebar): a press here drives sidebar
    // actions and is then consumed, so it can never leak into terminal
    // selection underneath (which would start a phantom selection at the
    // clamped cell). A SESSIONS row switches to that tab; an EXPLORER row
    // inserts the entry name at the focused prompt (the "open" action).
    if (kind == 0 and !zen and x < leftChromeW()) {
        if (x < chrome.rail_w) {
            railClick(y);
        } else if (sidebar_open) {
            if (hoverSidebarRow(x, y)) |hr| switch (hr.kind) {
                0 => {
                    mgr.selectTab(hr.idx);
                    relayout();
                    markDirty();
                },
                1 => {
                    const ent = &exp_entries[hr.idx];
                    if (ent.is_dir) {
                        toggleExpanded(ent.rel[0..ent.rel_len]);
                        markDirty();
                    } else if (mgr.byId(mgr.focused)) |s| {
                        s.write(ent.rel[0..ent.rel_len]);
                        markDirty();
                    }
                },
                else => {},
            };
        }
        return;
    }
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
    markDirty();
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
    markDirty();
}

export fn anvil_close_pane() callconv(.c) void {
    if (!ready) return;
    mgr.closeFocused();
    relayout();
    markDirty();
}

/// dir: 0 left, 1 right, 2 up, 3 down.
export fn anvil_focus_dir(dir: c_int) callconv(.c) void {
    if (!ready) return;
    if (dir < 0 or dir > 3) return;
    mgr.focusNeighbor(workspaceRect(), @enumFromInt(dir), &pane_buf);
    markDirty();
}

/// Grow the focused pane toward `dir` (0 left, 1 right, 2 up, 3 down).
export fn anvil_resize_pane(dir: c_int) callconv(.c) void {
    if (!ready or zoomed) return;
    if (dir < 0 or dir > 3) return;
    mgr.resizeFocused(@enumFromInt(dir), 0.05);
    relayout();
    markDirty();
}

/// Reset the active tab's splits to even 50/50.
export fn anvil_balance_panes() callconv(.c) void {
    if (!ready or zoomed) return;
    mgr.balanceActive();
    relayout();
    markDirty();
}

/// Toggle zoom: the focused pane fills the workspace, hiding its siblings.
export fn anvil_zoom_toggle() callconv(.c) void {
    if (!ready) return;
    zoomed = !zoomed;
    relayout();
    markDirty();
}

export fn anvil_new_tab() callconv(.c) void {
    if (!ready) return;
    const ws = workspaceRect();
    const g = renderer.paneGrid(ws.w, ws.h);
    var cwd_buf: [1024]u8 = undefined;
    var cwd: []const u8 = "";
    if (mgr.focusedSession()) |s| {
        const c = s.term.cwd();
        const n = @min(c.len, cwd_buf.len);
        @memcpy(cwd_buf[0..n], c[0..n]);
        cwd = cwd_buf[0..n];
    }
    mgr.newTabCwd(g.rows, g.cols, cwd) catch return;
    applyCursorDefault();
    relayout();
    markDirty();
}

/// Copy the focused pane's cwd into `buf`, returning its length (0 if none).
/// Used by the shim to open a new window in the same directory.
export fn anvil_focused_cwd(buf: [*]u8, cap: usize) callconv(.c) usize {
    if (!ready) return 0;
    const s = mgr.focusedSession() orelse return 0;
    const cwd = s.term.cwd();
    const n = @min(cwd.len, cap);
    @memcpy(buf[0..n], cwd[0..n]);
    return n;
}

/// Copy the active tab's display label into `buf`, returning its length.
/// Used by the shim to set the NSWindow title.
export fn anvil_window_title(buf: [*]u8, cap: usize) callconv(.c) usize {
    if (!ready) return 0;
    var tmp: [256]u8 = undefined;
    const label = tabLabel(mgr.active_tab, &tmp);
    const n = @min(label.len, cap);
    @memcpy(buf[0..n], label[0..n]);
    return n;
}

/// delta: signed tab offset, wraps.
export fn anvil_cycle_tab(delta: c_int) callconv(.c) void {
    if (!ready) return;
    mgr.cycleTab(@intCast(delta));
    relayout();
    markDirty();
}

/// idx: zero-based tab index. Out-of-range is a no-op.
export fn anvil_select_tab(idx: c_int) callconv(.c) void {
    if (!ready or idx < 0) return;
    mgr.selectTab(@intCast(idx));
    relayout();
    markDirty();
}

export fn anvil_close_tab() callconv(.c) void {
    if (!ready) return;
    mgr.closeTab();
    relayout();
    markDirty();
}

export fn anvil_palette_toggle() callconv(.c) void {
    if (!ready) return;
    if (cpal.open) cpal.hide() else cpal.show();
    markDirty();
}

export fn anvil_palette_open() callconv(.c) c_int {
    return if (cpal.open) 1 else 0;
}

export fn anvil_palette_char(c: u8) callconv(.c) void {
    cpal.typeChar(c);
    markDirty();
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
    markDirty();
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
    markDirty();
}

export fn anvil_search_open() callconv(.c) c_int {
    return if (srch.open) 1 else 0;
}

export fn anvil_search_char(c: u8) callconv(.c) void {
    if (!ready) return;
    srch.typeChar(c, &focused().term);
    if (srch.current()) |m| jumpToMatch(m);
    markDirty();
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
    markDirty();
}

export fn anvil_help_toggle() callconv(.c) void {
    if (help_open) {
        help_open = false;
    } else {
        cpal.hide();
        srch.hide();
        help_open = true;
    }
    markDirty();
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
    markDirty();
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
    markDirty();
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
    markDirty();
}

export fn anvil_cfg_error_open() callconv(.c) c_int {
    return if (cfg_error_len > 0) 1 else 0;
}

export fn anvil_cfg_error_dismiss() callconv(.c) void {
    cfg_error_len = 0;
    markDirty();
}

export fn anvil_caldera_drawer_toggle() callconv(.c) void {
    if (!ready) return;
    caldera.get(&caldera_snap);
    if (caldera_drawer) {
        caldera_drawer = false;
        markDirty();
    } else {
        if (caldera_snap.runs == 0) return;
        if (caldera_sel >= caldera_snap.runs) caldera_sel = 0;
        cpal.hide();
        srch.hide();
        help_open = false;
        caldera_drawer = true;
        markDirty();
    }
}

export fn anvil_caldera_drawer_open() callconv(.c) c_int {
    return if (caldera_drawer) 1 else 0;
}

/// Toggle the right context drawer (RUNS / TRACE / AGENT). Bound to Cmd+J.
export fn anvil_drawer_toggle() callconv(.c) void {
    if (!ready) return;
    drawer_open = !drawer_open;
    relayout();
    markDirty();
}

/// Zen / terminal-only mode: hide all chrome (top bar, rail, sidebar, drawer,
/// status bar, panel inset) and let the terminal fill the window. Toggled from
/// the rail's terminal icon or Cmd+Return.
export fn anvil_zen_toggle() callconv(.c) void {
    if (!ready) return;
    zen = !zen;
    relayout();
    markDirty();
}

/// key: 0 esc/close, 1 up, 2 down.
export fn anvil_caldera_drawer_key(key: c_int) callconv(.c) void {
    switch (key) {
        0 => caldera_drawer = false,
        1 => {
            if (caldera_sel > 0) caldera_sel -= 1;
        },
        2 => {
            if (caldera_snap.runs > 0 and caldera_sel < caldera_snap.runs - 1)
                caldera_sel += 1;
        },
        else => {},
    }
    markDirty();
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
        .bar_h = barH(),
        .bg = th.bg.f32x3(),
        .bg_alpha = effectiveBackgroundOpacity(),
        .bar_color = th.bar.f32x3(),
        .sep_color = th.separator.f32x3(),
        .dividers = @ptrCast(&divider_rects),
        .divider_count = 0,
        .overlay = &overlay,
        .overlay_count = 0,
        .palette_text_count = 0,
        .pending = &renderer.atlas.pending,
        .pending_count = 0,
        .pane_ranges = &pane_range_buf,
        .pane_range_count = 0,
    };
    if (!ready) return;

    const ws = workspaceRect();
    const tree = mgr.activeTree() orelse return;
    const np = layoutPanes(&pane_buf);
    var n: usize = 0;
    var pr_n: usize = 0;
    const multi = np > 1;
    for (pane_buf[0..np]) |p| {
        const s = mgr.byId(p.id) orelse continue;
        const ox = p.rect.x + renderer.pad_x;
        const oy = p.rect.y + renderer.pad_x;
        const start = n;

        // Compute scroll animation offset for this pane.
        const target_f: f32 = @floatFromInt(s.term.view_offset);
        var off_f: f32 = target_f;
        if (cfg.scroll_smooth and s.id == mgr.focused) {
            off_f = animateScroll(target_f, s.id, s.term.grid.rows);
        } else if (!cfg.scroll_smooth) {
            scr_anim_init = false;
        }
        const base: usize = @intFromFloat(@floor(off_f));
        const frac: f32 = off_f - @as(f32, @floatFromInt(base));
        const y_shift: f32 = -frac * renderer.cell_h;

        n += renderer.buildInstances(&s.term, ox, oy, y_shift, base, instances[n..]);

        // Dim unfocused panes (only meaningful when split).
        if (multi and s.id != mgr.focused) {
            for (instances[start..n]) |*ci| ci.flags |= inst.flag_dim;
        }
        const show_live_cursor = s.id == mgr.focused and !copy_mode.open and
            s.term.view_offset == 0 and cursorVisible(&s.term);
        if (show_live_cursor) {
            var ci = renderer.cursorInstance(&s.term, ox, oy);
            if (cfg.cursor_smooth) {
                const anim = animateCursor(ci.x, ci.y, s.id);
                ci.x = anim.x;
                ci.y = anim.y;
            } else {
                cur_anim_init = false;
            }
            instances[n] = ci;
            n += 1;
        }
        if (s.id == mgr.focused and copy_mode.open) {
            if (n < instances.len) {
                instances[n] = copyModeCaret(th, ox, oy);
                n += 1;
            }
        }

        // Record the per-pane scissor range. Scissor rect: pane area clamped
        // to [bar_h, win_h] so the title bar is always protected.
        if (pr_n < max_panes) {
            const sx: f32 = p.rect.x;
            const sy_raw: f32 = p.rect.y;
            const sy: f32 = @max(sy_raw, barH());
            const sw: f32 = p.rect.w;
            const sh: f32 = @max(0, p.rect.h - (sy - sy_raw));
            pane_range_buf[pr_n] = .{
                .offset = @intCast(start),
                .count = @intCast(n - start),
                .x = sx,
                .y = sy,
                .w = sw,
                .h = sh,
            };
            pr_n += 1;
        }
    }
    // Base shell overlay rects (panel frame + header strip + status-bar bg),
    // emitted every frame before any modal rects. These draw in the overlay
    // pass (over terminal cells, under palette text).
    const base_ri = if (zen) 0 else emitShellRects(th, np);

    out.count = @intCast(n); // terminal cells only; chrome glyphs are palette text
    out.pane_range_count = @intCast(pr_n);
    out.divider_count = if (zoomed) 0 else @intCast(tree.dividers(ws, divider_px, &divider_rects));

    // Chrome glyphs (command bar, header strip, status bar) are drawn in the
    // palette-text pass — after the overlay rects — so the header and status
    // backgrounds do not paint over their own labels. All palette text is
    // contiguous starting at out.count: chrome first, then any modal/extra text.
    // Zen mode suppresses all persistent chrome; modal overlays still emit below.
    var pt = n;
    if (!zen) {
        pt += emitCommandBar(th, pt, np);
        pt += emitPanelHeaders(th, pt, np);
        pt += emitStatusBar(th, pt);
        pt += emitRail(pt);
        pt += emitSidebar(pt);
        pt += emitDrawer(pt);
    }
    const chrome_text: usize = pt - n;

    // Modal overlays append after the base shell rects (base_ri slots used).
    if (caldera_drawer) {
        caldera.get(&caldera_snap);
        if (caldera_snap.runs == 0 or caldera_sel >= caldera_snap.runs) {
            caldera_drawer = false;
            out.palette_text_count = @intCast(chrome_text);
            out.overlay_count = @intCast(base_ri);
        } else {
            const r = emitCalderaDrawerAt(th, pt, base_ri);
            out.palette_text_count = @intCast(chrome_text + r.text);
            out.overlay_count = @intCast(base_ri + r.rects);
        }
    } else if (help_open) {
        const r = emitHelpAt(th, pt, base_ri);
        out.palette_text_count = @intCast(chrome_text + r.text);
        out.overlay_count = @intCast(base_ri + r.rects);
    } else if (cpal.open) {
        const r = emitPaletteAt(th, pt, base_ri);
        out.palette_text_count = @intCast(chrome_text + r.text);
        out.overlay_count = @intCast(base_ri + r.rects);
    } else if (srch.open) {
        const r = emitSearchAt(th, pt, base_ri);
        out.palette_text_count = @intCast(chrome_text + r.text);
        out.overlay_count = @intCast(base_ri + r.rects);
    } else {
        const rails = emitRunRailsAt(th, base_ri);
        const ex = emitExitedPanes(th, base_ri + rails, pt);
        const ce = emitCfgError(th, base_ri + rails + ex.rects, pt + ex.text);
        out.overlay_count = @intCast(base_ri + rails + ex.rects + ce.rects);
        out.palette_text_count = @intCast(chrome_text + ex.text + ce.text);
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

/// Emit one IBM Plex Sans glyph at (x, y), returning its proportional advance
/// in device px. Sans glyphs live in the shared atlas under `sans_tag`; the
/// shim rasterizes + measures them. A blank space advances the pen but writes
/// no instance. Used for chrome nav/heading labels (BRAND: UI text = Plex Sans).
/// Chrome text/icon size relative to the terminal cell. Must match CHROME_SCALE
/// in shim.m: glyphs are rasterized smaller and top-left aligned in their cell.
const chrome_scale: f32 = 0.8;

/// Height (device px) of a small chrome glyph quad, snapped to whole pixels so
/// the top-of-cell raster maps 1:1 to the drawable (no subpixel softening).
fn chromeH() f32 {
    return @round(renderer.cell_h * chrome_scale);
}

/// Width (device px) of a small chrome mono icon cell.
fn chromeIconW() f32 {
    return @round(renderer.cell_w * chrome_scale);
}

fn putSans(n: *usize, x: f32, y: f32, fg: theme.Rgb, bg: theme.Rgb, cp: u21) f32 {
    const adv = anvil_sans_advance(@as(u32, cp));
    if (cp == ' ') return adv;
    if (n.* >= instances.len) return adv;
    const slot = renderer.atlas.slotForKey(atlasmod.sans_tag | @as(u32, cp));
    // Snap glyph origin to whole device pixels: fractional positions make the
    // nearest-sampled atlas glyph straddle pixel boundaries and look blurry.
    // Width tiles to the next glyph's snapped origin (round(x+adv) - round(x))
    // rather than the raw advance: opaque glyph quads must meet edge-to-edge, or
    // a fractional overlap lets the next glyph's bg wipe the previous glyph's
    // rightmost ink column (a hairline seam after wide glyphs like 'm').
    const x0 = @round(x);
    // Cap the quad at one atlas cell. The grid is sized to the mono cell, but a
    // few Sans glyphs ('m', 'w', caps) advance slightly wider at chrome scale; a
    // quad wider than one cell makes the shader's UV slice spill into the next
    // atlas slot and sample its neighbour's left edge — a phantom stroke that
    // makes ".md" read as ".mld". The advance past one cell becomes inter-glyph
    // background (the pen still advances the full `adv`), which is correct.
    instances[n.*] = .{
        .x = x0,
        .y = @round(y),
        .fg = fg.f32x4(),
        .bg = bg.f32x4(),
        .uv = atlasmod.Atlas.slotUV(slot),
        .w = @min(@round(x + adv) - x0, renderer.cell_w),
        .h = chromeH(),
    };
    n.* += 1;
    return adv;
}

/// Lay out a UTF-8 run in IBM Plex Sans starting at (x, y); returns total width.
fn putSansRun(n: *usize, x: f32, y: f32, fg: theme.Rgb, bg: theme.Rgb, text: []const u8) f32 {
    var pen = x;
    var it = std.unicode.Utf8View.initUnchecked(text).iterator();
    while (it.nextCodepoint()) |cp| {
        pen += putSans(n, pen, y, fg, bg, @intCast(cp));
    }
    return pen - x;
}

/// Lay out a Sans run, clipping glyphs that would cross `max_x`. Returns width.
fn putSansClip(n: *usize, x: f32, y: f32, fg: theme.Rgb, bg: theme.Rgb, text: []const u8, max_x: f32) f32 {
    var pen = x;
    var it = std.unicode.Utf8View.initUnchecked(text).iterator();
    while (it.nextCodepoint()) |cp| {
        const adv = anvil_sans_advance(@as(u32, cp));
        if (pen + adv > max_x) break;
        pen += putSans(n, pen, y, fg, bg, @intCast(cp));
    }
    return pen - x;
}

/// Total Sans advance (device px) of a UTF-8 run, for right-aligned layout.
fn sansWidth(text: []const u8) f32 {
    var w: f32 = 0;
    var it = std.unicode.Utf8View.initUnchecked(text).iterator();
    while (it.nextCodepoint()) |cp| w += anvil_sans_advance(@as(u32, cp));
    return w;
}

/// Emit one small chrome mono icon (Nerd Font / box-drawing) at (x, y), scaled
/// and top-left aligned via the chrome keyspace. Returns the icon cell width.
fn putChromeIcon(n: *usize, x: f32, y: f32, fg: theme.Rgb, bg: theme.Rgb, cp: u21) f32 {
    if (n.* >= instances.len) return chromeIconW();
    const slot = renderer.atlas.slotForKey(atlasmod.chrome_tag | @as(u32, cp));
    instances[n.*] = .{
        .x = @round(x),
        .y = @round(y),
        .fg = fg.f32x4(),
        .bg = bg.f32x4(),
        .uv = atlasmod.Atlas.slotUV(slot),
        .w = chromeIconW(),
        .h = chromeH(),
    };
    n.* += 1;
    return chromeIconW();
}

/// The breadcrumb path for the focused pane, $HOME-contracted to `~`, always
/// written into (and returned as a slice of) `buf` so the result outlives any
/// transient source. Prefers the shell-reported cwd (OSC 7); before the shell
/// reports, falls back to the process cwd so the breadcrumb is never blank.
fn breadcrumbText(buf: []u8) []const u8 {
    const raw = explorerPath(); // focused cwd (OSC 7), else process cwd
    if (raw.len == 0) return "";
    // Contract a leading $HOME to `~`, copying into buf either way.
    if (std.c.getenv("HOME")) |hc| {
        const home = std.mem.span(hc);
        if (home.len > 0 and std.mem.startsWith(u8, raw, home)) {
            const rest = raw[home.len..];
            if (1 + rest.len <= buf.len) {
                buf[0] = '~';
                @memcpy(buf[1 .. 1 + rest.len], rest);
                return buf[0 .. 1 + rest.len];
            }
        }
    }
    const n = @min(raw.len, buf.len);
    @memcpy(buf[0..n], raw[0..n]);
    return buf[0..n];
}

/// Total width (device px) of the right-aligned tab strip, including the one
/// Sans space of lead/trail padding each label carries. `tb` is scratch for the
/// label formatter.
fn tabsWidth(tb: []u8, pad: f32) f32 {
    var w: f32 = 0;
    var ti: usize = 0;
    while (ti < mgr.tabs.items.len) : (ti += 1) {
        w += sansWidth(tabLabel(ti, tb)) + 2 * pad;
        if (ti == mgr.active_tab) w += chromeIconW() * tab_dot_adv; // status dot
    }
    return w;
}

/// Centered breadcrumb pill: the focused pane's cwd ($HOME-contracted), centered
/// in the band between the "Anvil" wordmark and the right-aligned tab strip, in
/// a recessed pill. The geometry is shared by emitShellRects (which paints the
/// pill background) and emitCommandBar (which draws the text), so they always
/// agree. Returns null when there is no session or the band is too tight.
const Crumb = struct {
    text: []const u8,
    last: usize, // byte offset of the final path segment (drawn brighter)
    px: f32,
    py: f32,
    pw: f32,
    ph: f32,
    tx: f32, // text origin x (inside the left pad)
    ty: f32, // text origin y
    maxx: f32, // right clip bound for the text
};

fn crumbBox(buf: []u8) ?Crumb {
    if (mgr.focusedSession() == null) return null;
    const path = breadcrumbText(buf);
    if (path.len == 0) return null;
    const pad = anvil_sans_advance(' ');
    var tb: [128]u8 = undefined;
    const left = tab_inset_x + sansWidth("Anvil") + 2 * pad;
    const right = win_w - tab_strip_margin - tabsWidth(&tb, pad) - 2 * pad;
    const band = right - left;
    if (band < 80) return null; // too cramped to read: skip the pill
    const padx = pad * 1.25;
    const tw = sansWidth(path);
    const ph = chromeH() + 8;
    const pw = @min(tw + 2 * padx, band);
    const px = left + (band - pw) / 2;
    const py = (bar_h - ph) / 2;
    var last: usize = 0;
    if (std.mem.lastIndexOfScalar(u8, path, '/')) |i| last = i + 1;
    return .{
        .text = path,
        .last = last,
        .px = px,
        .py = py,
        .pw = pw,
        .ph = ph,
        .tx = px + padx,
        .ty = (bar_h - chromeH()) / 2,
        .maxx = px + pw - padx,
    };
}

/// Base shell rects (drawn every frame, before any modal rects): the snug
/// terminal panel frame + slim header strip + bottom status bar background.
/// Returns the number of rects written, which becomes the modal base offset.
/// Number of rows a drawer section shows, mirroring emitDrawer's fallback: an
/// empty section still draws one "none" row.
fn drawerSectionRows(count: usize) f32 {
    return @floatFromInt(@max(count, 1));
}

fn emitShellRects(th: *const theme.Theme, np: usize) usize {
    _ = th;
    _ = np;
    const lc = leftChromeW();
    const rc = rightChromeW();
    const px = lc + chrome.panel_pad;
    const ptop = bar_h + chrome.panel_pad;
    const pw = win_w - lc - rc - 2 * chrome.panel_pad;
    const pbot = win_h - chrome.status_bar_h - chrome.panel_pad_bottom;
    const ph = pbot - ptop;
    const hdr_div_y = ptop + chrome.header_strip_h;
    const border = chrome.line;
    const body_top = bar_h;
    const body_h = win_h - bar_h - chrome.status_bar_h;

    var ri: usize = 0;
    // Left chrome fills: activity rail + (optional) sidebar.
    putRect(ri, 0, body_top, chrome.rail_w, body_h, chrome.graphite); // rail bg
    ri += 1;
    // Active rail tool: a recessed charcoal cell with a mineral left-edge tick —
    // the operator-console "selected tool" marker.
    {
        const cell_y = body_top + 18 + @as(f32, @floatFromInt(rail_active)) * chrome.rail_w;
        putRect(ri, 0, cell_y, chrome.rail_w, chrome.rail_w, chrome.charcoal); // active cell
        ri += 1;
        putRect(ri, 0, cell_y, 2, chrome.rail_w, chrome.mineral); // mineral tick
        ri += 1;
    }
    // Hover affordance: tint the rail cell under the cursor so the clickable
    // tools read as live. Skip the active cell (already reads as selected).
    if (hoverRailSlot(hover_x, hover_y)) |hi| {
        if (hi != rail_active) {
            const hy = body_top + 18 + @as(f32, @floatFromInt(hi)) * chrome.rail_w;
            putRect(ri, 0, hy, chrome.rail_w, chrome.rail_w, chrome.hover);
            ri += 1;
        }
    }
    if (sidebar_open) {
        putRect(ri, chrome.rail_w, body_top, sidebar_w, body_h, chrome.charcoal); // sidebar bg
        ri += 1;
        putRect(ri, chrome.rail_w, body_top, 1, body_h, border); // rail│sidebar divider
        ri += 1;
        // Boxed section-header rules (Hermes/Honcho operator-console look): a
        // hairline under SESSIONS and EXPLORER turns each label into a header
        // band. Positions mirror emitSidebar's y math exactly.
        const sx = chrome.rail_w;
        const sess_rule_y = body_top + chrome.sidebar_header_h - 1;
        putRect(ri, sx, sess_rule_y, sidebar_w, 1, border);
        ri += 1;
        const n_sess: f32 = @floatFromInt(mgr.tabs.items.len);
        const exp_top = body_top + chrome.sidebar_header_h + 16 + n_sess * chrome.row_h;
        putRect(ri, sx, exp_top + chrome.sidebar_header_h - 1, sidebar_w, 1, border);
        ri += 1;
        // Active SESSIONS row highlight.
        if (mgr.tabs.items.len > 0) {
            const ry = body_top + chrome.sidebar_header_h + 8 +
                @as(f32, @floatFromInt(mgr.active_tab)) * chrome.row_h;
            putRect(ri, chrome.rail_w + 4, ry, sidebar_w - 8, chrome.row_h, chrome.ash_soft);
            ri += 1;
            putRect(ri, chrome.rail_w, ry, 2, chrome.row_h, chrome.mineral); // active tick
            ri += 1;
        }
        // Hover highlight on the row under the cursor (skip the active session).
        if (hoverSidebarRow(hover_x, hover_y)) |h| {
            const hov_y = if (h.kind == 0)
                body_top + chrome.sidebar_header_h + 8 + @as(f32, @floatFromInt(h.idx)) * chrome.row_h
            else
                exp_row_y0 + @as(f32, @floatFromInt(h.idx)) * chrome.row_h;
            const skip = h.kind == 0 and h.idx == mgr.active_tab;
            if (!skip and hov_y > 0) {
                putRect(ri, chrome.rail_w + 4, hov_y, sidebar_w - 8, chrome.row_h, chrome.hover);
                ri += 1;
            }
        }
    }
    putRect(ri, lc - 1, body_top, 1, body_h, border); // left-chrome right edge
    ri += 1;
    // Right context drawer: charcoal fill + left separator hairline.
    if (drawer_open) {
        const dx = win_w - chrome.drawer_w;
        putRect(ri, dx, body_top, chrome.drawer_w, body_h, chrome.charcoal); // drawer bg
        ri += 1;
        putRect(ri, dx, body_top, 1, body_h, border); // drawer left edge
        ri += 1;
        // Boxed section-header rules (RUNS / TRACE / AGENT). Positions mirror
        // emitDrawer's y math; refresh the snapshot here so the row counts that
        // drive section heights match the same frame's drawer text.
        caldera.get(&drawer_snap);
        const sh = chrome.sidebar_header_h;
        const runs_n = drawerSectionRows(drawer_snap.runs);
        const trace_ev: usize = if (drawer_snap.runs > 0) drawer_snap.details[0].event_count else 0;
        const trace_n = drawerSectionRows(trace_ev);
        var dy: f32 = body_top + 8; // emitDrawer starts ctx.y here
        putRect(ri, dx, dy + sh - 1, chrome.drawer_w, 1, border); // RUNS rule
        ri += 1;
        dy += sh + 4 + runs_n * chrome.row_h + 8;
        putRect(ri, dx, dy + sh - 1, chrome.drawer_w, 1, border); // TRACE rule
        ri += 1;
        dy += sh + 4 + trace_n * chrome.row_h + 8;
        putRect(ri, dx, dy + sh - 1, chrome.drawer_w, 1, border); // AGENT rule
        ri += 1;
    }
    // Fills first (painter order), hairlines on top.
    putRect(ri, 0, win_h - chrome.status_bar_h, win_w, chrome.status_bar_h, chrome.charcoal); // status bg
    ri += 1;
    putRect(ri, px, ptop, pw, chrome.header_strip_h, chrome.charcoal); // panel header strip
    ri += 1;
    putRect(ri, 0, bar_h - 1, win_w, 1, border); // command-bar underline
    ri += 1;
    putRect(ri, 0, win_h - chrome.status_bar_h, win_w, 1, border); // status-bar topline
    ri += 1;
    putRect(ri, px, hdr_div_y, pw, 1, border); // header/body divider
    ri += 1;
    putRect(ri, px, ptop, pw, 1, border); // panel top edge
    ri += 1;
    putRect(ri, px, pbot - 1, pw, 1, border); // panel bottom edge
    ri += 1;
    putRect(ri, px, ptop, 1, ph, border); // panel left edge
    ri += 1;
    putRect(ri, px + pw - 1, ptop, 1, ph, border); // panel right edge
    ri += 1;
    // Centered breadcrumb pill: a recessed graphite plate with a hairline
    // border, painted behind the command-bar text (which draws in the later
    // palette-text pass).
    var cbuf: [192]u8 = undefined;
    if (crumbBox(&cbuf)) |c| {
        putRect(ri, c.px, c.py, c.pw, c.ph, chrome.graphite); // recessed fill
        ri += 1;
        putRect(ri, c.px, c.py, c.pw, 1, border); // top
        ri += 1;
        putRect(ri, c.px, c.py + c.ph - 1, c.pw, 1, border); // bottom
        ri += 1;
        putRect(ri, c.px, c.py, 1, c.ph, border); // left
        ri += 1;
        putRect(ri, c.px + c.pw - 1, c.py, 1, c.ph, border); // right
        ri += 1;
    }
    return ri;
}

/// Top command bar glyphs: accent wordmark, cwd breadcrumb, right-aligned tab
/// labels. The bar background is drawn by the renderer from `bar_color`.
fn emitCommandBar(th: *const theme.Theme, start: usize, np: usize) usize {
    _ = np;
    const ly: f32 = (bar_h - chromeH()) / 2;
    const pad = anvil_sans_advance(' '); // one Sans space as the spacing unit
    var n = start;
    var x: f32 = tab_inset_x;

    // Basin mark: a circle with a filled lower hemisphere (BRAND.md), drawn in
    // the mono chrome face which carries the geometric-shapes glyph, then the
    // wordmark — accent initial + bone tail — in IBM Plex Sans.
    x += putChromeIcon(&n, x, ly, chrome.mineral, th.bar, 0x25D2); // ◒ Basin
    x += chromeIconW() * 0.45;
    x += putSans(&n, x, ly, chrome.mineral, th.bar, 'A');
    x += putSansRun(&n, x, ly, chrome.bone, th.bar, "nvil");

    // Tab strip, right-aligned, always shown. The active tab carries a leading
    // mineral status dot and reads in bone; the rest are alloy.
    var tb: [128]u8 = undefined;
    var tx = win_w - tab_strip_margin - tabsWidth(&tb, pad);
    var ti: usize = 0;
    while (ti < mgr.tabs.items.len) : (ti += 1) {
        const active = ti == mgr.active_tab;
        tx += pad;
        if (active) {
            _ = putChromeIcon(&n, tx, ly, chrome.mineral, th.bar, 0x25CF); // ●
            tx += chromeIconW() * tab_dot_adv;
        }
        const fg = if (active) chrome.bone else chrome.alloy;
        tx += putSansRun(&n, tx, ly, fg, th.bar, tabLabel(ti, &tb));
        tx += pad;
    }

    // Centered breadcrumb pill: prefix in alloy, final segment in bone. The
    // pill plate itself is painted in emitShellRects; here we draw its text.
    var cbuf: [192]u8 = undefined;
    if (crumbBox(&cbuf)) |c| {
        var pen = c.tx;
        pen += putSansClip(&n, pen, c.ty, chrome.alloy, chrome.graphite, c.text[0..c.last], c.maxx);
        _ = putSansClip(&n, pen, c.ty, chrome.bone, chrome.graphite, c.text[c.last..], c.maxx);
    }
    return n - start;
}

/// Panel header strip label: focused pane program/title — cwd basename.
fn emitPanelHeaders(th: *const theme.Theme, start: usize, np: usize) usize {
    _ = th;
    _ = np;
    const ptop = bar_h + chrome.panel_pad;
    const ly: f32 = ptop + (chrome.header_strip_h - chromeH()) / 2;
    const bg = chrome.charcoal;
    var n = start;
    var x: f32 = leftChromeW() + chrome.panel_pad + renderer.pad_x;
    const max_x = win_w - rightChromeW() - chrome.panel_pad - renderer.pad_x;

    const s = mgr.focusedSession() orelse return 0;
    var prog = s.term.title();
    if (prog.len == 0) prog = "zsh";
    x += putSansClip(&n, x, ly, chrome.mist, bg, prog, max_x);
    // Separator (em-dash) + cwd basename in alloy. Use explorerPath so the
    // basename is present before the shell reports a cwd via OSC 7.
    const base = basename(explorerPath());
    if (base.len > 0) {
        x += putSansRun(&n, x, ly, chrome.ash, bg, " — ");
        x += putSansClip(&n, x, ly, chrome.alloy, bg, base, max_x);
    }
    return n - start;
}

/// Bottom status bar glyphs over the charcoal status background: git branch and
/// kube context on the left, a semantic ready label on the right.
fn emitStatusBar(th: *const theme.Theme, start: usize) usize {
    _ = th;
    const iw = chromeIconW();
    const ly: f32 = win_h - chrome.status_bar_h + (chrome.status_bar_h - chromeH()) / 2;
    const gap = anvil_sans_advance(' ');
    const bg = chrome.charcoal;
    var n = start;
    var x: f32 = chrome.panel_pad + renderer.pad_x;

    if (mgr.focusedSession()) |s| ctx_chip.update(s.term.cwd());

    const branch = ctx_chip.branch();
    if (branch.len > 0) {
        x += putChromeIcon(&n, x, ly, chrome.mineral, bg, glyph_git);
        x += gap * 0.5;
        x += putSansRun(&n, x, ly, chrome.mist, bg, branch[0..@min(branch.len, chip_max_branch)]);
        x += iw + gap;
    }

    const kube = ctx_chip.kube();
    if (kube.len > 0) {
        x += putChromeIcon(&n, x, ly, chrome.agent, bg, glyph_kube);
        x += gap * 0.5;
        x += putSansRun(&n, x, ly, chrome.mist, bg, kube[0..@min(kube.len, chip_max_kube)]);
    }

    // Right: a muted info cluster — cursor position, encoding, app version —
    // mirroring the operator-console status line. The trailing READY pip keeps
    // a semantic-color accent at the far edge.
    var rrow: u16 = 1;
    var rcol: u16 = 1;
    if (mgr.focusedSession()) |s| {
        rrow = s.term.cy + 1;
        rcol = s.term.cx + 1;
    }
    // Segmented operator-console status line: thin pipe separators between the
    // cursor position, encoding, version, and the trailing READY pip.
    var rbuf: [96]u8 = undefined;
    const info = std.fmt.bufPrint(&rbuf, "Ln {d}, Col {d}  |  UTF-8  |  anvil 0.1.0  |  ", .{ rrow, rcol }) catch "anvil 0.1.0  |  ";
    const ready_lbl = "READY";
    const rx = win_w - chrome.panel_pad - renderer.pad_x - sansWidth(ready_lbl);
    _ = putSansRun(&n, rx, ly, chrome.verified, bg, ready_lbl);
    const ix = rx - sansWidth(info);
    _ = putSansRun(&n, ix, ly, chrome.alloy, bg, info);
    return n - start;
}

// Nerd Font glyphs for the left activity rail.
const rail_glyphs = [_]u21{
    0xf120, // terminal
    0xf07b, // folder / explorer
    0xf002, // search
    0xf0e7, // bolt / runs
    0xf013, // gear / settings
};
const rail_active: usize = 0; // highlighted rail entry (terminal, for now)

/// The activity-rail icon index under (x, y), or null. Mirrors railClick's
/// hit-test exactly so hover feedback and click dispatch always agree.
fn hoverRailSlot(x: f32, y: f32) ?usize {
    if (zen or x < 0 or x >= chrome.rail_w) return null;
    const top = bar_h + 18;
    if (y < top) return null;
    const idx: usize = @intFromFloat((y - top) / chrome.rail_w);
    if (idx >= rail_glyphs.len) return null;
    return idx;
}

/// Dispatch a click on the activity rail. Maps device-y to a rail icon and runs
/// its action: terminal→zen, explorer→sidebar, search→find, runs→drawer.
fn railClick(y: f32) void {
    const top = bar_h + 18;
    if (y < top) return;
    const idx: usize = @intFromFloat((y - top) / chrome.rail_w);
    if (idx >= rail_glyphs.len) return;
    switch (idx) {
        0 => anvil_zen_toggle(), // terminal: zen toggle
        1 => { // explorer: toggle sidebar
            sidebar_open = !sidebar_open;
            relayout();
            markDirty();
        },
        2 => anvil_search_toggle(), // search
        3 => anvil_drawer_toggle(), // runs
        else => {}, // settings: no-op for now
    }
}

/// Left activity rail: a vertical stack of Nerd Font icons. The active entry
/// is mineral; the rest are alloy. Drawn as palette text over the rail bg.
fn emitRail(start: usize) usize {
    const cx = (chrome.rail_w - chromeIconW()) / 2;
    var n = start;
    // Center each glyph in its 56px slot (slots start at bar_h+18, matching the
    // active-cell rect and railClick hit-testing).
    var y: f32 = bar_h + 18 + (chrome.rail_w - chromeH()) / 2;
    const hov = hoverRailSlot(hover_x, hover_y);
    for (rail_glyphs, 0..) |g, i| {
        const active = i == rail_active;
        const hovered = !active and hov != null and hov.? == i;
        // Hovered tool brightens toward mist; active stays mineral; rest alloy.
        const c = if (active) chrome.mineral else if (hovered) chrome.mist else chrome.alloy;
        // Glyph bg is opaque, so it must match the cell beneath: charcoal active
        // cell, hover tint when hovered, else the graphite rail.
        const gbg = if (active) chrome.charcoal else if (hovered) chrome.hover else chrome.graphite;
        _ = putChromeIcon(&n, cx, y, c, gbg, g);
        y += chrome.rail_w;
    }
    return n - start;
}

/// One EXPLORER entry: a file or directory name in the focused pane's cwd.
const ExpEntry = struct {
    name: [128]u8 = undefined,
    len: usize = 0,
    rel: [256]u8 = undefined, // path relative to the root (for open + expand toggle)
    rel_len: usize = 0,
    is_dir: bool = false,
    depth: u8 = 0,
    expanded: bool = false,
};
var exp_entries: [64]ExpEntry = undefined;
var exp_n: usize = 0;
var exp_cwd: [512]u8 = undefined;
var exp_cwd_len: usize = 0;
var exp_rescan: bool = true; // force a rescan (root changed or a dir was toggled)
/// Device-y of the first EXPLORER entry row, for hit-testing.
var exp_row_y0: f32 = 0;

// Directories the user has expanded, keyed by path relative to the root.
const max_expanded = 24;
var expanded_buf: [max_expanded][256]u8 = undefined;
var expanded_len: [max_expanded]usize = undefined;
var expanded_n: usize = 0;

fn isExpanded(rel: []const u8) bool {
    var i: usize = 0;
    while (i < expanded_n) : (i += 1) {
        if (std.mem.eql(u8, expanded_buf[i][0..expanded_len[i]], rel)) return true;
    }
    return false;
}

/// Expand a collapsed dir, or collapse an expanded one. Forces a rescan.
fn toggleExpanded(rel: []const u8) void {
    var i: usize = 0;
    while (i < expanded_n) : (i += 1) {
        if (std.mem.eql(u8, expanded_buf[i][0..expanded_len[i]], rel)) {
            expanded_n -= 1; // remove by swapping the last entry into the hole
            if (i != expanded_n) {
                expanded_buf[i] = expanded_buf[expanded_n];
                expanded_len[i] = expanded_len[expanded_n];
            }
            exp_rescan = true;
            return;
        }
    }
    if (rel.len > 256 or expanded_n >= max_expanded) return;
    @memcpy(expanded_buf[expanded_n][0..rel.len], rel);
    expanded_len[expanded_n] = rel.len;
    expanded_n += 1;
    exp_rescan = true;
}

const max_tree_depth = 8;
const Local = struct { name: [128]u8 = undefined, len: usize = 0, is_dir: bool = false };

fn localLess(_: void, a: Local, b: Local) bool {
    if (a.is_dir != b.is_dir) return a.is_dir; // dirs first
    return std.mem.lessThan(u8, a.name[0..a.len], b.name[0..b.len]);
}

/// Scan the root into a flattened depth-first tree in `exp_entries`, descending
/// only into expanded directories. A no-op when the root and expand state are
/// unchanged, so it is safe to call per frame.
fn scanExplorer(path: []const u8) void {
    if (!exp_rescan and path.len == exp_cwd_len and std.mem.eql(u8, path, exp_cwd[0..exp_cwd_len])) return;
    exp_rescan = false;
    exp_n = 0;
    const m = @min(path.len, exp_cwd.len);
    @memcpy(exp_cwd[0..m], path[0..m]);
    exp_cwd_len = m;
    if (path.len == 0 or path.len >= 480) return;
    walkDir(path, "", 0);
}

/// Recurse one directory level into `exp_entries`. `abs` is the absolute dir
/// path, `rel` its path relative to the root ("" at the root). Only real
/// directories (DT_DIR, never symlinks) are descended, and depth + entry count
/// are capped, so this can neither cycle nor overflow.
fn walkDir(abs: []const u8, rel: []const u8, depth: u8) void {
    if (depth >= max_tree_depth or exp_n >= exp_entries.len) return;
    var pbuf: [512:0]u8 = undefined;
    if (abs.len >= pbuf.len) return;
    @memcpy(pbuf[0..abs.len], abs);
    pbuf[abs.len] = 0;
    const dp = cdir.opendir(@ptrCast(&pbuf)) orelse return;
    defer _ = cdir.closedir(dp);

    // Collect this directory's visible entries, then sort dirs-first/alpha.
    var locals: [96]Local = undefined;
    var cnt: usize = 0;
    while (cdir.readdir(dp)) |raw| {
        if (cnt >= locals.len) break;
        const name_ptr: [*:0]const u8 = @ptrCast(&raw.*.d_name);
        const name = std.mem.span(name_ptr);
        if (name.len == 0 or name[0] == '.' or name.len > 128) continue; // skip hidden
        @memcpy(locals[cnt].name[0..name.len], name);
        locals[cnt].len = name.len;
        locals[cnt].is_dir = raw.*.d_type == cdir.DT_DIR;
        cnt += 1;
    }
    std.mem.sort(Local, locals[0..cnt], {}, localLess);

    for (locals[0..cnt]) |*lc| {
        if (exp_n >= exp_entries.len) return;
        const nm = lc.name[0..lc.len];
        // Compose rel = rel + "/" + nm (or just nm at the root).
        var childrel: [256]u8 = undefined;
        var rl: usize = 0;
        if (rel.len > 0) {
            if (rel.len + 1 + nm.len > childrel.len) continue;
            @memcpy(childrel[0..rel.len], rel);
            childrel[rel.len] = '/';
            rl = rel.len + 1;
        } else if (nm.len > childrel.len) continue;
        @memcpy(childrel[rl .. rl + nm.len], nm);
        rl += nm.len;

        const is_exp = lc.is_dir and isExpanded(childrel[0..rl]);
        var e = &exp_entries[exp_n];
        @memcpy(e.name[0..nm.len], nm);
        e.len = nm.len;
        @memcpy(e.rel[0..rl], childrel[0..rl]);
        e.rel_len = rl;
        e.is_dir = lc.is_dir;
        e.depth = depth;
        e.expanded = is_exp;
        exp_n += 1;

        if (is_exp) {
            var childabs: [512]u8 = undefined;
            const need = abs.len + 1 + nm.len;
            if (need <= childabs.len) {
                @memcpy(childabs[0..abs.len], abs);
                childabs[abs.len] = '/';
                @memcpy(childabs[abs.len + 1 .. need], nm);
                walkDir(childabs[0..need], childrel[0..rl], depth + 1);
            }
        }
    }
}

/// The directory the EXPLORER lists: the focused pane's OSC-7 cwd, or the
/// process working directory until the shell reports one.
var exp_pwd_buf: [512]u8 = undefined;
fn explorerPath() []const u8 {
    if (mgr.focusedSession()) |s| {
        const c = s.term.cwd();
        if (c.len > 0) return c;
    }
    const r = cdir.getcwd(&exp_pwd_buf, exp_pwd_buf.len);
    if (r == null) return "";
    return std.mem.span(@as([*:0]const u8, @ptrCast(&exp_pwd_buf)));
}

/// SESSIONS sidebar: a section header plus one row per tab (session). The
/// active session is bone with a verified dot; the rest are mist with an
/// alloy dot. The active-row highlight rect is drawn in emitShellRects.
fn emitSidebar(start: usize) usize {
    if (!sidebar_open) return 0;
    const iw = chromeIconW();
    const bg = chrome.charcoal;
    const x0 = chrome.rail_w + renderer.pad_x + 6;
    var n = start;

    // Section header (Plex Sans).
    const hy = bar_h + (chrome.sidebar_header_h - chromeH()) / 2;
    _ = putSansRun(&n, x0, hy, chrome.alloy, bg, "SESSIONS");

    // Session rows.
    const right = chrome.rail_w + sidebar_w - renderer.pad_x;
    var y: f32 = bar_h + chrome.sidebar_header_h + 8;
    var tb: [128]u8 = undefined;
    var ti: usize = 0;
    while (ti < mgr.tabs.items.len) : (ti += 1) {
        const active = ti == mgr.active_tab;
        const dot = if (active) chrome.verified else chrome.alloy;
        const fg = if (active) chrome.bone else chrome.mist;
        const ry = y + (chrome.row_h - chromeH()) / 2;
        var x = x0;
        x += putChromeIcon(&n, x, ry, dot, bg, 0x25CF);
        x += iw * 0.5;
        _ = putSansClip(&n, x, ry, fg, bg, tabLabel(ti, &tb), right);
        y += chrome.row_h;
    }

    // EXPLORER: flat listing of the focused pane's cwd. Dirs (alloy folder
    // glyph) sort first; files (ash file glyph) follow. Click opens (see mouse).
    y += 8;
    const ehy = y + (chrome.sidebar_header_h - chromeH()) / 2;
    _ = putSansRun(&n, x0, ehy, chrome.alloy, bg, "EXPLORER");
    y += chrome.sidebar_header_h + 4;

    scanExplorer(explorerPath());
    exp_row_y0 = y;
    for (exp_entries[0..exp_n]) |*ent| {
        if (y + chrome.row_h > win_h - chrome.status_bar_h) break;
        const ry = y + (chrome.row_h - chromeH()) / 2;
        const icon: u21 = if (ent.is_dir) 0xf07b else 0xf016; // folder / file
        const ic = if (ent.is_dir) chrome.alloy else chrome.ash;
        const fg = if (ent.is_dir) chrome.mist else chrome.alloy;
        const indent = @as(f32, @floatFromInt(ent.depth)) * (iw * 1.1);
        var x = x0 + indent;
        // Disclosure chevron for dirs; files reserve the slot so names align.
        if (ent.is_dir) {
            const chev: u21 = if (ent.expanded) 0x25BE else 0x25B8; // ▾ / ▸
            _ = putChromeIcon(&n, x, ry, chrome.alloy, bg, chev);
        }
        x += iw * 0.9;
        x += putChromeIcon(&n, x, ry, ic, bg, icon);
        x += iw * 0.5;
        _ = putSansClip(&n, x, ry, fg, bg, ent.name[0..ent.len], right);
        y += chrome.row_h;
    }
    return n - start;
}

/// Latest Caldera snapshot for the persistent drawer (refreshed per frame).
var drawer_snap: caldera.Snapshot = .{};

/// Drawer layout cursor: x origin, right clip edge, and running y position.
const DrawerCtx = struct {
    x0: f32,
    right: f32,
    y: f32,
};

/// Map a Caldera row kind to its semantic status color.
fn rowKindColor(kind: caldera.RowKind) theme.Rgb {
    return switch (kind) {
        .run_passed => chrome.verified,
        .run_open => chrome.mineral,
        .attn_warning => chrome.attention,
        .attn_error => chrome.ember,
    };
}

/// Render a section header ("RUNS") into the drawer and advance the cursor.
fn drawerHeader(n: *usize, ctx: *DrawerCtx, label: []const u8) void {
    const hy = ctx.y + (chrome.sidebar_header_h - chromeH()) / 2;
    _ = putSansRun(n, ctx.x0, hy, chrome.alloy, chrome.charcoal, label);
    ctx.y += chrome.sidebar_header_h + 4;
}

/// Render one drawer row: a status dot then a clipped label. Advances the cursor.
fn drawerRow(n: *usize, ctx: *DrawerCtx, dot: theme.Rgb, fg: theme.Rgb, label: []const u8) void {
    const bg = chrome.charcoal;
    const ry = ctx.y + (chrome.row_h - chromeH()) / 2;
    var x = ctx.x0;
    x += putChromeIcon(n, x, ry, dot, bg, 0x25CF);
    x += chromeIconW();
    _ = putSansClip(n, x, ry, fg, bg, label, ctx.right);
    ctx.y += chrome.row_h;
}

/// Right context drawer (Option C): RUNS and TRACE from the Caldera snapshot,
/// AGENT placeholder (wired in #79). Each section falls back to a dim "none".
fn emitDrawer(start: usize) usize {
    if (!drawer_open) return 0;
    caldera.get(&drawer_snap);
    var n = start;
    var ctx = DrawerCtx{
        .x0 = win_w - chrome.drawer_w + renderer.pad_x + 6,
        .right = win_w - renderer.pad_x,
        .y = bar_h + 8,
    };

    // RUNS: one row per Caldera run row, colored by status.
    drawerHeader(&n, &ctx, "RUNS");
    if (drawer_snap.runs == 0) {
        drawerRow(&n, &ctx, chrome.ash, chrome.ash, "none");
    } else {
        for (drawer_snap.rows[0..drawer_snap.runs]) |*r| {
            drawerRow(&n, &ctx, rowKindColor(r.kind), chrome.mist, r.slice());
        }
    }
    ctx.y += 8;

    // TRACE: events of the most recent run.
    drawerHeader(&n, &ctx, "TRACE");
    const trace_events: usize = if (drawer_snap.runs > 0) drawer_snap.details[0].event_count else 0;
    if (trace_events == 0) {
        drawerRow(&n, &ctx, chrome.ash, chrome.ash, "none");
    } else {
        for (drawer_snap.details[0].events[0..trace_events]) |*ev| {
            drawerRow(&n, &ctx, chrome.mineral, chrome.mist, ev.slice());
        }
    }
    ctx.y += 8;

    // AGENT: one row per run — "agent · step", dot colored by status.
    drawerHeader(&n, &ctx, "AGENT");
    if (drawer_snap.runs == 0) {
        drawerRow(&n, &ctx, chrome.ash, chrome.ash, "none");
    } else {
        for (drawer_snap.details[0..drawer_snap.runs]) |*d| {
            const passed = std.mem.eql(u8, d.statusSlice(), "passed");
            const dot = if (passed) chrome.verified else chrome.agent;
            var lbuf: [96]u8 = undefined;
            const label = std.fmt.bufPrint(&lbuf, "{s} · {s}", .{ d.agentSlice(), d.stepSlice() }) catch d.agentSlice();
            drawerRow(&n, &ctx, dot, chrome.mist, label);
        }
    }

    return n - start;
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
fn emitRunRailsAt(th: *const theme.Theme, base_ri: usize) usize {
    const RunBlock = @import("vt/terminal.zig").RunBlock;
    const max_rail = overlay.len / 7;
    var blocks: [64]RunBlock = undefined;
    var ri: usize = base_ri;
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
    return ri - base_ri;
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
    if (idx >= instances.len) return;
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
fn emitPaletteAt(th: *const theme.Theme, start: usize, base_ri: usize) struct { text: usize, rects: usize } {
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

    var ri: usize = base_ri;
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
    return .{ .text = n - start, .rects = ri - base_ri };
}

/// Lay out the search bar: a one-line input box at the top-right of the window
/// showing the query and the current/total match count.
fn emitSearchAt(th: *const theme.Theme, start: usize, base_ri: usize) struct { text: usize, rects: usize } {
    const cw = renderer.cell_w;
    const ch = renderer.cell_h;
    const pad = renderer.pad_x;

    var bw: f32 = 50 * cw;
    const max_bw = win_w * 0.9;
    if (bw > max_bw) bw = max_bw;
    const bx = @floor(win_w - bw - pad);
    const by = bar_h + pad;

    var ri: usize = base_ri;
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
    return .{ .text = n - start, .rects = ri - base_ri };
}

/// Lay out the Caldera run-detail drawer: a centered panel showing the selected
/// run's header fields and all event summaries in order.
fn emitCalderaDrawerAt(th: *const theme.Theme, start: usize, base_ri: usize) struct { text: usize, rects: usize } {
    const cw = renderer.cell_w;
    const ch = renderer.cell_h;
    const pad = renderer.pad_x;

    const inner_cols: usize = 64;
    const pw = @as(f32, @floatFromInt(inner_cols)) * cw + pad * 2;

    const d = &caldera_snap.details[caldera_sel];
    // header: agent, step, status; then blank; then events
    const event_rows = d.event_count;
    const total_rows: usize = 4 + event_rows; // title + 3 header lines + events
    const max_rows_f = @floor(win_h * 0.85 / ch);
    const max_rows_n: usize = @intFromFloat(max_rows_f);
    const visible_rows = @min(total_rows, max_rows_n);
    const ph = @as(f32, @floatFromInt(visible_rows)) * ch;

    const px = @floor((win_w - pw) / 2);
    const py = @floor(win_h * 0.12);

    var ri: usize = base_ri;
    putRect(ri, px - 1, py - 1, pw + 2, ph + 2, th.separator);
    ri += 1;
    putRect(ri, px, py, pw, ph, th.bar);
    ri += 1;

    var n = start;
    const tx = px + pad;
    var row: usize = 0;

    // Title: run index + agent name
    {
        var tbuf: [80]u8 = undefined;
        const title = std.fmt.bufPrint(&tbuf, "Run {d}: {s}", .{ caldera_sel + 1, d.agentSlice() }) catch "Run Detail";
        const ry = py + @as(f32, @floatFromInt(row)) * ch;
        for (title, 0..) |c, i| {
            if (n >= instances.len) break;
            putGlyph(n, tx + @as(f32, @floatFromInt(i)) * cw, ry, th.sel_fg, th.bar, c);
            n += 1;
        }
        row += 1;
    }

    if (row < visible_rows) {
        var lbuf: [80]u8 = undefined;
        const label = std.fmt.bufPrint(&lbuf, "step: {s}", .{d.stepSlice()}) catch "";
        const ry = py + @as(f32, @floatFromInt(row)) * ch;
        for (label, 0..) |c, i| {
            if (n >= instances.len) break;
            putGlyph(n, tx + @as(f32, @floatFromInt(i)) * cw, ry, th.ansi[6], th.bar, c);
            n += 1;
        }
        row += 1;
    }

    if (row < visible_rows) {
        var lbuf: [80]u8 = undefined;
        const passed = std.mem.eql(u8, d.statusSlice(), "passed");
        const status_color = if (passed) th.ansi[2] else th.ansi[3];
        const label = std.fmt.bufPrint(&lbuf, "status: {s}", .{d.statusSlice()}) catch "";
        const ry = py + @as(f32, @floatFromInt(row)) * ch;
        for (label, 0..) |c, i| {
            if (n >= instances.len) break;
            putGlyph(n, tx + @as(f32, @floatFromInt(i)) * cw, ry, status_color, th.bar, c);
            n += 1;
        }
        row += 1;
    }

    for (0..event_rows) |ei| {
        if (row >= visible_rows) break;
        const ev = &d.events[ei];
        var lbuf: [80]u8 = undefined;
        const label = std.fmt.bufPrint(&lbuf, "{d}. {s}", .{ ei + 1, ev.slice() }) catch "";
        const ry = py + @as(f32, @floatFromInt(row)) * ch;
        for (label, 0..) |c, i| {
            if (n >= instances.len) break;
            putGlyph(n, tx + @as(f32, @floatFromInt(i)) * cw, ry, th.fg, th.bar, c);
            n += 1;
        }
        row += 1;
    }

    return .{ .text = n - start, .rects = ri - base_ri };
}

/// Lay out the keyboard shortcut cheatsheet overlay: a centered panel listing
/// all key bindings grouped by section.
fn emitHelpAt(th: *const theme.Theme, start: usize, base_ri: usize) struct { text: usize, rects: usize } {
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

    var ri: usize = base_ri;
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
                if (n >= instances.len) break;
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

    return .{ .text = n - start, .rects = ri - base_ri };
}

test "animateCursor: snaps on first call (cur_anim_init=false)" {
    cur_anim_init = false;
    const r = animateCursor(100, 200, 1);
    try std.testing.expectEqual(@as(f32, 100), r.x);
    try std.testing.expectEqual(@as(f32, 200), r.y);
    try std.testing.expect(cur_anim_init);
    try std.testing.expectEqual(@as(f32, 100), cur_anim_x);
    try std.testing.expectEqual(@as(f32, 200), cur_anim_y);
}

test "animateCursor: snaps on id change" {
    cur_anim_init = true;
    cur_anim_id = 1;
    cur_anim_x = 50;
    cur_anim_y = 50;
    const r = animateCursor(200, 300, 2);
    try std.testing.expectEqual(@as(f32, 200), r.x);
    try std.testing.expectEqual(@as(f32, 300), r.y);
    try std.testing.expectEqual(@as(usize, 2), cur_anim_id);
}

test "animateCursor: snaps on large jump" {
    cur_anim_init = true;
    cur_anim_id = 5;
    cur_anim_x = 0;
    cur_anim_y = 0;
    // renderer.cell_w = 16 at init; 6 cells = 96px. Jump to 500px is > 96px.
    const r = animateCursor(500, 500, 5);
    try std.testing.expectEqual(@as(f32, 500), r.x);
    try std.testing.expectEqual(@as(f32, 500), r.y);
}

test "animateCursor: already at target returns target without moving" {
    cur_anim_init = true;
    cur_anim_id = 3;
    cur_anim_x = 80;
    cur_anim_y = 64;
    cur_anim_last_ms = nowMs();
    const r = animateCursor(80, 64, 3);
    try std.testing.expectEqual(@as(f32, 80), r.x);
    try std.testing.expectEqual(@as(f32, 64), r.y);
}

test "animateScroll: snaps on first call (scr_anim_init=false)" {
    scr_anim_init = false;
    const off = animateScroll(5, 1, 24);
    try std.testing.expectEqual(@as(f32, 5), off);
    try std.testing.expect(scr_anim_init);
    try std.testing.expectEqual(@as(f32, 5), scr_anim_off);
}

test "animateScroll: snaps on id change" {
    scr_anim_init = true;
    scr_anim_id = 1;
    scr_anim_off = 3;
    const off = animateScroll(5, 2, 24);
    try std.testing.expectEqual(@as(f32, 5), off);
    try std.testing.expectEqual(@as(usize, 2), scr_anim_id);
}

test "animateScroll: snaps on large jump (> grid.rows)" {
    scr_anim_init = true;
    scr_anim_id = 7;
    scr_anim_off = 0;
    // rows=24; jump of 25 lines > snap_lines=24
    const off = animateScroll(25, 7, 24);
    try std.testing.expectEqual(@as(f32, 25), off);
}

test "animateScroll: settles to exact target without calling markDirty" {
    scr_anim_init = true;
    scr_anim_id = 9;
    scr_anim_off = 10;
    scr_anim_last_ms = nowMs();
    frame_dirty = false;
    const off = animateScroll(10, 9, 24);
    try std.testing.expectEqual(@as(f32, 10), off);
    try std.testing.expect(!frame_dirty); // idle: no markDirty
}

test "scroll offset floor/frac split" {
    const off_f: f32 = 3.7;
    const base: usize = @intFromFloat(@floor(off_f));
    const frac: f32 = off_f - @as(f32, @floatFromInt(base));
    try std.testing.expectEqual(@as(usize, 3), base);
    try std.testing.expect(@abs(frac - 0.7) < 1e-5);
    const cell_h: f32 = 32;
    const y_shift: f32 = -frac * cell_h;
    try std.testing.expect(@abs(y_shift - (-22.4)) < 1e-3);
}

test "hoverRailSlot: hit-tests rail icons, rejects outside the rail" {
    zen = false;
    const top = bar_h + 18; // first slot top
    const w = chrome.rail_w;
    // Each 56px slot maps to its icon index.
    try std.testing.expectEqual(@as(?usize, 0), hoverRailSlot(20, top));
    try std.testing.expectEqual(@as(?usize, 1), hoverRailSlot(20, top + w));
    try std.testing.expectEqual(@as(?usize, rail_glyphs.len - 1), hoverRailSlot(20, top + w * (rail_glyphs.len - 1)));
    // Below the last icon, above the first, right of the rail, and left of 0 miss.
    try std.testing.expectEqual(@as(?usize, null), hoverRailSlot(20, top + w * rail_glyphs.len));
    try std.testing.expectEqual(@as(?usize, null), hoverRailSlot(20, top - 1));
    try std.testing.expectEqual(@as(?usize, null), hoverRailSlot(chrome.rail_w, top));
    try std.testing.expectEqual(@as(?usize, null), hoverRailSlot(-1, top));
    // Zen mode hides the rail entirely.
    zen = true;
    try std.testing.expectEqual(@as(?usize, null), hoverRailSlot(20, top));
    zen = false;
}
