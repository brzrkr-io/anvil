---
status: active
type: concept
created: 2026-05-22
updated: 2026-05-29
sources: []
confidence: high
---

# Workspace Panes

How Anvil splits the terminal window into multiple panes, how they are
rendered, and how keyboard focus moves between them.

## Architecture

The pane system spans two modules and is wired in `src/app.zig`:

- `src/workspace/pane_tree.zig` — pure geometry, no AppKit/Metal/PTY. Owns the
  `PaneTree`, `PaneNode`, `Split`, `PaneId`, and `Rect` types.
- `src/session_manager.zig` — maps pane IDs to live sessions (Terminal + Pty).
- `src/render/renderer.zig` — `render()` lays out the tree and draws each pane's
  cells plus dividers and the focused-pane accent border.

## PaneTree

`PaneTree` is a binary tree of `PaneNode` (enum: `Leaf(PaneId)` or
`Split(Split)`). Every tab owns one tree. Invariants:

- The root is never null; there is always >= 1 leaf.
- Every `Split` has >= 2 children; `ratios` sums to 1.0.
- `tree.focused` always names a leaf present in the tree.

### Operations

| Function | Effect |
| --- | --- |
| `init_single(id)` | Create a one-leaf tree. |
| `split(dir, new)` | Split focused leaf (flat-split rule; sets `focused = new`). |
| `close_leaf(id)` | Remove a leaf, collapse single-child splits; returns next focus. |
| `layout(outer, div_px, out)` | Write `LayoutEntry{id, rect}` per leaf. |
| `hit_test(outer, div_px, px, py)` | Leaf id under a device-pixel point. |
| `neighbor(dir, outer, div_px)` | Nearest leaf in `dir` direction, or null at edge. |
| `leaf_count()` | Count of leaves. |

### Flat-split rule

When the focused leaf's immediate parent has the same `SplitDir` as the
requested split, the new pane is inserted as a sibling (no nesting). This
keeps the tree shallow and the geometry predictable.

## Layout

`layout` writes one `LayoutEntry{id, rect}` per leaf into a caller-owned
`Vec`. `rect` is in device pixels, y=0 at the top.

`divider_px = 8.0` — the gutter between adjacent panes. Dividers are drawn
by overdrawing pane content after all leaves are rendered (bleed guard).

## Focus Model (Phase 7)

`tree.focused` is the `PaneId` of the active pane. Only the focused pane
receives:
- Cursor rendering (`cursor_params` is `None` for unfocused panes in
  `draw_workspace`).
- PTY write (keystrokes route to `focused_pane().pty`).

### Click-to-focus

`on_mouse_down` calls `tree.hit_test` and sets `tree.focused = hit_id` when the
click lands in a different pane.

### Keyboard focus navigation

Four keybindings move focus to the geometric neighbor in each cardinal direction.
`tree.neighbor(dir, ...)` returns the nearest leaf in that direction (or null
at an edge). A null result is consumed with no effect.

Default chords (`cmd+shift+h/j/k/l` — vim-style, using `cmd` to avoid
sending terminal escape sequences to the shell):

| Action | Default chord |
| --- | --- |
| `focus_left` | `cmd+shift+h` |
| `focus_right` | `cmd+shift+l` |
| `focus_up` | `cmd+shift+k` |
| `focus_down` | `cmd+shift+j` |

Chord selection rationale: `ctrl+h/j/k/l` conflict with common terminal
sequences (backspace, newline, and VT control characters). `cmd+opt+h` is
intercepted by macOS as "Hide Others". `cmd+shift+h/j/k/l` have no system-level
or existing Anvil conflicts (verified against all `Keybindings` fields).

### Focused-pane visual indicator

`draw_dividers` draws a 2px accent border (`theme.accent` — Mineral teal
`#54b7c0` / `#286e76`) at the boundary of the focused pane's rect:
- Only when `entries.len >= 2` (single pane: no border — no chrome noise).
- Border sits in the divider gutter on gutter-adjacent sides, at the pane
  edge on window-boundary sides.
- Drawn after all pane content so it overdaws any cell bleed.

## Divider drag

`on_mouse_down` calls `find_divider_at` (slop = 4px around the 8px gutter).
On hit, a `DividerDrag` state is stored; `on_mouse_dragged` calls
`adjust_ratio` and reflows pane terminal sizes via SIGWINCH-suppressed
`terminal.resize`. `on_mouse_up` clears the drag state and sends SIGWINCH.

## Configuration

| Key | Default | Notes |
| --- | --- | --- |
| `split_right` | `cmd+d` | Horizontal split (new pane to the right). |
| `split_down` | `cmd+shift+d` | Vertical split (new pane below). |
| `close_pane` | `cmd+w` | Close focused pane (or tab if last pane). |
| `focus_left` | `cmd+shift+h` | Move focus left. |
| `focus_right` | `cmd+shift+l` | Move focus right. |
| `focus_up` | `cmd+shift+k` | Move focus up. |
| `focus_down` | `cmd+shift+j` | Move focus down. |

Max panes per tab: `MAX_PANES_PER_TAB = 8` (enforced in `split_focused_pane`).
