//! Anvil — M1 entry point. Wires the terminal model, the PTY, the
//! Metal renderer, the CoreGraphics rasterizer, and AppKit input into a
//! single-pane GPU terminal.

const std = @import("std");
const objc = @import("objc");
const c = objc.c;

const term = @import("terminal/terminal.zig");
const color = @import("render/color.zig");
const Font = @import("render/font.zig").Font;
const Raster = @import("render/raster.zig").Raster;
const Renderer = @import("render/metal.zig").Renderer;
const Theme = @import("config/theme.zig").Theme;
const theme_mod = @import("config/theme.zig");
const cfg_mod = @import("config/config.zig");
const keys = @import("app/keys.zig");
const shell_integration = @import("app/shell_integration.zig");
const tabs_mod = @import("app/tab.zig");
const tabbar = @import("render/tabbar.zig");
const hud_mod = @import("render/hud.zig");
const git = @import("prompt/git.zig");
const Search = @import("terminal/search.zig").Search;
const searchbar = @import("render/searchbar.zig");
const webview_mod = @import("webview/webview.zig");
const palette_mod = @import("app/palette.zig");
const bridge = @import("ipc/bridge.zig");
const selection_mod = @import("app/selection.zig");
const Selection = selection_mod.Selection;

const CGPoint = extern struct { x: f64, y: f64 };
const CGSize = extern struct { width: f64, height: f64 };
const CGRect = extern struct { origin: CGPoint, size: CGSize };

const app_icon_png = @embedFile("assets/app-icon.png");
const palette_html: [:0]const u8 = @embedFile("palette_html");

var config_path_buf: [std.fs.max_path_bytes]u8 = undefined;

// --- animation helpers ---------------------------------------------------

/// Exponential approach: move `cur` a fraction `rate` toward `target`.
fn approach(cur: f32, target: f32, rate: f32) f32 {
    return cur + (target - cur) * rate;
}

fn smoothstep(t: f32) f32 {
    const clamped = std.math.clamp(t, 0.0, 1.0);
    return clamped * clamped * (3.0 - 2.0 * clamped);
}

/// Cursor opacity for blink phase `p` in [0,1): solid, fade out, dim hold,
/// fade in — a soft blink rather than a hard on/off toggle.
fn cursorOpacity(p: f32) f32 {
    if (p < 0.50) return 1.0;
    if (p < 0.62) return 1.0 - smoothstep((p - 0.50) / 0.12);
    if (p < 0.88) return 0.0;
    return smoothstep((p - 0.88) / 0.12);
}

fn windowIsKey() bool {
    const win = g.view.msgSend(objc.Object, "window", .{});
    if (win.value == null) return false;
    return win.msgSend(bool, "isKeyWindow", .{});
}

/// Snap all animation state to the active tab's current values — used on tab
/// switch, resize, and startup so those discontinuities never animate.
fn snapAnim() void {
    const t = g.tabs.current();
    const cur = t.terminal.cursor();
    g.cursor_ax = @floatFromInt(cur.x);
    g.cursor_ay = @floatFromInt(cur.y);
    g.scroll_pos = @floatFromInt(t.terminal.viewportOffset());
    g.overscroll = 0;
    g.overscroll_target = 0;
}

// Uniform inset (device pixels) between the window edge and the terminal grid.
// The margin shows the background color; the grid simply has fewer cells.
const grid_pad: usize = 22;

// HUD data refresh rate: once every N ticks (≈60fps timer → ~1 s).
const hud_refresh_ticks: u32 = 60;

// Reused scratch buffer for draining per-tab PTY bytes each tick.
// One tab is drained at a time, so this is safe as a module global.
var feed_scratch: [256 * 1024]u8 = undefined;

const App = struct {
    alloc: std.mem.Allocator,
    tabs: tabs_mod.TabManager,
    font: Font,
    raster: Raster,
    renderer: Renderer,
    nsapp: objc.Object,
    view: objc.Object,
    scale: f64,
    dirty: bool,
    theme: Theme,
    cursor_cfg: cfg_mod.Config.CursorCfg,
    blink_phase: f32 = 0, // 0..1 cursor blink-fade phase
    cursor_ax: f32 = 0, // animated cursor column (viewport cells)
    cursor_ay: f32 = 0, // animated cursor row (viewport cells)
    scroll_pos: f32 = 0, // displayed viewport offset, fractional, driven by the gesture
    overscroll: f32 = 0, // rubber-band pull past a history edge, in device pixels
    overscroll_target: f32 = 0, // where the rubber-band is easing toward
    config: cfg_mod.Loaded,
    watcher: cfg_mod.Watcher,
    keys_new: ?cfg_mod.Chord = null,
    keys_close: ?cfg_mod.Chord = null,
    keys_next: ?cfg_mod.Chord = null,
    keys_prev: ?cfg_mod.Chord = null,
    keys_jump: [9]?cfg_mod.Chord = [_]?cfg_mod.Chord{null} ** 9,
    keys_search_open: ?cfg_mod.Chord = null,
    keys_search_next: ?cfg_mod.Chord = null,
    keys_search_prev: ?cfg_mod.Chord = null,
    keys_hud_toggle: ?cfg_mod.Chord = null,
    search: Search,
    search_open: bool = false,
    hud_visible: bool = true,
    hud: hud_mod.Hud = .{},
    hud_tick: u32 = 0, // counts up to hud_refresh_ticks then resets
    webview: webview_mod.Webview,
    palette: palette_mod.Palette = .{},
    system_dark: bool = false,
    selection: Selection = .{},
};
var g: App = undefined;

// --- Objective-C method implementations ----------------------------------

fn imShouldTerminate(_: c.id, _: c.SEL, _: c.id) callconv(.c) bool {
    return true;
}

fn imWindowDidResize(_: c.id, _: c.SEL, _: c.id) callconv(.c) void {
    onResize();
}

fn imViewDidEndLiveResize(_: c.id, _: c.SEL) callconv(.c) void {
    onEndLiveResize();
}

fn imTick(_: c.id, _: c.SEL, _: c.id) callconv(.c) void {
    onTick();
}

fn imAcceptsFirstResponder(_: c.id, _: c.SEL) callconv(.c) bool {
    return true;
}

fn imKeyDown(_: c.id, _: c.SEL, ev: c.id) callconv(.c) void {
    onKeyDown(.{ .value = ev });
}

fn imScrollWheel(_: c.id, _: c.SEL, ev: c.id) callconv(.c) void {
    onScroll(.{ .value = ev });
}

fn imMouseDown(_: c.id, _: c.SEL, ev: c.id) callconv(.c) void {
    onMouseDown(.{ .value = ev });
}

fn imMouseDragged(_: c.id, _: c.SEL, ev: c.id) callconv(.c) void {
    onMouseDragged(.{ .value = ev });
}

fn imMouseUp(_: c.id, _: c.SEL, ev: c.id) callconv(.c) void {
    onMouseUp(.{ .value = ev });
}

fn imPerformKeyEquivalent(_: c.id, _: c.SEL, ev: c.id) callconv(.c) bool {
    return onPerformKeyEquivalent(.{ .value = ev });
}

// --- event handling ------------------------------------------------------

