# Config + Theme System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Anvil a live-reloadable ZON config file and a theme system (two built-in Mineral themes plus per-color overrides).

**Architecture:** Two new modules under `src/config/` — `config.zig` (the `Config` struct, ZON loading into an arena, an mtime-polling `Watcher`) and `theme.zig` (the `Theme` struct, built-in themes, override resolution). The renderer becomes theme-driven instead of using hardcoded color constants. `main.zig` loads config at startup, polls for changes on the existing 60 Hz tick, and applies theme/cursor changes live.

**Tech Stack:** Zig 0.16, `std.zon.parse`, CoreGraphics/CoreText raster, AppKit.

**Spec:** `docs/superpowers/specs/2026-05-21-config-theme-design.md` — read it first.

**Branch:** Do this work on a branch `feat/m2-config-theme` cut from `main`. The repo currently carries uncommitted brand-alignment changes (`render/color.zig`, `render/font.zig`, `main.zig`, `todo.txt`, `wiki/log.md`); commit those first as their own commit so later task commits stay clean.

---

## Task 0: Branch and baseline

**Files:** none (git only)

- [ ] **Step 1: Create the branch**

```bash
git checkout -b feat/m2-config-theme
```

- [ ] **Step 2: Commit the pre-existing brand-alignment work**

```bash
git add src/render/color.zig src/render/font.zig src/main.zig todo.txt wiki/log.md
git commit -m "feat: brand-align M1 renderer (font, palette, background, accent)"
```

- [ ] **Step 3: Verify baseline is green**

Run: `zig build test`
Expected: all 102 tests pass, exit 0.

---

## Task 1: `Config` struct and ZON parsing

**Files:**
- Create: `src/config/config.zig`
- Modify: `src/main.zig` (test aggregator block, lines 380-387)

