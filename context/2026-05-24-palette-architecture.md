---
title: Command Palette Architecture (Cmd+K)
date: 2026-05-24
status: design
owner: systems-architect
---

# Command Palette Architecture

## Stop-signal first: most of this already exists

`ui/palette/index.html`, `crates/anvil-platform/src/webview.rs`,
`crates/anvil-control/src/bridge.rs`, `crates/anvil-workspace/src/palette.rs`
already ship a working palette: WKWebView overlay, ready handshake, JS-side
fuzzy filter, `Outbound::{Show,Hide}` / `Inbound::{Ready,Invoke(id),Dismiss}`,
Cmd+K binding, `Action` enum dispatched in `main.rs::handle_palette_action`.

This design does **not** introduce `Outbound::ShowPalette` / `PaletteResults`
/ `Inbound::PaletteQuery` / `PaletteAction` / `PaletteClose`. The prompt's
proposed contract is more elaborate than what's there; matching the existing
contract is cheaper and follows "Surface Conflicts, Don't Average Them." The
work is **growing the catalog and feeding dynamic items**, not rebuilding the
bridge. If very large catalogs or telemetry-driven re-rank ever force
native-side ranking, revisit then.

## 1. Item universe (~30 items)

**Static actions** — extend `palette::CATALOG`:

- `tab.new`, `tab.close`, `tab.next`, `tab.prev`
- `pane.split.h`, `pane.split.v`, `pane.close`, `pane.focus.next`
- `theme.toggle` (in addition to existing `theme.dark` / `theme.light`)
- `layout.cycle`
- `fold.toggle`, `fold.all`, `fold.none`
- `search.open`, plus existing `hud.toggle`, `cheatsheet.show`
- `agent.start`, `agent.approve`, `agent.reject`

**Dynamic items** — built per-summon from app state:

- One per open tab: `id = "tab.switch:{idx}"`, subtitle = pane count or cwd.
- Last N (cap 10) recent commands from active pane's OSC 133 PromptStart
  marks — reuse the slicing in `main.rs:1383-1404`. `id = "recall:{block_id}"`.
- Agent items included only when `anvil_agent` reports a live session /
  pending step.

Total ~25–35 — well under the threshold where JS filtering shows lag.

## 2. Ranking algorithm

Keep the JS subsequence filter (`ui/palette/index.html` line 62) but add
**score and sort**:

```
score(needle, hay) = base(match_type)
                   + word_start_bonus
                   + consecutive_run_bonus
                   - distance_penalty
                   + recency_boost(item_id)   // dynamic items only
```

- Substring: +100. Word-start: +30. Subsequence: +0.
- +5 per consecutive matched-char run. -1 per gap char.
- `recency_boost`: +20 last invoked, +10 last-but-one; stored in
  `localStorage` (JS only, no native plumbing).

Justification: ~30 lines of JS, beats pure subsequence on common cases
(`th` → `theme.toggle`), avoids fzf's smith-waterman complexity for a
30-item universe. Revisit if users complain — do not preempt.

## 3. IPC contract — extend, do not replace

`Outbound::Show` already carries `commands: Vec<Command>`. Dynamic items
ride that vec. No new variants. The only convention: prefix routing in
the existing `id` string field:

```
tab.switch:{usize}   → SwitchTab(usize)
recall:{u64}         → Recall(block_id)
pane.split.h         → static action
```

`Inbound::Invoke(String)` already carries the id; native learns more
prefixes.

## 4. State machine

`anvil-workspace::palette::Palette` already implements:

```
Closed → (Cmd+K) → Opening → (on_ready) → Open
Open   → (Esc | Invoke | backdrop) → Closed
```

`Opening` ("summon while webview not ready") is real and tested. No
additions. Up/Down/Enter live entirely in the webview.

## 5. Native action dispatch — extend `Action`

Grow the existing enum (`crates/anvil-workspace/src/palette.rs`):

```rust
pub enum Action {
    // existing 9 variants...
    TabNew, TabClose, TabNext, TabPrev,
    PaneSplitH, PaneSplitV, PaneClose, PaneFocusNext,
    ThemeToggle, LayoutCycle,
    FoldToggle, FoldAll, FoldNone,
    SearchOpen,
    AgentStart, AgentApprove, AgentReject,
    SwitchTab(usize),
    Recall(u64),
}
```

`action_for_id` parses prefixes first, then falls back to CATALOG lookup.
`main.rs::handle_palette_action` gains one arm per variant, each
delegating to an existing method (`close_active_tab`,
`toggle_fold_at_viewport_top`, etc.). No new orchestration layer.

## 6. Webview lifecycle — one gap to plug

`crates/anvil-platform/src/webview.rs` handles transparent background,
show/hide, first-responder swap, exposes `set_frame`. **But `set_frame`
is never called after init** — on window resize the overlay keeps its
initial size. For the palette (full-window backdrop) the gap is visible.

Plug: in `main.rs` resize handler (already runs on `window_did_resize`),
call `webview.set_frame(new_w, new_h)`. ~3 lines. Existing API, no new
platform plumbing.

## 7. HTML reusable as-is, with three edits

`ui/palette/index.html` needs:

1. Replace filter (lines 62–70) with score+sort from §2.
2. Persist recency to `localStorage` on `invoke`.
3. (no-op) subtitle on the right is already wired.

No structural changes. No CSS overhaul. Theme tokens flow through
`msg.theme` on `show` — already there.

## 8. Out of scope

- Custom palette items via config (TOML).
- Plugin / dynamic action loading.
- Multi-step palettes ("close which tab?" → second list).
- Native-side ranking; fuzzy-match libraries.
- Icons in rows; keyboard-shortcut hints in rows (cheatsheet covers).

## 9. Open questions

1. **Recall semantics.** Pick `Recall(block_id)` → (a) type command into
   active prompt for editing, or (b) submit immediately? Bias: (a),
   matches shell history.
2. **Tab.switch order.** Most-recent-first or tabbar index order? Bias:
   tabbar order — muscle memory.
3. **Agent items absent vs greyed when no session.** Bias: absent.
4. **Recency persistence scope.** Per-process or `localStorage`? Bias:
   `localStorage` — already a webview.

## Build order

1. **Extend `Action` + `CATALOG` + `action_for_id` parser** in
   `anvil-workspace`. Wire each new arm in `handle_palette_action` to
   existing methods. → verify: `cargo test -p anvil-workspace` + manual
   Cmd+K invocation of each static action.
2. **Plug the resize gap.** Call `webview.set_frame` from
   `window_did_resize` in `main.rs`. → verify: resize window, summon
   palette, backdrop fills new bounds.
3. **Dynamic items.** Build tab and recall entries in
   `send_palette_show` before encoding `Outbound::Show`. → verify: open
   three tabs, Cmd+K, three `tab.switch:N` rows; run a command, Cmd+K,
   see `recall:` entry.
4. **Ranking upgrade in `ui/palette/index.html`.** Score + sort +
   localStorage recency. → verify: `th` → `theme.toggle` ranks first;
   invoke an item, reopen, it surfaces near the top.
5. **Agent items, conditionally.** Read `anvil_agent` state in
   `send_palette_show`; include `AgentStart/Approve/Reject` only when
   relevant. → verify: no session → items absent; pending approval →
   `agent.approve` appears.

Build order: 1. Action enum + dispatch 2. Resize plug 3. Dynamic items
4. Ranking 5. Agent items — verify after each.
