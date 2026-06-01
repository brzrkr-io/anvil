---
status: active
type: decision
created: 2026-05-21
updated: 2026-05-21
sources:
  - ../../wiki/log.md
confidence: high
---

# 0003 — M1 Renderer Brand Alignment: ANSI-16 Palette

## Status

Active. Implemented 2026-05-21. See `wiki/log.md` (entry 2026-05-21 brand
alignment) for the exhaustive hex table.

## Context

The M1 renderer shipped before `BRAND.md` existed. Its ANSI-16 palette,
background, accent, and font were chosen ad hoc. The brand gate (run at M1
closeout) flagged all four as diverging from the Mineral palette and IBM Plex
type system defined in `BRAND.md` and `brand/tokens.json`.

## Decision

Align the renderer to brand semantics:

- **Background**: `#0c0d10` → `#0b0d0e` (anvil.graphite).
- **Accent / cursor (ANSI 6 cyan)**: `#2bb8b0` → `#2f7f86` (accent.mineral /
  status.info).
- **Font**: "Menlo" → `Font.initFirstAvailable` with chain IBMPlexMono →
  SFMono-Regular → Menlo.
- **ANSI-16 palette**: map each slot to the nearest brand semantic status
  color. The semantic mapping is: black=graphite, red=failure, green=verified,
  yellow=attention, cyan=mineral/info, magenta=agent, white=alloy/muted text;
  bright variants lightened ~15–20%.

### Ambiguities resolved

**ANSI 4 (blue)**: The Mineral palette has no blue token. Chose muted steel
`#4a6f8a`, consistent with the graphite/mineral aesthetic. This is a project
decision, not a brand token — if a blue token is added to `BRAND.md` later,
ANSI 4 should be updated.

**brand.risk (`#a8623a`)**: No ANSI slot maps cleanly to risk/orange. `yellow`
(ANSI 3) is owned by attention; `red` (ANSI 1) is owned by failure. `risk` is
accessible only via 256-color index or direct RGB in terminal output.

## Consequences

- `src/render/color.zig`, `src/render/font.zig`, `src/main.zig` updated.
- `src/config/theme.zig` encodes the decided palette as `mineral_dark` (and a
  corresponding `mineral_light` for light-mode use).
- 102/102 tests passed after the change.
- The two unresolved ambiguities (blue token gap, risk token gap) are visible
  here and should be revisited if `BRAND.md` gains a blue or risk token.
- See [[concepts/console-architecture]] for where the theme is applied in the
  render path.
