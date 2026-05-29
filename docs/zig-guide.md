# Anvil Zig Guide

Learning index for the ground-up Zig rewrite. Terminal-first, Ghostty-style.
Zig 0.16.0. Old Rust port is archived on branch `rust-port` + tag
`rust-port-archive`.

## Build & run

```sh
zig build          # compile -> zig-out/bin/anvil
zig build run      # compile + run (live shell in your terminal)
zig build test     # run all test blocks
```

## Layout

| File | Job |
|------|-----|
| [build.zig](../build.zig) | Build graph: the `anvil` core module, the exe, `run` + `test` steps. |
| [build.zig.zon](../build.zig.zon) | Package manifest (name, version, deps). |
| [src/main.zig](../src/main.zig) | Entry point. Raw-mode passthrough loop: keyboard → shell, shell → screen. |
| [src/root.zig](../src/root.zig) | The reusable terminal-core module (`@import("anvil")`). Re-exports `Pty`, `Terminal`, `Grid`, `Cell`, `Parser`. |
| [src/pty.zig](../src/pty.zig) | `Pty`: spawn `$SHELL` on a pty via `forkpty`, resize, teardown. The spine. |
| [src/vt/cell.zig](../src/vt/cell.zig) | `Cell` (codepoint + fg/bg `Color` + `Attrs`). The atom of the grid. |
| [src/vt/grid.zig](../src/vt/grid.zig) | `Grid`: flat `[]Cell` of rows×cols. `at`/`row`/`clear`/`scrollUp`. |
| [src/vt/terminal.zig](../src/vt/terminal.zig) | `Terminal`: cursor + pen state. print/cursor-move/erase/SGR. The screen model. |
| [src/vt/parser.zig](../src/vt/parser.zig) | `Parser`: byte-stream state machine (ground/escape/csi). UTF-8 + CSI → `Terminal` calls. |
| [src/platform/shim.m](../src/platform/shim.m) | Obj-C ceremony only: NSWindow, CAMetalLayer, render pipeline, glyph atlas, keyboard. Calls into Zig each frame. |
| [src/platform/window.zig](../src/platform/window.zig) | Zig wrapper over the shim's C entry points. |
| [src/platform/shaders.metal](../src/platform/shaders.metal) | Metal vertex/fragment shaders. Instanced cell quads; samples the glyph atlas. |
| [src/app.zig](../src/app.zig) | GUI app state: owns `Terminal`/`Parser`/`Pty`/`Renderer`. Exports the C frame hooks (`anvil_frame`, `anvil_poll`, `anvil_input`, …). |
| [src/render/palette.zig](../src/render/palette.zig) | xterm color resolution: ANSI-16, 256-cube, grayscale, truecolor, defaults. |
| [src/render/atlas.zig](../src/render/atlas.zig) | Glyph atlas layout: codepoint → normalized UV. |
| [src/render/renderer.zig](../src/render/renderer.zig) | Layout + per-cell instance generation from the grid; block cursor. |
| [src/render/instance.zig](../src/render/instance.zig) | `CellInstance` / `FrameData` — the Zig↔Metal data contract. |

Core logic (root.zig) is kept separate from the front-end (main.zig) on
purpose — M2's window/render will consume the core, same as Ghostty's libghostty.

## Zig concepts so far

- **Modules**: `b.addModule` exposes the core; the exe imports it as `anvil`.
  A module = source files + compile options (target, `link_libc`).
- **`@cImport` / `@cInclude`**: pull C headers straight in, no bindings layer.
  We call `forkpty`, `termios`, `ioctl`, `read`/`write` directly. This is the
  reason Zig fits a terminal: the OS is one include away.
- **`link_libc = true`**: required on a module for those C symbols to link.
- **Error unions (`!T`)** + `try` / `catch`: `spawn` returns `!Pty`; the loop
  uses `catch break` to bail on I/O errors.
- **`defer`**: cleanup that runs on scope exit — restoring termios, `pty.deinit()`.
- **`@intCast` / `@ptrCast`**: explicit conversions between C ints/pointers and
  Zig types. Zig never converts silently.
- **`test "..." { ... }`** blocks: live next to the code, run by `zig build test`.

## Gotcha: std.posix is mid-migration

Zig 0.16 is moving to a new `std.Io` model. `std.posix.read` exists but
`write` / `close` / `getenv` were removed. So raw I/O goes through libc
(`c.read`, `c.write`, `c.close`, `c.getenv`, `c.kill`) for now. Revert to
`std.posix` once Zig finishes the migration. `poll` still comes from `std.posix`.

## Roadmap

- **M0 — PTY passthrough** ✅ A real shell runs on the pty; bytes shuttle both ways.
- **M1 — VT core** ✅ parser (escape sequences) + cell grid. Pure, unit-tested,
  no rendering. 12 tests green.
- **M2 — Window + render** AppKit `NSWindow` + `CAMetalLayer`, draw the grid,
  wire input to the pty. First on-screen native terminal.
  - **M2.1** ✅ window on screen — shim creates NSWindow + Metal layer, clears each frame.
  - **M2.2** ✅ Metal pipeline — colored cell-background quads from the grid.
  - **M2.3** ✅ CoreText glyph atlas — rasterize + draw text.
  - **M2.4** ✅ wire PTY → `Parser` → `Terminal` → render, keyboard → pty, block cursor.

  M2 complete: a real interactive terminal. The 60 Hz tick drains the pty
  (non-blocking) → parser → grid, then renders; the view forwards keystrokes
  to the pty. Single-threaded, no locks.

## Frame loop (how a keypress becomes pixels)

1. `keyDown:` in the shim → `anvil_input(bytes)` → `pty.write` (Zig).
2. Shell processes input, writes output to the pty.
3. Next tick: shim calls `anvil_poll` → Zig drains the pty non-blocking,
   feeds `Parser` → mutates `Terminal` grid.
4. Shim calls `anvil_frame` → Zig builds `[]CellInstance` (+ cursor) from the grid.
5. Shim uploads instances, binds the atlas, issues one instanced draw.

## Obj-C interop: the shim rule

macOS surfaces (AppKit/Metal) are reached through a thin Obj-C shim
([src/platform/shim.m](../src/platform/shim.m)) that exposes plain C functions;
Zig calls them as C and links the frameworks (`Cocoa`, `QuartzCore`, `Metal`).

**Standing rule: logic in Zig, ceremony in shim.** The `.m` only allocs objects,
sets properties, and forwards callbacks. Every *decision* — what to draw, input
mapping, layout — lives in Zig where it's testable. Keeps the shim from becoming
a second god-file. No `objc_msgSend`-by-hand, no external objc wrapper.
