---
title: Proportional UI font path
date: 2026-05-26
status: spec
owner: systems-architect
---

# Proportional UI font path

## Goal

Add a second text rendering path that lays out and rasterizes proportional
(variable-advance) UI text for chrome surfaces — explorer, tabs, status,
breadcrumbs, overlays, toasts — while leaving the existing monospaced
cell-grid path responsible for code, terminal output, and any glyph that
must align to the PTY grid.

Non-goals: RTL/complex shaping, ligatures in UI text, font subsetting,
disk caching, swapping the monospace path.

## Architecture

### New module

`crates/anvil-platform/src/ui_text.rs` (single file, ~300 LOC target).

It owns:
- A `UiFont` struct: `CFRetained<CTFont>` plus cached ascent/descent/leading.
- A `UiPainter` that holds the active `UiFont` set (regular/bold per size
  bucket) and an LRU line cache.
- A `UiLineCache` keyed by `(family_id: u32, size_q: u16, weight: u8, color_key: u32, text: SmallString)`.

`crates/anvil-platform/src/lib.rs` adds `pub mod ui_text;` and re-exports
`UiPainter`, `UiWeight`, `UiTextError`.

`crates/anvil-render/src/raster.rs` gains a new trait alongside (not
replacing) `GlyphPainter`:

```rust
pub trait UiTextPainter {
    fn measure(&mut self, text: &str, size_pt: f64, weight: UiWeight) -> f64;
    fn draw_line(
        &mut self,
        text: &str,
        x_px: f64,
        baseline_y_px: f64,
        size_pt: f64,
        weight: UiWeight,
        fg: [u8; 3],
        pixels: &mut [u8],
        bitmap_w: usize,
        bitmap_h: usize,
    );
}

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub enum UiWeight { Regular, Medium, Semibold }
```

`Raster` gains convenience wrappers that mirror `glyph_at`:

```rust
pub fn ui_line(
    &mut self,
    painter: &mut dyn UiTextPainter,
    text: &str, x_px: f64, baseline_y_px: f64,
    size_pt: f64, weight: UiWeight, fg: [u8; 3],
);
pub fn ui_measure(
    &self,
    painter: &mut dyn UiTextPainter,
    text: &str, size_pt: f64, weight: UiWeight,
) -> f64;
```

The trait stays in `anvil-render` so the crate remains platform-free; the
CoreText impl lives in `anvil-platform`.

### Sizing

All sizes are **logical pt**, multiplied by `backing_scale` inside the
painter. Default sizes per surface (see table below). Sizes quantized to
0.5pt steps before being used as cache key.

### Rasterization pipeline

1. Build `CFAttributedString` with `kCTFontAttributeName = ui_font_for(size,weight)` and white fg.
2. `CTLine::with_attributed_string(&attr)`.
3. `CTLineGetTypographicBounds` for bounds. Mask: `w = width.ceil() + 2`, `h = (asc+desc).ceil() + 2`.
4. 8-bit gray `CGBitmapContext`. `CGContextSetShouldSmoothFonts(true)`. Subpixel positioning OFF.
5. `CTLineDraw(line, ctx)` at `(0.0, descent)`.
6. Store 8-bit luminance as coverage mask in `UiLineMask`.
7. LRU insert.

Composite mask over BGRA8 raster using existing per-pixel tinted alpha blend.

Destination: `dest_x = x_px.round() as isize`, `dest_y = (baseline_y_px - line.ascent).round() as isize`.

### Pixel snapping

- `x_px` and `baseline_y_px` floored to integer device pixels inside `draw_line`.
- Rasterize at integer x = 0 (no subpixel positioning per cached variant).
- Sizes quantized to 0.5pt.

### Caching

LRU: **1024 entries**, **4 MiB cap**. Color NOT in key — coverage mask + composite-time tint.

Invalidation on `set_family` / `set_scale` clears cache.

### Failure modes

| Failure | Behavior |
|---|---|
| Configured family load fails | Log once, fall back to `SF Pro`. |
| `SF Pro` load fails | Fall back to `CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, ..)`. |
| Glyph missing | CoreText falls back automatically. |
| `CTLineCreate` null | `draw_line` no-ops; `measure` returns 0.0; log once. |
| Zero text | Skip both. |
| Bitmap alloc fails | No-op, log once. |

