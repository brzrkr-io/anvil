# Config + Theme System — Design Spec

> Status: draft for review. Sub-project 1 of 4 in milestone **M2**.
> Created: 2026-05-21. Owner-approved direction; pending spec review.

## Goal

Give Anvil a user-editable config file and a theme system, with
**live reload** of theming. The user edits `~/.config/anvil/config.zon`,
saves, and the terminal redraws instantly with the new colors — no restart.

This is the foundational sub-project of M2: it establishes the config-file
pattern the later sub-projects (multi-tab, search) build keybindings on.

## Context

M1 shipped a single-pane terminal with hardcoded values: font name/size in
`src/main.zig`, window size in `src/main.zig`, scrollback capacity
(`default_capacity = 100_000`) in `src/terminal/scrollback.zig`, and the color
palette in `src/render/color.zig` (already brand-aligned). This sub-project
makes those values configurable and theme-driven.

## Decisions (settled with the owner)

1. **Scope** — configurable: scrollback size, font (family + size), theme,
   cursor (style + blink), window default size.
2. **Format** — ZON (Zig Object Notation), parsed natively by `std.zon`. Zero
   new dependencies. Located at `~/.config/anvil/config.zon`.
3. **Theming depth** — two built-in themes (`mineral-dark`, `mineral-light`)
   plus per-color overrides: `config.zon` selects a base theme and may override
   any individual color of it.
4. **Live reload** — yes, via polling the config file's modification time on
   the existing 60 Hz render tick. No file-watcher thread, no FSEvents binding.
5. **Live vs startup-only** — settings that are pure render-time reads reload
   live; settings that require rebuilding renderer resources apply at startup.
   - **Live:** `theme`, `theme_overrides`, `cursor.style`, `cursor.blink`.
   - **Startup-only:** `font`, `scrollback`, `window`. (These need font
     re-rasterization / ring re-allocation / a window resize — deliberately
     deferred. Font live-reload is a clean future extension once this lands.)

## Config file

Path: `~/.config/anvil/config.zon`. Resolved from `$HOME`. The
directory is **not** created by the app — a missing file is the normal first
run and silently yields defaults.

Full example with every field at its default:

```zon
.{
    // Rows of scrollback history retained. Startup-only.
    .scrollback = 100000,

    // Terminal font. Startup-only. `family` is a preferred face; if it does
    // not load, the brand fallback chain (SFMono-Regular, Menlo) is used.
    .font = .{
        .family = "IBM Plex Mono",
        .size = 14.0,
    },

    // Cursor appearance. Live-reloadable.
    .cursor = .{
        .style = .block, // .block | .bar | .underline
        .blink = true,
    },

    // Window size to open at, in points. Startup-only.
    .window = .{
        .width = 1024.0,
        .height = 640.0,
    },

    // Base theme. Live-reloadable. "mineral-dark" | "mineral-light".
    .theme = "mineral-dark",

    // Optional per-color overrides applied on top of the base theme.
    // Any field omitted keeps the base theme's value. Live-reloadable.
    .theme_overrides = .{
        .background = null,
        .foreground = null,
        .accent = null, // also the cursor color
        .ansi = .{
            .black = null,        .red = null,
            .green = null,        .yellow = null,
            .blue = null,         .magenta = null,
            .cyan = null,         .white = null,
            .bright_black = null, .bright_red = null,
            .bright_green = null, .bright_yellow = null,
            .bright_blue = null,  .bright_magenta = null,
            .bright_cyan = null,  .bright_white = null,
        },
    },
}
```

Color values are `#rrggbb` (or bare `rrggbb`) hex strings.

## Architecture

Two new modules under `src/config/`:

- **`src/config/config.zig`** — the `Config` struct, ZON loading, and the
  live-reload poller. One responsibility: turn the config file into a
  validated `Config` value, and report when the file has changed.
- **`src/config/theme.zig`** — the `Theme` struct, the two built-in themes, and
  `resolve()` which produces an active `Theme` from a base name + overrides.

Existing modules change to consume these instead of hardcoded values:

- `src/render/color.zig` — the palette becomes data inside a `Theme`. The
  brand-aligned values become `mineral_dark`'s data. `palette256` takes a
  `Theme`.
- `src/render/font.zig` — unchanged API; `main.zig` passes the configured
  family/size into `initFirstAvailable`.
- `src/terminal/terminal.zig` — `Terminal.init` gains a `scrollback_capacity`
  parameter (replacing the hardcoded `default_capacity`).
- `src/main.zig` — loads config at startup, threads values in, owns the active
  `Theme`, polls for changes on the tick, applies live changes, and implements
  cursor style + blink.

### `Config`

```zig
pub const CursorStyle = enum { block, bar, underline };

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
    pub const Overrides = struct {
        background: ?[]const u8 = null,
        foreground: ?[]const u8 = null,
        accent: ?[]const u8 = null,
        ansi: Ansi = .{},
        pub const Ansi = struct {
            black: ?[]const u8 = null,
            red: ?[]const u8 = null,
            // ... 16 optional named slots, see config example
        };
    };
};
```

All fields have Zig defaults, so a partial `config.zon` (only some fields
present) parses fine — `std.zon.parse` fills the rest from the struct defaults.

Strings are owned: the parse uses a dedicated arena allocator. A loaded config
carries its arena; freeing the config frees the arena.

### `Theme`

