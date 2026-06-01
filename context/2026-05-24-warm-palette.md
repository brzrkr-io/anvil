# Warm Palette Spec — Ember Dark + Ember Light

**Date:** 2026-05-24
**Status:** Design complete, awaiting builder implementation
**Related:** `docs/design/palette-warm.html`, `BRAND.md`, `crates/anvil-theme/src/theme.rs`

---

## Token table — both variants

| Token | ember-dark | ember-light | Notes |
|---|---|---|---|
| `background` | `#1a1815` | `#f4ede6` | Canvas. Dark: warm near-black (R>B by +5). Light: warm off-white, not pure white. |
| `surface` | `#252220` | `#fdfaf7` | Raised panels, active-tab bg, HUD cards. |
| `panel` | `#2f2c29` | `#ede5dc` | Modal overlays (cheatsheet card, deeper surface). |
| `panel_raised` | `#252220` | `#f0e9e2` | Block body tint in draw.rs. Slightly lifted from background. |
| `border` | `#3e3a36` | `#ccc4bb` | Panel edges, table rules. ≥1.4:1 vs surface. |
| `hairline` | `#312e2b` | `#d8d0c8` | 1px chrome separators (quieter than border). |
| `text` | `#dcd8d2` | `#1e1a17` | Primary foreground. Dark ≥9:1 vs background. |
| `text_muted` | `#9e9690` | `#6c6460` | Inactive tab labels, status bar metadata. |
| `text_subtle` | `#6a6360` | `#9e9690` | Chord labels, separator glyphs, very dim text. |
| `alloy` | `#8a827c` | `#7a726c` | Fold summaries, dim gutter metadata. |
| `accent_primary` | `#d05a36` | `#b33f25` | Cursor, basin mark, focus ring. ≥4.5:1 vs background. |
| `accent_bright` | `#de7048` | `#c5462a` | Active-tab rule, tab-bar accent rule. Brighter than primary. |
| `accent_ember` | `#c5462a` | `#c5462a` | Canonical brand ember. Unchanged per user constraint. |
| `graphite` | `#110f0d` | `#ede5db` | Chrome strip background (tab bar row). |
| `charcoal` | `#201d1b` | `#fdfaf7` | Active-tab panel fill, status bar fill. |
| `verified` | `#4e9e68` | `#2a7044` | Exit 0, clean git, passing checks. Reads green. |
| `failure` | `#c94038` | `#a83426` | Non-zero exit, error state. Reads red. |
| `attention` | `#c48c1c` | `#875e10` | Unread dot, stale context, pending action. Reads amber. |
| `agent` | `#7d72bc` | `#554a88` | Agent/automation/model activity dot. Reads violet. |
| `info` | `#3d8e94` | `#296a70` | Branch glyph, trace, info. Reads teal. |

## ANSI slots — ember-dark

| Slot | Name | Hex |
|---|---|---|
| 0 | black | `#1a1815` |
| 1 | red | `#c94038` |
| 2 | green | `#4e9e68` |
| 3 | yellow | `#c48c1c` |
| 4 | blue | `#6e9abf` |
| 5 | magenta | `#b09ad6` |
| 6 | cyan | `#3d8e94` |
| 7 | white | `#c8c4be` |
| 8 | bright-black | `#78706a` |
| 9 | bright-red | `#e05c54` |
| 10 | bright-green | `#68b87e` |
| 11 | bright-yellow | `#d8a030` |
| 12 | bright-blue | `#88b4d2` |
| 13 | bright-magenta | `#c4b4e8` |
| 14 | bright-cyan | `#5ab4bc` |
| 15 | bright-white | `#e8e4de` |

## ANSI slots — ember-light

| Slot | Name | Hex |
|---|---|---|
| 0 | black | `#1e1a17` |
| 1 | red | `#a83426` |
| 2 | green | `#2a7044` |
| 3 | yellow | `#875e10` |
| 4 | blue | `#3a6490` |
| 5 | magenta | `#5c5090` |
| 6 | cyan | `#296a70` |
| 7 | white | `#5a5450` |
| 8 | bright-black | `#5e5854` |
| 9 | bright-red | `#9e2e20` |
| 10 | bright-green | `#246038` |
| 11 | bright-yellow | `#7c540c` |
| 12 | bright-blue | `#345a84` |
| 13 | bright-magenta | `#504680` |
| 14 | bright-cyan | `#236068` |
| 15 | bright-white | `#5a5450` |