fn applyConfig(new_loaded: cfg_mod.Loaded) void {
    const nl = new_loaded;
    const nc = nl.config;
    g.theme = theme_mod.resolve(effectiveThemeName(g.nsapp, nc.theme), nc.theme_overrides);
    g.renderer.setClearColor(g.theme.background); // keep the GPU clear in sync
    g.cursor_cfg = nc.cursor;
    loadKeybindings(nc.keybindings);
    g.dirty = true;
    g.config.deinit(); // free the previous config's arena
    g.config = nl; // the new arena owns the strings resolve() just read
}

/// Parse the config's keybinding strings into matchable chords.
fn loadKeybindings(kb: cfg_mod.Keybindings) void {
    g.keys_new = cfg_mod.parseChord(kb.new_tab);
    g.keys_close = cfg_mod.parseChord(kb.close_tab);
    g.keys_next = cfg_mod.parseChord(kb.next_tab);
    g.keys_prev = cfg_mod.parseChord(kb.prev_tab);
    const strs = [9][]const u8{
        kb.tab_1, kb.tab_2, kb.tab_3, kb.tab_4, kb.tab_5,
        kb.tab_6, kb.tab_7, kb.tab_8, kb.tab_9,
    };
    for (strs, 0..) |s, i| g.keys_jump[i] = cfg_mod.parseChord(s);
    g.keys_search_open = cfg_mod.parseChord(kb.search_open);
    g.keys_search_next = cfg_mod.parseChord(kb.search_next);
    g.keys_search_prev = cfg_mod.parseChord(kb.search_prev);
    g.keys_hud_toggle = cfg_mod.parseChord(kb.hud_toggle);
}

// --- command palette -----------------------------------------------------

fn formatHex(buf: *[8]u8, rgb: [3]u8) []const u8 {
    return std.fmt.bufPrint(buf, "#{x:0>2}{x:0>2}{x:0>2}", .{ rgb[0], rgb[1], rgb[2] }) catch "#000000";
}

fn sendShow() void {
    var cmds: [palette_mod.catalog.len]bridge.Command = undefined;
    for (palette_mod.catalog, 0..) |e, i| {
        cmds[i] = .{ .id = e.id, .title = e.title, .subtitle = e.subtitle };
    }
    var bg: [8]u8 = undefined;
    var fg: [8]u8 = undefined;
    var ac: [8]u8 = undefined;
    const json = bridge.encode(g.alloc, .{ .show = .{
        .commands = &cmds,
        .theme = .{
            .background = formatHex(&bg, g.theme.background),
            .foreground = formatHex(&fg, g.theme.foreground),
            .accent = formatHex(&ac, g.theme.accent),
        },
    } }) catch return;
    defer g.alloc.free(json);
    const js = std.fmt.allocPrintSentinel(g.alloc, "window.anvil.receive({s});", .{json}, 0) catch return;
    defer g.alloc.free(js);
    g.webview.evalJS(js);
}

fn summonPalette() void {
    if (g.palette.summon()) {
        sendShow();
        g.webview.show();
    }
    // Not ready yet: handleReady() will send `show` and reveal the webview.
}

fn handleReady() void {
    if (g.palette.onReady()) {
        sendShow();
        g.webview.show();
    }
}

fn hidePalette() void {
    g.palette.dismiss();
    const json = bridge.encode(g.alloc, .hide) catch return;
    defer g.alloc.free(json);
    const js = std.fmt.allocPrintSentinel(g.alloc, "window.anvil.receive({s});", .{json}, 0) catch return;
    defer g.alloc.free(js);
    g.webview.evalJS(js);
    g.webview.hide(g.view);
}

fn setTheme(name: []const u8) void {
    g.theme = theme_mod.byName(name);
    g.renderer.setClearColor(g.theme.background);
    g.dirty = true;
}

fn runAction(action: palette_mod.Action) void {
    switch (action) {
        .theme_dark => setTheme("mineral-dark"),
        .theme_light => setTheme("mineral-light"),
        .config_reload => {
            if (g.watcher.path.len > 0) {
                applyConfig(cfg_mod.load(g.alloc, g.watcher.path));
            }
        },
        .clear_screen => {
            g.tabs.current().terminal.feed("\x1b[H\x1b[2J");
            g.dirty = true;
        },
        .scroll_top => {
            const t = &g.tabs.current().terminal;
            t.scrollViewport(@intCast(t.scrollbackLen()));
            g.scroll_pos = @floatFromInt(t.viewportOffset());
            g.overscroll_target = bounceImpulse();
            g.dirty = true;
        },
        .scroll_bottom => {
            g.tabs.current().terminal.scrollToBottom();
            g.scroll_pos = 0;
            g.overscroll_target = -bounceImpulse();
            g.dirty = true;
        },
        .app_quit => g.nsapp.msgSend(void, "terminate:", .{@as(c.id, null)}),
        .hud_toggle => {
            g.hud_visible = !g.hud_visible;
            resizeAllTabs();
            g.dirty = true;
        },
    }
}

fn handleWebMessage(json: []const u8) void {
    const msg = bridge.decode(g.alloc, json) catch |e| {
        std.debug.print("anvil: webview message decode failed: {s}\n", .{@errorName(e)});
        return;
    };
    defer msg.deinit(g.alloc);
    switch (msg) {
        .ready => handleReady(),
        .dismiss => hidePalette(),
        .invoke => |id| {
            if (palette_mod.actionForId(id)) |action| {
                hidePalette();
                runAction(action);
            } else {
                std.debug.print("anvil: unknown command id: {s}\n", .{id});
            }
        },
    }
}

fn onTick() void {
    if (g.watcher.path.len > 0) {
        if (g.watcher.poll(g.alloc)) |new_loaded| applyConfig(new_loaded);
    }

    if (std.mem.eql(u8, g.config.config.theme, "system")) {
        const now_dark = systemIsDark(g.nsapp);
        if (now_dark != g.system_dark) {
            g.system_dark = now_dark;
            g.theme = theme_mod.resolve(
                effectiveThemeName(g.nsapp, "system"),
                g.config.config.theme_overrides,
            );
            g.renderer.setClearColor(g.theme.background);
            g.dirty = true;
        }
    }

    // Drain every tab so background tabs stay current; render only the active.
    var i: usize = 0;
    var any_dead = false;
    while (i < g.tabs.count()) : (i += 1) {
        const tab = g.tabs.tabs.items[i];
        const bytes = tab.drain(&feed_scratch);
        if (bytes.len > 0) {
            tab.terminal.feed(bytes);
            if (i == g.tabs.active) {
                g.dirty = true;
                if (g.search_open) g.search.rescan(&g.tabs.current().terminal);
            }
        }
        if (tab.isDead()) any_dead = true;
    }

    // Blink fade — only while the window is focused, so an unfocused window
    // does not burn 60fps. When blink is off in config, the cursor is solid.
    if (g.cursor_cfg.blink and windowIsKey()) {
        g.blink_phase += 1.0 / 64.0;
        if (g.blink_phase >= 1.0) g.blink_phase -= 1.0;
        g.dirty = true;
    } else if (g.blink_phase != 0) {
        g.blink_phase = 0;
        g.dirty = true;
    }

    // Cursor glide.
    const t = g.tabs.current();
    const cur = t.terminal.cursor();
    const tx: f32 = @floatFromInt(cur.x);
    const ty: f32 = @floatFromInt(cur.y);
    if (@abs(tx - g.cursor_ax) > 0.002 or @abs(ty - g.cursor_ay) > 0.002) {
        g.cursor_ax = approach(g.cursor_ax, tx, 0.45);
        g.cursor_ay = approach(g.cursor_ay, ty, 0.45);
        if (@abs(tx - g.cursor_ax) <= 0.002) g.cursor_ax = tx;
        if (@abs(ty - g.cursor_ay) <= 0.002) g.cursor_ay = ty;
        g.dirty = true;
    }

    // Rubber-band: ease the overscroll toward its target, which itself decays
    // to zero — so the pull-in and the spring-back are both smooth (no snap).
    if (g.overscroll != 0 or g.overscroll_target != 0) {
        g.overscroll_target = approach(g.overscroll_target, 0, 0.32);
        g.overscroll = approach(g.overscroll, g.overscroll_target, 0.55);
        if (@abs(g.overscroll_target) < 0.5) g.overscroll_target = 0;
        if (@abs(g.overscroll) < 0.5 and g.overscroll_target == 0) g.overscroll = 0;
        g.dirty = true;
    }

    // HUD data refresh — throttled to ~once per second.
    if (g.hud_visible) {
        g.hud_tick += 1;
        if (g.hud_tick >= hud_refresh_ticks) {
            g.hud_tick = 0;
            refreshHud();
        }
    }

    if (g.dirty) {
        renderFrame();
        g.dirty = false;
    }

    if (any_dead) closeDeadTabs();
}

