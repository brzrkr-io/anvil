---
date: 2026-05-24
kind: routing-note
goal: Execute eleven-item D-parity batch on rust-port branch safely
---

# D-Parity Batch Routing

## Goal
Ship eleven D-parity / cleanup items. Each item must end green:
`cargo test --workspace` and `cargo clippy --workspace -- -D warnings`.

## Conflict Map (file → items)
- `crates/anvil/src/main.rs`: 0
- `crates/anvil-workspace/src/pane.rs`: 0
- `crates/anvil-render/src/draw.rs`: 0, 1, 3, 6, 7, 8, 10
- `crates/anvil-render/src/workspace.rs`: 0, 7, 8, 10
- `crates/anvil-render/src/raster.rs`: 3, 10
- `crates/anvil-render/src/tabbar.rs`: 2, 4, 10
- `crates/anvil-render/src/statusbar.rs`: 4, 5, 10
- `crates/anvil-render/benches/draw_viewport.rs`: 0, 7, 8
- `crates/anvil-term/src/terminal.rs`: 1
- `crates/anvil-prompt-core/src/render.rs`: 1, 9

## Routing
1. [item 0] builder — Revert in-progress spring physics; delete all `overscroll*` fields/params/callsites, leave clamped scroll; depends on: none; parallelizable-with: 1, 2, 9 (different files).
2. [item 9] builder — Drop `_segments` param from `prompt-core::full` + callers; depends on: none; parallelizable-with: 0, 2.
3. [item 2] builder — Diagnose basin `◒` invisibility (glyph coverage / x-position / titlebar occlusion); recommend fix, then apply; depends on: none; parallelizable-with: 0, 9.
4. [item 1] builder — Emit block header row (cmd · duration · exit) on first card row; touches `terminal.rs`, `draw.rs`, possibly `prompt-core`; depends on: 0 (draw.rs churn), 9 (prompt-core churn); parallelizable-with: none.
5. [item 3] builder — Widen / reposition block accent stripe; update pixel-sample unit tests in `draw.rs`; depends on: 1 (draw.rs); parallelizable-with: 4, 5.
6. [item 4] builder — Optical-center chrome glyphs in `tabbar.rs` + `statusbar.rs`; depends on: 2 (tabbar.rs); parallelizable-with: 3, 5.
7. [item 5] builder — Always-present agent dot in `statusbar.rs` (subtle when not Live); depends on: 4 (statusbar.rs); parallelizable-with: 3.
8. [item 6] builder — Delete dead `top_bar_rows()` callsite arithmetic; depends on: 1, 3 (draw.rs); parallelizable-with: 7 (paired).
9. [item 7] builder — Drop `top_bar_rows` param from draw_viewport / GPU / workspace / bench; remove `top_bar_rows()` + `bottom_bar_rows()` from `App`; depends on: 6 (paired — same builder, same PR); parallelizable-with: none.
10. [item 8] systems-architect → builder — Consolidate four viewport draw paths behind a backend trait; flag for design pass first; depends on: 0, 1, 3, 6, 7 (all draw.rs settles); parallelizable-with: none. LAST.
11. [item 10] librarian — Strip stale `Ported from src/render/*.zig` headers; depends on: 8 (touches same files last); parallelizable-with: none.

## Recommended Execution Order
0 → 9 → 2 (these three in parallel if safe) → 1 → 3 → 4 → 5 → 6+7 (one builder, one PR) → 8 (architect first) → 10.

Visual items (1, 2, 4, 5) deferred-verified by the main session at end via `scripts/run.sh`.
