---
date: 2026-05-24
kind: routing-note
goal: Fan out 21-item visual-polish + DevOps-integration batch across subagent waves
---

# Polish Batch Routing — 21 items

## Prerequisite flag

User stated the HUD spec lives at `context/2026-05-24-hud-redesign.md` and the
mockup at `docs/design/hud-mockup.html`. **Both are missing on disk.** Item A
is gated on design-lead producing those artifacts (per
`context/2026-05-24-hud-routing.md`).

## Perf guardrails (every builder dispatch)

- Paints go into dirty-row tracking; never `force_full_redraw`.
- Animation state (items 4, 17, 18, 19) gates `app.dirty = true` only while
  `phase != target`. Steady-state stays on partial-frame ~6 ms path.
- Row-fill paints use `Raster::fill_pixel_rect`. No per-cell loops.
- Chrome-strip text uses `glyph_at(x_px, y_px)`, not cell grid.

---

## Per-item routing

| # | Item | Role | Files | Same-file conflicts | Architect? |
|---|------|------|-------|---------------------|------------|
| A | HUD redesign | design-lead → architect (gate) → builder | `agent_panel.rs::draw_right_hud`, maybe new `palette.rs`, maybe workers | none in batch | if workers |
| 1 | Window rounding + shadow | builder | `anvil-platform/src/appkit.rs` | — | no |
| 2 | Active-tab accent rule | builder | `anvil-render/src/tabbar.rs` | 6, 18 | no |
| 3 | Hairline above prompt rows | builder | `anvil-render/src/draw.rs` | 4, 5, 10–13, 16 | no |
| 4 | Cursor easing | builder | `draw.rs::cursor_opacity`, `anvil/src/main.rs::App::tick` | draw: 3, 5, 10–13, 16; main: 14, 17, 18 | no |
| 5 | Selection overlay | builder | `draw.rs::draw_cell` | 3, 4, 10–13, 16 | no |
| 6 | Sub-cell chrome glyph metrics | architect → builder | `anvil-platform/src/font.rs`, `tabbar.rs`, `statusbar.rs` | font: 7, 9; tabbar: 2; statusbar: 15, 19, 20 | **yes** |
| 7 | OpenType `calt` ligatures | architect → builder | `anvil-platform/src/font.rs` | 6, 9 | **yes** |
| 8 | 8 px baseline grid | builder | `tabbar.rs`, `statusbar.rs` | 2, 6 / 6, 15, 19, 20 | **deferred** per user |
| 9 | Bold / italic glyph variants | architect → builder | `font.rs`, `glyph_atlas.rs`, `anvil-term` SGR → `draw.rs` | font: 6, 7; draw: 3, 4, 5, 10–13, 16 | **yes** |
| 10 | Block-header status pip | builder | `draw.rs::draw_block_header_*` | 3–5, 11–13, 16 | no |
| 11 | Folded-block one-liner | builder | `draw.rs` fold, `anvil/src/main.rs` hit-test | draw: 3–5, 10, 12, 13, 16; main: 4, 14, 17, 18 | no |
| 12 | Alternating row tint | builder | `draw.rs` | 3–5, 10, 11, 13, 16 | no |
| 13 | Inline diff colorization | architect → builder | `anvil-term` block diff flag, `draw.rs` per-row tint | draw: 3–5, 10–12, 16 | **yes** |
| 14 | Palette as 28 px strip | architect → builder | `anvil-platform/src/webview.rs`, `anvil/src/main.rs`, `ui/palette/` | main: 4, 11, 17, 18 | **yes** |
| 15 | Search bar pixel-strip | builder | `anvil-render/src/searchbar.rs` | — | no |
| 16 | Gutter quick-jump dots | builder | `draw.rs`, `anvil/src/main.rs` hit-test | draw: 3–5, 10–13; main: 4, 11, 14, 17, 18 | no |
| 17 | Smooth-scroll easing | builder | `anvil/src/main.rs`, `Pane` velocity | main: 4, 11, 14, 16, 18 | no |
| 18 | Tab open/close micro-anim | builder | `tabbar.rs`, `Pane`/Tab `anim_phase` in `main.rs` | tabbar: 2, 6; main: 4, 11, 14, 16, 17 | no |
| 19 | Bottom-bar agent pulse | builder | `anvil-render/src/statusbar.rs` | 6, 8, 20 | no |
| 20 | Cluster context segment | architect → builder | new kubectl worker, `statusbar.rs` | statusbar: 6, 8, 19 | **yes** |

---

## Dispatch waves

**Wave 0 — design + architecture gates (parallel).**
- design-lead: HUD spec + mockup (gates A).
- systems-architect, one combined pass with notes under `context/`:
  6+7+9 (font/atlas/SGR contract), 13 (term diff flag), 14 (palette-strip
  layout + webview frame model), 20 (kubectl worker lifecycle + caching).

**Wave 1 — independent-file builders (parallel).**
- 1 (appkit.rs), 15 (searchbar.rs), 2 (tabbar.rs), 19 (statusbar.rs).
- Each is the sole toucher of its file in this wave.

**Wave 2 — HUD build.** A: builder, after Wave 0 design + (if any) workers.
`agent_panel.rs` is exclusive; can run alongside Wave 1.

**Wave 3 — draw.rs serialized lane (one builder, batched).**
Order: 3 → 10 → 12 → 5 → 4 → 11 → 16 → 13.
Structure first (3, 10, 12), overlays next (5, 4), interactive (11, 16),
term-coupled last (13, after Wave 0 contract).

**Wave 4 — main.rs serialized lane (one builder, after Wave 3).**
4 (blink tick) → 17 (scroll) → 18 (tab anim) → 11 hit-test → 16 hit-test →
14 (palette layout).

**Wave 5 — font/atlas (one builder, after Wave 0).**
6 → 7 → 9. Touches `font.rs` plus atlas + render plumbing.

**Wave 6 — kubectl segment.** 20, after Wave 0 worker design.

**Deferred / out of scope:** 8 (per user), cheatsheet rebuild, main.rs split.

---

## Critical-path shortlist (time-constrained)

User proposed 1, 2, 5. I recommend **1, 2, 15, 19, A**:

- 1 — single-file, biggest first-launch identity win.
- 2 — on user shortlist; cheap, high signal.
- 15 — chrome coherence with bottom bar at zero conflict cost.
- 19 — proves the animation-gating discipline; statusbar.rs only.
- A — headline DevOps/AI surface once HUD spec lands.

Item 5 (selection overlay) is valuable but lives in the draw.rs serial lane;
landing it first forces Wave 3 to start before structural items 3/10/12 and
hurts throughput. Defer 5 to its Wave 3 slot.
