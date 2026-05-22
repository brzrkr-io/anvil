//! Anvil — M1 entry point. Wires the terminal model, the PTY, the
//! Metal renderer, the CoreGraphics rasterizer, and AppKit input into a
//! single-pane GPU terminal.

const std = @import("std");
const objc = @import("objc");
const c = objc.c;

const term = @import("terminal/terminal.zig");
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
const filetree_mod = @import("app/filetree.zig");
const filetree_render = @import("render/filetree.zig");
const cheatsheet_mod = @import("render/cheatsheet.zig");
const interact = @import("app/interact.zig");
const draw_mod = @import("render/draw.zig");
const workspace_mod = @import("render/workspace.zig");
const metal_mod = @import("render/metal.zig");

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

fn windowIsKey() bool {
    const win = g.view.msgSend(objc.Object, "window", .{});
    if (win.value == null) return false;
    return win.msgSend(bool, "isKeyWindow", .{});
}

/// Snap all animation state to the active tab's current values — used on tab
/// switch, resize, and startup so those discontinuities never animate.
fn snapAnim() void {
    const p = focusedPane();
    const cur = p.terminal.cursor();
    p.cursor_ax = @floatFromInt(cur.x);
    p.cursor_ay = @floatFromInt(cur.y);
    p.scroll_pos = @floatFromInt(p.terminal.viewportOffset());
    p.overscroll = 0;
    p.overscroll_target = 0;
}

// Uniform inset (device pixels) between the window edge and the terminal grid.
// The margin shows the background color; the grid simply has fewer cells.
const grid_pad: usize = 24;

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
    last_blink_opacity: f32 = -1, // last opacity value that triggered a redraw
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
    keys_tree_toggle: ?cfg_mod.Chord = null,
    keys_cheatsheet: ?cfg_mod.Chord = null,
    search: Search,
    cheatsheet_visible: bool = false,
    tree_visible: bool = false,
    tree: filetree_mod.FileTree = .{},
    search_open: bool = false,
    hud_visible: bool = true,
    hud: hud_mod.Hud = .{},
    hud_tick: u32 = 0, // counts up to hud_refresh_ticks then resets
    webview: webview_mod.Webview,
    palette: palette_mod.Palette = .{},
    system_dark: bool = false,
};
var g: App = undefined;