Mono path never touched on any failure.

### Configuration

Extend `FontCfg` in `crates/anvil-config/src/lib.rs`. Add nested `[font.ui]`:

```toml
[font]
family = "IBM Plex Mono"
size = 15.0

[font.ui]
family = "SF Pro Text"
size = 13.0
weight_regular = "regular"
weight_strong  = "semibold"
```

```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiFontCfg {
    pub family: String,
    pub size: f64,
    pub weight_regular: String,
    pub weight_strong:  String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FontCfg {
    pub family: String,
    pub size: f64,
    pub ui: UiFontCfg,
}
```

UI font config live-reloads (matches mono path policy).

### Per-surface size table

| Surface | Weight | Logical pt |
|---|---|---|
| Explorer row label | Regular | 13 |
| Explorer section header | Semibold | 11 |
| Editor tab label (active) | Medium | 13 |
| Editor tab label (inactive) | Regular | 13 |
| Context bar / breadcrumb segment | Regular | 12 |
| Status bar text | Regular | 12 |
| Overlay card title | Semibold | 14 |
| Overlay card body | Regular | 13 |
| Toast title | Medium | 13 |
| Toast body | Regular | 12 |
| Tooltip | Regular | 11 |

Constants in `crates/anvil-render/src/ui_text_sizes.rs` (data only).

## Call-site migration plan

1. **Explorer (`anvil-render/src/left_dock.rs`)** — `:474`, `:838`, `:1265`. Replace per-char loops in row labels + section headers + breadcrumb tail.
2. **Editor tab strip (`anvil-render/src/tabbar.rs`)** — label draws at `:289 :295 :313 :357 :423`. Keep icon glyphs (close `×`, Nerd Font) on mono.
3. **Status bar (`anvil-render/src/statusbar.rs`)** — `:94 :111 :161 :168 :175`. All text draws migrate.
4. **Context bar / breadcrumbs (`anvil-render/src/context_bar.rs`)** — `:309 :339 :343`.
5. **Overlays + toasts** — after track B lands.

Mono preserved for: terminal grid (`workspace.rs`), editor buffer body (`editor.rs`), command palette body, agent panel transcript.

## Wiring

`crates/anvil/src/main.rs` builds painters once and passes both to render loop:

```rust
let ui_painter = UiPainter::new(cfg.font.ui.clone(), backing_scale)?;
draw(.., &mut glyph_painter as &mut dyn GlyphPainter,
        &mut ui_painter   as &mut dyn UiTextPainter, ..);
```

Every chrome render function picks up `painter: &mut dyn UiTextPainter` argument in same migration commit that switches its glyph loop. Functions that don't draw text get nothing new.

## Verification

Unit tests in `ui_text.rs`:

- `ui_measure_zero_for_empty_string`.
- `ui_measure_monotonic_in_length`.
- `ui_measure_bold_ge_regular`.
- `ui_line_inks_pixels_for_nonempty_text`.
- `ui_line_zero_text_is_noop`.
- `ui_line_uses_baseline_y`.
- `ui_line_tints_to_fg_color`.
- `cache_evicts_when_capped`.

Integration smoke in `crates/anvil-render/tests/`:

- No-op `UiTextPainter` that records calls + counts. Drive `tabbar::draw`, assert expected call count + monotonic x positions.

No pixel-diff goldens — brittle across macOS versions / scales.

CI: `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`.

## Phases

| Phase | Scope | Estimate |
|---|---|---|
| 1. Foundation | `ui_text.rs`, trait, cache, config, wiring, unit tests | 2 days |
| 2. Explorer | `left_dock.rs` migration | 1 day |
| 3. Tabs + breadcrumbs + status | `tabbar.rs`, `context_bar.rs`, `statusbar.rs` | 1 day |
| 4. Overlays + toasts | After track B lands | 1 day |
| 5. Polish | Per-surface tuning, fallback log dedupe | 0.5 day |

## Open assumptions

- `CTLineDraw` against 8-bit gray context yields usable coverage mask (proven by existing `Rasterizer`).
- 1024-entry / 4 MiB cap is enough for chrome string set. Revisit if profile shows high miss rates.
- Backing scale stable for window lifetime; on display change clear cache + rebuild fonts.
