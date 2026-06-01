# 30-Item Polish + CRAP Pass — Routing

Goal: ship 30 enumerated polish enhancements across perf, UX, and aesthetics, then run a CRAP-scan refactor pass. Scope: `crates/anvil-render/`, `crates/anvil/src/main.rs`, `crates/anvil-workspace/`, plus targeted touches in `anvil-term`, `anvil-platform`, `anvil-control`. Verification: `cargo test --workspace` + `cargo clippy --workspace -- -D warnings` clean after every wave; perf guardrails (dirty-row gated, no `force_full_redraw`, animation flips `dirty` only on phase change) hold.

## Per-item matrix

| # | Target file(s) | Lane | Role | Wave |
|---|---|---|---|---|
| 1 | render/lib.rs, render/raster.rs, platform metal layer | gpu | architect → builder | A1, W2 |
| 2 | render/atlas.rs | atlas | builder | W1 |
| 3 | render/draw.rs, term/grid.rs | draw + term | builder | W2 |
| 4 | render/draw.rs | draw | builder | W3 |
| 5 | render/draw.rs, render/raster.rs | draw | builder | W4 |
| 6 | render/atlas.rs, render/lib.rs | atlas | builder | W2 |
| 7 | anvil-prompt-core, anvil-agent | parsers | builder | W1 |
| 8 | render/draw.rs | draw | builder | W3 |
| 9 | render/draw.rs, term/cell.rs | draw | builder | W4 |
| 10 | render/raster.rs | raster | builder | W1 |
| 11 | anvil/main.rs, render/draw.rs (header hit-test) | main | builder | W2 |
| 12 | render/draw.rs (gutter) | draw | builder | W5 |
| 13 | anvil/main.rs, control/, ui/palette/ | main + ipc | architect → builder | A1, W3 |
| 14 | render/searchbar.rs, term/search.rs | searchbar | builder | W1 |
| 15 | workspace/, anvil/main.rs | workspace | builder | W1 |
| 16 | anvil/main.rs | main | builder | W3 |
| 17 | anvil/main.rs, platform context-menu | main | architect → builder | A1, W3 |
| 18 | anvil/main.rs (keymap) | main | builder | W4 |
| 19 | render/tabbar.rs | tabbar | builder | W2 |
| 20 | anvil/main.rs, term/grid.rs (resize) | main + term | builder | W5 |
| 21 | render/draw.rs | draw | builder | W5 |
| 22 | render/tabbar.rs | tabbar | builder | W3 |
| 23 | anvil/main.rs, render/draw.rs (paint) | main | builder | W6 |
| 24 | render/draw.rs | draw | builder | W6 |
| 25 | render/draw.rs | draw | builder | W7 |
| 26 | platform window chrome | platform | builder | W4 |
| 27 | render/draw.rs (diff colorize) | draw | builder | W7 |
| 28 | render/atlas.rs, render/raster.rs | atlas | builder | W5 |
| 29 | render/draw.rs (SGR face select) | draw | builder | W8 |
| 30 | render/workspace.rs, workspace/ | workspace | architect → builder | A1, W6 |

Conflict groups (must serialize):
- `draw.rs`: 3, 4, 5, 8, 9, 12, 21, 24, 25, 27, 29 — one at a time.
- `tabbar.rs`: 19, 22 — serial.
- `main.rs`: 11, 13, 16, 17, 18, 20, 23 — serial.
- `atlas.rs`: 2, 6, 28 — serial.
- `workspace.rs`/workspace crate: 15, 30 — serial.
- `raster.rs`: 1, 10 — serial.

## Architect pre-pass (A1, parallel)

Dispatch `systems-architect` for 4 design notes, each ≤1 page, saved under `context/2026-05-24-30/`:
- A1.1 — Item 1: GPU default gating, fallback rules, Metal probe.
- A1.2 — Item 13: palette IPC contract (sources: recent cmds, agents, contexts, files), fuzzy ranking site.
- A1.3 — Item 17: context-menu pattern (AppKit NSMenu vs in-Metal); reusable.
- A1.4 — Item 30: focused-pane glow without repainting unfocused panes (dirty-rect strategy).

Acceptance: each note names files to touch, perf budget, dirty-row impact.

## Dispatch waves

Each wave = parallel builders, ≤3 files each. Wait for clippy+tests green before the next wave.

- Wave 1 (5 parallel): #2 atlas LRU, #7 parser pool, #10 fill_pixel_rect ext, #14 block search, #15 divider drag.
- Wave 2 (5 parallel): #1 GPU default (post A1.1), #3 dirty-row bitmap, #6 ASCII preheat, #11 fold/unfold, #19 tab reorder.
- Wave 3 (5 parallel): #4 damage coalesce, #8 scroll quantize, #13 palette (post A1.2), #16 jump-to-prompt, #22 tab anim.
- Wave 4 (5 parallel): #5 selection shader, #9 header cache, #17 context menu (post A1.3), #18 split keybinds, #26 traffic-light tint.
- Wave 5 (4 parallel): #12 gutter dots, #20 living scrollback, #21 scroll easing, #28 ligatures.
- Wave 6 (3 parallel): #23 header pulse, #24 cursor ease, #30 active-pane glow (post A1.4).
- Wave 7 (2 parallel): #25 accent alpha, #27 diff colorize.
- Wave 8 (1): #29 SGR face select.

## Final phase — CRAP scan + refactor

The repo has no `cargo crap` installed. Dispatch builder to:
1. Try `cargo install cargo-crap`; on failure fall back to `cargo-llvm-cov` + complexity via `scc --complexity` or `tokei`.
2. Add `scripts/crap.sh` wrapper.
3. Emit top-10 high-CRAP functions in the workspace.

Then `systems-architect` triages (split / extract / leave-with-justification); builder refactor PRs ≤3 files each; reviewer closes the phase.

## Librarian

After Wave 8 lands, dispatch `librarian` once: append `wiki/log.md` and add a wiki page summarizing the 30-item polish run and CRAP refactor outcomes.