## Rationale

- **Backgrounds** are warm (R ≥ G ≥ B by 3-5 units) but not sepia. Dark `#1a1815` is brightness 0x18, safe for 5K Retina all-day reading. Light `#f4ede6` is a warm bone — not pure white.
- **Accent primary** is lightened (dark) / darkened (light) from canonical `#c5462a` to hit ≥4.5:1 WCAG against background. `accent_ember` stays `#c5462a` in both variants for the brand canonical signal.
- **Warm shift on neutrals.** All greys carry R ≥ G ≥ B with 3-5 unit spread — below sepia threshold, above the cool blue of the old Mineral palette.
- **Semantic tokens** kept hue-recognizable: green still reads green, etc. Teal `info` slightly orange-shifted from Mineral's `#2f7f86` to `#3d8e94`.

## Toggle UX

- **Keybind:** `cmd+shift+t` (no conflict with `cmd+t` family).
- **Config value:** `theme = "ember-dark"` or `"ember-light"`. Toggle writes opposite name to `config.theme` in-memory and triggers full redraw.
- **Default on first launch:** `ember-dark`. `Config::default()` field changes from `"mineral-dark"` to `"ember-dark"`.
- **System auto-mode (`theme = "system"`):** maps to `ember-dark` / `ember-light` instead of mineral.

## What changes for the builder

### 1. `crates/anvil-theme/src/theme.rs`
- Add `EMBER_DARK: Theme` and `EMBER_LIGHT: Theme` constants.
- **Widen the `Theme` struct** to carry chrome tokens. Current fields (`background`, `foreground`, `accent`, `surface`, `border`, `ansi[16]`) are insufficient for the chrome renderers. Add: `graphite`, `charcoal`, `panel`, `panel_raised`, `hairline`, `text_muted`, `text_subtle`, `alloy`, `accent_primary`, `accent_bright`, `accent_ember`, `verified`, `failure`, `attention`, `agent`, `info`.
- Add `"ember-dark"` and `"ember-light"` arms to `by_name`.
- Update WCAG tests to cover both new themes.

### 2. `crates/anvil-render/src/tabbar.rs`
Migrate local consts → `Theme` fields:

| Local const | Theme field |
|---|---|
| `GRAPHITE` | `theme.graphite` |
| `CHARCOAL` | `theme.charcoal` |
| `CHROME_BORDER` | `theme.hairline` |
| `TEXT_MUTED` | `theme.text_muted` |
| `MIST` | `theme.text` |
| `ASH` | `theme.text_subtle` |
| `ATTENTION` | `theme.attention` |
| local `ACCENT_BRIGHT` | `theme.accent_bright` |

### 3. `crates/anvil-render/src/statusbar.rs`

| Local const | Theme field |
|---|---|
| `CHARCOAL` | `theme.charcoal` |
| `CHROME_BORDER` | `theme.hairline` |
| `TEXT_MUTED` | `theme.text_muted` |
| `VERIFIED` | `theme.verified` |
| `FAILURE` | `theme.failure` |
| `AGENT_VIOLET` | `theme.agent` |
| `TEXT_SUBTLE` | `theme.text_subtle` |