/// Return the focused pane of the current tab. Always valid after startup.
fn focusedPane() *@import("workspace/pane.zig").Pane {
    return g.tabs.current().focusedPane();
}

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
    g.last_blink_opacity = -1; // reset so blink invariant holds on live reload
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
    g.keys_tree_toggle = cfg_mod.parseChord(kb.tree_toggle);
    g.keys_cheatsheet = cfg_mod.parseChord(kb.cheatsheet_toggle);
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
            focusedPane().terminal.feed("\x1b[H\x1b[2J");
            g.dirty = true;
        },
        .scroll_top => {
            const p = focusedPane();
            p.terminal.scrollViewport(@intCast(p.terminal.scrollbackLen()));
            p.scroll_pos = @floatFromInt(p.terminal.viewportOffset());
            p.overscroll_target = bounceImpulse();
            g.dirty = true;
        },
        .scroll_bottom => {
            const p = focusedPane();
            p.terminal.scrollToBottom();
            p.scroll_pos = 0;
            p.overscroll_target = -bounceImpulse();
            g.dirty = true;
        },
        .app_quit => g.nsapp.msgSend(void, "terminate:", .{@as(c.id, null)}),
        .hud_toggle => {
            g.hud_visible = !g.hud_visible;
            g.dirty = true;
        },
        .tree_toggle => toggleTree(),
        .cheatsheet_show => {
            g.cheatsheet_visible = true;
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

    // Drain every tab's panes so background tabs stay current; render only the active.
    var i: usize = 0;
    var any_dead = false;
    while (i < g.tabs.count()) : (i += 1) {
        const tab = g.tabs.tabs.items[i];
        var pit = tab.registry.map.valueIterator();
        while (pit.next()) |pane_ptr| {
            const pane = pane_ptr.*;
            const bytes = pane.drain(&feed_scratch);
            if (bytes.len > 0) {
                pane.terminal.feed(bytes);
                if (i == g.tabs.active and pane.id == tab.tree.focused) {
                    g.dirty = true;
                    if (g.search_open) g.search.rescan(&focusedPane().terminal);
                }
            }
            if (pane.isDead()) any_dead = true;
        }
    }

    // Blink fade — only while the window is focused, so an unfocused window
    // does not burn 60fps. When blink is off in config (and not overridden by
    // DECSCUSR), the cursor is solid.
    const effective_blink = focusedPane().terminal.app_cursor_blink orelse g.cursor_cfg.blink;
    if (effective_blink and windowIsKey()) {
        g.blink_phase += 1.0 / 64.0;
        if (g.blink_phase >= 1.0) g.blink_phase -= 1.0;
        // Only redraw for blink when the opacity value actually changes.
        // cursorOpacity is constant during the solid-hold (0..0.5) and off-hold
        // (0.62..0.88) stretches, so skip the dirty mark during those holds.
        const new_opacity = draw_mod.cursorOpacity(g.blink_phase);
        if (new_opacity != g.last_blink_opacity) {
            g.last_blink_opacity = new_opacity;
            g.dirty = true;
        }
    } else if (g.blink_phase != 0) {
        g.blink_phase = 0;
        g.last_blink_opacity = -1; // force redraw on next blink start
        g.dirty = true;
    }

    // Cursor glide.
    {
        const p = focusedPane();
        const cur = p.terminal.cursor();
        const tx: f32 = @floatFromInt(cur.x);
        const ty: f32 = @floatFromInt(cur.y);
        if (@abs(tx - p.cursor_ax) > 0.002 or @abs(ty - p.cursor_ay) > 0.002) {
            p.cursor_ax = approach(p.cursor_ax, tx, 0.30);
            p.cursor_ay = approach(p.cursor_ay, ty, 0.30);
            if (@abs(tx - p.cursor_ax) <= 0.002) p.cursor_ax = tx;
            if (@abs(ty - p.cursor_ay) <= 0.002) p.cursor_ay = ty;
            g.dirty = true;
        }

        // Rubber-band: ease the overscroll toward its target, which itself decays
        // to zero — so the pull-in and the spring-back are both smooth (no snap).
        if (p.overscroll != 0 or p.overscroll_target != 0) {
            p.overscroll_target = approach(p.overscroll_target, 0, 0.32);
            p.overscroll = approach(p.overscroll, p.overscroll_target, 0.55);
            if (@abs(p.overscroll_target) < 0.5) p.overscroll_target = 0;
            if (@abs(p.overscroll) < 0.5 and p.overscroll_target == 0) p.overscroll = 0;
            g.dirty = true;
        }
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
        if (g.tabs.tabs.items[i].focusedPane().isDead()) {
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
    const tree_px = if (g.tree_visible) filetree_render.tree_cols * cw else 0;
    const cols = @max((dw -| 2 * grid_pad -| tree_px) / cw, 1);
    const total_rows = @max((dh -| 2 * grid_pad) / ch, 1);
    const rows = @max(total_rows -| topBarRows() -| bottomBarRows(), 1);

    for (g.tabs.tabs.items) |tab| {
        var pit = tab.registry.map.valueIterator();
        while (pit.next()) |pane_ptr| {
            pane_ptr.*.terminal.resize(cols, rows);
            pane_ptr.*.pty.resize(@intCast(cols), @intCast(rows));
        }
    }
    focusedPane().selection.clear();
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
        // F1-F12 virtual keycodes.
        122 => .f1,
        120 => .f2,
        99 => .f3,
        118 => .f4,
        96 => .f5,
        97 => .f6,
        98 => .f7,
        100 => .f8,
        101 => .f9,
        109 => .f10,
        103 => .f11,
        111 => .f12,
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
        g.dirty = true;
        return true;
    };
    if (g.keys_tree_toggle) |chd| if (chordMatches(chd, mods, cp)) {
        toggleTree();
        return true;
    };
    if (g.keys_cheatsheet) |chd| if (chordMatches(chd, mods, cp)) {
        g.cheatsheet_visible = !g.cheatsheet_visible;
        g.dirty = true;
        return true;
    };
    return false;
}

/// Toggle the file-tree panel: flip visibility, re-root tree when opening,
/// reflow the terminal grid, and redraw.
fn toggleTree() void {
    g.tree_visible = !g.tree_visible;
    if (g.tree_visible) {
        if (currentCwd()) |cwd| g.tree.setRoot(cwd);
    }
    resizeAllTabs();
    g.dirty = true;
}

/// ⌘↑ — scroll the viewport up to the nearest `prompt_start` mark above the
/// current viewport top. If none is found above, does nothing.
fn jumpToPrevPrompt() void {
    const t = &focusedPane().terminal;
    const marks = t.promptMarks();
    if (marks.len == 0) return;

    // The content row currently at the top of the viewport.
    const top_content = t.contentRowOfViewport(0);

    // Find the rightmost mark with content_row strictly less than top_content.
    // marks[i].line is an absolute line; content_row = abs - evicted_lines.
    const ev = t.evicted_lines;
    var best: ?usize = null; // content row of the best candidate
    for (marks) |m| {
        if (m.kind != .prompt_start) continue;
        if (m.line < ev) continue; // evicted from scrollback — can't navigate there
        const cr = m.line - ev;
        if (cr < top_content) {
            if (best == null or cr > best.?) best = cr;
        }
    }
    if (best) |cr| {
        const p = focusedPane();
        p.terminal.scrollToLine(cr);
        p.scroll_pos = @floatFromInt(p.terminal.viewportOffset());
        p.overscroll_target = bounceImpulse();
        g.dirty = true;
    }
}

/// ⌘↓ — scroll the viewport down to the nearest `prompt_start` mark below the
/// current viewport top. Past the last mark, scrolls to the live bottom.
fn jumpToNextPrompt() void {
    const t = &focusedPane().terminal;
    const marks = t.promptMarks();

    // The content row currently at the top of the viewport.
    const top_content = t.contentRowOfViewport(0);

    const ev = t.evicted_lines;
    var best: ?usize = null; // content row of the best candidate
    for (marks) |m| {
        if (m.kind != .prompt_start) continue;
        if (m.line < ev) continue;
        const cr = m.line - ev;
        if (cr > top_content) {
            if (best == null or cr < best.?) best = cr;
        }
    }

    if (best) |cr| {
        const p = focusedPane();
        p.terminal.scrollToLine(cr);
        p.scroll_pos = @floatFromInt(p.terminal.viewportOffset());
        p.overscroll_target = -bounceImpulse();
        g.dirty = true;
    } else {
        // No mark below — jump to the live bottom.
        const p = focusedPane();
        p.terminal.scrollToBottom();
        p.scroll_pos = 0;
        p.overscroll_target = -bounceImpulse();
        g.dirty = true;
    }
}

/// Scroll the active tab so the current search match is visible.
fn scrollToCurrentMatch() void {
    if (g.search.currentMatch()) |m| {
        const p = focusedPane();
        p.terminal.scrollToLine(m.row);
        p.scroll_pos = @floatFromInt(p.terminal.viewportOffset());
    }
}

/// The active tab's cwd (OSC 7), or null if unknown.
/// Returns the filesystem path (file:// host stripped), not the raw URL.
fn currentCwd() ?[]const u8 {
    const cwd = focusedPane().terminal.cwdPath();
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
        // ⌘↑ / ⌘↓ — jump between OSC 133 prompt marks.
        // Arrow keys are function keys; check keyCode directly before trying
        // to read a character codepoint (they don't have one).
        const keycode = event.msgSend(c_ushort, "keyCode", .{});
        if (!mods.shift and !mods.control and !mods.option) {
            if (keycode == 126) { // Up arrow
                jumpToPrevPrompt();
                return;
            } else if (keycode == 125) { // Down arrow
                jumpToNextPrompt();
                return;
            }
        }

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
                if (focusedPane().selection.active) {
                    copySelection();
                    return;
                }
            }
        }
        return; // other ⌘ combos still go to the system
    }

    // While the cheatsheet is visible, any keystroke closes it (swallowed).
    // ⌘/ to toggle is already handled in the mods.command branch above.
    if (g.cheatsheet_visible) {
        g.cheatsheet_visible = false;
        g.dirty = true;
        return;
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
                g.search.setQuery(&focusedPane().terminal, q);
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
                    g.search.setQuery(&focusedPane().terminal, tmp[0 .. base.len + n]);
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
    const fp = focusedPane();
    _ = fp.pty.write(bytes) catch {};
    fp.terminal.scrollToBottom();
    fp.scroll_pos = 0;
    fp.selection.clear();
    g.dirty = true;
}

