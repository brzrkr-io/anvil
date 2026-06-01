---
date: 2026-05-25
kind: design-spec
status: live
goal: spec for Hermes IDE polish items 7, 8, 10 (Explorer directory click, scroll affordance, Outline empty-state)
---

# IDE Polish — Items 7, 8, 10

Inherits all token values, dimensions, and operator-console direction from
`context/2026-05-25-ide-polish-slice-decisions.md`. Only additions and
overrides are stated here.

---

## Item 7 — Explorer Directory Click (expand-in-place)

**Decision: single click expands or collapses a directory in-place. No breadcrumb. No drill-in.**

### State model

Add `expanded_dirs: HashSet<usize>` (keyed by entry index in the current
`DirSnapshot`) to `App`. On a `ExplorerHit::Row(i)` click where
`entry.is_dir == true`: toggle presence in `expanded_dirs`, then re-snapshot
the listing at that directory path — or, for this slice, simply toggle the
visual chevron without re-reading the filesystem (stub expansion; real child
entries are a follow-on task). The set clears whenever the root `DirSnapshot`
changes.

### Chevron glyph

| State | Glyph | Unicode |
|---|---|---|
| Collapsed (default) | `▸` | U+25B8 |
| Expanded | `▾` | U+25BE |

The collapsed `▸` is already rendered at `PAD_X` in the current code for
`is_dir == true`. The expanded state swaps it to `▾`. No other character
change.

**Color:** `text_subtle` at rest; `foreground` when the directory row is
open-active (matches the items 2–6 selected-row foreground rule).

**Leading offset:** the chevron occupies the same icon slot already used —
`rect.x + PAD_X`, one `cell_w` wide. Label starts at `rect.x + PAD_X +
cell_w * 2.0` (one cell gap, matching the existing file-icon + label
layout). No geometry change.

### Hit target

The chevron is part of the full-row hit target. No micro-target. Clicking
anywhere on the row expands or collapses the directory. This matches VS Code,
Zed, and the operator-console density principle (no precision sub-targets).

### Indent per depth

**8px per depth level** (one `cell_w` at the monospace metrics used
throughout the dock). Apply to both chevron start-x and label start-x:
`x_offset = depth as f64 * 8.0`.

Top-level entries remain at depth 0, so existing layout is unchanged.

### Selection on expand/collapse

Expanding or collapsing a directory does **not** change `active_file_path`.
The selected (open-active) file row is unchanged. If the selected file is
inside a directory that gets collapsed, its visual selection is simply not
rendered (the row is no longer visible) — the selection state is preserved in
`active_file_path` and reappears when the directory expands again.

### Animation

None. Instant state swap. Operator-console feel; animation would add latency
with no legibility benefit.

---

## Item 8 — Scroll Affordance

**Decision: thin passive fade-in/fade-out indicator on the right edge. No drag handle. Hidden when content fits.**

### Geometry

| Property | Value |
|---|---|
| Width | 3px |
| Right-edge inset | 0px (flush with the dock's inner right edge, 1px left of the hairline separator) |
| Height | proportional thumb: `(visible_rows / total_rows) * content_h`, minimum 20px |
| Top offset | `content_y_start + (scroll_offset / total_rows) * content_h` |

### Color

`text_subtle` — the quietest non-transparent token in the palette. Keeps the
indicator passive and non-interactive-looking.

### Visibility

- **Hidden entirely** when `total_entries <= available_rows` (no overflow).
- **Visible with opacity** otherwise:
  - Fade in to 60% alpha on scroll (any `explorer_scroll_offset` change).
  - Hold at 60% alpha for 600ms after the last scroll event.
  - Fade out to 0% over 200ms (ease-out).
  - At rest with no recent scroll: 0% (invisible).

Implementation note: the renderer receives a `scroll_indicator_alpha: f32`
value (0.0–1.0) computed by the input layer and passed alongside
`explorer_scroll_offset`. The render path paints the thumb using
`fill_pixel_rect_alpha(x, y, 3.0, thumb_h, theme.text_subtle,
scroll_indicator_alpha * 0.6)`. The timer/decay lives in `App`, not the
renderer.

### Hover / active state

No brightening on cursor-over-track. The indicator is a passive read cue,
not a drag handle. Drag-to-scroll is out of scope for this slice.

---

## Item 10 — Outline Empty-State

**Decision: collapse to quiet 22px section header only. Remove placeholder body copy.**

### When empty (`outline == None` or `outline == Some(&[])`)

Render the `OUTLINE` header row at 22px height (`HEADER_H` from the items
2–6 spec applies; use the same constant). No body content rows. No "No
symbols yet" or "Open a source file" copy.

**Header label:** `"OUTLINE"` — unchanged text, same position.

**Header label color:** `text_subtle` (not `accent_bright`). This demotes
the empty Outline header below the Explorer header (`accent_bright`),
signalling that it carries no active content. When real symbol rows are
present (future task), restore `accent_bright` to match Explorer.

### Divider rule

The 1px `hairline` rule between Explorer and Outline (drawn at `rect.y +
explorer_h` in `draw_left_dock_with_scroll`) stays in place regardless of
Outline empty state. It preserves the panel grid legibility.

### Height when empty

The Outline rect occupies its normal 40% of the dock height. Only the
content area below the 22px header is empty (no fill, no text). The panel
slot is reserved for future symbol rows without layout engine changes.

### Collapsibility

Not in this slice. No click handler on the Outline header. Future task.

---

## New Token Usage (additions to items 2–6 table)

No new tokens. All tokens cited above (`text_subtle`, `hairline`, `foreground`,
`accent_bright`, `accent_primary`) are present in the items 2–6 spec and in
`crates/anvil-theme/src/theme.rs` MINERAL_DARK / MINERAL_LIGHT.

---

## Builder Notes

Files likely to change:

- `crates/anvil-render/src/left_dock.rs` — chevron swap (Item 7); scroll
  thumb paint (Item 8); Outline header color + remove body copy (Item 10).
- `crates/anvil/src/main.rs` — `expanded_dirs: HashSet<usize>` field on
  `App`; toggle on dir-row click; `scroll_indicator_alpha: f32` field +
  600ms/200ms decay timer; thread `scroll_indicator_alpha` into draw call.
- `draw_left_dock_with_scroll` signature gains `scroll_indicator_alpha: f32`.

Tests requiring updates:

- `outline_unavailable_always_shown` — `'N'` in `text_subtle` remains valid
  but the second row ("Open a source file") must no longer appear. Add a
  negative assertion that no glyph from "Open" renders in the blend-50 color.
- `left_dock_renders_outline_no_symbols` — same; confirm only the header row
  is drawn when `Some(&[])`.
- Add `outline_empty_header_uses_text_subtle` asserting the `'O'` glyph from
  "OUTLINE" renders in `th.text_subtle` (not `th.accent_bright`) when
  `outline == None`.
