---
date: 2026-05-24
role: systems-architect
status: proposed
scope: crates/anvil-render/src/draw.rs
---

# Viewport draw consolidation — design

## Goal

Collapse the four viewport draw paths in `crates/anvil-render/src/draw.rs`
(CPU live, CPU smooth, GPU live, GPU smooth) into one shared loop. No
visual change, no behavioural change.

## Backend abstraction — `&mut dyn ViewportSink`

Generics were considered but rejected: two monomorphisations, larger
compile output, perf upside theoretical (per-frame cost is dominated by
glyph painting and per-row terminal walks, not ~6 virtual calls per row).
Enum dispatch adds a match for no real gain. Trait object: one body,
trivial test wiring — picked.

The sink hides everything CPU vs GPU differ on: row-bg clear (CPU only;
GPU no-op), block tint row fill, accent stripe, single cell, fold summary,
block header, prompt rule, cursor. Per-row terminal queries stay in the
unified body. The smooth-scroll shift is the sink's responsibility: it is
constructed with a `y_shift_px` and applies it to every pushed `xy`
internally, removing arithmetic from the loop.

## Unified signature

```rust
trait ViewportSink {
    fn clear_row_bg(&mut self, ry: usize, m: FontMetrics, bg: [u8; 3]);
    fn fill_block_tint(&mut self, ry: usize, cols: usize, m: FontMetrics);
    fn draw_cell(&mut self, x: usize, y: usize, ry: usize, content_row: usize,
                 cell: Cell, m: FontMetrics, theme: &Theme,
                 sel: Selection, search: Option<&Search>);
    fn draw_accent_bar(&mut self, ry: usize, m: FontMetrics, rgb: [u8; 3]);
    fn draw_fold_summary(&mut self, ry: usize, cols: usize, hidden: usize,
                         m: FontMetrics, theme: &Theme);
    fn draw_block_header(&mut self, ry: usize, cols: usize, block: &Block,
                         cmd_text: &str, m: FontMetrics, theme: &Theme);
    fn draw_prompt_rule(&mut self, ry: f64, m: FontMetrics, rgb: [u8; 3],
                        x_start: f64, x_end: f64);
    fn draw_cursor(&mut self, t: &Terminal, cp: CursorParams,
                   m: FontMetrics, theme: &Theme, top_bar_rows: usize);
}

fn draw_viewport_into(sink: &mut dyn ViewportSink, /* same args as
    draw_viewport minus the raster/painter */);
```

Live vs smooth collapse: at the top of `draw_viewport_into`, decide
`off_opt` (None for live, Some(base+1) for smooth) and `row_end` (rows or
rows+1). Inside the loop, compute `crow` from `off_opt`; everything else
is unconditional. `dirty` is gated by `scroll_pos == 0.0` in one line.

Public `draw_viewport` and `draw_viewport_gpu` become thin shims that
construct `CpuSink` / `GpuSink` and call `draw_viewport_into`. **Public
signatures stay identical** — every existing test compiles unchanged.

## Shared helpers (collapse)

- `draw_block_header_cpu` / `_gpu` → one free function
  `compute_block_header_chars(block, cmd_text, cols)` returning a SmallVec
  of `(char, [u8;3])`, plus `ViewportSink::draw_block_header` consuming
  it. Largest single dedup.
- Inline block-tint blocks → `ViewportSink::fill_block_tint`.
- Accent stripe → `ViewportSink::draw_accent_bar`.
- Fold summary → `ViewportSink::draw_fold_summary`.

## `CellBatch::push_bg`

Promote the inline `push_bg` closure to an inherent method
`CellBatch::push_bg(xy, wh, rgb)` calling
`push_cell(xy, wh, None, rgb, rgb)`. A name for what already exists; used
by `GpuSink` for tint, accent, cursor strips.

## Migration order (5 commits, each green)

1. **Extract block-header core.** Replace `_cpu` / `_gpu` with one
   free function + two shims. Verify: tests, screenshot at scroll_pos = 0.
2. **Add `CellBatch::push_bg`.** Replace inline closure. Mechanical.
3. **Introduce `ViewportSink` + `CpuSink`.** Re-route `draw_viewport`
   through private `draw_viewport_into`. CPU smooth/live still branch;
   GPU untouched. Verify: screenshots at 0 and 5.5.
4. **Collapse live vs smooth inside `draw_viewport_into`.** Delete the
   `if scroll_pos == 0.0` split. Verify: 0 / 5.5 / 23.7 identical.
5. **Add `GpuSink`; re-route `draw_viewport_gpu`.** Delete GPU loop
   bodies. Verify: live run, fractional scroll, resize.

## Test impact

All `draw.rs` tests (smoke tests at L1326, L1359, L1579, L1616, L1653,
plus search/cursor variants) call public `draw_viewport` — no changes.
`CpuSink` wraps the `Raster + dyn GlyphPainter` tests already build, so
no new test infra. Add one new test:
`draw_viewport_gpu_matches_cpu_on_block_geometry` renders the same
fixture into both sinks and asserts tint / accent / header positions
agree. Catches future drift.

## Discard test (proves the refactor worked)

1. `cargo test --workspace` green before and after every commit.
2. `cargo clippy --workspace -- -D warnings` clean.
3. Manual via `scripts/run.sh`: `ls && echo done && sleep 1 && false`,
   then visually diff at scroll_pos = 0, ≈ 5.5, ≈ 23.7. All identical to
   pre-refactor.
4. Net line drop in `draw.rs` ≥ 250 lines.

## Risks

- **Off-by-one (`0..rows` vs `0..=rows`).** `row_end` computed once at the
  top; never branch on `scroll_pos` inside the loop. Caught by the 5.5
  screenshot.
- **`absolute_line_of_content(crow)` vs `terminal.row_abs(y)`.** Both
  resolve to the same absolute line when `off = 0`. Keep `crow`
  computation inside the loop; verify with the 0 screenshot.
- **`raster.y_shift_px` lifecycle.** Today: set before CPU smooth loop,
  reset after. `CpuSink::new_smooth(...)` sets on construction; reset on
  `Drop`. Tests stay agnostic.
- **`dirty` only in live path.** One-line gate; do not scatter.
- **GPU `_top_bar_rows` unused.** Origin_y already encodes pane top.
  Preserve this in `GpuSink`.

## Out of scope

Any visual change (block tint / accent / header logic stable as of
fff27e3). Chrome render code (tabbar, statusbar, searchbar). `prompt-core`,
terminal model. Replacing CPU path with GPU path — both stay; this is
factoring only.

## Proposed `wiki/decisions/` entry

`wiki/decisions/viewport-sink-trait.md` — record the choice of
trait-object sink over generics / enum dispatch, the rationale (perf is
not the bottleneck; test ergonomics and a single body are the win), and
the contract that `xy` arguments to sink methods are pre-shift (the sink
applies smooth-scroll `y_shift_px` internally).
