---
date: 2026-05-24
kind: design-spec
status: approved
goal: Elevate HUD visual quality — highest-ROI deltas from the v2 baseline
touches: crates/anvil-render/src/agent_panel.rs
---

# HUD Pretty — Refinement Spec

## Baseline

`draw_right_hud` (~L579): glass surface fill (alpha 0.88/0.72) + single left hairline. Section headers plain text at `app_theme.text_subtle` + hairline at midline of blank row below (~L1416: `row + 0.5`). No vertical brand mark. No top-edge surface line. `GlassTones` still live.

## Changes

### 1. Accent bar left of every section header

Add new helper:

```rust
fn draw_section_accent_bar(
    raster: &mut Raster, metrics: FontMetrics,
    start_col: usize, row: usize, color: [u8; 3],
) {
    let (cw, ch) = (metrics.cell_w, metrics.cell_h);
    let bar_h = (ch * 0.55).max(3.0);
    let x = raster.pad_x + start_col as f64 * cw + 1.0;
    let y = raster.pad_y + row as f64 * ch + (ch - bar_h) * 0.5;
    raster.fill_pixel_rect(x, y, 2.0, bar_h, color);
}
```

Call with `app_theme.accent_primary` immediately before each `draw_section_header` (7 sites). 2px ember bar, gutter-left, vertically centered on the header row. Brand anchor every section without a new color.

### 2. Top-edge panel highlight

After the existing left hairline (~L620), insert:

```rust
raster.fill_pixel_rect(surface_rect.x, surface_rect.y, surface_rect.w, 1.0, tones.edge);
```

Left + top hairlines form an L-frame. HUD reads as docked slab, not floating overlay.

### 3. Hairline tight under header text

In `draw_section_rule` (~L1416): change `+ 0.5` to `+ 0.1`:

```rust
let y = raster.pad_y + (row as f64 + 0.1) * ch;
```

Rule sits just below the label. Section reads as framed, not split.

### 4. Section header color: text_subtle → text_muted

At all 7 `draw_section_header` call sites, change `color` from `app_theme.text_subtle` to `app_theme.text_muted`. More presence at header level.

### 5. Inter-section gap

Before each section's accent bar + header (except the first), insert one blank row:

```rust
if r < bottom { r += 1; }
```

7 insertions. Sections read as distinct modules.

### 6. CI: separate duration to right-aligned column

For `CiState::Ok` and `CiState::Failed` (~L940-973): replace combined label string with two draws:
- Left: branch name at `inner_col + 2` in `app_theme.foreground`.
- Right: `format!("{}s", ci.duration_s)` via `draw_text_right(... max_col, r, &dur, app_theme.text_muted)`.

`draw_text_right` already exists (~L1814). Polarity matches SYSTEM row.

### 7. Dirty indicator: `*N modified` → `*N`

~L838: `format!(" *{} modified", local.git_dirty)` → `format!(" *{}", local.git_dirty)`. Frees 9 cells for longer branch names.

### 8. Empty sections stay collapsed

Keep as-is. Placeholder `—` row would add noise with no signal.

## Priority

1. Accent bar — biggest visual lift.
2. Top-edge highlight — one line, immediate depth.
3. Hairline position — one digit, large feel difference.
4. Header color bump — 7 trivial edits.
5. Inter-section gap — 7 insertions, strongest readability gain.
6. CI duration right-align — polarity matches SYSTEM.
7. Dirty compact — minor cleanup.