/// Close any tab whose shell has exited. Terminates the app if none remain.
fn closeDeadTabs() void {
    const bar_before = topBarRows();
    var i: usize = 0;
    while (i < g.tabs.count()) {
        if (g.tabs.tabs.items[i].isDead()) {
            if (!g.tabs.closeAt(i)) {
                g.nsapp.msgSend(void, "terminate:", .{@as(c.id, null)});
                return;
            }
            // The list shifted; do not advance i.
        } else i += 1;
    }
    if (topBarRows() != bar_before) resizeAllTabs();
    snapAnim();
    g.dirty = true;
}

/// Size every tab's terminal + pty to the current window, minus the bar row.
/// Resize the GPU surface — raster bitmap, Metal drawable, webview frame — to
/// the window's current pixel size. Cheap; safe to run on every resize tick.
fn resizeSurface() void {
    const b = g.view.msgSend(CGRect, "bounds", .{});
    const dw: usize = @intFromFloat(@max(b.size.width * g.scale, 1));
    const dh: usize = @intFromFloat(@max(b.size.height * g.scale, 1));
    g.raster.resize(dw, dh) catch {};
    g.renderer.resize(dw, dh);
    g.webview.setFrame(b.size.width, b.size.height);
}

/// Reflow every tab's terminal + pty to the window's current cell grid. This
/// sends SIGWINCH and makes the shell redraw, so it runs once when a resize
/// settles — not on every live-resize tick (that spammed the shell and made
/// the text jitter).
fn resizeAllTabs() void {
    const b = g.view.msgSend(CGRect, "bounds", .{});
    const dw: usize = @intFromFloat(@max(b.size.width * g.scale, 1));
    const dh: usize = @intFromFloat(@max(b.size.height * g.scale, 1));
    const cw: usize = @intFromFloat(g.font.metrics.cell_w);
    const ch: usize = @intFromFloat(g.font.metrics.cell_h);
    const raw_cols = @max((dw -| 2 * grid_pad) / cw, 1);
    const hud_reserve = if (g.hud_visible) hud_mod.hud_cols + 1 else 0;
    const cols = @max(raw_cols -| hud_reserve, 1);
    const total_rows = @max((dh -| 2 * grid_pad) / ch, 1);
    const rows = @max(total_rows -| topBarRows() -| bottomBarRows(), 1);

    for (g.tabs.tabs.items) |tab| {
        tab.terminal.resize(cols, rows);
        tab.pty.resize(@intCast(cols), @intCast(rows));
    }
    g.selection.clear();
    snapAnim();
    g.dirty = true;
}

fn viewInLiveResize() bool {
    return g.view.msgSend(bool, "inLiveResize", .{});
}

fn onResize() void {
    resizeSurface();
    // During a live drag only the pixel surface tracks the window; the cell
    // grid is left alone so the shell is not reflowed on every frame. The
    // grid is reflowed once when the drag ends (viewDidEndLiveResize).
    if (!viewInLiveResize()) resizeAllTabs();
    renderFrame();
}

/// A live-resize drag finished — reflow the grid once, now that the size has
/// settled.
fn onEndLiveResize() void {
    resizeAllTabs();
    renderFrame();
}

const Pressed = struct { key: keys.Key, mods: keys.Mods };

fn extractKey(event: objc.Object) ?Pressed {
    const flags = event.msgSend(c_ulong, "modifierFlags", .{});
    const mods: keys.Mods = .{
        .shift = flags & (1 << 17) != 0,
        .control = flags & (1 << 18) != 0,
        .option = flags & (1 << 19) != 0,
        .command = flags & (1 << 20) != 0,
    };
    if (mods.command) return null; // leave Cmd shortcuts to the system

    const keycode = event.msgSend(u16, "keyCode", .{});
    const named: ?keys.Key = switch (keycode) {
        36, 76 => .enter,
        48 => .tab,
        51 => .backspace,
        53 => .escape,
        123 => .left,
        124 => .right,
        125 => .down,
        126 => .up,
        115 => .home,
        119 => .end,
        116 => .page_up,
        121 => .page_down,
        117 => .delete,
        else => null,
    };
    if (named) |key| return .{ .key = key, .mods = mods };

    const src = if (mods.control or mods.option)
        event.msgSend(objc.Object, "charactersIgnoringModifiers", .{})
    else
        event.msgSend(objc.Object, "characters", .{});
    const cp = firstCodepoint(src) orelse return null;
    return .{ .key = .{ .text = cp }, .mods = mods };
}

fn firstCodepoint(nsstr: objc.Object) ?u21 {
    if (nsstr.value == null) return null;
    const cstr = nsstr.msgSend(?[*:0]const u8, "UTF8String", .{}) orelse return null;
    const s = std.mem.span(cstr);
    if (s.len == 0) return null;
    const len = std.unicode.utf8ByteSequenceLength(s[0]) catch return null;
    if (s.len < len) return null;
    return std.unicode.utf8Decode(s[0..len]) catch null;
}

/// True when the macOS system appearance is dark.
fn systemIsDark(nsapp: objc.Object) bool {
    const appearance = nsapp.msgSend(objc.Object, "effectiveAppearance", .{});
    if (appearance.value == null) return true;
    const name = appearance.msgSend(objc.Object, "name", .{});
    const cstr = name.msgSend(?[*:0]const u8, "UTF8String", .{}) orelse return true;
    return std.mem.indexOf(u8, std.mem.span(cstr), "Dark") != null;
}

/// Resolve the configured theme name, mapping "system" to the dark/light
/// Mineral theme that matches the current macOS appearance.
fn effectiveThemeName(nsapp: objc.Object, cfg_theme: []const u8) []const u8 {
    if (std.mem.eql(u8, cfg_theme, "system")) {
        return if (systemIsDark(nsapp)) "mineral-dark" else "mineral-light";
    }
    return cfg_theme;
}

/// Lowercase an ASCII letter codepoint. Non-ASCII codepoints are returned
/// unchanged. Avoids the `& 0x7f` truncation that would misidentify high
/// codepoints as ASCII characters.
fn asciiLowerCp(cp: u21) u21 {
    return if (cp >= 'A' and cp <= 'Z') cp + 32 else cp;
}

