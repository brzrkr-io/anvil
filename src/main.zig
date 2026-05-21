//! Caldera Console — M1 entry point. Wires the terminal model, the PTY, the
//! Metal renderer, the CoreGraphics rasterizer, and AppKit input into a
//! single-pane GPU terminal.

const std = @import("std");
const objc = @import("objc");
const c = objc.c;

const term = @import("terminal/terminal.zig");
const Pty = @import("pty/pty.zig").Pty;
const Font = @import("render/font.zig").Font;
const Raster = @import("render/raster.zig").Raster;
const Renderer = @import("render/metal.zig").Renderer;
const Theme = @import("config/theme.zig").Theme;
const theme_mod = @import("config/theme.zig");
const cfg_mod = @import("config/config.zig");
const keys = @import("app/keys.zig");

const CGPoint = extern struct { x: f64, y: f64 };
const CGSize = extern struct { width: f64, height: f64 };
const CGRect = extern struct { origin: CGPoint, size: CGSize };

const app_icon_png = @embedFile("assets/app-icon.png");

var config_path_buf: [std.fs.max_path_bytes]u8 = undefined;

// --- PTY -> main-thread handoff ------------------------------------------
// The reader thread appends bytes here; the 60 Hz tick drains them.
var pty_buf: [1 << 20]u8 = undefined;
var pty_len: usize = 0;
var pty_mutex: std.atomic.Mutex = .unlocked;
var pty_dead: bool = false;
var feed_scratch: [1 << 20]u8 = undefined;

/// Brief spin-lock around the PTY handoff buffer — critical sections are just
/// memcpys, so spinning (with a yield) is cheaper than a futex.
fn lockPty() void {
    while (!pty_mutex.tryLock()) std.Thread.yield() catch {};
}
fn unlockPty() void {
    pty_mutex.unlock();
}

