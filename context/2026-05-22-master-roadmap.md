# Anvil Workspace Push — Master Roadmap

Date: 2026-05-22. Consolidates five architecture designs (workspace/splits,
IDE layout mode, nvim-RPC bridge, command blocks, agent surface) into one
phased plan with non-colliding identifiers.

## North star

Anvil becomes the single console for 100% of the user's work. End-state is a
fully native Zig app, including a native editor — reached incrementally.
Anvil is also the desktop UI for `caldera-os` (the local AI control plane);
it calls `caldera-local`'s HTTP API, never duplicating its runtime.

## Committed (this push)

| Commit | Summary |
|---|---|
| `8c65190` | perf(render): per-frame present mode + skip blink-hold redraws |
| `e12205c` | feat(ui): 21-item brand polish pass across all surfaces |
| `131909f` | test(harden): regression net — resize / frame cost / separators |
| `d8a295b` | feat(workspace): pane-tree pure logic (splits phase A1) |
| `c0955fd` | refactor(workspace): Tab owns a pane tree + registry (A2–A4) |
| `888f68b` | feat(workspace): multi-pane render path (A5) |
| `bb7db06` | feat(workspace): real splitting — split/close/resize/dividers (A6) |

Tests green, 345 and rising.

## In flight (background builders)

- **A7** — pane focus model (keyboard nav, focused-pane indicator). Completes splits.
- **CB1** — command-block data model in `terminal.zig` (`Block`, `blockAt`).

## Tracks and phases

Five tracks, each with a prefixed phase id so numbers never collide.

### Track A — Splits (workspace)
A1 pane-tree logic ✅ · A2–A4 Pane/registry refactor ✅ · A5 multi-pane render ✅
· A6 split/close/resize/dividers ✅ · **A7 focus model** (in flight) → splits complete.

### Track B — Command blocks
Interactive OSC 133 blocks: fold / jump / copy command output.
- **CB1** block model in `terminal.zig` (in flight)
- CB2 folded state + per-frame `FoldMap` (`pane.zig` / `fold_map.zig`)
- CB3 fold-aware `drawViewport` row loop
- CB4 affordance rendering (gutter exit marker, folded summary)
- CB5 interaction (gutter click, `⌘.`, `⌥`-click copy)
- CB6 live-block polish

### Track C — IDE layout mode
Dock chrome around the pane tree; `terminal | ide | codex` toggle.
- ID1 `docks.zig` geometry + `LayoutMode`
- ID2 context bar + status bar (real data only)
- ID3 Explorer (file tree + git badges) + Outline slot
- ID4 Run panel — **this is Track E's `agent_panel.zig` in docked placement**
- ID5 the `⌘⇧E` toggle + tab-strip restyle

### Track D — nvim-RPC bridge
Live editor chrome without a native editor: nvim `--listen` + msgpack-RPC.
- BR1 msgpack codec (`editor/nvim_rpc.zig`)
- BR2 Unix-socket transport + one-shot calls
- BR3 `EditorBridge` thread + `EditorSnapshot`
- BR4 "New Editor Pane" action + chrome wiring
- BR5 Outline LSP pull + debounce

### Track E — Agent surface (caldera-local-backed HUD)
The "way better HUD": one `agent_panel.zig`, floating in terminal mode,
docked in IDE mode. Surfaces caldera agent runs, approvals, findings.
- **AG1** "way better HUD" redesign — `agent_panel.zig`, honest empty state, no caldera dependency
- AG2 caldera-local client + poller (`caldera/client.zig`, `caldera/poller.zig`)
- AG3 interaction (approve / ack / start run)
- (AG-docked) IDE docked placement = Track C ID4

## Recommended build order

1. Land **A7** → splits complete. Commit.
2. Land **CB1**, reviewer pass (highest-risk model math). Commit.
3. **AG1** — the way-better HUD. High user value, explicitly requested, no deps.
4. **AG2** — caldera client; lights up AG1 with live agent data.
5. **CB2–CB6** — finish command blocks.
6. **ID1–ID3** — IDE chrome; ID4 reuses `agent_panel.zig` from AG1.
7. **ID5** — the layout toggle.
8. **AG3** — agent interaction.
9. **BR1–BR5** — nvim bridge (rung A of the editor path).

Rationale: splits first (in flight); the agent HUD next because the user
asked for it directly and it has no dependencies; command blocks are
independent; IDE chrome needs splits + the agent panel; the bridge needs the
IDE chrome to wrap.

## Decision-doc numbering (fix the `0004` collision)

The architects each proposed `wiki/decisions/0004-*`. Assign distinct numbers:
- `0004-workspace-splits.md`
- `0005-ide-layout-mode.md` (includes the nvim-RPC bridge)
- `0006-command-blocks.md`
- `0007-agent-surface.md`

`regression-harness-foundation.md` exists unnumbered — librarian to reconcile.

## Locked decisions

- IDE center pane = a terminal pane running Neovim/LazyVim. No native editor yet.
- Native Zig editor is the endgame (rung C); reach it via rung A (nvim in a
  pane + RPC) → rung B (nvim `--embed`, Anvil renders) → rung C.
- Agent data source is `caldera-local`; honest empty state when absent — never
  fabricated agent data.
- Per-frame zero-heap-allocation invariant (hardening) holds for every new
  render path.
