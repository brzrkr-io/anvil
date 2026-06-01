---
status: active
type: concept
created: 2026-05-24
updated: 2026-05-29
sources:
  - ../../src/app.zig
  - ../../src/workspace/pane_tree.zig
  - ../../src/render/renderer.zig
confidence: high
---

# Layout Modes, Docks, and Areas

Anvil supports two named layout modes that control which chrome docks are
visible and how the pane area is carved from the window. All geometry lives in
`src/workspace/pane_tree.zig`.

`Codex` was removed (2026-05-24). It was never more than a placeholder identical
to Terminal at runtime and is no longer referenced anywhere.

## LayoutMode

```zig
const LayoutMode = enum { terminal, ide };
```

- `Terminal` — full-window pane area, no side docks, no top bar. The default.
- `Ide` — right HUD dock (`right_w = 280 pt`), top context bar (`top_h = 24 pt`),
  no left dock.

`App.layout_mode` (default `Terminal`) can be overridden at startup with
`ANVIL_LAYOUT_MODE=ide|terminal`. Unknown values fall back to `Terminal`.

## Docks and DockMetrics

`DockMetrics` holds physical pixel dimensions (already DPI-scaled):

| Field | Terminal | Ide |
|-------|----------|-----|
| `left_w` | 0 | 0 |
| `right_w` | 0 | 280 × scale |
| `top_h` | 0 | 24 × scale |
| `bottom_h` | 0 | 0 |

`Docks::for_mode(mode, scale)` returns the appropriate `DockMetrics`.
`Docks::compute_areas(window_rect, metrics)` returns an `Areas` struct.

## Areas

`Areas` partitions the window rect into non-overlapping regions:

- `pane_area` — the rectangle given to `PaneTree` for terminal content.
- `right_dock` — the HUD panel surface.
- `top_bar` — the context bar surface (zero-height when `top_h == 0`).

The 9 unit tests in `mode.rs` verify width sums, height sums, non-overlap,
zero-width Terminal docks, scale linearity, and round-trip.

In `app.zig`, the window inner rect and pane area rect are computed from dock
metrics before being passed to layout consumers (resize, event hit-test,
divider drag, focus).

## Context Bar (ID2)

When `layout_mode == .ide`, `app.zig` calls the context bar render path in
`src/render/renderer.zig` with `areas.top_bar` as the target rect.

The bar renders:
- Charcoal background + 1 px hairline bottom border.
- Left section: project icon, cwd basename, branch name in `theme.accent` /
  `theme.text_subtle`. All omitted when data absent.
- Right section: kube cluster, `head_short` commit hash. Omitted when absent.

Source: [[shell-integration]] (OSC 7 provides the cwd; `LocalContext` provides
git/kube data from the HUD poller).

## Mode Cycle (ID5)

`Cmd+Shift+E` toggles `terminal → ide → terminal`. The keybind is hardcoded in
`app.zig`; no config key yet. Each cycle calls `toggle_hud()` to expand/contract
the window width by `HUD_WIDTH_PT`, then `resize_all_tabs` to reflow the pane area.

## Contradictions / Low-confidence

- The `Cmd+Shift+E` keybind is not yet in `Keybindings` config; it is
  hardcoded. A config key is expected before the next release.
