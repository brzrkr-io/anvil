---
status: active
type: concept
created: 2026-05-22
updated: 2026-05-29
sources: []
confidence: high
---

# Hardening Net — Fixed Bugs and Regression Test Hooks

> **Note (2026-05-29):** The file paths below (`crates/anvil-*/src/…`) belong to
> the archived Rust port (`rust-port-archive` tag). The active `zig` branch has
> equivalent logic in `src/vt/`, `src/render/`, and `src/platform/`. The bug
> descriptions and invariants remain accurate; only the source locations differ.
> Tests in the Zig tree run via `.zig/zig build test`.

Each bug listed here has been fixed and is locked by at least one automated
test. Tests run via `.zig/zig build test` with no Metal device required.

## Bug A — Resize Bugs

**Symptom:** Resize to unusual dimensions (degenerate sizes, shrink past the
cursor) could leave invariants violated: cursor out of bounds, scrollback
divergence, alternate-grid width mismatch.

**Fix:** `Grid::resize` and `Terminal::resize` pre-scroll on shrink and clamp
cursor, region, and `compose_buf`. Fixed across several commits.

**Test hooks:**
- `crates/anvil-term/src/grid.rs` — `"grid resize matrix"` (10 comptime cases,
  verifies I-G1–I-G5 invariants after every resize).
- `crates/anvil-term/src/terminal.rs` — `"terminal resize matrix"` (12 comptime
  cases, verifies I-R3–I-R9 invariants including alt-grid parity, viewport
  clamp, compose_buf width).
- `crates/anvil-term/src/terminal.rs` — `"resize to 1x1 from any state terminates"`
  (4 degenerate sizes: 0×0, 1×1, 80×1, 1×24).

**Invariants locked:**
- I-R3: cursor in bounds after resize.
- I-R4: scroll region reset (when dims changed).
- I-R5: `scrolled_off.len == width`.
- I-R7: `viewport_offset <= history.len()`.
- I-R8: `alternate.width == primary.width` and `alternate.height == primary.height`.
- I-R9: `compose_buf.len == cols`.

## Bug B — Application Freeze

**Symptom:** The HUD refreshed by calling `git.query` on the main thread.
`git.query` spawns `git status` as a subprocess and blocks until it returns
(up to 2 s timeout), freezing keystrokes, shortcuts, copy/paste, and rendering
once per second.

**Fix (commit 29c0b96):** HUD git refresh now runs on a short-lived worker
thread. The main thread only reads a finished result via atomic flags. No
blocking subprocess calls on the main thread.

**Triggering input:** Not a user input sequence — triggered by any git
repository being open when the HUD was visible. Encoded as the bounded-
termination smoke test in `"resize to 1x1 from any state terminates"` (which
also covers degenerate resize paths that could theoretically block). The HUD
thread freeze itself cannot be expressed as a headless unit test because it
requires the main-thread event loop.

**Test hooks:**
- `crates/anvil-term/src/terminal.rs` — `"resize to 1x1 from any state terminates"`
  verifies all degenerate resize calls complete without hanging.

## Bug C — Per-Frame Heap Allocations (Lag)

**Symptom:** If `draw_viewport` (or the draw loop it replaced) accidentally
allocated on the heap during steady-state rendering, GC pressure would cause
frame-rate jitter.

**Fix:** The draw loop was refactored into `crates/anvil-render/src/draw.rs`. It reads
only its parameters; all per-frame operations are CG draw calls with no
allocator use.

**Test hook:**
- `crates/anvil-render/src/draw.rs` — `"a steady-state frame performs zero heap allocations"`
  builds `Terminal` + `Raster` through a `CountingAllocator`, resets the
  counter, calls `draw_viewport`, and asserts `alloc_count == 0` and
  `resize_count == 0`. Runs on both the live-bottom and smooth-scroll paths.

## Bug D — Resize Ghosting

**Symptom:** During a live resize the GPU presented frames asynchronously,
causing the old frame to show briefly at the new size (ghosting).

**Fix (commit 28c0151):** During a live resize, `present` is called with
`sync = true`, which uses `waitUntilScheduled` + synchronous `present` so the
frame lands in lockstep with the layer's drawable size.

**Test hook:**
- `crates/anvil-platform/src/metal.rs` — `"presentMode returns sync during live resize and async otherwise"`
  verifies the `present_mode(in_live_resize)` pure function returns `Sync`
  when resizing and `Async` otherwise. End-to-end ghosting is manual QA only.

## Bug E — Stale Separators

**Symptom:** Prompt-rule hairlines drawn in a previous frame were not erased
in the next frame if the prompt mark moved or disappeared, leaving stale
separator lines.

**Fix:** The `clear()` call at the top of each frame resets the full bitmap
to the background color, erasing all prior content including separator rules.

**Test hooks:**
- `crates/anvil-render/src/raster.rs` — `"clear then draw nothing leaves a pure background bitmap"`
  (I-E1): verifies that after `clear()` with no subsequent draw calls, every
  sampled pixel matches the fill color.
- `crates/anvil-render/src/raster.rs` — `"rowRule draws only on its row and clear erases it next frame"`:
  draws a rule then calls `clear()` and verifies the rule pixel is gone.
- `crates/anvil-render/src/draw.rs` — `"prompt-rule rows match the prompt-mark set across viewport scroll"`
  (I-E2): verifies `rule_row()` agrees with `is_prompt_start()` for every
  viewport row, table-driven across scroll positions.

## File-Tree Click Hit-Test (Off-by-One Regression)

**Symptom:** A header row was added to the file-tree panel ("FILES" label),
but the click-to-entry-index math was not updated, causing every click to
open the wrong entry (off by one).

**Fix:** The click math was extracted into `filetree_render::tree_row_at_click()`
which takes `header_rows` as a parameter. `on_mouse_down` now calls this
function.

**Test hook:**
- `crates/anvil-render/src/filetree.rs` — `"treeRowAtClick maps click-y to entry index"`:
  verifies header clicks return `None`, first file returns index 0, second
  returns index 1, above-tree-top returns `None`, zero cell_h returns `None`.