const App = struct {
    alloc: std.mem.Allocator,
    terminal: term.Terminal,
    pty: Pty,
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

// --- event handling ------------------------------------------------------

fn applyConfig(new_loaded: cfg_mod.Loaded) void {
    const nl = new_loaded;
    const nc = nl.config;
    g.theme = theme_mod.resolve(nc.theme, nc.theme_overrides);
    g.renderer.setClearColor(g.theme.background); // keep the GPU clear in sync
    g.cursor_cfg = nc.cursor;
    g.dirty = true;
    g.config.deinit(); // free the previous config's arena
    g.config = nl; // the new arena owns the strings resolve() just read
}

fn onTick() void {
    if (g.watcher.path.len > 0) {
        if (g.watcher.poll(g.alloc)) |new_loaded| {
            applyConfig(new_loaded);
        }
    }
    lockPty();
    const n = pty_len;
    if (n > 0) {
        @memcpy(feed_scratch[0..n], pty_buf[0..n]);
        pty_len = 0;
    }
    const dead = pty_dead;
    unlockPty();

    if (n > 0) {
        g.terminal.feed(feed_scratch[0..n]);
        g.dirty = true;
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
    if (dead) g.nsapp.msgSend(void, "terminate:", .{@as(c.id, null)});
}

fn onResize() void {
    const b = g.view.msgSend(CGRect, "bounds", .{});
    const dw: usize = @intFromFloat(@max(b.size.width * g.scale, 1));
    const dh: usize = @intFromFloat(@max(b.size.height * g.scale, 1));
    const cw: usize = @intFromFloat(g.font.metrics.cell_w);
    const ch: usize = @intFromFloat(g.font.metrics.cell_h);
    const cols = @max(dw / cw, 1);
    const rows = @max(dh / ch, 1);

    g.terminal.resize(cols, rows);
    g.pty.resize(@intCast(cols), @intCast(rows));
    g.raster.resize(dw, dh) catch {};
    g.renderer.resize(dw, dh);
    g.dirty = true;
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

fn onKeyDown(event: objc.Object) void {
    const p = extractKey(event) orelse return;
    var buf: [16]u8 = undefined;
    const bytes = keys.encode(p.key, p.mods, false, &buf);
    _ = g.pty.write(bytes) catch {};
    g.terminal.scrollToBottom();
    g.dirty = true;
}

fn onScroll(event: objc.Object) void {
    const dy = event.msgSend(f64, "scrollingDeltaY", .{});
    if (dy == 0) return;
    const mag: f64 = @max(@abs(dy) / 8.0, 1.0);
    const lines: isize = @intFromFloat(mag);
    g.terminal.scrollViewport(if (dy > 0) lines else -lines);
    g.dirty = true;
}

// --- rendering -----------------------------------------------------------

fn renderFrame() void {
    g.raster.clear(g.theme.background);
    const rows = g.terminal.rows();
    const cols = g.terminal.cols();

    var y: usize = 0;
    while (y < rows) : (y += 1) {
        const line = g.terminal.viewportRow(y);
        var x: usize = 0;
        while (x < cols and x < line.len) : (x += 1) {
            drawCell(x, y, line[x], false);
        }
    }

    const cur = g.terminal.cursor();
    if (cur.visible and g.terminal.viewportOffset() == 0 and cur.y < rows and cur.x < cols) {
        drawCursor(cur.x, cur.y);
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
    if (is_cursor or !std.mem.eql(u8, &bg, &g.theme.background)) {
        g.raster.cellBg(g.font, x, y, bg);
    }
    if (cell.cp != ' ' and cell.cp != 0) {
        g.raster.cellGlyph(g.font, x, y, g.font.glyph(cell.cp), fg);
    }
}

fn drawCursor(x: usize, y: usize) void {
    const line = g.terminal.viewportRow(y);
    const cell: term.Cell = if (x < line.len) line[x] else .{};
    if (g.cursor_cfg.blink and !g.blink_on) {
        // Blinked off: draw the cell with no cursor styling.
        drawCell(x, y, cell, false);
        return;
    }
    switch (g.cursor_cfg.style) {
        .block => drawCell(x, y, cell, true),
        .bar => {
            drawCell(x, y, cell, false);
            g.raster.cellInset(g.font, x, y, g.theme.accent, 0.0, 0.0, 0.15, 1.0);
        },
        .underline => {
            drawCell(x, y, cell, false);
            g.raster.cellInset(g.font, x, y, g.theme.accent, 0.0, 0.0, 1.0, 0.12);
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

// --- PTY reader thread ---------------------------------------------------

fn ptyReaderThread() void {
    var local: [64 << 10]u8 = undefined;
    while (true) {
        const n = g.pty.read(&local) catch break;
        if (n == 0) break;
        var off: usize = 0;
        while (off < n) {
            lockPty();
            const space = pty_buf.len - pty_len;
            if (space == 0) {
                unlockPty();
                continue; // tick will drain shortly
            }
            const take = @min(space, n - off);
            @memcpy(pty_buf[pty_len..][0..take], local[off..][0..take]);
            pty_len += take;
            unlockPty();
            off += take;
        }
    }
    lockPty();
    pty_dead = true;
    unlockPty();
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

    const active_theme = theme_mod.resolve(cfg.theme, cfg.theme_overrides);

    // Font: configured family first, then fallbacks. dupeZ into the config
    // arena so the slice outlives this stack frame (font stack needs [:0]const u8).
    const fam_z = loaded.arena.allocator().dupeZ(u8, cfg.font.family) catch "IBMPlexMono";
    const font_names = [_][:0]const u8{ fam_z, "SFMono-Regular", "Menlo" };
    const font = Font.initFirstAvailable(&font_names, cfg.font.size * scale) catch |e| fail("font", e);
    const dw: usize = @intFromFloat(cfg.window.width * scale);
    const dh: usize = @intFromFloat(cfg.window.height * scale);
    const cw: usize = @intFromFloat(font.metrics.cell_w);
    const ch: usize = @intFromFloat(font.metrics.cell_h);
    const cols = @max(dw / cw, 1);
    const rows = @max(dh / ch, 1);

    g = .{
        .alloc = alloc,
        .terminal = term.Terminal.init(alloc, cols, rows, cfg.scrollback) catch |e| fail("terminal", e),
        .pty = Pty.spawnShell(alloc, @intCast(cols), @intCast(rows)) catch |e| fail("pty", e),
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
    };
    g.renderer.setClearColor(active_theme.background);

    _ = std.Thread.spawn(.{}, ptyReaderThread, .{}) catch |e| fail("thread", e);
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
    _ = @import("render/tabbar.zig");
    _ = @import("terminal/terminal.zig");
    _ = @import("pty/pty.zig");
}
