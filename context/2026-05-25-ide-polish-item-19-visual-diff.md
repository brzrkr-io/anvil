---
date: 2026-05-25
kind: design-spec
status: live
goal: visual diff against docs/design/layout-mockups.html Option A; delta spec for items 11, 13, 14
---

# IDE Chrome Visual Diff — Option A Target vs. Shipped State

All token names reference `crates/anvil-theme/src/theme.rs`. "Option A" is the
`.chrome-tab-row` section of `docs/design/layout-mockups.html`.

---

## Delta 1 — Context bar has a raw-hex background and ember tint wash

**Priority:** P0
**Closes:** Item 11 (editor chrome tighten)

**What the mockup shows.** Option A has exactly ONE chrome row (`chrome-tab-row`, 36pt, `background: graphite`). There is no second header bar above or below the tab row.

**What the code currently does.** In IDE mode, `draw_context_bar` (`crates/anvil-render/src/context_bar.rs:55–57`) paints an additional strip with a raw hex fill `[0x0f, 0x0d, 0x0b]` (an off-palette near-black), overlaid with an `accent_ember` wash at α=0.025 and a bottom edge in `accent_bright` at α=0.22. This strip sits **above** the tab bar, injecting a second chrome row that doesn't exist in Option A and introduces ember tint into what should be a neutral graphite header.

**The fix.** The context bar background must be `theme.graphite`. Remove the raw hex literal and the `accent_ember` overlay entirely. Change line 55 to `theme.graphite`. Delete line 56 (the ember wash). The bottom edge hairline (line 57) should become `theme.hairline` at full opacity (1.0), not `accent_bright` at 0.22.

```rust
// context_bar.rs line 55–57 — current
raster.fill_pixel_rect(bx, by, bar_w, bar_h, [0x0f, 0x0d, 0x0b]);
raster.fill_pixel_rect_alpha(bx, by, bar_w, bar_h, theme.accent_ember, 0.025);
raster.fill_pixel_rect_alpha(bx, by + bar_h - 1.0, bar_w, 1.0, theme.accent_bright, 0.22);

// After fix
raster.fill_pixel_rect(bx, by, bar_w, bar_h, theme.graphite);
raster.fill_pixel_rect(bx, by + bar_h - 1.0, bar_w, 1.0, theme.hairline);
```

---

## Delta 2 — Traffic lights are drawn with raw RGB in the context bar, not via macOS system

**Priority:** P0
**Closes:** Item 11 (editor chrome tighten)

**What the mockup shows.** Option A uses the native macOS window traffic lights (red/yellow/green dots rendered by AppKit at the OS level), with the tab row's content starting after a reserved zone of ~80pt.

**What the code currently does.** `context_bar.rs:63–66` manually paints three colored rectangles (`[0xe5, 0x6b, 0x5e]`, `[0xd6, 0x9a, 0x45]`, `[0x5b, 0xa9, 0x77]`) as 10×10 squares at 17pt spacing. These are raw hex values with no palette token, overlapping with the real macOS traffic lights which also render. The result is doubled/shifted traffic light rendering in IDE mode.

