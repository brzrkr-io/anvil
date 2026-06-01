---
status: active
type: concept
created: 2026-05-21
updated: 2026-05-29
sources:
  - ../../src/app.zig
  - ../../src/session_manager.zig
  - ../../src/render/renderer.zig
  - ../../src/config.zig
confidence: high
---

# Tab System

Anvil supports multiple terminal tabs in a single window. Each tab
owns its own shell, terminal model, and PTY reader thread.

## Tab

`Tab` (managed in `src/session_manager.zig`) is heap-allocated so its address is stable for the
reader loop. It owns:

- `terminal: Terminal` — the VT model (grid, scrollback, title/cwd state).
- `pty: Pty` — the master end of the pseudoterminal connected to the shell.
- A per-tab read buffer drained each poll tick.
- A `Session` (terminal + PTY) managing the child shell process.

### Lifecycle

```
SessionManager.addTab(cols, rows, scrollback, cwd)
  -> allocates Session, inits Terminal, spawns Pty
SessionManager.closeTab(id)
  -> closes pty fd, deinits terminal
```

### Per-Tab Poll Drain

`app.zig anvil_poll` drains each session's PTY output each tick, calling
`terminal.feed(bytes)`. Background tabs are drained each tick (keeping their
models current) but only the active tab triggers a render.

## TabManager

`TabManager` owns the `Vec<Box<Tab>>` list and the active index.

Key operations:

| Method | Behavior |
|--------|---------|
| `new_tab(cols, rows, scrollback, cwd)` | Creates a tab, starts its reader, appends, makes active |
| `close_active` / `close_at(index)` | Deinits the tab, adjusts `active` via `next_active_after_close` |
| `next` / `prev` | Wraps the active index |
| `switch_to(index)` | Clamps the active index |

A hard cap of 32 tabs (`MAX_TABS`) bounds the per-tab thread and buffer cost.

## Bar-Visibility Rule

The tab bar is shown only when 2 or more tabs are open (the "low-profile" rule):

With a single tab the bar is hidden and the full window height is available to
the terminal. Opening a second tab triggers a resize so every shell receives the
correct `SIGWINCH` for the new grid size (one row shorter). Closing back to one
tab similarly reflows. The bar always occupies the top chrome row.

## Keybinding Chords

Tab shortcuts are configured in `config.toml` under the `keybindings` key:

```toml
[keybindings]
new_tab   = "cmd+t"
close_tab = "cmd+w"
next_tab  = "cmd+shift+]"
prev_tab  = "cmd+shift+["
tab_1     = "cmd+1"
# ...
tab_9     = "cmd+9"
```

Chord parsing in `src/config.zig` splits the string on `+`, recognises
`cmd`/`shift`/`ctrl`/`opt` modifier tokens (case-insensitive), and lowercases
the key character. Parsed chords are stored in `App` state in `src/app.zig`
and updated at startup and on every live config reload — keybindings take effect
without restarting the app.

## Modules

- `src/app.zig` — tab/session routing, keybinding dispatch, resize_all_tabs
- `src/session_manager.zig` — `SessionManager`: session list, active index, add/close
- `src/session.zig` — `Session`: one terminal + parser + PTY
- `src/render/renderer.zig` — tab bar draw (equal-width segments, active accent)
- `src/config.zig` — chord parsing, `Keybindings`
