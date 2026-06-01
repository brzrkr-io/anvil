# Visual Diff Spec: Current Anvil vs Option A (Ember Command Deck, v1)

date: 2026-05-26
author: design-lead
status: ready-for-builder
target: sketches/native-editor-directions/index.html — section.variant.v1, lines 141-154

---

## Summary

Functional IDE skeleton lost clarity at every section boundary. 4 structural failures (P0), 3 chrome problems (P1), 4 refinements (P2). Fix P0s first — they cause "cutting each other off" + "shitty lazyvim" complaints.

---

## Delta Spec

### D1 — Context bar overlaps tab strip (P0)

**What user sees:** Orange "IDE" chip row sits on top of chrome tab row. "IDE   anvil[scratch]" + "IDE" in chrome row merge as one band.

**What Option A shows:** `.topbar` (34pt, traffic-lights + chip + path) is one distinct strip. `.tabs` are a separate second strip immediately below it with a clear hairline between.

**Root cause:** `Docks::for_mode_with_left_dock_w` sets `top_h = 28.0 * scale * ui_scale` (`mode.rs:144`). Chrome tab row (`chrome_top_px`) computed separately in `main.rs` and placed at y=0. Context bar rect starts at `y + top_h`, but `y` is `window_inner.y` which is already below OS title — not below tab strip.

**Fix:** Context bar must sit BELOW the chrome row. Either:
- Change `compute_areas` so `context_bar_y = window_inner.y + chrome_top_px` and `pane_area.y = context_bar_y + top_h`. Thread `chrome_top_px` into `Docks::compute_areas` or pass adjusted `window_inner.y`.

**LoC:** 15-20

---

### D2 — Explorer too narrow / sections collide (P0)

**What user sees:** Explorer cramped. "sections cutting each other off" likely from D1 geometry overlap.

**Fix:** Set `left_dock_w_pt` default from `300.0` to `260.0` in `for_mode_with_left_dock` (`mode.rs:92`). Verify `left_dock.h = (h - top_h - bottom_h - chrome_top_px).max(0)` after D1.

**LoC:** 5

---

### D3 — Editor tab strip: cell-width tabs with proportional labels (P0)

**What user sees:** "scratch" tab sits inside wide blank rect. Tab width computed as cells * cell_w, but labels render proportional. Massive blank gap inside each tab.

**Root cause:** `tabbar.rs:148` computes tab widths in cell units but uses `ui_line` for labels.

**Fix:** Replace cell-based formula with proportional measure:
```rust
let label_w = ui_painter.measure(label, TAB_LABEL_PT, weight);
let tab_w = (label_w + 2.0 * TAB_PAD_PX * scale).max(9.0 * cell_w).min(24.0 * cell_w);
```

**LoC:** 10-15

---

### D4 — Welcome/scratch state: no visual identity (P0)

**What user sees:** Scratch shows "Anvil / Cmd+P open file / Cmd+E new editor" in plain muted mono. Looks like blank terminal.

**What Option A shows:** Full editor chrome — tab strip with ember rule, gutter, syntax, status. Scratch empty state must render tab strip + status bar + centered welcome block in `--ember2`.

**Fix:** In `workspace.rs::draw_empty_pane`:
- Call `draw_editor_chrome` to render tab strip + status bar scaffold even with no buffer.
- Paint welcome hints via `ui_line` at proportional size, centered in body rect.
- Remove "TERMINAL ⌘T" placeholder.

**LoC:** 25-35

---

### D5 — Bottom drawer: no chrome identity strip (P1)

**What user sees:** Terminal drawer has only top hairline. No "TERMINAL" label.

**What Option A shows:** `⌁ hermes` agent prompt in `--ember2` then output. Top of drawer immediately legible.

**Fix:** In `draw_terminal_drawer_chrome`, paint a 28pt charcoal header strip at top of drawer. Label "TERMINAL" in `text_subtle`. Reserve `28pt * ui_scale` so terminal cells start below.

**LoC:** 20

---

### D6 — Context bar background matches tab strip exactly (P1)

**What user sees:** Once D1 fixed, context bar fills `theme.graphite` — same as chrome strip → one wide undifferentiated band.

**Fix:** `context_bar.rs:59` — change fill from `theme.graphite` to `theme.charcoal`. One-line change.

**LoC:** 1

---

### D7 — Explorer header row height out of proportion (P1)

**What user sees:** "EXPLORER" header at 36pt is too tall vs 34pt content rows. No visual hierarchy.

**Fix:** `HEADER_H_BASE` in `left_dock.rs:136` from `36.0` to `30.0`.

**LoC:** 1

---

### D8 — Explorer section label too loud (P1)

**What user sees:** "EXPLORER"/"OUTLINE" in `accent_bright` Semibold at full saturation reads as alert, not label.

**Fix:** In `draw_explorer_section` (`left_dock.rs:563-571`) — Semibold → Regular, `EXPLORER_HEADER_PT` 11→10. Same for OUTLINE (`left_dock.rs:1111-1119`).

**LoC:** 4

---

### D9 — Editor pane top accent rule too aggressive (P2)

**What user sees:** Pane-level accent rule frames focused pane top. Unfocused pane has no framing — floats without context.

**What Option A shows:** All panes have same tab strip fill. Only active TAB inside the strip gets the ember rule. No pane-level accent.

**Fix:** Remove pane-level accent rule (workspace.rs:419-421). Active tab's 2px top rule is enough.

**LoC:** 3

---

### D10 — Bottom status bar background missing (P2)

**What user sees:** Status bar text runs into bottom edge with no fill behind.

**Fix:** Ensure outer bottom bar has `theme.panel` fill + `theme.hairline` top edge drawn before text. Verify `chrome_bottom_px` accounts for both editor status bar + outer status bar.

**LoC:** 5-8

---

### D11 — Scratch welcome uses mono glyph loop, not proportional (P2)

Resolved by D4. **LoC:** 0

---

## Build order

D1 → D2 → D3 (geometry must be correct first)
D4 (scratch state, independent)
D5 (drawer, independent)
D6, D7, D8 (one-liners, any order)
D9, D10 after D1 geometry stable