**The fix.** Remove the manual traffic-light draw loop from `draw_context_bar` (`context_bar.rs:61–67`). The reserved traffic-light zone is already handled by `TRAFFIC_LIGHT_RESERVE_PT = 80.0` in `tabbar.rs`. The context bar should start its content at `bx + 12.0 + 80.0 * scale` (matching the tab bar's `tl_reserve_px`), or simply reuse the existing `tl_reserve_px` math. Do not paint fake dots.

---

## Delta 3 — Active tab accent rule is 3px full-width; mockup shows 2px inset

**Priority:** P0
**Closes:** Item 14 (real tab / open-buffer UI for native editor)

**What the mockup shows.** Option A's `.tab.active::after` is `height: 2px`, `left: 4px`, `right: 4px` — a 2px rule inset 4pt from each edge of the tab, in `accent` (the primary mineral teal, not the bright variant).

**What the code currently does.** `tabbar.rs:175–176` paints `fill_pixel_rect(x, rule_y, tw, 3.0, theme.accent_bright)` — 3 device pixels, full width (no inset), using `accent_bright` (`#54b7c0`, the brighter/lighter teal). The accent rule is 50% thicker and edge-to-edge, making it a heavy band rather than a subtle selection indicator.

**The fix.** Change the rule to 2px, inset 4pt (scaled) from each side, and use `theme.accent_primary` instead of `theme.accent_bright`.

```rust
// tabbar.rs lines 175–176 — current
let rule_y = chrome_top_px - 4.0;
raster.fill_pixel_rect(x, rule_y, tw, 3.0, theme.accent_bright);

// After fix
let inset = 4.0 * window_scale;
let rule_y = chrome_top_px - 3.0;
raster.fill_pixel_rect(x + inset, rule_y, tw - 2.0 * inset, 2.0, theme.accent_primary);
```

Note: `window_scale` is already in scope at the call site (`tabbar.rs` receives it as a parameter).

---

## Delta 4 — Inactive tab right-edge hairline missing on final tab

**Priority:** P1
**Closes:** Item 13 (editor typography hierarchy — chrome quieter)

**What the mockup shows.** Option A's `.tab` (inactive) label color is `text-muted` (`#a1a4a9`). That token is correct in code. The structural gap is that adjacent inactive tabs share a single hairline (left of each), so the last tab before the `+` button has no right edge, bleeding into the right indicator area.

**The fix.** After the tab loop, draw a single right-edge hairline at `x` (the position after all tabs) if any inactive tabs were drawn:

```rust
// After tab loop in tabbar.rs (after line 258, before add-tab `+` draw)
if n > 0 && tabs.active != n - 1 {
    raster.fill_pixel_rect(x, 0.0, 1.0, chrome_top_px - 1.0, theme.hairline);
}
```

---

## Delta 5 — Gutter in editor pane uses `charcoal`; should be `graphite`

**Priority:** P1
**Closes:** Item 11 (editor chrome tighten)

**What the mockup shows.** Option A's terminal/editor content area background is `var(--graphite)` (`#0b0d0e`). The gutter (left number column) should be one step darker/quieter than the editor surface so it recedes, not advances.

**What the code currently does.** `editor.rs:93` fills the gutter with `theme.charcoal` (`#161a1c`). The editor surface (`editor.rs:92`) uses `theme.surface` (`#22262f`). Using `charcoal` for the gutter creates a visual tie between the tab strip and the gutter.

**The fix.** Use `theme.graphite` for the editor gutter background to create a clear three-level hierarchy: graphite gutter → surface editor content → charcoal active tab.

```rust
// editor.rs:93 — after fix
raster.fill_pixel_rect(rect.x, rect.y, gutter_w, rect.h, theme.graphite);
```

---

## Delta 6 — Status bar always shows agent dot; mockup does not

**Priority:** P2
**Closes:** Item 11 (secondary — status bar chrome)

**What the mockup shows.** Option A has no status bar at all. Option D (shipped choice) shows `● claude idle` only in the right section. Current code unconditionally renders "● idle" even when agent is `NotInstalled`.

**The fix.** Suppress the agent dot and "idle" label when `agent_snap.connection != Live`.

```rust
// statusbar.rs lines 103–108 — after fix
let agent_text = if agent_active && agent_snap.running_count > 0 {
    format!("\u{25cf} {} running", agent_snap.running_count)
} else if agent_active {
    "\u{25cf} idle".to_string()
} else {
    String::new()
};
```

Update tests `dot_always_present_when_disconnected` and `idle_muted` to assert no dot is emitted when `connection == NotInstalled`.

---

## Delta 7 — Context bar draws raw hex with `top_h: 0.0` (dead code path)

**Priority:** P2
**Closes:** Item 11 (cleanup)

**What the mockup shows.** Option A has `top_h = 0` in `Docks` for IDE mode (confirmed at `mode.rs:118`).

**What the code currently does.** `draw_context_bar` is called whenever `layout_mode == Ide`. With `top_h == 0.0` the function returns early at line 45. Code is dead but the raw hex literals in `context_bar.rs:55` are landmines for the moment `top_h` is set non-zero.

**The fix.** Covered by Delta 1 (replace raw hex with `theme.graphite`).

---

## New Item 21 — Light mode token parity

`MINERAL_LIGHT.graphite` and `MINERAL_LIGHT.background` are identical (`[0xee, 0xf1, 0xf2]`). In light mode the chrome strip and content area are indistinguishable. Candidate fix: `MINERAL_LIGHT.graphite = [0xe4, 0xe8, 0xea]` (step toward `mist`). Requires contrast check. Not blocking items 11/13/14.

---

## Hermes Items Closed by This Spec

| Item | Deltas |
|------|--------|
| 11 — Editor chrome tighten | 1, 2, 5, 6, 7 |
| 13 — Editor typography hierarchy | 4 |
| 14 — Real tab / open-buffer UI | 3 |
| 21 (new) — Light mode graphite/background token parity | separate |

---

## Open Questions

1. `window_scale` confirmed available in `draw_tab_bar` signature (param 8).
2. Delta 6 test rewrite: confirm "no dot when not connected" rule is intended.
3. Item 21: requires contrast check before committing a new value.