`draw_status_bar` must take `&Theme` (currently doesn't).

### 4. `crates/anvil-render/src/cheatsheet.rs`

| Local const | Theme field |
|---|---|
| `CHARCOAL` | `theme.panel` |
| `CHROME_BORDER` | `theme.hairline` |
| `TEXT_MUTED` | `theme.text_muted` |
| `MIST` | `theme.text` |
| `TEXT_SUBTLE` | `theme.text_subtle` |

`draw` must take `&Theme`.

### 5. `crates/anvil-render/src/draw.rs`

| Local const | Theme field |
|---|---|
| `ACCENT_BRIGHT` | `theme.accent_bright` |
| `VERIFIED` | `theme.verified` |
| `FAILURE` | `theme.failure` |
| `ALLOY` | `theme.alloy` |
| `PANEL_RAISED` | `theme.panel_raised` |

### 6. `crates/anvil-config/src/lib.rs`
- `Config::default()` `theme`: `"mineral-dark"` → `"ember-dark"`.
- Add `toggle_theme: "cmd+shift+t"` keybinding.
- Update system-dark auto-mapping to ember variants.

## Revision 2 — user feedback (2026-05-24)

**User feedback**: "light mode is hard to distinguish" + "I want the orange as my highlight color when I drag across stuff" (selection).

### Light-mode token revisions

The first pass of light tokens didn't have enough separation between text-muted, text-subtle, and secondary accents. Tightened spread + darker secondaries:

| Token | v1 light | **v2 light** | Why |
|---|---|---|---|
| `text_muted` | `#6c6460` | `#52453c` | Pull deeper so it's clearly distinct from body text without becoming subtle. |
| `text_subtle` | `#9e9690` | `#8a7e72` | Warmer + slightly darker for readability on `#f4ede6`. |
| `border` | `#ccc4bb` | `#bdb0a0` | One step darker so 1px hairlines actually show. |
| `hairline` | `#d8d0c8` | `#cabea8` | Same. |
| `info` (teal) | `#296a70` | `#1f5e66` | Darker — was washing into text on `#f4ede6`. |
| `verified` (green) | `#2a7044` | `#1f5e36` | Same. |
| `attention` (amber) | `#875e10` | `#6f4d08` | Same. |
| `agent` (violet) | `#554a88` | `#443879` | Same. |
| `accent_primary` | `#b33f25` | `#a23718` | Slightly deeper ember for cursor + basin. |

### Syntax token revisions (light variant only)

The light syntax was too crowded around the same dark-warm hex range. Spread the lightness further so keyword/string/number/function/type/builtin all read as distinct:

| Light syntax | v1 | **v2** |
|---|---|---|
| `keyword` | `#a03520` | `#94300f` (deeper red-orange) |
| `string` | `#5c6a20` | `#4d5a12` (deeper olive) |
| `number` | `#9c6410` | `#7a4a04` (darker amber) |
| `function` | `#2a7a8a` | `#1c6776` (deeper teal) |
| `type` | `#8a6c28` | `#6b5018` (darker bronze) |
| `comment` | `#9a8c7a` | `#a89c8a` (slightly LIGHTER for italic readability) |
| `operator` | `#6c5e50` | `#5a4e40` |
| `punctuation` | `#8a7a6a` | `#7a6c5e` |
| `builtin` | `#6a4e86` | `#4f3576` (deeper violet) |

### Selection — ember orange, both variants

User's explicit ask: "i want the orange as my highlight color when i drag across stuff."

Selection treatment for items #5 in the polish batch:
- **Both variants**: selection background is `accent_ember` (`#c5462a`) at **22% alpha** — same hex, alpha gives it the wash without fighting the text underneath.
- Dark variant: `rgba(197, 70, 42, 0.22)` over `#1a1815`.
- Light variant: `rgba(197, 70, 42, 0.18)` over `#f4ede6` (slightly less alpha; ember reads more saturated against light bg).
- Selected text stays its normal foreground color — no fg override.
- Paint as ONE row-wide `fill_pixel_rect` per selected row (don't loop per cell).

### Selection-when-active vs system-wide selection

Anvil's drag-select highlight uses the same ember-22%-alpha treatment EVERYWHERE: terminal cells, HUD rows, palette items, cheatsheet rows. Consistent across the app.

### What the builder updates

In `crates/anvil-render/src/draw.rs::draw_cell`, the existing `if selection.contains(...)` branch that paints `mix(theme.background, theme.accent, 0.25)` becomes `theme.accent_ember` (alpha applied via the row-fill rect, NOT a mix). Selection paint moves OUT of `draw_cell` and BECOMES a per-row pre-pass before the cells render (so the alpha overlay sits BENEATH the text glyphs, not on top).

## Open question for librarian

BRAND.md says "ember means active execution only — do not make the brand orange." This spec contradicts that for the Anvil terminal app: ember orange becomes the primary accent in the desktop terminal. The librarian should record a decision in `wiki/decisions/` noting:
- BRAND.md applies to broader brand + Control Room surfaces.
- The Anvil terminal uses `ember-dark`/`ember-light` natively with ember orange as primary UI accent.
- Mineral teal is preserved as the `info`/`trace` semantic color.