/// Does this AppKit key event match `chord`?
fn chordMatches(chord: cfg_mod.Chord, mods: keys.Mods, cp: u21) bool {
    return chord.cmd == mods.command and
        chord.shift == mods.shift and
        chord.ctrl == mods.control and
        chord.opt == mods.option and
        chord.key == asciiLowerCp(cp);
}

/// If the event triggers a tab action, run it and return true (consume it).
fn handleTabKey(mods: keys.Mods, cp: u21) bool {
    if (g.keys_new) |ch| if (chordMatches(ch, mods, cp)) {
        addTab(currentCwd());
        return true;
    };
    if (g.keys_close) |ch| if (chordMatches(ch, mods, cp)) {
        const bar_before = topBarRows();
        if (!g.tabs.closeActive()) {
            g.nsapp.msgSend(void, "terminate:", .{@as(c.id, null)});
        } else {
            // Only reflow when bar visibility actually changed (2->1 tabs).
            // Closing among 3+ tabs leaves the grid size unchanged — an
            // unconditional resize would SIGWINCH every surviving shell.
            if (topBarRows() != bar_before) resizeAllTabs();
            g.dirty = true;
        }
        return true;
    };
    if (g.keys_next) |ch| if (chordMatches(ch, mods, cp)) {
        closeSearch();
        g.tabs.next();
        snapAnim();
        g.dirty = true;
        return true;
    };
    if (g.keys_prev) |ch| if (chordMatches(ch, mods, cp)) {
        closeSearch();
        g.tabs.prev();
        snapAnim();
        g.dirty = true;
        return true;
    };
    for (g.keys_jump, 0..) |maybe, i| {
        if (maybe) |ch| if (chordMatches(ch, mods, cp)) {
            closeSearch();
            g.tabs.switchTo(i);
            snapAnim();
            g.dirty = true;
            return true;
        };
    }
    if (g.keys_search_open) |chd| if (chordMatches(chd, mods, cp)) {
        openSearch();
        return true;
    };
    if (g.keys_search_next) |chd| if (chordMatches(chd, mods, cp)) {
        if (!g.search_open) openSearch();
        g.search.next();
        scrollToCurrentMatch();
        g.dirty = true;
        return true;
    };
    if (g.keys_search_prev) |chd| if (chordMatches(chd, mods, cp)) {
        if (!g.search_open) openSearch();
        g.search.prev();
        scrollToCurrentMatch();
        g.dirty = true;
        return true;
    };
    if (g.keys_hud_toggle) |chd| if (chordMatches(chd, mods, cp)) {
        g.hud_visible = !g.hud_visible;
        resizeAllTabs();
        g.dirty = true;
        return true;
    };
    return false;
}

/// Scroll the active tab so the current search match is visible.
fn scrollToCurrentMatch() void {
    if (g.search.currentMatch()) |m| {
        g.tabs.current().terminal.scrollToLine(m.row);
        g.scroll_pos = @floatFromInt(g.tabs.current().terminal.viewportOffset());
    }
}

/// The active tab's cwd (OSC 7), or null if unknown.
/// Returns the filesystem path (file:// host stripped), not the raw URL.
fn currentCwd() ?[]const u8 {
    const cwd = g.tabs.current().terminal.cwdPath();
    return if (cwd.len > 0) cwd else null;
}

/// Create a new tab sized for the current window; the 1->2 transition makes the
/// bar appear, so resize every tab afterward.
fn addTab(cwd: ?[]const u8) void {
    closeSearch();
    const b = g.view.msgSend(CGRect, "bounds", .{});
    const dw: usize = @intFromFloat(@max(b.size.width * g.scale, 1));
    const dh: usize = @intFromFloat(@max(b.size.height * g.scale, 1));
    const cw: usize = @intFromFloat(g.font.metrics.cell_w);
    const ch: usize = @intFromFloat(g.font.metrics.cell_h);
    const cols = @max((dw -| 2 * grid_pad) / cw, 1);
    // New tab will make the bar visible (>=2 tabs): reserve its row.
    const rows = @max(((dh -| 2 * grid_pad) / ch) -| 1, 1);
    g.tabs.newTab(cols, rows, g.config.config.scrollback, cwd) catch |e| {
        std.debug.print("anvil: new tab failed: {s}\n", .{@errorName(e)});
        return;
    };
    resizeAllTabs();
    snapAnim();
    g.dirty = true;
}

/// `performKeyEquivalent:` runs before `keyDown:` and before AppKit's key-view
/// loop — the only reliable place to catch the Tab key, which the key-view
/// loop would otherwise consume. Handles Ctrl+Tab / Ctrl+Shift+Tab tab cycling.
fn onPerformKeyEquivalent(event: objc.Object) bool {
    const flags = event.msgSend(c_ulong, "modifierFlags", .{});
    const control = flags & (1 << 18) != 0;
    const command = flags & (1 << 20) != 0;
    const shift = flags & (1 << 17) != 0;
    if (control and !command) {
        const kc = event.msgSend(c_ushort, "keyCode", .{});
        if (kc == 48) { // 48 = Tab
            closeSearch();
            if (shift) g.tabs.prev() else g.tabs.next();
            snapAnim();
            g.dirty = true;
            return true;
        }
    }
    return false;
}

fn onKeyDown(event: objc.Object) void {
    // Tab shortcuts (⌘…) are checked before the normal key path, which
    // deliberately ignores ⌘ combos.
    const flags = event.msgSend(c_ulong, "modifierFlags", .{});
    const mods: keys.Mods = .{
        .shift = flags & (1 << 17) != 0,
        .control = flags & (1 << 18) != 0,
        .option = flags & (1 << 19) != 0,
        .command = flags & (1 << 20) != 0,
    };
    if (mods.command) {
        const src = event.msgSend(objc.Object, "charactersIgnoringModifiers", .{});
        if (firstCodepoint(src)) |cp| {
            if (handleTabKey(mods, cp)) return;
            // ⌘K — summon the command palette.
            if (asciiLowerCp(cp) == 'k' and !mods.shift and !mods.control and !mods.option) {
                summonPalette();
                return;
            }
            // ⌘C — copy selection to clipboard.
            if (asciiLowerCp(cp) == 'c' and !mods.shift and !mods.control and !mods.option) {
                if (g.selection.active) {
                    copySelection();
                    return;
                }
            }
        }
        return; // other ⌘ combos still go to the system
    }

    // While the search bar is open, keystrokes edit the query, not the shell.
    if (g.search_open) {
        const p = extractKey(event) orelse return;
        switch (p.key) {
            .escape => closeSearch(),
            .enter => {
                g.search.next();
                scrollToCurrentMatch();
                g.dirty = true;
            },
            .backspace => {
                // Drop the last UTF-8 codepoint from the query.
                var qlen = g.search.query_len;
                while (qlen > 0 and (g.search.query_buf[qlen - 1] & 0xC0) == 0x80) qlen -= 1;
                if (qlen > 0) qlen -= 1;
                const q = g.search.query_buf[0..qlen];
                g.search.setQuery(&g.tabs.current().terminal, q);
                scrollToCurrentMatch();
                g.dirty = true;
            },
            .text => |cp| {
                // Append the codepoint's UTF-8 to the query and re-scan.
                var tmp: [256]u8 = undefined;
                const base = g.search.query();
                if (base.len + 4 <= tmp.len) {
                    @memcpy(tmp[0..base.len], base);
                    const n = std.unicode.utf8Encode(cp, tmp[base.len..]) catch 0;
                    g.search.setQuery(&g.tabs.current().terminal, tmp[0 .. base.len + n]);
                    scrollToCurrentMatch();
                    g.dirty = true;
                }
            },
            else => {}, // arrows etc. ignored while searching
        }
        return; // search swallows the key — never reaches the shell
    }

    const p = extractKey(event) orelse return;
    var buf: [16]u8 = undefined;
    const bytes = keys.encode(p.key, p.mods, false, &buf);
    _ = g.tabs.current().pty.write(bytes) catch {};
    g.tabs.current().terminal.scrollToBottom();
    g.scroll_pos = 0;
    g.selection.clear();
    g.dirty = true;
}

