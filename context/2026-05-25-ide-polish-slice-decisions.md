---
date: 2026-05-25
kind: design-spec
status: live
goal: resolve D1–D4 blocking decisions for IDE polish slice (Hermes items 2–6)
---

# IDE Polish Slice — Design Decisions

Scope: Hermes items 2–6. One slice. No new tokens. Touches `left_dock.rs`,
`workspace.rs`, `main.rs` only.

## Architecture Note — The Drawer Is a PaneTree Pane

The bottom terminal drawer is not a dedicated chrome widget. It is the bottom
leaf of the center column's PaneTree 70/30 horizontal split. Its appearance is
controlled by `draw_workspace` in `crates/anvil-render/src/workspace.rs`.

Two drawer states exist today:

- **Live PTY:** `draw_viewport` runs + `draw_terminal_drawer_chrome` overlay
  applies a charcoal wash over `background`. Wash is near-invisible:
  `charcoal` (#161a1c) over `background` (#181a21) at 34% alpha ≈ 2 luma units.
- **No PTY yet:** Empty-pane branch falls straight to a solid `background`
  (#181a21) fill with no chrome — this is the "dominant black band."

Both states need to change. Fix is a solid `panel` base, not a wash.

---

## D1 — Drawer Empty-State Treatment

**Decision: shrink to a compact fixed-height header strip (22px) within the
existing pane rect. Do not collapse the pane.**

Collapsing to a lozenge would require the renderer to resize the PaneTree split
at paint time — a layout-engine concern outside the renderer's responsibility.
The renderer paints into the rect it is given.

When no PTY is attached: solid `panel` (#1d2129) fill across the entire drawer
rect. Top 22px shows `charcoal` (#161a1c) strip with `text_subtle` label
`TERMINAL  ⌘T` left-aligned at the standard `PAD_X` offset.

When a PTY is live: replace charcoal-wash-over-background with a solid `panel`
base under `draw_viewport`. Top separator rule is always `hairline` — structure,
not state (drop the active/inactive color toggle).

---

## D2 — Hover Treatment

**Decision: solid tinted fill only. No left-edge marker on hover.**

Left-edge marker is reserved exclusively for the open-active (selected) state.
BRAND.md: "color communicates state, not decoration." Two left-rail signals at
different states would flatten the hierarchy.

Hover fill: solid `panel` (#1d2129). Lifts cleanly off `charcoal` (#161a1c) —
three steps up the Mineral stack with real perceptual gap. No alpha, no glow.
Text color unchanged on hover.

Hover and selected do not overlap: selected row suppresses hover rendering.
Builder must check `hovered_row == row_index` only when `selected == false`.

---

## D3 — Left-Rail Color for Active File

**Decision: `accent_primary` (mineral teal).**

BRAND.md: mineral/teal = "trace, provenance, active nav, focused operational
state." Active file in explorer is an active navigation target — exactly that
semantic. `accent_ember` is execution-only; neutral high-contrast tokens read
as structural chrome. In MINERAL_DARK, `accent_primary`, `accent_bright`, and
`info` are all `#54b7c0` — one unambiguous teal signal.

2px width and per-row vertical inset (2px top + 2px bottom) already correct in
existing implementation. No geometry change.

---

## D4 — Alt-Row Fill

**Decision: remove alt-row fill from this slice.**

Existing alt-row fill is `surface` (#22262f) at 5% alpha over `charcoal`
(#161a1c). Effective perceptual difference: ≈1 luma unit — invisible. Adds a
`row_i % 2` branch to the hot draw path and conflicts with hover when hover
lands on an even-index row (two simultaneous fills). With hover + selected +
left-rail carrying all distinguishable state, alt-row is redundant. Remove the
branch. Revisit only if scan-line striping returns with a token that has real
perceptual distance.

---

## Token Table

All tokens from `crates/anvil-theme/src/theme.rs` MINERAL_DARK. No new colors.

### Explorer Rows

| State | Row Background | File Label | Dir Label | Dir Icon Glyph | File Icon Glyph |
|---|---|---|---|---|---|
| Rest | — (sidebar `charcoal` shows through) | `text_muted` | `text_muted` | `text_subtle` | `hairline` |
| Hover | `panel` solid | `text_muted` | `text_muted` | `text_subtle` | `hairline` |
| Open-active | `panel` solid | `foreground` | `foreground` | `accent_primary` | `accent_primary` |
| Left rail (selected only) | `accent_primary` 2px solid | — | — | — | — |

Explorer header label ("EXPLORER"): `accent_primary` — unchanged.
Explorer header meta (cwd basename): `text_subtle` — unchanged.

### Drawer

| State | Full-Pane Fill | Header Strip | Header Label | Top Separator |
|---|---|---|---|---|
| No PTY (empty) | `panel` solid | `charcoal` solid, 22px tall | `text_subtle` `TERMINAL  ⌘T` | `hairline` 1px |
| Live PTY | `panel` solid under viewport | n/a | n/a | `hairline` 1px |

Drop the active/inactive top-rule color toggle (currently `accent_primary` vs
`hairline`). Structure wins over state at the separator level.

---

## Dimensions

All values at 1× logical pixels.

| Element | Value | Change |
|---|---|---|
| Drawer default height | 30% of center column h | No change |
| Drawer empty-state header strip | 22px | New |
| Explorer row height (`ROW_H`) | 22px | Was 32px |
| Explorer header row height (`HEADER_H`) | 28px | Was 30px |
| Explorer horizontal padding (`PAD_X`) | 10px | Was 12px |
| Explorer left-rail width | 2px | No change |
| Explorer left-rail vertical inset | 2px top + 2px bottom | No change |

32px row height is terminal-cell legacy. 22px matches the IDE redesign spec
(§5, `2026-05-24-ide-redesign.md`), VS Code, and Zed. BRAND.md: "compact
operational layouts."

---

## Before / After

**Before.** Explorer sidebar inherits terminal-cell geometry: 32px rows, 12px
padding, no hover state. Selecting a file applies a blunt `accent_primary` wash
across the full row — reads like a terminal selection, not a sidebar nav item.
5% `surface` alt-row stripe is invisible. Bottom drawer pane — empty or live —
reads as a dominant dark void: charcoal wash over `background` has near-zero
perceptual weight; empty-pane-no-PTY path bypasses even that wash and produces
a raw `background` rectangle that competes visually with the editor above.

**After.** Explorer rows at 22px with 10px padding read as a real sidebar rail:
compact, operational. Hover applies solid `panel` fill — one clear step up from
`charcoal`, no left marker, no text color change. Open-active row uses solid
`panel` + 2px `accent_primary` left rail + `foreground` text: structured,
operator-console feel. Alt-row noise gone. Drawer shows a compact 22px
`charcoal` header strip with quiet `text_subtle` prompt label when empty — pane
reads as waiting infrastructure, not dead black band. Live terminal gets solid
`panel` base — consistent surface that recedes behind the editor.

---

## Builder Notes

Files that change (no others):

- `crates/anvil-render/src/left_dock.rs` — `ROW_H` (22.0), `PAD_X` (10.0),
  `HEADER_H` (28.0). Remove alt-row branch. Change selected-row fill to solid
  `panel`. Add `hovered_row: Option<usize>` parameter to `draw_left_dock` and
  `draw_left_dock_with_scroll`; paint solid `panel` hover fill when
  `hovered_row == Some(visible_i)` and row is not selected.
- `crates/anvil-render/src/workspace.rs` — `draw_terminal_drawer_chrome`:
  replace charcoal wash with solid `panel` fill; always `hairline` for top rule.
  Empty-pane branch: replace `background` fill with `panel` fill + 22px
  `charcoal` header strip + `text_subtle` label.
- `crates/anvil/src/main.rs` — track hovered explorer row from mouse-move;
  thread `hovered_row: Option<usize>` into `draw_left_dock_with_scroll` call.
  Mouse-move handler should hit-test against `left_dock_hits` and update a new
  `hovered_explorer_row: Option<usize>` field on `App`.

Tests requiring updates after ROW_H change:
- `explorer_rows_have_mouse_sized_full_width_targets`: assertion `h >= 30.0` →
  `h >= 20.0`.
- `explorer_rows_return_click_hits`: pixel coordinates change (rows now at y=28,
  50, 72 instead of 30, 62).
- `explorer_scroll_offset_preserves_original_row_indices`: y coordinate
  update.

Add a new test asserting hover paints `panel` only when not selected.
