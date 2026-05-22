//! Caldera Console — M1 entry point. Wires the terminal model, the PTY, the
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
const Search = @import("terminal/search.zig").Search;
const searchbar = @import("render/searchbar.zig");
const webview_mod = @import("webview/webview.zig");
const palette_mod = @import("app/palette.zig");
const bridge = @import("ipc/bridge.zig");

const CGPoint = extern struct { x: f64, y: f64 };
const CGSize = extern struct { width: f64, height: f64 };
const CGRect = extern struct { origin: CGPoint, size: CGSize };

const app_icon_png = @embedFile("assets/app-icon.png");
const palette_html: [:0]const u8 = @embedFile("palette_html");

var config_path_buf: [std.fs.max_path_bytes]u8 = undefined;

// Uniform inset (device pixels) between the window edge and the terminal grid.
// The margin shows the background color; the grid simply has fewer cells.
const grid_pad: usize = 22;

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
    blink_on: bool = true,
    blink_ticks: u32 = 0,
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
    search: Search,
    search_open: bool = false,
    webview: webview_mod.Webview,
    palette: palette_mod.Palette = .{},
    system_dark: bool = false,
};
var g: App = undefined;

// --- Objective-C method implementations ----------------------------------

fn imShouldTerminate(_: c.id, _: c.SEL, _: c.id) callconv(.c) bool {
    return true;
}

fn imWindowDidResize(_: c.id, _: c.SEL, _: c.id) callconv(.c) void {
    onResize();
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
    const js = std.fmt.allocPrintSentinel(g.alloc, "window.caldera.receive({s});", .{json}, 0) catch return;
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
    const js = std.fmt.allocPrintSentinel(g.alloc, "window.caldera.receive({s});", .{json}, 0) catch return;
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
            g.dirty = true;
        },
        .scroll_bottom => {
            g.tabs.current().terminal.scrollToBottom();
            g.dirty = true;
        },
        .app_quit => g.nsapp.msgSend(void, "terminate:", .{@as(c.id, null)}),
    }
}