fn addOverscroll(excess_rows: f32) void {
    const ch: f32 = @floatCast(g.font.metrics.cell_h);
    const limit = ch * 1.5;
    // Feed the target, not the displayed value — onTick eases `overscroll`
    // toward it, so a hard scroll cannot snap the rubber-band in one frame.
    const resist = 1.0 - @min(@abs(g.overscroll_target) / limit, 1.0);
    g.overscroll_target = std.math.clamp(g.overscroll_target + excess_rows * ch * 0.30 * resist, -limit, limit);
}

fn bounceImpulse() f32 {
    return @as(f32, @floatCast(g.font.metrics.cell_h)) * 0.9;
}

fn onScroll(event: objc.Object) void {
    const dy = event.msgSend(f64, "scrollingDeltaY", .{});
    if (dy == 0) return;
    const t = g.tabs.current();
    const d: f32 = @floatCast(dy / 8.0);
    const max_pos: f32 = @floatFromInt(t.terminal.scrollbackLen());
    var np = g.scroll_pos + d;
    if (np > max_pos) {
        addOverscroll(np - max_pos);
        np = max_pos;
    } else if (np < 0) {
        addOverscroll(np);
        np = 0;
    }
    g.scroll_pos = np;
    t.terminal.setViewportOffset(@intFromFloat(@round(np)));
    g.dirty = true;
}

/// Convert an NSEvent's locationInWindow to a (viewport row, col) cell.
/// Returns null if the point is outside the terminal grid (e.g. in the tab bar).
/// Clamps to the grid edge when `clamp` is true (for drag events).
fn eventCell(event: objc.Object, clamp: bool) ?struct { row: usize, col: usize } {
    const win_pt = event.msgSend(CGPoint, "locationInWindow", .{});
    const view_pt = g.view.msgSend(CGPoint, "convertPoint:fromView:", .{
        win_pt, @as(c.id, null),
    });
    const b = g.view.msgSend(CGRect, "bounds", .{});
    const cw: f64 = g.font.metrics.cell_w;
    const ch: f64 = g.font.metrics.cell_h;
    const t = g.tabs.current();
    const rows: f64 = @floatFromInt(t.terminal.rows());
    const cols: f64 = @floatFromInt(t.terminal.cols());

    // Device-pixel coordinates, raster origin (top-left).
    const raster_h = b.size.height * g.scale;
    const px_x = view_pt.x * g.scale;
    const px_y = raster_h - view_pt.y * g.scale;

    // Grid origin in device pixels.
    const pad: f64 = @floatFromInt(grid_pad);
    const top_bar_px: f64 = @floatFromInt(topBarRows());
    const grid_top = pad + top_bar_px * ch;
    const grid_left = pad;

    const rel_y = px_y - grid_top;
    const rel_x = px_x - grid_left;

    if (!clamp) {
        if (rel_y < 0 or rel_x < 0) return null;
        if (rel_y >= rows * ch or rel_x >= cols * cw) return null;
    }

    const raw_row = rel_y / ch;
    const raw_col = rel_x / cw;
    const row: usize = @intFromFloat(std.math.clamp(raw_row, 0.0, rows - 1));
    const col: usize = @intFromFloat(std.math.clamp(raw_col, 0.0, cols - 1));
    return .{ .row = row, .col = col };
}

fn onMouseDown(event: objc.Object) void {
    const win_pt = event.msgSend(CGPoint, "locationInWindow", .{});
    const view_pt = g.view.msgSend(CGPoint, "convertPoint:fromView:", .{
        win_pt, @as(c.id, null),
    });
    const b = g.view.msgSend(CGRect, "bounds", .{});

    // Check if the click is in the tab bar (top row).
    if (g.tabs.barVisible()) {
        const ch_pt = g.font.metrics.cell_h / g.scale; // bar height in points
        if (view_pt.y >= b.size.height - ch_pt) {
            // Tab bar click — switch tab.
            const n = g.tabs.count();
            if (n == 0) return;
            const frac = std.math.clamp(view_pt.x / b.size.width, 0.0, 0.999);
            const idx: usize = @intFromFloat(frac * @as(f64, @floatFromInt(n)));
            g.tabs.switchTo(idx);
            snapAnim();
            g.dirty = true;
            return;
        }
    }

    // Grid click — begin selection.
    const cell = eventCell(event, false) orelse {
        g.selection.clear();
        g.dirty = true;
        return;
    };
    const content_row = g.tabs.current().terminal.contentRowOfViewport(cell.row);
    g.selection = .{
        .active = true,
        .anchor = .{ .row = content_row, .col = cell.col },
        .head = .{ .row = content_row, .col = cell.col },
    };
    g.dirty = true;
}

fn onMouseDragged(event: objc.Object) void {
    if (!g.selection.active) return;
    const cell = eventCell(event, true) orelse return;
    const content_row = g.tabs.current().terminal.contentRowOfViewport(cell.row);
    g.selection.head = .{ .row = content_row, .col = cell.col };
    g.dirty = true;
}

fn onMouseUp(event: objc.Object) void {
    _ = event;
    // A click with no drag (anchor == head) means no real selection.
    if (g.selection.active and
        g.selection.anchor.row == g.selection.head.row and
        g.selection.anchor.col == g.selection.head.col)
    {
        g.selection.clear();
    }
    g.dirty = true;
}

/// Extract selected text from the active tab's terminal and write it to the
/// macOS general pasteboard.
fn copySelection() void {
    const term_obj = &g.tabs.current().terminal;
    const o = g.selection.ordered();
    const s = o.s;
    const e = o.e;

    var text: std.ArrayList(u8) = .empty;
    defer text.deinit(g.alloc);

    var row = s.row;
    while (row <= e.row) : (row += 1) {
        const cells = term_obj.line(row);
        const col_start: usize = if (row == s.row) s.col else 0;
        const col_end: usize = if (row == e.row) e.col else cells.len;
        const safe_end = @min(col_end, cells.len);

        // Find the last non-blank cell to trim trailing whitespace.
        var last = col_start;
        var ci = col_start;
        while (ci < safe_end) : (ci += 1) {
            if (cells[ci].cp != 0 and cells[ci].cp != ' ') last = ci + 1;
        }

        var x = col_start;
        while (x < last) : (x += 1) {
            const cp = cells[x].cp;
            if (cp == 0) {
                text.append(g.alloc, ' ') catch {};
            } else {
                var enc_buf: [4]u8 = undefined;
                const n = std.unicode.utf8Encode(cp, &enc_buf) catch continue;
                text.appendSlice(g.alloc, enc_buf[0..n]) catch {};
            }
        }

        if (row < e.row) text.append(g.alloc, '\n') catch {};
    }

    // NSString stringWithUTF8String: requires a null-terminated string.
    const str_z = g.alloc.dupeZ(u8, text.items) catch return;
    defer g.alloc.free(str_z);

    const ns_str = objc.getClass("NSString").?.msgSend(
        objc.Object,
        "stringWithUTF8String:",
        .{str_z.ptr},
    );

    const pb = objc.getClass("NSPasteboard").?.msgSend(
        objc.Object,
        "generalPasteboard",
        .{},
    );
    pb.msgSend(void, "clearContents", .{});
    const pb_type = nsString("public.utf8-plain-text");
    _ = pb.msgSend(bool, "setString:forType:", .{ ns_str, pb_type });
}

