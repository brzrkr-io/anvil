---
status: active
type: concept
created: 2026-05-21
updated: 2026-05-29
sources:
  - ../../src/config.zig
  - ../../src/render/theme.zig
  - ../../src/app.zig
confidence: high
---

# Config System

Anvil reads a TOML config file from
`~/.config/anvil/config.toml` at startup and polls it every tick for
live changes.

## Config File

The file is TOML. All fields are optional; missing fields keep their defaults.

```toml
scrollback = 100000        # scrollback ring capacity (rows)

[font]
family = "IBM Plex Mono"   # font family name
size = 14.0                # point size

[cursor]
style = "block"            # "block" | "bar" | "underline"
blink = true

[window]
width = 1024.0             # initial window width (points)
height = 640.0             # initial window height (points)

theme = "mineral-dark"     # "mineral-dark" | "mineral-light"

[theme_overrides]
background = "#0b0d0e"     # optional #rrggbb overrides
accent = "#2f7f86"
# ansi.green = "#3f8a5b"   etc.
```

## Ownership Model

Every parse result (`Loaded`) owns a heap `Config`; `Loaded::drop` frees it.
The app stores the active config and replaces it atomically in `apply_config`.

## Live / Startup-Only Split

| Setting | When applied |
|---------|-------------|
| `theme`, `theme_overrides`, `cursor` | Live — applied within one tick (~16 ms) |
| `font`, `scrollback`, `window` | Startup only — require relaunch |

The live-reload path (`apply_config` in `src/app.zig`) calls `theme.resolve`
before dropping the old config. `resolve` copies colors out of the config
strings into plain rgb values, so `app.theme` and `app.cursor_cfg` hold no
config references after the call.

## Watcher

`config::Watcher` polls the file's `mtime` via `stat` once per 60 Hz tick. A
missing file or a parse error falls back to defaults; a parse error advances the
recorded mtime so the error is logged once, not every tick. No background thread
or FSEvents involvement.

## Built-in Themes

| Name | Background | Foreground | Accent |
|------|-----------|-----------|--------|
| `mineral-dark` (default) | graphite `#0b0d0e` | `#e8eaee` | mineral `#2f7f86` |
| `mineral-light` | bone `#eef1f2` | charcoal `#161a1c` | mineral `#2f7f86` |

Both themes use the Mineral brand palette for the ANSI 16-color slots. See
[[decisions/0003-m1-brand-palette]] for the mapping rationale.

## Modules

- `src/config.zig` — `Config`, `Overrides`, `CursorStyle`, `defaults`, `load`, `Watcher`
- `src/render/theme.zig` — `Theme`, `mineral_dark`, `mineral_light`, `by_name`, `resolve`, `palette256`
- `src/render/palette.zig` — cell→color resolution