```zig
pub const Theme = struct {
    background: [3]u8,
    foreground: [3]u8,
    accent: [3]u8,        // cursor color
    ansi: [16][3]u8,
};

pub const mineral_dark: Theme = .{ ... };  // current brand-aligned palette
pub const mineral_light: Theme = .{ ... }; // new, contrast-checked for light bg

pub fn byName(name: []const u8) Theme;     // unknown -> mineral_dark
pub fn resolve(base: []const u8, ov: Config.Overrides) Theme;
```

`resolve()` starts from `byName(base)` and applies each present override. An
override whose hex string is invalid is skipped (the base value is kept) and
logged — a typo while editing must not crash the app.

`mineral_dark` is the existing palette from `src/render/color.zig`
(post-brand-alignment). `mineral_light` reuses the brand accent/status hues on
a light material background (`bone #eef1f2` background, `ink/charcoal` text);
its 16 ANSI values are darker normal / lighter bright variants chosen for
contrast on a light background — exact values pinned in the implementation plan
and checked against `BRAND.md`.

## Data flow

**Startup** (`main.zig`):
1. `Config.load(alloc)` → reads `config.zon`, parses, validates, clamps.
2. `Theme.resolve(config.theme, config.theme_overrides)` → active `Theme`.
3. Font: `initFirstAvailable([config.font.family, "SFMono-Regular", "Menlo"], size)`.
4. `Terminal.init(alloc, cols, rows, config.scrollback)`.
5. Window opened at `config.window` size.
6. Renderer/`App` store the active `Theme` and `CursorCfg`.

**Per tick** (`onTick`, 60 Hz, throttled to ~4×/sec for the poll):
1. `config_mgr.poll()` → `?Config`. Non-null when the file's mtime changed and
   the new content parsed successfully.
2. On a new config: re-`resolve()` the theme, swap `g.theme`, update
   `g.cursor_cfg`, set `g.dirty = true`. Free the previous config arena.
   Startup-only fields that differ are not applied (optionally logged once).
3. Cursor blink: advance a blink phase; on a phase flip set `g.dirty = true`.

The terminal model is untouched by themes/cursor — color and cursor styling are
render-time concerns in `main.zig`/`render/`.

## Cursor style + blink

Cursor blink is currently M1 tech debt (static block cursor). Exposing
`cursor.blink` requires implementing it:

- **Blink** — a blink phase toggled every ~530 ms off the 60 Hz tick. When
  `cursor.blink` is false the cursor is always shown. A phase flip marks the
  frame dirty.
- **Style** — `block` is the current full-cell highlight. `bar` draws a thin
  vertical bar at the cell's left edge; `underline` draws a thin bar at the
  cell's bottom. This needs one new rasterizer primitive (a partial-cell fill);
  `block` keeps using `raster.cellBg`.

## Error handling

| Situation | Behavior |
|---|---|
| Config file missing | Silent fallback to defaults (normal first run). |
| Malformed ZON / unknown field / wrong type | Keep current config (or defaults at startup); log one line to stderr with the parse error. On live reload, record the mtime so the error is not re-logged every tick. |
| Unknown `theme` name | Fall back to `mineral-dark`; log. |
| Invalid hex in an override | Skip that one override, keep the base color; log. |
| Out-of-range value (`scrollback` 0, `font.size` ≤ 0, tiny `window`) | Clamp to a sane minimum; log. |
| Configured `font.family` does not load | Fall through the brand fallback chain (SFMono-Regular, Menlo). |

The app has no error UI yet; stderr is the v1 channel.

## Testing

`zig build test` is the gate; all 102 existing tests stay green. New unit
tests, per module:

**`config.zig`**
- Valid full ZON → every field parsed correctly.
- Partial ZON (only some fields) → present fields applied, rest defaulted.
- Missing file → all defaults.
- Malformed ZON → defaults returned, no crash.
- Clamping: `scrollback = 0`, `font.size = 0`, tiny window → clamped values.
- Poller: unchanged mtime → `poll()` returns null; changed mtime + valid
  content → returns the new config; changed mtime + bad content → returns null,
  does not re-report on the next unchanged poll.

**`theme.zig`**
- `byName` for `"mineral-dark"`, `"mineral-light"`, and an unknown name
  (→ dark).
- `resolve` with no overrides → equals `byName(base)`.
- `resolve` with a valid override → that field changed, others unchanged.
- `resolve` with an invalid-hex override → base value kept.

**Cursor (in `main.zig`'s testable helpers)**
- Blink phase toggles state across the blink interval.
- `cursor.blink = false` → cursor always shown.

## Out of scope for v1 (deliberate)

- Live reload of `font`, `scrollback`, `window` (startup-only here).
- Writing/persisting config from the app (the user hand-edits it; window size
  is an open-at default, not persisted).
- Keybindings (added when multi-tab needs them — the format extends cleanly).
- Full inline custom theme definitions (overrides cover the v1 need).
- An FSEvents/kqueue file watcher (mtime polling is sufficient and simpler).
- Multiple config-file search paths or a `$XDG_CONFIG_HOME` override.

## File summary

| File | Change |
|---|---|
| `src/config/config.zig` | Create — `Config`, ZON load, mtime poller. |
| `src/config/theme.zig` | Create — `Theme`, built-ins, `resolve`. |
| `src/render/color.zig` | Modify — palette becomes `Theme` data; `palette256` takes a `Theme`. |
| `src/terminal/terminal.zig` | Modify — `Terminal.init` takes `scrollback_capacity`. |
| `src/render/raster.zig` | Modify — add a partial-cell fill primitive for bar/underline cursors. |
| `src/main.zig` | Modify — load config, thread values, own active `Theme`, poll + apply live changes, cursor style/blink. |
| `src/render/font.zig` | Unchanged API; called with the configured family/size. |