// --- HUD data refresh ----------------------------------------------------

/// Query physical RAM size in bytes via sysctl, or 0 on failure.
fn queryRamTotal() u64 {
    var val: u64 = 0;
    var len: usize = @sizeOf(u64);
    _ = std.c.sysctlbyname("hw.memsize", &val, &len, null, 0);
    return val;
}

/// Query macOS vm page counts to estimate memory-in-use percentage.
/// Returns 0–100, or 0 on failure.
fn queryMemPct() u8 {
    // host_statistics64 with HOST_VM_INFO64 fills a vm_statistics64_data_t.
    // Rather than declaring the full mach struct, we query two sysctl integers:
    //   vm.page_free_count  — free pages
    //   vm.page_active_count — active pages  (not available via sysctl on macOS)
    // Fallback: use "vm.pagesize" + "hw.memsize" + "vm.page_free_count" to
    // compute  used% = 1 - (free_pages * pagesize / total_ram).
    const ram = queryRamTotal();
    if (ram == 0) return 0;

    var page_size: u64 = 4096;
    var ps_len: usize = @sizeOf(u64);
    _ = std.c.sysctlbyname("hw.pagesize", &page_size, &ps_len, null, 0);

    var free_pages: u64 = 0;
    var fp_len: usize = @sizeOf(u64);
    _ = std.c.sysctlbyname("vm.page_free_count", &free_pages, &fp_len, null, 0);

    const free_bytes = free_pages * page_size;
    if (free_bytes >= ram) return 0;
    const used_bytes = ram - free_bytes;
    const pct = @divTrunc(used_bytes * 100, ram);
    return @intCast(@min(pct, 100));
}

// --- background git query ------------------------------------------------
// `git status` is a subprocess that can take tens to hundreds of ms (up to a
// 2 s timeout). Running it on the main thread froze the whole app — input,
// copy/paste, rendering — once a second. Each refresh now hands the query to
// a short-lived worker thread; the main thread only reads a finished result.
// Single-producer / single-consumer with at most one worker live at a time,
// so two atomic flags suffice — no mutex (Zig 0.16 has no std.Thread.Mutex).

const GitJob = struct {
    in_flight: std.atomic.Value(bool) = std.atomic.Value(bool).init(false),
    ready: std.atomic.Value(bool) = std.atomic.Value(bool).init(false),
    cwd: [std.fs.max_path_bytes]u8 = undefined,
    cwd_len: usize = 0,
    // result fields — written by the worker, read by the main thread once
    // `ready` is observed true.
    state: hud_mod.GitState = .no_repo,
    branch: [128]u8 = undefined,
    branch_len: usize = 0,
    dirty: u32 = 0,
    ahead: u32 = 0,
    behind: u32 = 0,
};
var git_job: GitJob = .{};

/// Worker body: run `git status` once for `git_job.cwd`, then publish. The
/// main thread wrote `cwd` and called `spawn` (which establishes happens-
/// before), so reading `git_job.cwd` here is safe.
fn gitJobRun() void {
    var branch_scratch: [128]u8 = undefined;
    const info = git.query(std.heap.c_allocator, git_job.cwd[0..git_job.cwd_len], &branch_scratch);
    if (info) |gi| {
        git_job.state = if (gi.dirty > 0) .dirty else .ok;
        const bl = @min(gi.branch.len, git_job.branch.len);
        @memcpy(git_job.branch[0..bl], gi.branch[0..bl]);
        git_job.branch_len = bl;
        git_job.dirty = gi.dirty;
        git_job.ahead = gi.ahead;
        git_job.behind = gi.behind;
    } else {
        git_job.state = .no_repo;
        git_job.branch_len = 0;
    }
    git_job.ready.store(true, .release); // release: publishes the fields above
    git_job.in_flight.store(false, .release);
}

/// Populate `g.hud` from live data: git status, last-run state, memory.
fn refreshHud() void {
    const cur_term = &g.tabs.current().terminal;

    // --- git: consume a finished result, then kick off the next query ---
    if (git_job.ready.load(.acquire)) {
        g.hud.git = git_job.state;
        g.hud.branch_len = git_job.branch_len;
        @memcpy(g.hud.branch[0..git_job.branch_len], git_job.branch[0..git_job.branch_len]);
        g.hud.git_dirty = git_job.dirty;
        g.hud.git_ahead = git_job.ahead;
        g.hud.git_behind = git_job.behind;
        git_job.ready.store(false, .monotonic);
    }
    if (!git_job.in_flight.load(.acquire)) {
        const cwd = cur_term.cwdPath();
        if (cwd.len > 0 and cwd.len <= git_job.cwd.len) {
            @memcpy(git_job.cwd[0..cwd.len], cwd);
            git_job.cwd_len = cwd.len;
            git_job.in_flight.store(true, .release);
            if (std.Thread.spawn(.{}, gitJobRun, .{})) |t| {
                t.detach();
            } else |_| {
                git_job.in_flight.store(false, .release);
            }
        }
    }

    // --- last run ---
    const lr = cur_term.lastRun();
    if (lr.running) {
        // currently running — show idle until done
        g.hud.run = .idle;
    } else if (lr.duration_ms == 0 and lr.exit_code == 0 and !lr.running) {
        // no run recorded yet
        g.hud.run = .idle;
    } else {
        g.hud.run = if (lr.exit_code == 0) .ok else .failed;
        g.hud.run_exit = lr.exit_code;
        g.hud.run_duration_ms = lr.duration_ms;
    }

    // --- mem ---
    g.hud.mem_pct = queryMemPct();

    g.dirty = true;
}

// --- rendering -----------------------------------------------------------

/// Open the search bar (re-scanning the active tab) and reflow for the row.
fn openSearch() void {
    if (g.search_open) return;
    g.search_open = true;
    g.search.setQuery(&g.tabs.current().terminal, g.search.query());
    resizeAllTabs();
    g.dirty = true;
}
/// Close the search bar and reflow.
fn closeSearch() void {
    if (!g.search_open) return;
    g.search_open = false;
    resizeAllTabs();
    g.dirty = true;
}

/// Rows taken by the tab bar at the top (0 or 1).
fn topBarRows() usize {
    return if (g.tabs.barVisible()) 1 else 0;
}
/// Rows taken by the search bar at the bottom (0 or 1).
fn bottomBarRows() usize {
    return if (g.search_open) 1 else 0;
}

