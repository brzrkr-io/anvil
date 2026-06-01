---
status: active
type: decision
created: 2026-05-24
updated: 2026-05-24
sources: []
confidence: high
---

# Decision: ViewportSink trait for unified viewport draw loop

## Context

`draw.rs` contained four near-identical viewport draw loops:

- CPU live (scroll_pos == 0.0)
- CPU smooth (scroll_pos != 0.0)
- GPU live
- GPU smooth

Each loop performed the same per-row sequence: dirty gate, row-bg clear, block
lookup, fold skip, cell draw, fold summary, block header, prompt rule. The only
differences were: (a) CPU writes pixels into `Raster`; GPU pushes `CellInstance`
records into `CellBatch`. (b) Smooth-scroll shifts all y-coordinates by a
fractional pixel offset. Any bug or logic change had to be applied to all four.

## Decision

Introduce a `ViewportSink` trait (trait-object, `&mut dyn ViewportSink`) and a
single `draw_viewport_into` function. Two implementations: `CpuSink` and
`GpuSink`. Public functions `draw_viewport` and `draw_viewport_gpu` become thin
shims that construct the appropriate sink and call `draw_viewport_into`.

## Alternatives considered

**Generics (`fn draw_viewport_into<S: ViewportSink>`)**: rejected. Two
monomorphisations, larger compile output, no measurable perf benefit — per-frame
cost is dominated by glyph painting and per-row terminal walks, not the ~6
virtual dispatch calls per row the trait object incurs.

**Enum dispatch**: rejected. A `match` per call site adds noise for no gain over
a vtable; the enum arms would be identical to the trait impls.

## Contract

`xy` arguments to sink methods are pre-shift viewport rows. Each sink applies its
own smooth-scroll offset internally: `CpuSink` relies on `raster.y_shift_px`
being set before `draw_viewport_into` is called; `GpuSink` stores an explicit
`shift: f32` field set at construction time.

## Outcome

Net ~177 lines removed from `draw.rs` (file: 2241 → 2064 lines from fff27e3 to
8321b0d). The smooth-scroll and live-bottom paths share one loop body; off-by-one
risks are confined to `off_opt` / `row_end` computed once at the shim level.
Block rendering tests and public API unchanged.
