# ID1 — Docks Geometry + LayoutMode (Rust port)

Date: 2026-05-24. Track C, phase ID1. Geometric foundation for ID2–ID5 and
the docked agent panel (ID4 = AG-docked). Pure geometry — no chrome content
ships in ID1.

## 1. `LayoutMode` enum

```
pub enum LayoutMode { Terminal, Ide, Codex }
```

- **Lives in** `anvil-workspace` (new `mode.rs`). Geometry concern; the crate
  already owns `Rect`/`PaneTree` and is depended on by both `anvil-render` and
  `anvil`. Keeps dock math free of render/AppKit types.
- **Default**: `Terminal`. Matches today exactly.
- **Persistence**: in-memory on `App` for ID1. Not per-tab. Not in TOML. ID5
  wires `⌘⇧E`; ID1 only ships the data type and the geometry it drives.

## 2. `Docks` struct

```
pub struct Docks {
    pub left_w:   f64,  // device px; 0 in Terminal mode
    pub right_w:  f64,  // device px; 0 in Terminal mode (HUD-floating)
    pub top_h:    f64,  // 0 until ID2
    pub bottom_h: f64,  // existing status bar; ID1 reads it
}
```

- **Lives in** `anvil-workspace::mode` alongside `LayoutMode`.
- **Owner**: derived per-frame in `anvil/src/main.rs` from
  `(LayoutMode, scale, font metrics)`. Not stored — keeps resize correct
  without an explicit invalidate step.
- Inert (rects/widths only). Policy lives in one function:
  `Docks::for_mode(mode, scale, &metrics)`.

## 3. Rect math

Given `inner: Rect` (window content minus OS title strip and bottom status
bar — see §5), `Docks::compute_areas(inner) -> Areas`:

```
pub struct Areas { left_dock, top_bar, pane_area, right_dock, bottom_bar }
```

Single-source-of-truth formula:

```
top_bar    = (inner.x,                     inner.y,                  inner.w,                     top_h)
left_dock  = (inner.x,                     inner.y + top_h,          left_w,                      inner.h - top_h - bottom_h)
right_dock = (inner.x + inner.w - right_w, inner.y + top_h,          right_w,                     inner.h - top_h - bottom_h)
pane_area  = (inner.x + left_w,            inner.y + top_h,          inner.w - left_w - right_w,  inner.h - top_h - bottom_h)
bottom_bar = (inner.x,                     inner.y+inner.h-bottom_h, inner.w,                     bottom_h)
```

Mode → widths (ID1 defaults, no user resize yet):

