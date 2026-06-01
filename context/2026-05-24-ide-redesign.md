# IDE Layout Mode — Full Redesign

Date: 2026-05-24. Replaces ID1–ID4 size assumptions. This is the approved
design for the IDE mode rebuild. All geometry is at 1× (logical px); multiply
by `scale` for device pixels, matching `Docks::for_mode` convention.

---

## 1. Topology

One concrete layout. VS Code-style cockpit.

```
┌────────────────────────────────────────────────────────────────────────────┐
│  [tab strip — same as Terminal mode, no change]                            │ 28 px
├───────────────────────────────────────────────────────────────────────────-│
│  cwd · branch                              edit: foo.rs · kube · HEAD     │ 32 px  context bar
├──────────────┬─────────────────────────────────────────┬───────────────────┤
│              │                                         │                   │
│   EXPLORER   │           editor pane (nvim)            │   RIGHT PANEL     │
│   320 px     │           center, 70% of center h       │   380 px          │
│              │                                         │                   │
│   ──────────-│─────────────────────────────────────────│ [OUTLINE|HUD|AGT] │
│   OUTLINE    │         terminal dock                   │                   │
│   (collapses │         bottom, 30% of center h         │                   │
│   if empty)  │         [term tab strip]                │                   │
│              │                                         │                   │
├──────────────┴─────────────────────────────────────────┴───────────────────┤
│  status bar (existing, ~22 px)                                              │
└────────────────────────────────────────────────────────────────────────────┘
```

Five horizontal bands: tab strip (28 px) → context bar (32 px) → three-column
content → status bar (22 px). Center column splits vertically: editor top (70%),
terminal dock bottom (30%). Right panel is a tabbed stack.

---

## 2. Dock Sizing Rationale

**Left dock — 320 px.** The current 260 px is too narrow for two indent levels
plus a 40-char path in IBM Plex Mono 12 pt at 2×. 320 px fits four indent
levels and ~36 chars before truncation — matching VS Code (240 px) and Zed
(300 px) scaled to a retina workflow. Fixed for v1; user-resizable deferred.

**Right panel — 380 px.** The existing HUD strip is ~240 px (hud_cols * cell_w
+ grid_pad). 380 px fits the Caldera HUD section content without wrapping agent
IDs and trace tokens. Wider than VS Code (300 px) because the HUD content is
denser than a typical sidebar. Fixed for v1.

**Center editor pane — remainder.** At 1440 px window width: 1440 − 320 − 380
= 740 px of center. A 740 × ~600 px nvim pane is usable for real code. At
minimums (left hidden, right hidden): full window minus status bars.

**Vertical split — 70/30.** Editor gets majority. 30% for the terminal dock at
1200 px window height (minus ~82 px chrome) = ~338 px, which is ≈ 14 terminal
rows. Enough for build output, not enough to compete with the editor visually.

Modern reference: VS Code default with terminal panel open ≈ 68/32. Zed ≈
70/30. 70/30 is the industry read-and-run ratio.

---

## 3. Editor Pane

**Stay empty until Cmd+E; do not auto-spawn.** Rationale: auto-spawn risks
spawning nvim in the wrong directory (cwd not yet resolved at mode-switch time)
and adds hidden PTY cost to a mode toggle. The existing BR4 Cmd+E path is
correct; it stays the entry point.

Empty state of the editor region when no editor pane exists: a plain
`anvil.charcoal` fill with a single centered line in `text_muted` —
`Press ⌘E to open editor` — in IBM Plex Mono. No decorative art.

When an editor pane is live, it occupies the full top-center region. It is
always the topmost pane in the center column's PaneTree split. v1 enforces one
editor pane (existing BR4 singleton invariant).

---

## 4. Terminal Panes in IDE Mode

Terminal panes move to the bottom dock (bottom 30% of center column). They keep
a compact tab strip of their own — identical to the existing tab bar in miniature
but scoped to the terminal dock only. Height: 22 px. Thin `anvil.ash` hairline
above it separates editor and terminal regions visually.

