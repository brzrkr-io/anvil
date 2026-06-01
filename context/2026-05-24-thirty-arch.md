# 30-Item Architect Pre-Pass (A1)

Four design notes for items routed to `systems-architect` before builder dispatch.

## A1.1 — Item 1: GPU render path default

Current: `let use_gpu_render = matches!(std::env::var("ANVIL_RENDER").as_deref(), Ok("gpu"));` (`main.rs:3299`). `AtlasPainter` is built only when set (`main.rs:3508`), with CPU fallback already wired on Metal-device failure (`main.rs:3517-3524`).

**Decision: NO — keep CPU default; flip in W2 after a parity audit.** Fallback mechanics are sound, but `draw_viewport_gpu` is not feature-equivalent to CPU. The risk of regressing a default install outweighs the perf upside.

Prerequisite verifications (W2 acceptance gates):
1. Cursor rendering (block/beam/underline) matches CPU pixel-for-pixel on the focused pane.
2. Selection alpha (`theme.selection`) renders identically through atlas composite.
3. Block accent strip and header overlay are still painted. Today GPU path only draws cells + dividers (`main.rs:1452`).
4. `ANVIL_RENDER_DEBUG` overlay still functions or is explicitly disabled.
5. No CPU-only callsites remain inside the per-pane loop at `main.rs:1595-1654`.

Migration: change the gate to `Ok("cpu")` opt-out. Keep the runtime fallback. Do not remove the CPU path — it stays the fallback and test substrate.

- Decision: keep CPU default; flip post-parity audit in W2.
- Files touched: `crates/anvil/src/main.rs:3299`.
- Risks: visual regression on default installs; Metal init cost at startup (already paid via fallback).
- Builder note: gate flip is one line; the work is the audit. Add an integration test covering cursor/selection pixels on both paths first.

## A1.2 — Item 13: Cmd+K palette (new IPC contract)

Current: palette IPC is `Outbound::Show { commands, theme }` and `Inbound::Ready|Dismiss|Invoke(id)` (`crates/anvil-control/src/bridge.rs`). Cmd+P ships a static list (`main.rs:1684`); webview filters locally.

For corpora that span subsystems (commands, agents, kube, files), native-side ranking is the right boundary.

**Decision: extend the existing palette webview; do not add a second HTML surface.**

New variants (additive, `"type"` discriminator):
- `Outbound::ShowPalette { corpora: Vec<String>, theme }` — opens in Cmd+K mode.
- `Outbound::PaletteResults { items: Vec<PaletteItem> }` where `PaletteItem { id, title, subtitle, kind }`.
- `Inbound::PaletteQuery { corpora: Vec<String>, query: String }` — debounced on web side.
- `Inbound::PaletteAction { kind, id }` — typed replacement for `Invoke` (keep `Invoke` for Cmd+P back-compat).

Corpora (all live in `App`, no new state):
- `command` — last N entries from focused `Terminal` via `block_at` + `command_line`.
- `agent` — `App.agent_snap` Live tasks.
- `kube` — `App.local_ctx.kube_context`; log-only stub for now.
- `file` — `App.local_ctx.recent_files`.

Dispatch in `main.rs::webview_message` alongside the `Inbound::Invoke` arm (`main.rs:2914`):
- `command` → write text to focused PTY.
- `agent` → focus agent panel, select task.
- `kube` → `eprintln!` stub.
- `file` → existing open path.

Ranking: substring + start-of-word boost, in a new `crates/anvil/src/palette.rs`. No new crate, no fuzzy dep.

- Decision: extend palette; native-side ranking; new `palette.rs`.
- Files touched: `crates/anvil-control/src/bridge.rs`, `crates/anvil/src/main.rs`, `crates/anvil/src/palette.rs` (new), `ui/palette/index.html`.
- Risks: single-instance webview — Cmd+P and Cmd+K can't coexist; verify dismissal resets corpora.
- Builder note: variants are additive; keep `Invoke`. Substring + word-boundary score, no external crate.

## A1.3 — Item 17: Right-click context menu

Current: `AppHandler::mouse_down` is left-only (`appkit.rs:110`). No `rightMouseDown:` selector. No NSMenu anywhere.

**Decision: native AppKit NSMenu via a new `AppHandler::context_menu_request` callback.** Webview overlay is wrong for point-spawn menus — positioning, dismissal, event capture are exactly what AppKit gives for free.

Design:
- Add `rightMouseDown:` selector in `appkit.rs`. Decode event, call `context_menu_request(loc, mods, view_bounds) -> Vec<MenuItem>`.
- New `MenuItem { id: String, title: String, enabled: bool }`. Platform constructs `NSMenu` from the items, pops via `popUpMenuPositioningItem:atLocation:inView:`, routes selection through an Objective-C action target into `AppHandler::menu_invoke(id)`.
- Hit-test in `App::context_menu_request`: map `loc` → `(pane_id, absolute_row)` via existing pane layout, then `Terminal::block_at(abs_row)`. `None` → empty Vec → no menu.
- Items: `block.copy_command`, `block.copy_output`, `block.rerun`, `block.fold` (reuses item #11). Omit `block.delete` — needs new `Terminal` API; flag separately.
- Block identity: `command_line: usize`. Capture it in the action closure; do not encode it in the menu item id string.

- Decision: native NSMenu via `context_menu_request` AppHandler hook.
- Files touched: `crates/anvil-platform/src/appkit.rs`, `crates/anvil/src/main.rs`.
- Risks: AppHandler trait expansion ripples to test stubs; NSMenu action target lifetime needs the existing `HandlerPtr` pattern (`appkit.rs:260`).
- Builder note: omit `block.delete` from first menu. Reuse `HandlerPtr` for the menu target.

## A1.4 — Item 30: Active-pane glow

Current: `draw_workspace` calls `draw_dividers` last (`workspace.rs:122`). `draw_dividers` explicitly ignores `focused_id` (`workspace.rs:161`); test `focused_pane_has_no_accent_border` asserts no accent today.

**Decision: 1px inset border on the focused pane, painted in `draw_dividers` after the gutter fills, using `theme.accent` solid.** No new draw pass.

Design:
- After gutter fills, find `e.id == focused_id` and paint four 1px rects inset by 1px on each edge of `e.rect` via `fill_pixel_rect` (already used; widened by item #10).
- Color: `theme.accent` solid. Matches the cursor's focus signal — one design language. Alpha tuning is a builder call at visual review.
- Divider hit-zone interaction (#15): the border lives inside `e.rect`; the divider hit zone lives in the gutter. No overlap. Verify with a test asserting no accent pixel in gutter coords for a two-pane layout.
- Dirty-row impact: only the focused pane's 4 edge rows on focus change. `focus_neighbor` already triggers `force_full_redraw`. Steady-state cost: zero.
- Flip the existing test → `focused_pane_has_accent_border`, asserting accent at expected inset coords and absence on non-focused entries.

- Decision: 1px solid `theme.accent` inset border in `draw_dividers`; flip the parity test.
- Files touched: `crates/anvil-render/src/workspace.rs`.
- Risks: visual conflict with header strip on the focused pane's top edge; inspect at visual review.
- Builder note: paint after dividers so the border sits over gutter bleed; flip the existing test.