fn renderFrame() void {
    g.raster.clear(g.theme.background);

    const t = g.tabs.current();
    const rows = t.terminal.rows();
    const cols = t.terminal.cols();

    // Separator color: a quiet hairline that adapts to the theme.
    const rule_rgb = color.mix(g.theme.background, g.theme.foreground, 0.14);

    if (g.scroll_pos == 0 and g.overscroll == 0) {
        var y: usize = 0;
        while (y < rows) : (y += 1) {
            const crow = t.terminal.contentRowOfViewport(y);
            const line = t.terminal.viewportRow(y);
            var x: usize = 0;
            while (x < cols and x < line.len) : (x += 1) drawCell(x, y, crow, line[x]);
            if (t.terminal.isPromptStart(t.terminal.absoluteLineOfContent(crow))) {
                const ry: f64 = @floatFromInt(y + topBarRows());
                g.raster.rowRule(g.font, ry, rule_rgb);
            }
        }
    } else {
        // Displayed offset = scroll_pos. Render the integer offset (base+1)
        // and slide the grid by (1-frac) cells plus any overscroll translation.
        // A positive y_shift_px moves cells UP on screen; overscroll > 0 means
        // pulled past the top, so the grid slides DOWN — that requires a
        // NEGATIVE y_shift_px contribution.
        const base: usize = @intFromFloat(@floor(g.scroll_pos));
        const frac: f64 = @as(f64, g.scroll_pos) - @floor(@as(f64, g.scroll_pos));
        const scroll_shift = (1.0 - frac) * g.font.metrics.cell_h;
        g.raster.y_shift_px = scroll_shift - @as(f64, g.overscroll);
        const hist = t.terminal.scrollbackLen();
        const off = base + 1;
        var y: usize = 0;
        while (y <= rows) : (y += 1) {
            // Content row for viewportRowAt(off, y): same formula as
            // contentRowOfViewport but substituting `off` for viewport_offset.
            // Use saturating arithmetic to guard against off > hist + y.
            const crow: usize = if (off > y) (hist + y) -| off else hist + y - off;
            const line = t.terminal.viewportRowAt(off, y);
            var x: usize = 0;
            while (x < cols and x < line.len) : (x += 1) drawCell(x, y, crow, line[x]);
            if (t.terminal.isPromptStart(t.terminal.absoluteLineOfContent(crow))) {
                const ry: f64 = @floatFromInt(y + topBarRows());
                g.raster.rowRule(g.font, ry, rule_rgb);
            }
        }
        g.raster.y_shift_px = 0;
    }

    // Tab bar on top of the grid so a gliding top row cannot bleed into it.
    if (g.tabs.barVisible()) {
        tabbar.drawTabBar(&g.raster, g.font, g.theme, &g.tabs);
    }

    const cur = t.terminal.cursor();
    if (cur.visible and t.terminal.viewportOffset() == 0 and
        g.scroll_pos == 0 and cur.x < cols and cur.y < rows)
    {
        // Ride the rubber-band: shift the cursor with the grid during a bounce.
        g.raster.y_shift_px = -@as(f64, g.overscroll);
        drawCursor();
        g.raster.y_shift_px = 0;
    }

    if (g.search_open) {
        const ch: usize = @intFromFloat(g.font.metrics.cell_h);
        const total_rows = @max((g.raster.height -| 2 * grid_pad) / ch, 1);
        searchbar.drawSearchBar(&g.raster, g.font, g.theme, &g.search, total_rows - 1);
    }

    if (g.hud_visible) {
        // The HUD occupies the rightmost hud_cols columns of the raster, after
        // the one-column separator gutter. start_col = terminal cols + 1 gutter.
        const start_col = cols + 1;
        const ch: usize = @intFromFloat(g.font.metrics.cell_h);
        const total_rows_for_hud = @max((g.raster.height -| 2 * grid_pad) / ch, 1);
        const visible_rows = @max(total_rows_for_hud -| topBarRows() -| bottomBarRows(), 1);
        hud_mod.draw(
            &g.raster,
            g.font,
            g.theme,
            g.hud,
            start_col,
            visible_rows,
            topBarRows(),
        );
    }

    g.renderer.present(g.raster.bytes());
}

fn drawCell(x: usize, y: usize, content_row: usize, cell: term.Cell) void {
    var fg = resolve(cell.fg, g.theme.foreground);
    var bg = resolve(cell.bg, g.theme.background);
    if (cell.attrs.inverse) {
        const t = fg;
        fg = bg;
        bg = t;
    }
    if (g.selection.active and g.selection.contains(content_row, x)) {
        bg = color.mix(g.theme.background, g.theme.accent, 0.28);
    }
    if (g.search_open) {
        switch (g.search.classify(content_row, x)) {
            .current => bg = g.theme.accent,
            .other => bg = g.theme.ansi[8],
            .none => {},
        }
    }
    const ry = y + topBarRows(); // raster row: offset by top bar when visible
    if (!std.mem.eql(u8, &bg, &g.theme.background)) {
        g.raster.cellBg(g.font, x, ry, bg);
    }
    if (cell.cp != ' ' and cell.cp != 0) {
        g.raster.cellGlyph(g.font, x, ry, g.font.glyph(cell.cp), fg);
    }
}

fn drawCursor() void {
    const opacity: f32 = if (g.cursor_cfg.blink) cursorOpacity(g.blink_phase) else 1.0;
    const ax: f64 = g.cursor_ax;
    const ay: f64 = @as(f64, g.cursor_ay) + @as(f64, @floatFromInt(topBarRows()));
    const cursor_rgb = color.mix(g.theme.background, g.theme.accent, opacity);

    switch (g.cursor_cfg.style) {
        .block => {
            g.raster.cellInset(g.font, ax, ay, cursor_rgb, 0.0, 0.0, 1.0, 1.0);
            // Re-draw the glyph of the cell the cursor rect mostly covers, in
            // the inverted color, on top of the accent rect.
            const ic: usize = @intFromFloat(@round(g.cursor_ax));
            const ir: usize = @intFromFloat(@round(g.cursor_ay));
            const t = g.tabs.current();
            if (ir < t.terminal.rows() and ic < t.terminal.cols()) {
                const line = t.terminal.viewportRow(ir);
                if (ic < line.len) {
                    const cell = line[ic];
                    if (cell.cp != ' ' and cell.cp != 0) {
                        const base_fg = resolve(cell.fg, g.theme.foreground);
                        const glyph_fg = color.mix(base_fg, g.theme.background, opacity);
                        g.raster.cellGlyph(g.font, ic, ir + topBarRows(), g.font.glyph(cell.cp), glyph_fg);
                    }
                }
            }
        },
        .bar => g.raster.cellInset(g.font, ax, ay, cursor_rgb, 0.0, 0.0, 0.15, 1.0),
        .underline => g.raster.cellInset(g.font, ax, ay, cursor_rgb, 0.0, 0.0, 1.0, 0.12),
    }
}

fn resolve(col: term.Color, default: [3]u8) [3]u8 {
    return switch (col) {
        .default => default,
        .palette => |p| g.theme.palette256(p),
        .rgb => |v| v,
    };
}

// --- setup ---------------------------------------------------------------

fn nsString(text: [:0]const u8) objc.Object {
    return objc.getClass("NSString").?
        .msgSend(objc.Object, "stringWithUTF8String:", .{text.ptr});
}

fn setApplicationIcon(app: objc.Object) void {
    const data = objc.getClass("NSData").?.msgSend(objc.Object, "dataWithBytes:length:", .{
        app_icon_png, @as(c_ulong, app_icon_png.len),
    });
    const image = objc.getClass("NSImage").?
        .msgSend(objc.Object, "alloc", .{})
        .msgSend(objc.Object, "initWithData:", .{data});
    app.msgSend(void, "setApplicationIconImage:", .{image});
}