Implementation note: the center column's `pane_area` is still one `PaneTree`
rect. The 70/30 vertical split is an initial PaneTree split created by the mode
transition — a horizontal split with the editor pane at top (ratio 0.70) and
a terminal pane at bottom (ratio 0.30). The existing PaneTree horizontal split
already handles this; the mode transition just calls it automatically when
entering IDE mode from Terminal mode (if no split already exists) and moves the
focused terminal pane to the bottom slot.

If the user has no terminal pane open, the terminal dock shows the same
`anvil.charcoal` fill and `Press ⌘T to open terminal` copy.

---

## 5. File Explorer

Width: 320 px. Expands the current ID3 implementation.

Changes from ID3:
- **Git status badges.** One character wide, right-aligned in the row. `M`
  amber (`status.attention`), `A` teal (`status.info`), `?` alloy, `D` red
  (`status.failure`). Source: `git status --porcelain` via the existing
  `LocalContext` git poller extended with per-file status. This is additive
  to the ID3 snapshot — same thread, wider result.
- **Row height 22 px** (up from 20 px). More breathing room per the "useless
  and tiny" feedback.
- **Scrollable.** Vertical scroll via trackpad delta events; no scrollbar
  chrome. Track `scroll_offset_px: f64` on the left dock hit-test state.
- **Click-to-open.** File click sends `nvim_command(":e <path>")` via
  EditorBridge `nvim_exec_lua`. Deferred until BR4/BR5 are live; gate on
  `editor_bridge.connection == Live`.

Recursive expansion and the `fs_worker` thread model remain identical to ID3.

---

## 6. Outline

Occupies the bottom portion of the left dock. Dynamic height: collapses to a
22 px header-only row when `outline_state == NoServer` and expands to a
configured split once an LSP attaches. Default split: Explorer 60%, Outline
40% of left dock height (unchanged from ID3 proposal).

When `outline_state == NoServer`: header row only, label `OUTLINE`, sub-label
`No language server` in `text_muted`. No wasted blank space.

When `outline_state == Ready && outline is non-empty`: rows at 20 px with
kind tags (ASCII — `fn`, `cl`, `st`, `tr`, `en`, `fd`, `co`) in `text_muted`
and symbol name in `text` color. Click-to-jump via `nvim_win_set_cursor` RPC
— this is the one BR5 deferral to lift here since it's the reason to show the
outline at all.

Scrollable on overflow (same trackpad-delta mechanism as explorer).

---

## 7. Right Panel

**Tabbed stack with three tabs: HUD · OUTLINE · AGENT.**

- `HUD` — the existing Caldera HUD (`draw_right_hud`). Default visible tab.
- `OUTLINE` — mirrors the left-dock outline but larger. Reserved for a future
  "wide outline" view; v1 shows the same content as the left dock, same data.
  Deferred to post-BR5 — do not implement in the first build.
- `AGENT` — agent action surface (AG3 approve/start). Shows pending agent
  actions, current caldera session, and the approve/reject keybind reminder.

Tab strip at top of right panel: 22 px, `anvil.ash` background, active tab
underlined in `anvil.accent.mineral`. Tab labels in IBM Plex Mono caps. Thin
`anvil.ash` hairline separator below the tab strip.

v1 ships only HUD tab and AGENT tab. OUTLINE tab stub (grayed, no click).

---

## 8. Context Bar

Expand from 24 px to **32 px**. The extra 8 px gives the bar enough breathing
room to render IBM Plex Sans at 12 pt without cramping the ascenders.

Content unchanged from ID2/BR4: `cwd · branch` left, `edit: name[•] · kube ·
HEAD` right. Delimiter between sections is `·` in `text_subtle`.

One addition: when the editor pane has unsaved changes (`modified == true`),
the `•` indicator uses `status.attention` amber rather than `theme.accent`
teal. This matches the Mineral rule — amber for attention, teal for trace.