| Mode      | left_w        | right_w                                          | top_h |
|-----------|---------------|--------------------------------------------------|-------|
| Terminal  | 0             | 0 (HUD floats via existing `hud_visible` carve-out) | 0  |
| Ide       | 260 * scale   | hud_cols * cell_w + cell_w (matches today's AG1) | 0 (ID2) |
| Codex     | 0             | hud_cols * cell_w (wider; TBD)                   | 0     |

Zero-width docks (`Terminal`): rects collapse to `w = 0`; draw code is
gated on `rect.w > 0`. `pane_area.{w,h}` clamp to `>= cell_w/cell_h`
(same reason the existing `inner_rect` clamps).

## 4. Resize behaviour

- **Min / default / max** for `left_w` in device px: `min = 160*scale`,
  `default = 260*scale`, `max = 480*scale`. `right_w` matches the existing
  AG1 column math so AG1 sizing is untouched.
- **User-resizable: NO in ID1.** Drag-resize needs a divider hit-test + drag
  state machine; until ID2/ID3 ship the docks have nothing in them. Fixed
  widths first lets ID2/ID3 ship without geometry churn. Revisit post-ID5.

## 5. Integration with existing `PaneTree`

**No `PaneTree` changes.** It already takes an arbitrary `outer: Rect`. ID1
only changes the rect we pass:

- Today: `App::inner_rect()` returns the pane-area rect directly — it
  subtracts top chrome, bottom status bar, and the AG1 HUD strip.
- After ID1: rename `inner_rect → window_inner`; it returns the rect *before*
  dock subtraction (window content minus top-strip/bottom-status only). The
  per-frame call site computes:

  ```
  let docks  = Docks::for_mode(self.layout_mode, self.window_scale, &self.font.metrics);
  let areas  = docks.compute_areas(self.window_inner());
  // pane_area replaces every prior use of inner_rect() in workspace draw paths
  ```

- Two call sites in `main.rs` switch to `areas.pane_area`: `resize_all_tabs`
  (~line 625) and the `draw_workspace` invocation (~line 1550).
- The AG1 HUD's `right_margin / right_gutter` carve-out moves out of
  `inner_rect` into `Docks::for_mode` as `right_w` when HUD-visible / IDE
  mode. Net pixel layout in Terminal mode is unchanged.

## 6. Mode transition invariants

1. `PaneTree` topology, focus, ratios preserved verbatim. Mode change resizes
   panes; never splits, closes, or rebalances.
2. After new `Areas` is computed, `resize_all_tabs` runs once (same path as a
   window resize) so every `Terminal`'s `cols/rows` matches `pane_area`.
   Without this, PTY dimensions drift.
3. HUD visibility (`hud_visible`) is independent of `LayoutMode` in ID1.
   Terminal keeps the `⌘\` HUD toggle; IDE/Codex force HUD-as-right-dock
   visible. ID5 may collapse these; ID1 doesn't.
4. No animation. A mode switch is a single-frame layout change.

## 7. Agent panel reuse

In IDE mode the right dock **is** the AG1 HUD strip. Concretely:

- `Docks::for_mode(Ide, ...).right_w = hud_cols * cell_w + cell_w`. No new panel.
- `agent_panel::draw_hud` keeps its signature; it already paints a fixed-
  width right-anchored strip. ID1 changes only the visibility predicate
  driving the carve-out: `mode == Ide || (mode == Terminal && hud_visible)`.

## 8. Out of scope for ID1

- ID2 — top context bar + status bar refresh. `top_h = 0`; status bar untouched.
- ID3 — left-dock content (explorer/outline). ID1 ships an empty
  theme-background rect of the right width.
- ID4 — Run panel content. AG1 already paints; ID1 reroutes its geometry
  from floating HUD column to right dock column in IDE mode.
- ID5 — `⌘⇧E` toggle, tab-strip restyle, mode persistence.
- Tabstrip / palette geometry. They sit above `window_inner`, unaffected.
- User-resizable dividers between dock and pane area.

## 9. Open questions

1. **Default left-dock width.** Proposing **260 device px at 1×** — fits two
   indent levels + a 32-char filename in IBM Plex Mono. User decision.
2. **Persistence across restarts.** `config.toml` startup-only key, or
   session-only? Proposing **session-only for ID1**, revisit in ID5.
3. **Codex mode shape.** Roadmap: "TBD, possibly agent-centric". ID1 ships
   the variant with a no-op layout (= Terminal) so we don't commit early.
4. **HUD toggle in IDE mode.** Should `⌘\` still hide the right dock?
   Proposing **locked-visible**: use `⌘⇧E` to leave IDE if you want it gone.

## Verification plan

- Unit tests in `anvil-workspace::mode`:
  - `compute_areas` sums: `left_dock.w + pane_area.w + right_dock.w == inner.w` for every mode.
  - Pairwise non-overlap of `Areas` rects.
  - Terminal mode: `pane_area == inner` minus `bottom_bar`.
  - Widths scale linearly with `scale`.
  - Round-trip `Terminal → Ide → Terminal` returns identical `Areas`.
- Integration: after a mode switch, `resize_all_tabs` yields
  `cols >= 1, rows >= 1` for every pane at a minimum-sized window.
- Manual: force `layout_mode = Ide` at startup; panes shrink by
  `left_w + right_w` and AG1 paints in the right strip exactly where it does today.

Build order:
1. `anvil-workspace::mode` — `LayoutMode`, `Docks`, `Areas`, `for_mode`,
   `compute_areas` + unit tests. — verify: `cargo test -p anvil-workspace` green.
2. `main.rs` rename `inner_rect → window_inner`; add
   `App::layout_mode: LayoutMode` (default `Terminal`); compute `Areas` once
   per frame; replace every `inner_rect()` call feeding pane-tree code with
   `areas.pane_area`. — verify: `cargo test --workspace` green; app launches
   in Terminal mode with byte-identical pane layout.
3. Move AG1 right-strip carve-out from `inner_rect` into `Docks::for_mode`
   (`right_w` when `hud_visible || mode == Ide`). — verify: HUD toggle still
   works, no pixel drift in Terminal mode; forcing `layout_mode = Ide`
   reserves the strip even with `hud_visible = false`.
4. Add debug env-var `ANVIL_LAYOUT_MODE=ide` so ID2/ID3 builders can develop
   against IDE mode without ID5's toggle. — verify: with `ide`, the left
   dock paints a 260px theme-background strip and pane area shrinks by both
   dock widths.

— verify after each.