fn addOverscroll(excess_rows: f32) void {
    const ch: f32 = @floatCast(g.font.metrics.cell_h);
    const limit = ch * 1.5;
    // Feed the target, not the displayed value — onTick eases `overscroll`
    // toward it, so a hard scroll cannot snap the rubber-band in one frame.
    const p = focusedPane();
    const resist = 1.0 - @min(@abs(p.overscroll_target) / limit, 1.0);
    p.overscroll_target = std.math.clamp(p.overscroll_target + excess_rows * ch * 0.30 * resist, -limit, limit);
}

fn bounceImpulse() f32 {
    return @as(f32, @floatCast(g.font.metrics.cell_h)) * 0.5;
}

fn onScroll(event: objc.Object) void {
    const dy = event.msgSend(f64, "scrollingDeltaY", .{});
    if (dy == 0) return;

    // Mouse reporting: forward scroll as button 64 (up) or 65 (down) to PTY.
    const tm = &focusedPane().terminal;
    if (tm.modes.mouse_button or tm.modes.mouse_x10) {
        if (eventCell(event, false)) |cl| {
            const btn: u8 = if (dy > 0) 64 else 65;
            writeMouseEvent(btn, cl.col, cl.row, true);
        }
        return;
    }

    const p = focusedPane();
    const d: f32 = @floatCast(dy / 8.0);
    const max_pos: f32 = @floatFromInt(p.terminal.scrollbackLen());
    var np = p.scroll_pos + d;
    if (np > max_pos) {
        addOverscroll(np - max_pos);
        np = max_pos;
    } else if (np < 0) {
        addOverscroll(np);
        np = 0;
    }
    p.scroll_pos = np;
    p.terminal.setViewportOffset(@intFromFloat(@round(np)));
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
    const rows: f64 = @floatFromInt(focusedPane().terminal.rows());
    const cols: f64 = @floatFromInt(focusedPane().terminal.cols());

    // Device-pixel coordinates, raster origin (top-left).
    const raster_h = b.size.height * g.scale;
    const px_x = view_pt.x * g.scale;
    const px_y = raster_h - view_pt.y * g.scale;

    // Grid origin in device pixels.
    const pad: f64 = @floatFromInt(grid_pad);
    const top_bar_px: f64 = @floatFromInt(topBarRows());
    const grid_top = pad + top_bar_px * ch;
    // When the tree panel is visible the terminal grid is shifted right.
    const tree_offset_px = if (g.tree_visible)
        @as(f64, @floatFromInt(filetree_render.tree_cols)) * cw
    else
        @as(f64, 0);
    const grid_left = pad + tree_offset_px;

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

/// Write `\x15${EDITOR:-open} '<path>'\n` to the active tab's PTY.
/// Single-quotes in `path` are escaped as `'\''`.
fn ptyWriteOpenFile(path: []const u8) void {
    const pty = &focusedPane().pty;
    _ = pty.write("\x15${EDITOR:-open} '") catch {};
    // Escape single quotes: replace each ' with '\''.
    var i: usize = 0;
    while (i < path.len) {
        const next = std.mem.indexOfScalarPos(u8, path, i, '\'') orelse {
            _ = pty.write(path[i..]) catch {};
            break;
        };
        _ = pty.write(path[i..next]) catch {};
        _ = pty.write("'\\''") catch {};
        i = next + 1;
    }
    _ = pty.write("'\n") catch {};
}

/// Write `\x15open '<url>'\n` to the active tab's PTY.
fn ptyWriteOpenUrl(url: []const u8) void {
    const pty = &focusedPane().pty;
    _ = pty.write("\x15open '") catch {};
    var i: usize = 0;
    while (i < url.len) {
        const next = std.mem.indexOfScalarPos(u8, url, i, '\'') orelse {
            _ = pty.write(url[i..]) catch {};
            break;
        };
        _ = pty.write(url[i..next]) catch {};
        _ = pty.write("'\\''") catch {};
        i = next + 1;
    }
    _ = pty.write("'\n") catch {};
}

/// Write an encoded mouse event to the active tab's PTY.
/// `button`: 0 left, 1 middle, 2 right; add 32 for drag, 64/65 for scroll.
/// `cell_col`, `cell_row`: 0-based viewport cell (converted to 1-based internally).
/// `press`: true for button-down / motion, false for release.
fn writeMouseEvent(button: u8, cell_col: usize, cell_row: usize, press: bool) void {
    const fp = focusedPane();
    var mbuf: [32]u8 = undefined;
    const bytes = keys.encodeMouse(
        button,
        cell_col + 1, // 1-based
        cell_row + 1, // 1-based
        press,
        fp.terminal.modes.mouse_sgr,
        &mbuf,
    );
    _ = fp.pty.write(bytes) catch {};
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

    // Check if the click is inside the tree panel.
    if (g.tree_visible) {
        const cw_pt = g.font.metrics.cell_w / g.scale;
        const pad_pt = @as(f64, @floatFromInt(grid_pad)) / g.scale;
        const panel_w_pt = @as(f64, @floatFromInt(filetree_render.tree_cols)) * cw_pt;
        // AppKit view coordinates: x from left, y from bottom.
        if (view_pt.x >= pad_pt and view_pt.x < pad_pt + panel_w_pt) {
            // Map click y to a tree entry index.
            const ch_pt = g.font.metrics.cell_h / g.scale;
            const top_bar_h_pt = @as(f64, @floatFromInt(topBarRows())) * ch_pt;
            // View y is 0 at bottom; tree starts at top (below tab bar).
            const tree_top_pt = top_bar_h_pt + pad_pt;
            const click_y_from_top = b.size.height - view_pt.y;
            if (filetree_render.treeRowAtClick(click_y_from_top, tree_top_pt, ch_pt, 1)) |row_in_tree| {
                if (row_in_tree < g.tree.count) {
                    const e = &g.tree.entries[row_in_tree];
                    g.tree.selected_idx = row_in_tree;
                    if (e.is_dir) {
                        g.tree.toggle(row_in_tree);
                    } else {
                        // File click: open the file via the shell.
                        // Write: Ctrl-U (clear line) + "${EDITOR:-open} '<path>'" + Enter.
                        ptyWriteOpenFile(e.pathSlice());
                    }
                    g.dirty = true;
                }
            }
            return; // tree panel click — do not start text selection
        }
    }

    // Mouse reporting: forward to PTY instead of starting a local selection.
    const tm = &focusedPane().terminal;
    if (tm.modes.mouse_button or tm.modes.mouse_x10) {
        if (eventCell(event, false)) |cl| {
            writeMouseEvent(0, cl.col, cl.row, true); // left button press
        }
        return;
    }

    // ⌘-click: open a file path or URL under the cursor. Does not start selection.
    const flags = event.msgSend(c_ulong, "modifierFlags", .{});
    if (flags & (1 << 20) != 0) { // NSEventModifierFlagCommand
        if (eventCell(event, false)) |cl| {
            const t = &focusedPane().terminal;
            const crow = t.contentRowOfViewport(cl.row);
            const cells = t.line(crow);
            // Decode the row to UTF-8 so we can do string token extraction.
            var line_buf: [4096]u8 = undefined;
            var line_len: usize = 0;
            // Also build a column→byte-offset map (one entry per cell).
            var col_to_byte: [4096]usize = undefined;
            var ci: usize = 0;
            while (ci < cells.len and line_len + 4 < line_buf.len) : (ci += 1) {
                col_to_byte[ci] = line_len;
                const cp = cells[ci].cp;
                if (cp == 0 or cp == ' ') {
                    line_buf[line_len] = ' ';
                    line_len += 1;
                } else {
                    const n = std.unicode.utf8Encode(cp, line_buf[line_len..]) catch {
                        line_buf[line_len] = ' ';
                        line_len += 1;
                        continue;
                    };
                    line_len += n;
                }
            }
            const line_str = line_buf[0..line_len];
            const byte_col = if (cl.col < ci) col_to_byte[cl.col] else line_len;
            const raw_tok = interact.tokenAtCol(line_str, byte_col);
            const tok = interact.stripLineSuffix(raw_tok);
            const cwd = currentCwd() orelse "";
            switch (interact.classify(tok, cwd)) {
                .url => ptyWriteOpenUrl(tok),
                .path => ptyWriteOpenFile(tok),
                .none => {},
            }
        }
        return; // ⌘-click never starts a text selection
    }

    // Grid click — begin selection.
    const cell = eventCell(event, false) orelse {
        focusedPane().selection.clear();
        g.dirty = true;
        return;
    };
    const fp2 = focusedPane();
    const content_row = fp2.terminal.contentRowOfViewport(cell.row);
    fp2.selection = .{
        .active = true,
        .anchor = .{ .row = content_row, .col = cell.col },
        .head = .{ .row = content_row, .col = cell.col },
    };
    g.dirty = true;
}

fn onMouseDragged(event: objc.Object) void {
    // Mouse reporting: forward drag as button-motion event.
    const tm = &focusedPane().terminal;
    if (tm.modes.mouse_button) {
        if (eventCell(event, false)) |cl| {
            writeMouseEvent(0 + 32, cl.col, cl.row, true); // left drag = button 0 + 32
        }
        return;
    }
    const fp3 = focusedPane();
    if (!fp3.selection.active) return;
    const cell = eventCell(event, true) orelse return;
    const content_row = fp3.terminal.contentRowOfViewport(cell.row);
    fp3.selection.head = .{ .row = content_row, .col = cell.col };
    g.dirty = true;
}

fn onMouseUp(event: objc.Object) void {
    // Mouse reporting: forward release to PTY (SGR only; legacy X10 has no release).
    const tm = &focusedPane().terminal;
    if (tm.modes.mouse_button and tm.modes.mouse_sgr) {
        if (eventCell(event, false)) |cl| {
            writeMouseEvent(0, cl.col, cl.row, false); // left button release
        }
        g.dirty = true;
        return;
    }
    if (tm.modes.mouse_button) {
        // Legacy mouse_button mode: no explicit release byte needed (app infers from next press).
        g.dirty = true;
        return;
    }
    // A click with no drag (anchor == head) means no real selection.
    const fp4 = focusedPane();
    if (fp4.selection.active and
        fp4.selection.anchor.row == fp4.selection.head.row and
        fp4.selection.anchor.col == fp4.selection.head.col)
    {
        fp4.selection.clear();
    }
    g.dirty = true;
}

/// Extract selected text from the active tab's terminal and write it to the
/// macOS general pasteboard.
fn copySelection() void {
    const fp5 = focusedPane();
    const term_obj = &fp5.terminal;
    const o = fp5.selection.ordered();
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

/// Populate `g.hud` from live data: cwd, git status, last-run state.
fn refreshHud() void {
    const cur_term = &focusedPane().terminal;

    // --- cwd ---
    const cwd = cur_term.cwdPath();
    const cwd_len = @min(cwd.len, g.hud.cwd.len);
    @memcpy(g.hud.cwd[0..cwd_len], cwd[0..cwd_len]);
    g.hud.cwd_len = cwd_len;

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

    g.dirty = true;
}

// --- rendering -----------------------------------------------------------

/// Open the search bar (re-scanning the active tab) and reflow for the row.
fn openSearch() void {
    if (g.search_open) return;
    g.search_open = true;
    g.search.setQuery(&focusedPane().terminal, g.search.query());
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

    // When the tree panel is visible, the terminal grid is shifted right by the
    // panel width. Tab bar, HUD, search bar, and tree panel all draw in absolute
    // space (origin_x = 0, origin_y = 0 — the defaults after drawWorkspace resets them).
    const tree_offset_px: f64 = if (g.tree_visible)
        @as(f64, @floatFromInt(filetree_render.tree_cols)) * g.font.metrics.cell_w
    else
        0;

    // Inner content rect: the window area minus top bar and file-tree panel.
    // origin_y uses pad_y + top bar height so drawWorkspace passes top_bar_rows=0.
    const cell_h_f64: f64 = g.font.metrics.cell_h;
    const inner: workspace_mod.layout_mod.Rect = .{
        .x = g.raster.pad_x + tree_offset_px,
        .y = g.raster.pad_y + @as(f64, @floatFromInt(topBarRows())) * cell_h_f64,
        .w = @as(f64, @floatFromInt(g.raster.width)) - 2 * g.raster.pad_x - tree_offset_px,
        .h = @as(f64, @floatFromInt(g.raster.height)) - 2 * g.raster.pad_y -
            @as(f64, @floatFromInt(topBarRows())) * cell_h_f64,
    };

    const search_ptr: ?*const Search = if (g.search_open) &g.search else null;
    const focused_id = g.tabs.current().tree.focused;
    workspace_mod.drawWorkspace(
        &g.raster,
        &g.tabs.current().tree,
        &g.tabs.current().registry,
        inner,
        workspace_mod.divider_px,
        g.font,
        g.theme,
        search_ptr,
        focused_id,
        g.blink_phase,
        g.cursor_cfg,
    );

    // Origin is reset to 0 by drawWorkspace. Draw UI chrome in absolute space.

    // Tab bar on top of the grid so a gliding top row cannot bleed into it.
    if (g.tabs.barVisible()) {
        tabbar.drawTabBar(&g.raster, g.font, g.theme, &g.tabs);
    }

    if (g.search_open) {
        const ch: usize = @intFromFloat(g.font.metrics.cell_h);
        const total_rows = @max((g.raster.height -| 2 * grid_pad) / ch, 1);
        searchbar.drawSearchBar(&g.raster, g.font, g.theme, &g.search, total_rows - 1);
    }

    if (g.hud_visible) {
        // The HUD floats in the top-right corner of the WINDOW. Position it
        // from the raster's full width in cells — NOT terminal `cols`, which
        // shrinks when the file tree is open (that put the card mid-window).
        const ch: usize = @intFromFloat(g.font.metrics.cell_h);
        const cw: usize = @intFromFloat(g.font.metrics.cell_w);
        const total_rows = @max((g.raster.height -| 2 * grid_pad) / ch, 1);
        const total_cols = @max((g.raster.width -| 2 * grid_pad) / cw, 1);
        hud_mod.draw(
            &g.raster,
            g.font,
            g.theme,
            g.hud,
            total_cols,
            total_rows,
            topBarRows(),
        );
    }

    if (g.tree_visible) {
        const ch: usize = @intFromFloat(g.font.metrics.cell_h);
        const total_rows = @max((g.raster.height -| 2 * grid_pad) / ch, 1);
        filetree_render.draw(
            &g.raster,
            g.font,
            g.theme,
            &g.tree,
            total_rows,
            topBarRows(),
        );
    }

    if (g.cheatsheet_visible) {
        const ch: usize = @intFromFloat(g.font.metrics.cell_h);
        const cw: usize = @intFromFloat(g.font.metrics.cell_w);
        const total_rows = @max((g.raster.height -| 2 * grid_pad) / ch, 1);
        const total_cols = @max((g.raster.width -| 2 * grid_pad) / cw, 1);
        cheatsheet_mod.draw(
            &g.raster,
            g.font,
            g.theme,
            total_cols,
            total_rows,
        );
    }

    g.renderer.present(g.raster.bytes(), metal_mod.presentMode(viewInLiveResize()) == .sync);
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
    _ = @import("render/draw.zig");
    _ = @import("render/metal.zig");
    _ = @import("render/filetree.zig");
    _ = @import("app/keys.zig");
    _ = @import("app/tab.zig");
    _ = @import("app/shell_integration.zig");
    _ = @import("render/tabbar.zig");
    _ = @import("render/searchbar.zig");
    _ = @import("render/hud.zig");
    _ = @import("render/cheatsheet.zig");
    _ = @import("terminal/terminal.zig");
    _ = @import("terminal/search.zig");
    _ = @import("pty/pty.zig");
    _ = @import("ipc/bridge.zig");
    _ = @import("app/palette.zig");
    _ = @import("app/selection.zig");
    _ = @import("app/filetree.zig");
    _ = @import("app/interact.zig");
    _ = @import("testing/counting_allocator.zig");
    _ = @import("workspace/layout.zig");
    _ = @import("workspace/pane.zig");
    _ = @import("render/workspace.zig");
}