Background: `anvil.charcoal`. Bottom hairline: 1 px `theme.hairline`.

---

## 9. Tabstrip in IDE Mode

**Keep the existing tabstrip unchanged. No tab hiding.**

Rationale: the user may have multiple tabs each containing a different project.
Hiding tabs would lose the ability to switch projects. The tab strip is 28 px
and already compact. In IDE mode the tab refers to the entire pane layout for
that tab, not just the editor pane — consistent with Terminal mode semantics.

The only change: when a tab contains the active editor pane, prefix the tab
title with the buffer basename. Example: `main.rs — zsh` (buffer · shell). This
is the existing BR4 tab-title proposal, now approved.

---

## 10. Transitions

**Hard cut. No animation.**

Mode toggle (Cmd+Shift+E) computes new `Areas`, calls `resize_all_tabs`, and
redraws in one frame. The visual jump is intentional — IDE mode is a distinct
cockpit, not a panel that slides in.

On transition Terminal → IDE:
1. `Docks::for_mode(Ide, ...)` returns new left/right/top widths.
2. `resize_all_tabs` reflows all PTYs to the new pane area.
3. If PaneTree has exactly one pane (common case), Anvil inserts a horizontal
   split automatically: existing pane moves to bottom slot (terminal dock, 30%),
   a new empty pane occupies top slot (editor region, 70%). Editor pane is NOT
   auto-spawned (see §3).
4. If PaneTree already has a horizontal split, the top pane becomes the editor
   region and the layout rebalances to 70/30 if the ratio differs.

On transition IDE → Terminal:
1. Docks collapse. Pane area expands.
2. The horizontal split is preserved — user explicitly closes panes as needed.
   Do not auto-collapse the split on mode exit.

---

## 11. Out of Scope

- Multi-editor (split editor, two nvim panes side-by-side).
- Drag-resizable dock dividers.
- Persistent layout state across restarts.
- File rename, new file, delete from explorer.
- Keyboard navigation in the explorer or outline.
- FSEvents live file-watch (explorer refreshes on cwd change and expand only).
- Right-panel OUTLINE tab content (stub only in v1).

---

## 12. Open Questions

1. **Auto-split on IDE entry.** Should Anvil auto-insert the 70/30 horizontal
   split when entering IDE mode from a single-pane Terminal tab? Risk: destroys
   a carefully arranged multi-pane layout. Recommend: only auto-split if the
   current tab has exactly one pane; otherwise leave PaneTree untouched and
   let the user do `:vsplit` from nvim or Cmd+H from Anvil.

2. **Right panel default tab.** HUD as default makes sense for a DevOps user
   (operational state first). But if no Caldera session is active the HUD is
   empty; AGENT tab may be more useful. Recommend: HUD tab by default, show
   a `No active session` empty state in the HUD rather than auto-switching tabs.

3. **Left dock width at 320 px.** This is 60 px wider than the current 260 px.
   Verify it feels right at a standard 27" 5K display (5120 × 2880, HiDPI)
   before shipping.

4. **Terminal dock tab strip.** Should the terminal dock tab strip show all
   tabs or only terminal panes within the current IDE tab? Recommend: show only
   panes within the current tab's PaneTree bottom slot — consistent with how
   the main tab strip already scopes to tabs.

---

## Build order

1. Expand `Docks::for_mode(Ide)` to emit `left_w = 320 * scale`, `top_h = 32 * scale`. Update tests. — verify: `cargo test -p anvil-workspace` green.
2. Expand context bar height to 32 px; add `status.attention` color path for `modified` indicator.
3. Expand left dock row height to 22 px; add vertical scroll state + trackpad delta handling; add git-status badge column.
4. Outline click-to-jump: wire `nvim_win_set_cursor` call from left-dock hit-test when `editor_bridge.connection == Live`.
5. Right panel tab strip (HUD + AGENT tabs; OUTLINE stub).
6. Auto-split logic on Terminal → IDE transition (single-pane case only).

Verify after each step: `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` green.
