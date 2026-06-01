---
status: active
type: concept
created: 2026-05-21
updated: 2026-05-29
sources:
  - ../../context/2026-05-21-m1-complete.md
confidence: high
---

# Console Architecture

Orientation map for the Anvil codebase. Verified against the Zig rewrite
(`zig` branch). See [[decisions/0002-tech-stack]] for the original stack rationale
and [[decisions/0005-render-host]] for the native-Metal-only decision.

## Data Flow

```
PTY output (child process)
  └─ app.zig poll loop — reads PTY master fd, calls terminal.feed()
       └─ vt/terminal.zig Terminal.feed()
            └─ vt/parser.zig Parser — Williams DFA drives terminal as handler
                 └─ vt/grid.zig Grid — active screen matrix
                 └─ vt/scrollback.zig Scrollback — ring buffer
       └─ app.zig render tick — calls renderer.render()
            └─ render/atlas.zig GlyphAtlas — lazy CoreText glyph atlas → Metal texture
            └─ render/renderer.zig Renderer.render() — instance draw, Metal command buffer
```

Input path:

```
NSEvent (AppKit, via platform/shim.m)
  └─ app.zig anvil_key_event() — encodes keystroke → PTY write
  └─ app.zig anvil_scroll() → Terminal.scroll_viewport()
  └─ app.zig anvil_mouse_event() → selection / hit-test
```

## Module Map

```
src/main.zig                   — binary entry point
src/app.zig                    — App state; C-ABI exports called by Obj-C shim
                                 (anvil_poll, anvil_resize, anvil_render, anvil_key_event,
                                  anvil_mouse_event, anvil_scroll, anvil_set_theme)
src/root.zig                   — anvil module root; re-exports; test aggregator
src/platform/
  shim.m                       — Obj-C: NSWindow, CAMetalLayer, run loop, CoreText font
  window.zig                   — window/layer Zig bindings
  shaders.metal                — MSL; compiled at runtime via newLibraryWithSource:
src/render/
  renderer.zig                 — GPU pipeline; Metal command buffer; instance draws
  atlas.zig                    — lazy glyph atlas; CoreText → MTLTexture
  instance.zig                 — per-cell draw instance layout
  palette.zig                  — cell→color resolution (ANSI-16, 256-color, truecolor)
  theme.zig                    — Mineral palette definitions (dark/light/system)
src/vt/
  parser.zig                   — Williams DFA VT/ANSI parser; byte-oriented, stateful
  terminal.zig                 — public VT model; owns grid, alt grid, scrollback, parser
  grid.zig                     — active screen matrix; cursor, scroll region, SGR pen
  scrollback.zig               — ring of trimmed rows
  cell.zig                     — Cell (codepoint + color + attrs); pure data
  width.zig                    — wcwidth Unicode cell-width
src/workspace/
  pane_tree.zig                — PaneTree layout engine; split/close/focus; pure geometry
src/session.zig                — Session: one terminal + parser + PTY
src/session_manager.zig        — multi-session/tab router
src/pty.zig                    — forkpty + nonblocking master I/O (libc @cImport)
src/config.zig                 — config load + live reload
src/palette.zig                — command-palette model
src/search.zig                 — scrollback search
src/caldera.zig                — Caldera IPC bridge
src/keys.zig                   — keyboard → terminal byte encoding
```

## Runtime Model

- The Obj-C shim (`platform/shim.m`) owns the NSWindow and CAMetalLayer. It
  calls C-ABI exports on `app.zig` for every interesting event.
- `anvil_poll` is called on the run-loop timer (60 Hz). It drains PTY output
  into `terminal.feed()` then calls `anvil_render` if the frame is dirty.
- `anvil_render` drives `Renderer.render()`, which issues Metal draw calls
  for each visible cell instance plus chrome quads.
- Resize: `anvil_resize` recomputes cols/rows from device-pixel bounds and
  calls `terminal.resize()`, `pty.resize()`, `renderer.resize()`.

## Key Design Points

- The Metal renderer uses a **glyph atlas** (`render/atlas.zig`): glyphs are
  rasterized once via CoreText and cached as atlas tiles in a `MTLTexture`.
  Per-frame, cell instances reference atlas tiles; no per-frame CPU raster.
- Color resolution: `Cell.fg`/`bg` are resolved in `render/palette.zig`
  through `Theme.palette256()`.
- All app chrome (tabs, dividers, search bar, palette) is rendered as
  native Metal instance/solid-rect draws. No WKWebView. See [[decisions/0005-render-host]].
- Runtime shader compilation: `shaders.metal` is compiled by the Metal driver
  at app startup via `newLibraryWithSource:`. No offline `metal` toolchain needed.