fn handleWebMessage(json: []const u8) void {
    const msg = bridge.decode(g.alloc, json) catch |e| {
        std.debug.print("caldera-console: webview message decode failed: {s}\n", .{@errorName(e)});
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
                std.debug.print("caldera-console: unknown command id: {s}\n", .{id});
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

    if (g.cursor_cfg.blink) {
        g.blink_ticks += 1;
        if (g.blink_ticks >= 32) {
            g.blink_ticks = 0;
            g.blink_on = !g.blink_on;
            g.dirty = true;
        }
    } else if (!g.blink_on) {
        g.blink_on = true;
        g.dirty = true;
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
    g.dirty = true;
}

/// Size every tab's terminal + pty to the current window, minus the bar row.
fn resizeAllTabs() void {
    const b = g.view.msgSend(CGRect, "bounds", .{});
    const dw: usize = @intFromFloat(@max(b.size.width * g.scale, 1));
    const dh: usize = @intFromFloat(@max(b.size.height * g.scale, 1));
    const cw: usize = @intFromFloat(g.font.metrics.cell_w);
    const ch: usize = @intFromFloat(g.font.metrics.cell_h);
    const cols = @max((dw -| 2 * grid_pad) / cw, 1);
    const total_rows = @max((dh -| 2 * grid_pad) / ch, 1);
    const rows = @max(total_rows -| topBarRows() -| bottomBarRows(), 1);

    for (g.tabs.tabs.items) |tab| {
        tab.terminal.resize(cols, rows);
        tab.pty.resize(@intCast(cols), @intCast(rows));
    }
    g.raster.resize(dw, dh) catch {};
    g.renderer.resize(dw, dh);
    g.dirty = true;
}

fn onResize() void {
    resizeAllTabs();
    const b = g.view.msgSend(CGRect, "bounds", .{});
    g.webview.setFrame(b.size.width, b.size.height);
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
        g.dirty = true;
        return true;
    };
    if (g.keys_prev) |ch| if (chordMatches(ch, mods, cp)) {
        closeSearch();
        g.tabs.prev();
        g.dirty = true;
        return true;
    };
    for (g.keys_jump, 0..) |maybe, i| {
        if (maybe) |ch| if (chordMatches(ch, mods, cp)) {
            closeSearch();
            g.tabs.switchTo(i);
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
    return false;
}

/// Scroll the active tab so the current search match is visible.
fn scrollToCurrentMatch() void {
    if (g.search.currentMatch()) |m| {
        g.tabs.current().terminal.scrollToLine(m.row);
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
        std.debug.print("caldera-console: new tab failed: {s}\n", .{@errorName(e)});
        return;
    };
    resizeAllTabs();
    g.dirty = true;
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
    g.dirty = true;
}

fn onScroll(event: objc.Object) void {
    const dy = event.msgSend(f64, "scrollingDeltaY", .{});
    if (dy == 0) return;
    const mag: f64 = @max(@abs(dy) / 8.0, 1.0);
    const lines: isize = @intFromFloat(mag);
    g.tabs.current().terminal.scrollViewport(if (dy > 0) lines else -lines);
    g.dirty = true;
}

fn onMouseDown(event: objc.Object) void {
    if (!g.tabs.barVisible()) return;

    const win_pt = event.msgSend(CGPoint, "locationInWindow", .{});
    const view_pt = g.view.msgSend(CGPoint, "convertPoint:fromView:", .{
        win_pt, @as(c.id, null),
    });
    const b = g.view.msgSend(CGRect, "bounds", .{});
    // bounds/point are bottom-left origin: the bar occupies the top.
    const ch_pt = g.font.metrics.cell_h / g.scale; // bar height in points
    if (view_pt.y < b.size.height - ch_pt) return; // click below the bar

    const n = g.tabs.count();
    if (n == 0) return;
    const frac = std.math.clamp(view_pt.x / b.size.width, 0.0, 0.999);
    const idx: usize = @intFromFloat(frac * @as(f64, @floatFromInt(n)));
    g.tabs.switchTo(idx);
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

    if (g.tabs.barVisible()) {
        tabbar.drawTabBar(&g.raster, g.font, g.theme, &g.tabs);
    }

    const t = g.tabs.current();
    const rows = t.terminal.rows();
    const cols = t.terminal.cols();

    var y: usize = 0;
    while (y < rows) : (y += 1) {
        const line = t.terminal.viewportRow(y);
        var x: usize = 0;
        while (x < cols and x < line.len) : (x += 1) {
            drawCell(x, y, line[x], false);
        }
    }

    const cur = t.terminal.cursor();
    if (cur.visible and t.terminal.viewportOffset() == 0 and cur.y < rows and cur.x < cols) {
        drawCursor(cur.x, cur.y);
    }

    if (g.search_open) {
        const ch: usize = @intFromFloat(g.font.metrics.cell_h);
        const total_rows = @max((g.raster.height -| 2 * grid_pad) / ch, 1);
        searchbar.drawSearchBar(&g.raster, g.font, g.theme, &g.search, total_rows - 1);
    }

    g.renderer.present(g.raster.bytes());
}

fn drawCell(x: usize, y: usize, cell: term.Cell, is_cursor: bool) void {
    var fg = resolve(cell.fg, g.theme.foreground);
    var bg = resolve(cell.bg, g.theme.background);
    if (cell.attrs.inverse) {
        const t = fg;
        fg = bg;
        bg = t;
    }
    if (is_cursor) {
        bg = g.theme.accent;
        fg = g.theme.background;
    }
    if (g.search_open and !is_cursor) {
        const crow = g.tabs.current().terminal.contentRowOfViewport(y);
        switch (g.search.classify(crow, x)) {
            .current => bg = g.theme.accent,
            .other => bg = g.theme.ansi[8],
            .none => {},
        }
    }
    const ry = y + topBarRows(); // raster row: offset by top bar when visible
    if (is_cursor or !std.mem.eql(u8, &bg, &g.theme.background)) {
        g.raster.cellBg(g.font, x, ry, bg);
    }
    if (cell.cp != ' ' and cell.cp != 0) {
        g.raster.cellGlyph(g.font, x, ry, g.font.glyph(cell.cp), fg);
    }
}

fn drawCursor(x: usize, y: usize) void {
    const t = g.tabs.current();
    const line = t.terminal.viewportRow(y);
    const cell: term.Cell = if (x < line.len) line[x] else .{};
    if (g.cursor_cfg.blink and !g.blink_on) {
        // Blinked off: draw the cell with no cursor styling.
        drawCell(x, y, cell, false);
        return;
    }
    const ry = y + topBarRows(); // raster row: offset by top bar when visible
    switch (g.cursor_cfg.style) {
        .block => drawCell(x, y, cell, true),
        .bar => {
            drawCell(x, y, cell, false);
            g.raster.cellInset(g.font, x, ry, g.theme.accent, 0.0, 0.0, 0.15, 1.0);
        },
        .underline => {
            drawCell(x, y, cell, false);
            g.raster.cellInset(g.font, x, ry, g.theme.accent, 0.0, 0.0, 1.0, 0.12);
        },
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
    std.debug.print("caldera-console: {s} init failed: {s}\n", .{ what, @errorName(err) });
    std.process.exit(1);
}

pub fn main() void {
    const alloc = std.heap.c_allocator;

    const nsapp = objc.getClass("NSApplication").?
        .msgSend(objc.Object, "sharedApplication", .{});
    nsapp.msgSend(void, "setActivationPolicy:", .{@as(c_long, 0)});
    setApplicationIcon(nsapp);

    // Delegate class: app lifecycle, resize, and the render tick.
    const Delegate = objc.allocateClassPair(objc.getClass("NSObject").?, "CalderaDelegate").?;
    _ = Delegate.addMethod("applicationShouldTerminateAfterLastWindowClosed:", imShouldTerminate);
    _ = Delegate.addMethod("windowDidResize:", imWindowDidResize);
    _ = Delegate.addMethod("tick:", imTick);
    objc.registerClassPair(Delegate);

    // View class: keyboard and scroll input.
    const View = objc.allocateClassPair(objc.getClass("NSView").?, "CalderaTerminalView").?;
    _ = View.addMethod("acceptsFirstResponder", imAcceptsFirstResponder);
    _ = View.addMethod("keyDown:", imKeyDown);
    _ = View.addMethod("scrollWheel:", imScrollWheel);
    _ = View.addMethod("mouseDown:", imMouseDown);
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
    window.msgSend(void, "setTitle:", .{nsString("Caldera Console")});

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

    // Font: configured family first, then fallbacks. dupeZ into the config
    // arena so the slice outlives this stack frame (font stack needs [:0]const u8).
    const fam_z = loaded.arena.allocator().dupeZ(u8, cfg.font.family) catch "IBMPlexMono";
    const font_names = [_][:0]const u8{ fam_z, "SFMono-Regular", "Menlo" };
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
    _ = @import("terminal/terminal.zig");
    _ = @import("terminal/search.zig");
    _ = @import("pty/pty.zig");
    _ = @import("ipc/bridge.zig");
    _ = @import("app/palette.zig");
}
