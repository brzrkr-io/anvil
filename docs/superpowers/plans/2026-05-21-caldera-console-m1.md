# Caldera Console M1 — Terminal Core Implementation Plan

**Goal:** A genuinely usable single-pane GPU terminal: spawns a shell, parses
its VT/ANSI output, renders text via Metal, takes keyboard input, and keeps
**very deep scrollback**. Built to run modern AI CLIs (Claude Code, Codex,
aider) flawlessly.

**Architecture:** A clean split — a pure-Zig terminal model (parser → grid →
scrollback, no platform deps) driven by a PTY layer, displayed by a Metal text
renderer. The model is fully unit-testable in isolation; only the renderer and
input touch AppKit/Metal.

**Tech stack:** Zig 0.16, `zig-objc`, AppKit, Metal (runtime-compiled MSL —
no offline shader toolchain), CoreText (font + glyph rasterization), libc PTY.

---

## Design decisions

- **Parser:** Paul Williams' ANSI VT500 DFA (a well-known public-domain state
  diagram — implemented fresh, not copied). UTF-8 decoded in the ground state.
- **Deep scrollback (user priority):** a ring buffer of *trimmed* rows — each
  scrollback row allocates only its used cells (trailing blanks dropped), so
  capacity can be huge. Default capacity **100,000 rows** (`scrollback.default_capacity`,
  a single const — trivially raised). Oldest rows evicted when full.
- **AI-readiness:** (1) the parser must correctly handle what modern AI CLIs
  emit — alt-screen (`?1049h/l`), SGR colors incl. 256/truecolor, cursor
  positioning, line/char insert-delete, erase. (2) **OSC 133 semantic prompt
  marks** (`A` prompt-start, `B` command-start, `C` output-start, `D`
  command-done) recorded on the model, so command and output regions are
  machine-identifiable — the hook for later AI features.
- **Alt screen:** the Terminal holds a primary grid + an alternate grid.
- **Renderer:** instanced quads — one quad, one instance per visible cell
  (bg color) and per glyph. Glyph atlas rasterized on demand via CoreText +
  CoreGraphics. MSL compiled at runtime with `newLibraryWithSource:`.
- **Loop:** a display-linked tick drains the PTY → `terminal.feed` → redraw.

## Module map (new files under `src/`)

```
terminal/
  cell.zig        Cell, Color (default | palette u8 | rgb), Attrs (packed flags)
  parser.zig      VT/ANSI DFA; emits to an `anytype` handler
  grid.zig        active screen: cells, cursor, scroll region, SGR pen, erase/scroll
  scrollback.zig  ring buffer of trimmed rows; default 100k capacity
  terminal.zig    Terminal: parser handler; applies events; alt screen; viewport;
                  OSC 133 semantic zones
pty/
  pty.zig         openpty + fork/exec login shell; read/write; TIOCSWINSZ
render/
  font.zig        CoreText monospace font; cell metrics; codepoint -> glyph
  atlas.zig       glyph atlas Metal texture; CoreGraphics rasterization
  metal.zig       (extend) text pipeline, runtime MSL, per-cell instance draw
app/
  window.zig      (extend) custom NSView for keyDown input; display-link tick
main.zig          (extend) wire terminal + pty + renderer + input
```

## Execution phases

- **Phase 1 — terminal core** (`cell`/`parser`/`grid`/`scrollback`/`terminal`):
  pure Zig, TDD, no AppKit/Metal. Verified with standalone `zig test`.
- **Phase 2 — PTY** (`pty.zig`): spawn the shell, read/write, resize. Verified
  by spawning a command and asserting its output.
- **Phase 3 — rendering** (`font`/`atlas`/`metal`): glyph atlas + Metal text
  pipeline. Verified by screenshot.
- **Phase 4 — integration** (`main`/`window`/`build.zig`): wire it all, key
  input, display-link loop, resize. Verified by running a shell interactively.

Phases 1 and 2 are independent and run in parallel. Phase 3 proceeds alongside.
Phase 4 integrates. Commit after every working increment.

## Verification

- Core: `zig test` on each module — parser/grid/scrollback/terminal behaviors.
- PTY: spawn `printf` / `echo`, assert bytes read back.
- End-to-end: `zig build run` opens the terminal; a shell prompt renders;
  typing works; `ls`, `vim`/alt-screen, and a long output scroll into
  scrollback; scrolling up shows history. Screenshot-verified.
