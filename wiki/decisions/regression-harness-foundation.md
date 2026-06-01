---
status: active
type: decision
created: 2026-05-22
updated: 2026-05-22
sources: []
confidence: high
---

# Decision — Regression Harness Foundation

## Context

Five bugs were fixed prior to this decision: resize invariant violations (A),
a main-thread freeze from blocking `git status` (B), per-frame heap allocations
(C), resize ghosting (D), and stale separator lines (E). Without automated
regression tests, any of these could silently recur.

The goal was to lock these bugs with tests that:
- Run entirely under `zig build test` (no Metal device, no PTY, no AppKit
  event loop required).
- Are fast and deterministic.
- Do not require mocking the Metal renderer or the CoreGraphics rasterizer.

## Decisions Made

### Suite runs entirely under `zig build test`, no Metal device

All tests use the existing `std.testing.allocator` or `CountingAllocator`.
Raster tests create real CoreGraphics bitmap contexts (they work in unit
tests because CG does not require a display). Metal device creation is not
tested — that path is covered by the app launching successfully.

### Resize locked by comptime matrices

Resize invariants are checked via `inline for` over a `const cases` array of
`ResizeCase` / `GridResizeCase` structs. This gives named failure output
("case 'grow then shrink round trip' failed: ...") while keeping a single
test declaration in the file. Ten grid-level cases and twelve terminal-level
cases cover the cross-product of grow/shrink/degenerate/alt-screen scenarios.

### Per-frame cost locked by zero-heap-allocation contract via CountingAllocator

`src/testing/counting_allocator.zig` wraps any child allocator and counts
calls. After initial setup, `reset()` clears counters and `drawViewport` is
called. `alloc_count == 0` and `resize_count == 0` is the invariant. This
does not prevent CG draw-call overhead (which is expected and harmless), only
Zig heap pressure.

### Ghosting end-to-end is manual QA only

`presentMode(in_live_resize)` is a pure function and is unit-tested. The
actual GPU synchronization (whether the frame visually ghosted) requires a
live Metal device and a live resize drag — this is manual QA. The unit test
locks the interface contract so the logic cannot be accidentally removed or
inverted without a test failure.

### No headless render harness, no Metal mock, no `g` abstraction

The global `g` struct in `main.zig` is not abstracted. `drawViewport` takes
its parameters explicitly, but the rest of `renderFrame` still reads `g`
directly. The zero-allocation test constructs a real Raster and Terminal and
calls `drawViewport` with explicit parameters — no fake renderer needed.

### `ruleRow` extracted as a pure function

`ruleRow(terminal, viewport_y, off)` encodes the prompt-separator decision
so it can be table-tested without a raster. It lives in `src/render/draw.zig`
alongside `drawViewport`. The raster's `rowRule` method is kept as-is; the
decision of *whether* to draw a rule is now separately testable.

### `treeRowAtClick` extracted for hit-test correctness

The click-to-entry-index formula is extracted into a pure function in
`src/render/filetree.zig`. The `onMouseDown` call site now uses it. The unit
test verifies the header-click null return and the first-file/second-file
index mapping, locking the off-by-one that was fixed.

## Consequences

- 312 → 324 tests (12 new).
- `src/render/draw.zig` is a new file (the per-row draw loop, previously inline
  in `renderFrame`). It has no platform imports beyond Raster/Font.
- `src/testing/counting_allocator.zig` is a new utility usable by any future
  test that needs allocation pressure or OOM injection.
- `src/render/metal.zig` gains `presentMode` / `PresentMode` as a public,
  testable function.
- `src/render/filetree.zig` gains `treeRowAtClick` as a public, testable
  function.
- The `drawCell`, `drawCursor`, and `resolve` helpers move from `main.zig`
  into `draw.zig`; `main.zig`'s `renderFrame` becomes a thin wrapper.