The `Config` struct, the `Overrides` struct, `CursorStyle`, and `parseSlice` — parse a ZON string into a `Config`. Strings are owned by an arena (Zig 0.16's `std.zon.parse.free` cannot free struct-default string literals, so the whole result lives in an arena instead).

- [ ] **Step 1: Write `src/config/config.zig` with the types and `parseSlice`**

```zig
//! User configuration, loaded from ~/.config/anvil/config.zon.
//!
//! The file is ZON (Zig Object Notation). Parsing allocates strings into an
//! arena owned by the returned `Loaded` value — `std.zon.parse.free` cannot be
//! used here because absent fields keep struct-default string *literals*, and
//! freeing static memory is illegal. `Loaded.deinit` frees the arena.

const std = @import("std");

pub const CursorStyle = enum { block, bar, underline };

/// Optional per-color overrides applied on top of the chosen base theme.
/// Every field is optional; an absent field keeps the base theme's value.
pub const Overrides = struct {
    background: ?[]const u8 = null,
    foreground: ?[]const u8 = null,
    accent: ?[]const u8 = null,
    ansi: Ansi = .{},

    pub const Ansi = struct {
        black: ?[]const u8 = null,
        red: ?[]const u8 = null,
        green: ?[]const u8 = null,
        yellow: ?[]const u8 = null,
        blue: ?[]const u8 = null,
        magenta: ?[]const u8 = null,
        cyan: ?[]const u8 = null,
        white: ?[]const u8 = null,
        bright_black: ?[]const u8 = null,
        bright_red: ?[]const u8 = null,
        bright_green: ?[]const u8 = null,
        bright_yellow: ?[]const u8 = null,
        bright_blue: ?[]const u8 = null,
        bright_magenta: ?[]const u8 = null,
        bright_cyan: ?[]const u8 = null,
        bright_white: ?[]const u8 = null,
    };
};

pub const Config = struct {
    scrollback: usize = 100_000,
    font: FontCfg = .{},
    cursor: CursorCfg = .{},
    window: WindowCfg = .{},
    theme: []const u8 = "mineral-dark",
    theme_overrides: Overrides = .{},

    pub const FontCfg = struct {
        family: []const u8 = "IBM Plex Mono",
        size: f64 = 14.0,
    };
    pub const CursorCfg = struct {
        style: CursorStyle = .block,
        blink: bool = true,
    };
    pub const WindowCfg = struct {
        width: f64 = 1024.0,
        height: f64 = 640.0,
    };

    /// Pull every out-of-range value back to a usable minimum.
    fn clamp(self: *Config) void {
        if (self.scrollback < 1) self.scrollback = 1;
        if (!(self.font.size >= 4.0)) self.font.size = 14.0; // also catches NaN
        if (!(self.window.width >= 200.0)) self.window.width = 1024.0;
        if (!(self.window.height >= 150.0)) self.window.height = 640.0;
    }
};

/// A parsed config plus the arena that owns its strings. Always `deinit` it.
pub const Loaded = struct {
    arena: std.heap.ArenaAllocator,
    config: Config,

    pub fn deinit(self: *Loaded) void {
        self.arena.deinit();
    }
};

/// Defaults with an empty arena — used for a missing file or a parse failure.
pub fn defaults(backing: std.mem.Allocator) Loaded {
    return .{ .arena = std.heap.ArenaAllocator.init(backing), .config = .{} };
}

/// Parse a ZON source string into a `Loaded`. On any ZON error the error text
/// is printed to stderr and `error.ParseFailed` is returned (the caller then
/// falls back to `defaults`).
pub fn parseSlice(backing: std.mem.Allocator, source: [:0]const u8) error{ParseFailed}!Loaded {
    var arena = std.heap.ArenaAllocator.init(backing);
    errdefer arena.deinit();
    const a = arena.allocator();

    var diag: std.zon.parse.Diagnostics = .{};
    var cfg = std.zon.parse.fromSliceAlloc(Config, a, source, &diag, .{
        .free_on_error = false, // the arena cleans up wholesale
    }) catch {
        std.debug.print("anvil: config parse error:\n{f}", .{diag});
        return error.ParseFailed;
    };
    cfg.clamp();
    return .{ .arena = arena, .config = cfg };
}
```

- [ ] **Step 2: Add the new module to the test aggregator**

In `src/main.zig`, the `test { ... }` block at the end (lines 380-387) imports each module so `zig build test` runs its tests. Add **only** the `config.zig` line now — `theme.zig` does not exist until Task 3, so its line is added in Task 3 Step 4 to keep the build green between tasks:

```zig
test {
    _ = @import("config/config.zig");
    _ = @import("render/color.zig");
    _ = @import("render/font.zig");
    _ = @import("render/raster.zig");
    _ = @import("app/keys.zig");
    _ = @import("terminal/terminal.zig");
    _ = @import("pty/pty.zig");
}
```

- [ ] **Step 3: Write the failing tests at the end of `src/config/config.zig`**

```zig
const testing = std.testing;

test "parses a full config" {
    const src =
        \\.{
        \\    .scrollback = 5000,
        \\    .font = .{ .family = "Menlo", .size = 16.0 },
        \\    .cursor = .{ .style = .bar, .blink = false },
        \\    .window = .{ .width = 800.0, .height = 600.0 },
        \\    .theme = "mineral-light",
        \\    .theme_overrides = .{ .accent = "#3aa0a8" },
        \\}
    ;
    var loaded = try parseSlice(testing.allocator, src);
    defer loaded.deinit();
    const c = loaded.config;
    try testing.expectEqual(@as(usize, 5000), c.scrollback);
    try testing.expectEqualStrings("Menlo", c.font.family);
    try testing.expectEqual(@as(f64, 16.0), c.font.size);
    try testing.expectEqual(CursorStyle.bar, c.cursor.style);
    try testing.expectEqual(false, c.cursor.blink);
    try testing.expectEqualStrings("mineral-light", c.theme);
    try testing.expectEqualStrings("#3aa0a8", c.theme_overrides.accent.?);
}

test "a partial config keeps defaults for absent fields" {
    var loaded = try parseSlice(testing.allocator, ".{ .scrollback = 200 }");
    defer loaded.deinit();
    try testing.expectEqual(@as(usize, 200), loaded.config.scrollback);
    try testing.expectEqualStrings("IBM Plex Mono", loaded.config.font.family);
    try testing.expectEqual(CursorStyle.block, loaded.config.cursor.style);
    try testing.expectEqualStrings("mineral-dark", loaded.config.theme);
}

test "malformed ZON returns ParseFailed" {
    try testing.expectError(error.ParseFailed, parseSlice(testing.allocator, ".{ .scrollback ="));
    try testing.expectError(error.ParseFailed, parseSlice(testing.allocator, ".{ .nonsense = 1 }"));
}

test "out-of-range values are clamped" {
    var loaded = try parseSlice(testing.allocator,
        ".{ .scrollback = 0, .font = .{ .size = 0.0 }, .window = .{ .width = 1.0, .height = 1.0 } }");
    defer loaded.deinit();
    try testing.expectEqual(@as(usize, 1), loaded.config.scrollback);
    try testing.expectEqual(@as(f64, 14.0), loaded.config.font.size);
    try testing.expectEqual(@as(f64, 1024.0), loaded.config.window.width);
    try testing.expectEqual(@as(f64, 640.0), loaded.config.window.height);
}

test "defaults has an empty config" {
    var loaded = defaults(testing.allocator);
    defer loaded.deinit();
    try testing.expectEqual(@as(usize, 100_000), loaded.config.scrollback);
}
```

- [ ] **Step 4: Run the tests**

Run: `zig build test`
Expected: PASS — the new config tests pass, all 102 prior tests still pass. If `std.zon.parse.fromSliceAlloc` or `Diagnostics` formatting differs in the installed Zig, adjust to the real `std.zon` API (verify against `$(zig env | std_dir)/zon/parse.zig`) — the contract is unchanged: parse a `[:0]const u8` into `Config`, arena-owned.

- [ ] **Step 5: Commit**

```bash
git add src/config/config.zig src/main.zig
git commit -m "feat(config): Config struct and ZON parsing"
```

---

## Task 2: Config file loading and the live-reload `Watcher`

**Files:**
- Modify: `src/config/config.zig`

Resolve `~/.config/anvil/config.zon`, load it, and add a `Watcher` that detects file changes by modification time.

- [ ] **Step 1: Add path resolution and `load` to `src/config/config.zig`** (before the test block)

```zig
/// Write the absolute config path into `buf`. Returns the slice, or null when
/// `$HOME` is unset (then the app simply runs on defaults).
pub fn resolvePath(buf: []u8) ?[]const u8 {
    const home = std.posix.getenv("HOME") orelse return null;
    return std.fmt.bufPrint(buf, "{s}/.config/anvil/config.zon", .{home}) catch null;
}

/// Read and parse the config file at `path`. A missing file or any read/parse
/// error yields `defaults` — running the app is never blocked by a bad config.
pub fn load(backing: std.mem.Allocator, path: []const u8) Loaded {
    const file = std.fs.openFileAbsolute(path, .{}) catch |e| {
        if (e != error.FileNotFound)
            std.debug.print("anvil: cannot open config: {s}\n", .{@errorName(e)});
        return defaults(backing);
    };
    defer file.close();

    var buf: [1 << 20]u8 = undefined;
    const n = file.readAll(&buf) catch return defaults(backing);
    if (n >= buf.len) {
        std.debug.print("anvil: config file too large, using defaults\n", .{});
        return defaults(backing);
    }
    buf[n] = 0; // ZON parser needs a sentinel-terminated source
    return parseSlice(backing, buf[0..n :0]) catch defaults(backing);
}
```

- [ ] **Step 2: Add the `Watcher` to `src/config/config.zig`**

```zig
/// Polls the config file's modification time so the render loop can reload it
/// without a file-watcher thread. Cheap: one `stat` per poll.
pub const Watcher = struct {
    path: []const u8,      // borrowed; must outlive the Watcher
    last_mtime: i128 = 0,  // 0 = nothing seen yet (or no file)

    pub fn init(path: []const u8) Watcher {
        return .{ .path = path };
    }

    /// Current mtime of the file in nanoseconds, or 0 if it cannot be stat'd.
    fn mtime(self: *const Watcher) i128 {
        const file = std.fs.openFileAbsolute(self.path, .{}) catch return 0;
        defer file.close();
        const st = file.stat() catch return 0;
        return st.mtime;
    }

    /// If the file changed since the last call, load and return the new config;
    /// otherwise return null. A parse failure still advances the recorded mtime
    /// so the error is reported once, not every poll.
    pub fn poll(self: *Watcher, backing: std.mem.Allocator) ?Loaded {
        const m = self.mtime();
        if (m == self.last_mtime) return null;
        self.last_mtime = m;
        if (m == 0) return defaults(backing); // file was removed -> defaults
        return load(backing, self.path);
    }
};
```

- [ ] **Step 3: Write the failing tests** (add to the test block in `src/config/config.zig`)

```zig
test "load of a missing file yields defaults" {
    var loaded = load(testing.allocator, "/nonexistent/caldera-test-config.zon");
    defer loaded.deinit();
    try testing.expectEqual(@as(usize, 100_000), loaded.config.scrollback);
}

test "Watcher detects a change and reloads" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var path_buf: [std.fs.max_path_bytes]u8 = undefined;
    const path = try tmp.dir.realpath(".", &path_buf);
    var full_buf: [std.fs.max_path_bytes]u8 = undefined;
    const full = try std.fmt.bufPrint(&full_buf, "{s}/config.zon", .{path});

    try tmp.dir.writeFile(.{ .sub_path = "config.zon", .data = ".{ .scrollback = 11 }" });
    var w = Watcher.init(full);

    var first = w.poll(testing.allocator) orelse return error.ExpectedReload;
    defer first.deinit();
    try testing.expectEqual(@as(usize, 11), first.config.scrollback);

    // No change -> no reload.
    try testing.expect(w.poll(testing.allocator) == null);
}
```

- [ ] **Step 4: Run the tests**

Run: `zig build test`
Expected: PASS. If `File.readAll`, `File.stat`, or `tmpDir` APIs differ, adjust to the installed Zig 0.16 std — the behavior contract is unchanged.

- [ ] **Step 5: Commit**

```bash
git add src/config/config.zig
git commit -m "feat(config): file loading and mtime-polling Watcher"
```

---

## Task 3: `Theme` struct, built-in themes, `byName`, `palette256`

**Files:**
- Create: `src/config/theme.zig`
- Modify: `src/main.zig` (add `_ = @import("config/theme.zig");` to the test block)

The `Theme` struct holds every color the renderer needs. The ANSI palette and the 256-color lookup move out of `render/color.zig` into `Theme`.

- [ ] **Step 1: Write `src/config/theme.zig` with the `Theme` type and built-ins**

The `mineral_dark` values are the current brand-aligned palette from `src/render/color.zig` (`default_bg`, `default_fg`, `ansi16`, accent = ANSI slot 6). The `mineral_light` values are new — Mineral hues on a light `bone` background, with "bright" slots *darkened* (light-theme convention) for contrast.

```zig
//! Terminal color themes. A `Theme` is plain data; `resolve` produces an
//! active theme from a base name plus optional per-color overrides.

const std = @import("std");
const config = @import("config.zig");

pub const Theme = struct {
    background: [3]u8,
    foreground: [3]u8,
    accent: [3]u8, // cursor color
    ansi: [16][3]u8,

    /// xterm-style 256-color lookup. Slots 0-15 come from `ansi`; 16-231 are
    /// the 6x6x6 cube; 232-255 are the grayscale ramp.
    pub fn palette256(self: Theme, index: u8) [3]u8 {
        if (index < 16) return self.ansi[index];
        if (index < 232) {
            const i: usize = @as(usize, index) - 16;
            const levels = [6]u8{ 0, 95, 135, 175, 215, 255 };
            return .{ levels[(i / 36) % 6], levels[(i / 6) % 6], levels[i % 6] };
        }
        const v: u8 = @intCast(8 + 10 * (@as(u16, index) - 232));
        return .{ v, v, v };
    }
};

pub const mineral_dark: Theme = .{
    .background = .{ 0x0b, 0x0d, 0x0e },
    .foreground = .{ 0xe8, 0xea, 0xee },
    .accent = .{ 0x2f, 0x7f, 0x86 },
    .ansi = .{
        .{ 0x0b, 0x0d, 0x0e }, .{ 0xb1, 0x3a, 0x30 }, .{ 0x3f, 0x8a, 0x5b }, .{ 0xb0, 0x7a, 0x14 },
        .{ 0x4a, 0x6f, 0x8a }, .{ 0x6a, 0x5f, 0xa3 }, .{ 0x2f, 0x7f, 0x86 }, .{ 0x86, 0x91, 0x9a },
        .{ 0x37, 0x40, 0x46 }, .{ 0xd4, 0x4a, 0x3f }, .{ 0x52, 0xb0, 0x70 }, .{ 0xd4, 0x9a, 0x28 },
        .{ 0x6a, 0x9a, 0xb8 }, .{ 0x8f, 0x84, 0xc8 }, .{ 0x4a, 0xa8, 0xb0 }, .{ 0xe8, 0xea, 0xee },
    },
};

pub const mineral_light: Theme = .{
    .background = .{ 0xee, 0xf1, 0xf2 }, // bone
    .foreground = .{ 0x16, 0x1a, 0x1c }, // charcoal
    .accent = .{ 0x2f, 0x7f, 0x86 },     // mineral
    .ansi = .{
        .{ 0x16, 0x1a, 0x1c }, .{ 0xb1, 0x3a, 0x30 }, .{ 0x3f, 0x8a, 0x5b }, .{ 0x8a, 0x5f, 0x10 },
        .{ 0x4a, 0x6f, 0x8a }, .{ 0x6a, 0x5f, 0xa3 }, .{ 0x2f, 0x7f, 0x86 }, .{ 0xd2, 0xd8, 0xdb },
        .{ 0x86, 0x91, 0x9a }, .{ 0x8f, 0x2e, 0x26 }, .{ 0x2f, 0x6b, 0x45 }, .{ 0xb0, 0x7a, 0x14 },
        .{ 0x3a, 0x58, 0x6e }, .{ 0x53, 0x4a, 0x82 }, .{ 0x25, 0x64, 0x6a }, .{ 0xee, 0xf1, 0xf2 },
    },
};

/// Resolve a base theme by name. An unknown name falls back to `mineral_dark`.
pub fn byName(name: []const u8) Theme {
    if (std.mem.eql(u8, name, "mineral-light")) return mineral_light;
    if (std.mem.eql(u8, name, "mineral-dark")) return mineral_dark;
    std.debug.print("anvil: unknown theme \"{s}\", using mineral-dark\n", .{name});
    return mineral_dark;
}
```

- [ ] **Step 2: Write the failing tests** (end of `src/config/theme.zig`)

```zig
const testing = std.testing;

test "byName resolves built-in themes" {
    try testing.expectEqual(mineral_dark.background, byName("mineral-dark").background);
    try testing.expectEqual(mineral_light.background, byName("mineral-light").background);
}

test "byName falls back to dark for an unknown name" {
    try testing.expectEqual(mineral_dark.background, byName("nope").background);
}

test "palette256 covers the three ranges" {
    try testing.expectEqual([3]u8{ 0x2f, 0x7f, 0x86 }, mineral_dark.palette256(6));
    try testing.expectEqual([3]u8{ 0, 0, 0 }, mineral_dark.palette256(16));
    try testing.expectEqual([3]u8{ 255, 255, 255 }, mineral_dark.palette256(231));
    try testing.expectEqual([3]u8{ 8, 8, 8 }, mineral_dark.palette256(232));
    try testing.expectEqual([3]u8{ 238, 238, 238 }, mineral_dark.palette256(255));
}
```

- [ ] **Step 3: Add theme to the test aggregator**

In `src/main.zig`'s `test { ... }` block add (next to the `config/config.zig` line from Task 1):

```zig
    _ = @import("config/theme.zig");
```

- [ ] **Step 4: Run the tests**

Run: `zig build test`
Expected: PASS — the new theme tests run and pass, all prior tests still pass. (A new pure-data module has no meaningful fail-first state; the import line is what makes its tests execute.)

- [ ] **Step 5: Commit**

```bash
git add src/config/theme.zig src/main.zig
git commit -m "feat(theme): Theme struct, built-in Mineral themes, palette256"
```

---

## Task 4: Theme override resolution

**Files:**
- Modify: `src/config/theme.zig`

`resolve()` builds an active `Theme` from a base name plus `config.Overrides`. Invalid hex in an override is skipped (base value kept) and logged.

- [ ] **Step 1: Add `hexToRgb` and `resolve` to `src/config/theme.zig`** (before the test block)

```zig
/// Parse a `#rrggbb` (or bare `rrggbb`) string into RGB bytes.
pub fn hexToRgb(hex: []const u8) error{InvalidHex}![3]u8 {
    var s = hex;
    if (s.len > 0 and s[0] == '#') s = s[1..];
    if (s.len != 6) return error.InvalidHex;
    return .{
        std.fmt.parseInt(u8, s[0..2], 16) catch return error.InvalidHex,
        std.fmt.parseInt(u8, s[2..4], 16) catch return error.InvalidHex,
        std.fmt.parseInt(u8, s[4..6], 16) catch return error.InvalidHex,
    };
}

/// Apply one optional override onto `slot`. A bad hex string is logged and
/// leaves `slot` unchanged.
fn applyOverride(slot: *[3]u8, maybe_hex: ?[]const u8) void {
    const hex = maybe_hex orelse return;
    slot.* = hexToRgb(hex) catch {
        std.debug.print("anvil: invalid theme color \"{s}\", ignored\n", .{hex});
        return;
    };
}

/// Build the active theme: base theme `name` with `ov` applied on top.
pub fn resolve(name: []const u8, ov: config.Overrides) Theme {
    var t = byName(name);
    applyOverride(&t.background, ov.background);
    applyOverride(&t.foreground, ov.foreground);
    applyOverride(&t.accent, ov.accent);
    applyOverride(&t.ansi[0], ov.ansi.black);
    applyOverride(&t.ansi[1], ov.ansi.red);
    applyOverride(&t.ansi[2], ov.ansi.green);
    applyOverride(&t.ansi[3], ov.ansi.yellow);
    applyOverride(&t.ansi[4], ov.ansi.blue);
    applyOverride(&t.ansi[5], ov.ansi.magenta);
    applyOverride(&t.ansi[6], ov.ansi.cyan);
    applyOverride(&t.ansi[7], ov.ansi.white);
    applyOverride(&t.ansi[8], ov.ansi.bright_black);
    applyOverride(&t.ansi[9], ov.ansi.bright_red);
    applyOverride(&t.ansi[10], ov.ansi.bright_green);
    applyOverride(&t.ansi[11], ov.ansi.bright_yellow);
    applyOverride(&t.ansi[12], ov.ansi.bright_blue);
    applyOverride(&t.ansi[13], ov.ansi.bright_magenta);
    applyOverride(&t.ansi[14], ov.ansi.bright_cyan);
    applyOverride(&t.ansi[15], ov.ansi.bright_white);
    return t;
}
```

- [ ] **Step 2: Write the failing tests** (add to `src/config/theme.zig`'s test block)

```zig
test "resolve with no overrides equals the base theme" {
    const t = resolve("mineral-dark", .{});
    try testing.expectEqual(mineral_dark.background, t.background);
    try testing.expectEqual(mineral_dark.ansi[2], t.ansi[2]);
}

test "resolve applies a valid override" {
    const t = resolve("mineral-dark", .{
        .background = "#101316",
        .ansi = .{ .green = "#52b070" },
    });
    try testing.expectEqual([3]u8{ 0x10, 0x13, 0x16 }, t.background);
    try testing.expectEqual([3]u8{ 0x52, 0xb0, 0x70 }, t.ansi[2]);
    try testing.expectEqual(mineral_dark.foreground, t.foreground); // untouched
}

test "resolve keeps the base value for an invalid-hex override" {
    const t = resolve("mineral-dark", .{ .accent = "not-a-color" });
    try testing.expectEqual(mineral_dark.accent, t.accent);
}

test "hexToRgb parses and rejects" {
    try testing.expectEqual([3]u8{ 0x0b, 0x0d, 0x0e }, try hexToRgb("#0b0d0e"));
    try testing.expectEqual([3]u8{ 0x0b, 0x0d, 0x0e }, try hexToRgb("0b0d0e"));
    try testing.expectError(error.InvalidHex, hexToRgb("#fff"));
    try testing.expectError(error.InvalidHex, hexToRgb("#zzzzzz"));
}
```

- [ ] **Step 3: Run the tests**

Run: `zig build test`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/config/theme.zig
git commit -m "feat(theme): per-color override resolution"
```

---

## Task 5: Make the renderer theme-driven

**Files:**
- Modify: `src/render/color.zig` (remove the moved palette constants)
- Modify: `src/main.zig` (add `theme` to `App`; route color lookups through it)

The palette now lives in `Theme`. `render/color.zig` keeps only `ClearColor`/`hexToClearColor`. `main.zig` holds the active `Theme` and uses it everywhere it currently uses `color.default_*` / `color.palette256`.

- [ ] **Step 1: Trim `src/render/color.zig`**

Delete `mineral_dark_bg`, `default_fg`, `default_bg`, the `ansi16` array, the `palette256` function, and the `"palette256 covers the three ranges"` test (it moved to `theme.zig` in Task 3). Keep `ClearColor`, `hexToClearColor`, and its four hex tests. The file's final content:

```zig
const std = @import("std");

pub const ClearColor = struct { r: f64, g: f64, b: f64, a: f64 };

/// Parse a `#rrggbb` (or bare `rrggbb`) hex string into a normalized,
/// fully-opaque ClearColor. Returns error.InvalidHex on bad length or digits.
pub fn hexToClearColor(hex: []const u8) !ClearColor {
    var s = hex;
    if (s.len > 0 and s[0] == '#') s = s[1..];
    if (s.len != 6) return error.InvalidHex;
    const r = std.fmt.parseInt(u8, s[0..2], 16) catch return error.InvalidHex;
    const g = std.fmt.parseInt(u8, s[2..4], 16) catch return error.InvalidHex;
    const b = std.fmt.parseInt(u8, s[4..6], 16) catch return error.InvalidHex;
    return .{
        .r = @as(f64, @floatFromInt(r)) / 255.0,
        .g = @as(f64, @floatFromInt(g)) / 255.0,
        .b = @as(f64, @floatFromInt(b)) / 255.0,
        .a = 1.0,
    };
}

test "parses #rrggbb" {
    const c = try hexToClearColor("#0b0d0e");
    try std.testing.expectApproxEqAbs(@as(f64, 0x0b) / 255.0, c.r, 1e-9);
    try std.testing.expectApproxEqAbs(@as(f64, 0x0d) / 255.0, c.g, 1e-9);
    try std.testing.expectApproxEqAbs(@as(f64, 0x0e) / 255.0, c.b, 1e-9);
    try std.testing.expectEqual(@as(f64, 1.0), c.a);
}

test "accepts hex without leading #" {
    const c = try hexToClearColor("0b0d0e");
    try std.testing.expectApproxEqAbs(@as(f64, 0x0b) / 255.0, c.r, 1e-9);
}

test "rejects wrong length" {
    try std.testing.expectError(error.InvalidHex, hexToClearColor("#fff"));
    try std.testing.expectError(error.InvalidHex, hexToClearColor(""));
}

test "rejects non-hex digits" {
    try std.testing.expectError(error.InvalidHex, hexToClearColor("#zzzzzz"));
}
```

If `hexToClearColor`/`ClearColor` turn out to be unused after this (check with `rg "color\.(hexToClearColor|ClearColor|mineral_dark_bg)" src`), leave them — they are a small, harmless public API. Do not delete code other tasks might need.

- [ ] **Step 2: Add `theme` to `App` and the import in `src/main.zig`**

Add the import near the other `render` imports (line ~14):

```zig
const Theme = @import("config/theme.zig").Theme;
const theme_mod = @import("config/theme.zig");
```

Add a field to the `App` struct (after `dirty: bool,`):

```zig
    theme: Theme,
```

- [ ] **Step 3: Route `renderFrame`/`drawCell`/`resolve` through `g.theme`**

Replace the body of `renderFrame`, `drawCell`, and `resolve` so colors come from `g.theme`:

```zig
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
        const line = g.terminal.viewportRow(cur.y);
        if (cur.x < line.len) drawCell(cur.x, cur.y, line[cur.x], true);
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

fn resolve(col: term.Color, default: [3]u8) [3]u8 {
    return switch (col) {
        .default => default,
        .palette => |p| g.theme.palette256(p),
        .rgb => |v| v,
    };
}
```

(The cursor drawing here stays full-block; Task 7 replaces it with style/blink-aware drawing.)

- [ ] **Step 4: Initialize `App.theme` in `main`**

In `main`, before the `g = .{ ... }` initializer, add:

```zig
    const active_theme = theme_mod.byName("mineral-dark");
```

and add `.theme = active_theme,` to the `g = .{ ... }` struct literal. (Task 8 replaces this with the configured theme.)

- [ ] **Step 5: Run the build and tests**

Run: `zig build test`
Expected: PASS — color tests trimmed, all others green.

Run: `zig build run`
Expected: window opens, shell renders with the graphite background and brand palette exactly as before — this task is a no-visible-change refactor.

- [ ] **Step 6: Commit**

```bash
git add src/render/color.zig src/main.zig
git commit -m "refactor(render): drive colors from a Theme value"
```

---

## Task 6: Configurable scrollback capacity

**Files:**
- Modify: `src/terminal/terminal.zig` (`Terminal.init` signature, line 103; the `makeTerminal` test helper, line 651)
- Modify: `src/main.zig` (the `Terminal.init` call site, line 356)

`Terminal.init` takes the scrollback capacity instead of hardcoding `scrollback.default_capacity`.

- [ ] **Step 1: Change `Terminal.init` in `src/terminal/terminal.zig`**

Replace the signature and the `history` line (lines 103-108):

```zig
    /// Create a terminal with a `width x height` screen and a scrollback ring
    /// of `scrollback_capacity` rows.
    pub fn init(alloc: std.mem.Allocator, width: usize, height: usize, scrollback_capacity: usize) !Terminal {
        var primary = try grid.Grid.init(alloc, width, height);
        errdefer primary.deinit();
        var alternate = try grid.Grid.init(alloc, width, height);
        errdefer alternate.deinit();
        var history = try scrollback.Scrollback.init(alloc, scrollback_capacity);
```

- [ ] **Step 2: Update the `makeTerminal` test helper** (line 651)

```zig
/// Build a terminal of `cols_n x rows_n`. Caller deinits.
fn makeTerminal(cols_n: usize, rows_n: usize) !Terminal {
    return Terminal.init(testing.allocator, cols_n, rows_n, scrollback.default_capacity);
}
```

- [ ] **Step 3: Update the call site in `src/main.zig`** (line ~356)

For now pass the existing default so behavior is unchanged; Task 8 swaps in `config.scrollback`:

```zig
        .terminal = term.Terminal.init(alloc, cols, rows, 100_000) catch |e| fail("terminal", e),
```

- [ ] **Step 4: Run the tests**

Run: `zig build test`
Expected: PASS — all terminal tests still green (they go through `makeTerminal`). If any other `Terminal.init` call site exists, `rg "Terminal.init" src` and update it the same way.

- [ ] **Step 5: Commit**

```bash
git add src/terminal/terminal.zig src/main.zig
git commit -m "feat(terminal): configurable scrollback capacity"
```

---

## Task 7: Cursor style and blink

**Files:**
- Modify: `src/render/raster.zig` (add a partial-cell fill)
- Modify: `src/main.zig` (cursor config on `App`, blink phase, style-aware drawing)

`block` is the existing full-cell highlight; `bar` and `underline` need a partial-cell fill. Blink toggles a phase off the 60 Hz tick.

- [ ] **Step 1: Add `cellInset` to `src/render/raster.zig`** (after `cellBg`, before `cellGlyph`)

```zig
    /// Fill a sub-rectangle of a cell, insetting from the cell edges by the
    /// given fractions of cell width/height. Used for bar/underline cursors.
    /// `fx`,`fy` are the left/top inset fractions; `fw`,`fh` the size fractions.
    pub fn cellInset(
        self: *Raster,
        font: Font,
        col: usize,
        row: usize,
        rgb: [3]u8,
        fx: f64,
        fy: f64,
        fw: f64,
        fh: f64,
    ) void {
        const r = self.cellRect(font, col, row);
        setFill(self.ctx, rgb);
        capi.CGContextFillRect(self.ctx, .{
            .origin = .{ .x = r.origin.x + r.size.width * fx, .y = r.origin.y + r.size.height * fy },
            .size = .{ .width = r.size.width * fw, .height = r.size.height * fh },
        });
    }
```

Note: `cellRect` is y-up (origin bottom-left), so an *underline* sits at `fy = 0` (cell bottom) and a *bar* at `fx = 0` (cell left).

- [ ] **Step 2: Write the failing test for `cellInset`** (add to `src/render/raster.zig` test block)

```zig
test "cellInset fills a sub-rectangle of a cell" {
    const f = try Font.init("Menlo", 26.0);
    defer f.deinit();
    var r = try Raster.init(std.testing.allocator, 400, 200);
    defer r.deinit();
    r.clear(.{ 0, 0, 0 });
    // A left bar: 15% width, full height, of cell (2,1).
    r.cellInset(f, 2, 1, .{ 90, 0, 0 }, 0.0, 0.0, 0.15, 1.0);
    const lx: usize = @intFromFloat(f.metrics.cell_w * 2.0 + 1);
    const ly: usize = @intFromFloat(f.metrics.cell_h * 1.5);
    try std.testing.expectEqual([3]u8{ 90, 0, 0 }, pixelAt(&r, lx, ly));
    // The cell's right half stays clear.
    const rx: usize = @intFromFloat(f.metrics.cell_w * 2.8);
    try std.testing.expectEqual([3]u8{ 0, 0, 0 }, pixelAt(&r, rx, ly));
}
```

- [ ] **Step 3: Run the test**

Run: `zig build test`
Expected: PASS.

- [ ] **Step 4: Add cursor state to `src/main.zig`**

Add the import (near other config imports):

```zig
const cfg_mod = @import("config/config.zig");
```

Add fields to `App` (after `theme: Theme,`):

```zig
    cursor_cfg: cfg_mod.Config.CursorCfg,
    blink_on: bool = true,
    blink_ticks: u32 = 0,
```

- [ ] **Step 5: Implement blink in `onTick`**

In `onTick`, after the PTY-drain block and before `if (g.dirty)`, add the blink advance (32 ticks ≈ 533 ms at 60 Hz):

```zig
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
```

- [ ] **Step 6: Make the cursor draw style/blink-aware**

In `renderFrame`, replace the cursor block with a call to a new `drawCursor`:

```zig
    const cur = g.terminal.cursor();
    if (cur.visible and g.terminal.viewportOffset() == 0 and cur.y < rows and cur.x < cols) {
        drawCursor(cur.x, cur.y);
    }
```

Add `drawCursor` after `drawCell`:

```zig
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
```

- [ ] **Step 7: Initialize `cursor_cfg` in `main`**

Add `.cursor_cfg = .{},` to the `g = .{ ... }` initializer (Task 8 swaps in the configured value).

- [ ] **Step 8: Run the build, tests, and app**

Run: `zig build test`
Expected: PASS.

Run: `zig build run`
Expected: window opens; the block cursor now blinks (~0.5 s period).

- [ ] **Step 9: Commit**

```bash
git add src/render/raster.zig src/main.zig
git commit -m "feat(render): cursor style (block/bar/underline) and blink"
```

---

## Task 8: Wire config at startup and live reload

**Files:**
- Modify: `src/main.zig`
- Modify: `src/render/metal.zig` (add a `setClearColor` method so the GPU clear color tracks the theme)

Load the config at startup, thread every value in, poll the file on the tick, and apply theme + cursor changes live. Then close out the docs.

**Why `metal.zig` is touched:** `Renderer.init` (`src/render/metal.zig:75`) hardcodes the GPU clear color to `#0b0d0e` via `color.hexToClearColor`. That clear color sits behind the full-screen raster texture, so it is normally invisible — but on a resize flash, or after a live switch to `mineral-light`, a stale graphite clear would show through. The renderer needs to track the active theme's background.

- [ ] **Step 1: Add config storage to `App` and a path buffer**

Add fields to `App` (after `blink_ticks`):

```zig
    config: cfg_mod.Loaded,
    watcher: cfg_mod.Watcher,
```

Add a module-level buffer for the resolved config path (near the PTY buffers):

```zig
var config_path_buf: [std.fs.max_path_bytes]u8 = undefined;
```

- [ ] **Step 1b: Add `setClearColor` to `Renderer` in `src/render/metal.zig`**

After the `resize` method (around line 98), add:

```zig
    /// Update the GPU clear color. It sits behind the full-screen texture, so
    /// this only matters on resize flashes — but it must track the theme.
    pub fn setClearColor(self: *Renderer, rgb: [3]u8) void {
        self.clear = .{
            .red = @as(f64, @floatFromInt(rgb[0])) / 255.0,
            .green = @as(f64, @floatFromInt(rgb[1])) / 255.0,
            .blue = @as(f64, @floatFromInt(rgb[2])) / 255.0,
            .alpha = 1.0,
        };
    }
```

(`Renderer.init` may keep its `#0b0d0e` literal as the pre-theme default — `main` overrides it immediately in Step 3b.)

- [ ] **Step 2: Load config at the top of `main`**

At the start of `main`, after `const alloc = ...`, load the config and resolve the theme:

```zig
    const config_path: ?[]const u8 = cfg_mod.resolvePath(&config_path_buf);
    var loaded: cfg_mod.Loaded = if (config_path) |p| cfg_mod.load(alloc, p) else cfg_mod.defaults(alloc);
    const cfg = loaded.config;
    const active_theme = theme_mod.resolve(cfg.theme, cfg.theme_overrides);
```

- [ ] **Step 3: Use config values for font, window, terminal**

- Replace the `font_point_size`/`init_win_w_pt`/`init_win_h_pt` *uses* (not the consts — leave the consts or delete them; if deleted, `rg` for every use). Use `cfg.font.size`, `cfg.window.width`, `cfg.window.height`.
- Build the font-name list with the configured family first. The family is a `[]const u8`; `Font.init` needs `[:0]const u8`. Duplicate it null-terminated into the config arena:

```zig
    const fam_z = loaded.arena.allocator().dupeZ(u8, cfg.font.family) catch "IBMPlexMono";
    const font_names = [_][:0]const u8{ fam_z, "SFMono-Regular", "Menlo" };
    const font = Font.initFirstAvailable(&font_names, cfg.font.size * scale) catch |e| fail("font", e);
```

- Window `rect` uses `cfg.window.width` / `cfg.window.height` for its `size`.
- `Terminal.init(alloc, cols, rows, cfg.scrollback)`.
- The `g = .{ ... }` initializer: `.theme = active_theme`, `.cursor_cfg = cfg.cursor`, `.config = loaded`, `.watcher = cfg_mod.Watcher.init(config_path orelse "")`.

- [ ] **Step 3b: Sync the GPU clear color to the theme at startup**

Immediately after the `g = .{ ... }` initializer (so `g.renderer` exists), add:

```zig
    g.renderer.setClearColor(active_theme.background);
```

This makes the clear color correct even when the configured startup theme is `mineral-light`.

- [ ] **Step 4: Poll and apply in `onTick`**

At the very top of `onTick`, before the PTY drain, add live reload:

```zig
    if (g.watcher.path.len > 0) {
        if (g.watcher.poll(g.alloc)) |new_loaded| {
            applyConfig(new_loaded);
        }
    }
```

Add `applyConfig` near the other helpers:

```zig
/// Apply a freshly-loaded config. Theme and cursor changes take effect now;
/// font/scrollback/window changes are startup-only and ignored here.
fn applyConfig(new_loaded: cfg_mod.Loaded) void {
    var nl = new_loaded;
    const c = nl.config;
    g.theme = theme_mod.resolve(c.theme, c.theme_overrides);
    g.renderer.setClearColor(g.theme.background); // keep the GPU clear in sync
    g.cursor_cfg = c.cursor;
    g.dirty = true;
    g.config.deinit(); // free the previous config's arena
    g.config = nl;      // the new arena owns the strings resolve() just read
}
```

Note ordering: `resolve` copies colors out of the arena strings into `[3]u8`, and `g.theme`/`g.cursor_cfg` are plain values — so it is safe to free the old arena after `resolve`. The new `Loaded` is stored to keep its arena alive for the *next* reload's diff and so its lifetime is owned.

- [ ] **Step 5: Run the build and tests**

Run: `zig build test`
Expected: PASS — all tests green (no new tests here; this is integration wiring).

- [ ] **Step 6: Verify live reload by hand**

Run: `zig build run`. Then in another terminal:

```bash
mkdir -p ~/.config/anvil
printf '.{ .theme = "mineral-light" }\n' > ~/.config/anvil/config.zon
```

Expected: within ~1 s the running terminal repaints with the light theme — no restart. Then:

```bash
printf '.{ .theme = "mineral-dark", .cursor = .{ .style = .bar } }\n' > ~/.config/anvil/config.zon
```

Expected: it flips back to dark and the cursor becomes a left bar. Delete the file → reverts to defaults. (If you do not want to touch your real `~/.config`, back up any existing file first.)

- [ ] **Step 7: Commit**

```bash
git add src/main.zig
git commit -m "feat(config): load config at startup and live-reload theme/cursor"
```

- [ ] **Step 8: Close out docs**

- `todo.txt`: under M2, check off the config/theme item; note it shipped with live reload.
- `wiki/`: add a short concept page `wiki/concepts/config-system.md` (frontmatter per `wiki/index.md`) describing the ZON config, the arena-ownership rule, and the live/startup-only split; link it from `wiki/index.md` Key Pages.
- Append a `wiki/log.md` entry for the config/theme sub-project.

- [ ] **Step 9: Final verification and commit the docs**

Run: `zig build test`
Expected: all tests pass.

```bash
git add todo.txt wiki/
git commit -m "docs: record M2 config/theme sub-project"
```

---

## Done criteria

- `zig build test` passes; the 102 M1 tests plus the new config/theme tests are all green.
- `zig build run` opens the app; editing `~/.config/anvil/config.zon` live-reloads theme and cursor; font/scrollback/window apply on next launch.
- `mineral-dark` and `mineral-light` both selectable; `theme_overrides` change individual colors.
- This completes M2 sub-project 1 of 4. Next: multi-tab, then in-terminal search, then shell integration — each gets its own brainstorm → spec → plan cycle.
