---
status: active
type: decision
created: 2026-05-30
updated: 2026-05-30
sources:
  - ../../src/chrome.zig
  - ../../src/render/theme.zig
  - ../../editors/nvim/colors/anvil.lua
  - ../../BRAND.md
  - ../../tools/check-render.py
confidence: high
---

# Mineral Warm Palette (2026-05-30)

## Decision

The Mineral palette evolved to "Mineral Warm" on 2026-05-30 for both light and
dark modes. This supersedes the palette values recorded in
[[decisions/0003-m1-brand-palette|0003 M1 Brand Palette]]; see the
Contradiction note below.

### What changed

- Backgrounds shifted from cool blue-graphite to warm near-black with a
  brown-rosewood undertone.
- Primary accent: teal → coral-rose (`mineral` token, #c2614a).
- Ember accent: → burnt orange (#d4601e).
- Text whites → near-neutral cream (`bone` #f0ebe4, `mist` #d8cfc8).
- All 16 ANSI hues warm-recalibrated while remaining mutually distinct.

### What did not change

- Semantic-state vocabulary (verified / attention / risk / failure / agent /
  info / trace) is intact; hues are warm-shifted but semantics are unchanged.
- "Color communicates state, not decoration" — still holds.
- "No literal volcano imagery" — still holds.

## Key Tokens (dark mode)

| Name | Hex | Role |
|------|-----|------|
| graphite | #0e0b0a | primary canvas |
| charcoal | #1c1614 | raised panels |
| mineral | #c2614a | primary accent (coral-rose) |
| ember | #d4601e | secondary accent (burnt orange) |
| mist | #d8cfc8 | primary text |
| bone | #f0ebe4 | emphasis text |
| ash | #3e3028 | dim glyphs / separators |
| alloy | #8a8076 | muted label text |
| verified | #5a8c45 | semantic: verified |
| attention | #b8821a | semantic: attention |
| agent | #8c5fa0 | semantic: agent |

Read `src/chrome.zig` for the complete `Surface` struct and `surface_dark` /
`surface_light` token tables. Read `src/render/theme.zig` for the full 16-color
ANSI tables in `mineral_dark` and `mineral_light`.

## Three-Surface Cohesion

One palette drives three surfaces intentionally:

1. **Chrome** (`src/chrome.zig` `surface_dark` / `surface_light`) — window
   furniture: tab bar, sidebar, status bar, dividers.
2. **Terminal ANSI** (`src/render/theme.zig` `mineral_dark` / `mineral_light`)
   — the 16-color palette seen by shell output and programs.
3. **Syntax highlighting** (`src/session.zig` `roleColor` → ANSI index) — file
   viewer token colors, which resolve through the same ANSI table.

All three surfaces are derived from the same Mineral Warm source so that prompt,
terminal output, and syntax-highlighted files visibly match.

## Variant Dependency

The terminal palette is selected by the `theme_variant` config key. Only the
`mineral` (and `mineral-high`) variants are warm. Third-party variants
(`tokyo-night`, `gruvbox`, `catppuccin`, `nord`, `dracula`, `everforest`)
retain their own cool palettes. Full three-surface cohesion requires
`theme_variant = "mineral"` or `"mineral-high"`.

## Contradiction: 0003 M1 Brand Palette

[[decisions/0003-m1-brand-palette|Decision 0003]] records the earlier
cool-graphite Mineral palette with a teal accent and references source files
that no longer match (`src/render/color.zig`, `src/render/font.zig` — paths
from the Rust port; not present in the `zig` branch). The hex values in 0003
(e.g., `#0b0d0e` for graphite, `#2f7f86` for teal accent) are superseded by
this decision. Decision 0003 should be marked `status: superseded` once this
page is confirmed stable.

## Low-Confidence / Open Items

- The `risk` semantic token gap noted in 0003 (no clean ANSI slot) is not
  resolved by this palette; risk is still accessible only via 256-color index
  or direct RGB.
- `mineral-high` variant shares the same ANSI table as `mineral` (only
  `bg`/`fg`/`bar`/`separator` differ); this is confirmed intentional in
  `src/render/theme.zig`.