fn fail(what: []const u8, err: anyerror) noreturn {
    std.debug.print("anvil: {s} init failed: {s}\n", .{ what, @errorName(err) });
    std.process.exit(1);
}

pub fn main() void {
    const alloc = std.heap.c_allocator;

    const nsapp = objc.getClass("NSApplication").?
        .msgSend(objc.Object, "sharedApplication", .{});
    nsapp.msgSend(void, "setActivationPolicy:", .{@as(c_long, 0)});
    setApplicationIcon(nsapp);

    // Delegate class: app lifecycle, resize, and the render tick.
    const Delegate = objc.allocateClassPair(objc.getClass("NSObject").?, "AnvilDelegate").?;
    _ = Delegate.addMethod("applicationShouldTerminateAfterLastWindowClosed:", imShouldTerminate);
    _ = Delegate.addMethod("windowDidResize:", imWindowDidResize);
    _ = Delegate.addMethod("tick:", imTick);
    objc.registerClassPair(Delegate);

    // View class: keyboard and scroll input.
    const View = objc.allocateClassPair(objc.getClass("NSView").?, "AnvilTerminalView").?;
    _ = View.addMethod("acceptsFirstResponder", imAcceptsFirstResponder);
    _ = View.addMethod("keyDown:", imKeyDown);
    _ = View.addMethod("scrollWheel:", imScrollWheel);
    _ = View.addMethod("viewDidEndLiveResize", imViewDidEndLiveResize);
    _ = View.addMethod("performKeyEquivalent:", imPerformKeyEquivalent);
    _ = View.addMethod("mouseDown:", imMouseDown);
    _ = View.addMethod("mouseDragged:", imMouseDragged);
    _ = View.addMethod("mouseUp:", imMouseUp);
    objc.registerClassPair(View);

    const delegate = Delegate.msgSend(objc.Object, "alloc", .{})
        .msgSend(objc.Object, "init", .{});

    // Load config early so window size can be driven by the configured values.
    const config_path: ?[]const u8 = cfg_mod.resolvePath(&config_path_buf);
    var loaded: cfg_mod.Loaded = if (config_path) |p| cfg_mod.load(alloc, p) else cfg_mod.defaults(alloc);
    const cfg = loaded.config;

    const rect: CGRect = .{
        .origin = .{ .x = 0, .y = 0 },
        .size = .{ .width = cfg.window.width, .height = cfg.window.height },
    };
    const style: c_ulong = 1 | 2 | 4 | 8; // titled|closable|miniaturizable|resizable
    const window = objc.getClass("NSWindow").?.msgSend(objc.Object, "alloc", .{})
        .msgSend(objc.Object, "initWithContentRect:styleMask:backing:defer:", .{
        rect, style, @as(c_ulong, 2), false,
    });
    window.msgSend(void, "setTitle:", .{nsString("Anvil")});

    const view = View.msgSend(objc.Object, "alloc", .{})
        .msgSend(objc.Object, "initWithFrame:", .{rect});
    const layer = objc.getClass("CAMetalLayer").?.msgSend(objc.Object, "layer", .{});
    view.msgSend(void, "setLayer:", .{layer});
    view.msgSend(void, "setWantsLayer:", .{true});

    window.msgSend(void, "setContentView:", .{view});
    window.msgSend(void, "setDelegate:", .{delegate});
    window.msgSend(void, "center", .{});
    window.msgSend(void, "makeKeyAndOrderFront:", .{@as(c.id, null)});
    window.msgSend(void, "makeFirstResponder:", .{view});
    nsapp.msgSend(void, "setDelegate:", .{delegate});

    const scale = window.msgSend(f64, "backingScaleFactor", .{});

    const active_theme = theme_mod.resolve(effectiveThemeName(nsapp, cfg.theme), cfg.theme_overrides);

    // Font: the bundled Nerd Font first (IBM Plex Mono plus icon glyphs), then
    // configured family, then fallbacks. dupeZ into the config arena so the
    // slice outlives this stack frame (font stack needs [:0]const u8).
    @import("render/font.zig").registerBundled();
    const fam_z = loaded.arena.allocator().dupeZ(u8, cfg.font.family) catch "IBMPlexMono";
    const font_names = [_][:0]const u8{ "BlexMono Nerd Font Mono", fam_z, "SFMono-Regular", "Menlo" };
    const font = Font.initFirstAvailable(&font_names, cfg.font.size * scale) catch |e| fail("font", e);
    const dw: usize = @intFromFloat(cfg.window.width * scale);
    const dh: usize = @intFromFloat(cfg.window.height * scale);
    const cw: usize = @intFromFloat(font.metrics.cell_w);
    const ch: usize = @intFromFloat(font.metrics.cell_h);
    const cols = @max((dw -| 2 * grid_pad) / cw, 1);
    const rows = @max((dh -| 2 * grid_pad) / ch, 1);

    shell_integration.setup(cfg.shell_integration);

    var tabs = tabs_mod.TabManager.init(alloc);
    tabs.newTab(cols, rows, cfg.scrollback, null) catch |e| fail("tab", e);

    const wv = webview_mod.Webview.init(window, view, cfg.window.width, cfg.window.height, palette_html);

    g = .{
        .alloc = alloc,
        .tabs = tabs,
        .font = font,
        .raster = Raster.init(alloc, dw, dh) catch |e| fail("raster", e),
        .renderer = Renderer.init(layer, dw, dh) catch |e| fail("renderer", e),
        .nsapp = nsapp,
        .view = view,
        .scale = scale,
        .dirty = true,
        .theme = active_theme,
        .cursor_cfg = cfg.cursor,
        .config = loaded,
        .watcher = cfg_mod.Watcher.init(config_path orelse ""),
        .search = Search.init(alloc),
        .webview = wv,
    };
    g.system_dark = systemIsDark(nsapp);
    g.renderer.setClearColor(active_theme.background);
    g.raster.pad_x = @floatFromInt(grid_pad);
    g.raster.pad_y = @floatFromInt(grid_pad);
    loadKeybindings(cfg.keybindings);
    webview_mod.on_message = handleWebMessage;

    renderFrame();
    snapAnim();

    _ = objc.getClass("NSTimer").?.msgSend(
        objc.Object,
        "scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:",
        .{ @as(f64, 1.0 / 60.0), delegate, objc.sel("tick:").value, @as(c.id, null), true },
    );

    nsapp.msgSend(void, "activateIgnoringOtherApps:", .{true});
    nsapp.msgSend(void, "run", .{});
}

test {
    _ = @import("config/config.zig");
    _ = @import("config/theme.zig");
    _ = @import("render/color.zig");
    _ = @import("render/font.zig");
    _ = @import("render/raster.zig");
    _ = @import("app/keys.zig");
    _ = @import("app/tab.zig");
    _ = @import("app/shell_integration.zig");
    _ = @import("render/tabbar.zig");
    _ = @import("render/searchbar.zig");
    _ = @import("render/hud.zig");
    _ = @import("terminal/terminal.zig");
    _ = @import("terminal/search.zig");
    _ = @import("pty/pty.zig");
    _ = @import("ipc/bridge.zig");
    _ = @import("app/palette.zig");
    _ = @import("app/selection.zig");
}
