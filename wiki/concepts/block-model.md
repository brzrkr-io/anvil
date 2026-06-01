---
status: active
type: concept
created: 2026-05-24
updated: 2026-05-29
sources:
  - ../../src/vt/terminal.zig
  - ../../src/render/renderer.zig
  - ../../src/app.zig
confidence: high
---

# OSC 133 Block Model

Anvil uses OSC 133 semantic marks to divide terminal output into named "blocks"
— one per shell command. Each block has a visible header row, a body, and
state-driven visual treatment.

## Block Structure

A `Block` (in `src/vt/terminal.zig`) is derived from a pair of
`PromptMark` entries (CommandStart + CommandDone). Fields:

| Field | Source |
|-------|--------|
| `command_text` | content row at `PromptMark.col` (read by `read_command_text` at render time) |
| `command_start_col` | cursor column captured at the `133;C` mark |
| `duration_ms` | elapsed ms between `133;C` and `133;D` |
| `exit_code` | parsed from `133;D;<exit>` payload |
| `state` | `Running` / `Ok` / `Failed` derived from exit code + live `shell_running` |
| `diff_kind` | `DiffKind.none` or `DiffKind.unified` (detected at `133;D`) |
| `completed_at` | optional timestamp — set on `CommandDone` for the completion pulse |

The block list is rebuilt from `marks` on each render pass; it is not stored
persistently (marks are the source of truth).

## Visual Treatment

### Header Row

A synthesized row is prepended to each block's output region. `draw_block_header`
in `draw.rs` renders:

- Left accent bar: 2 px stripe in `theme.accent_primary` (Ok/running/failed
  uses `accent_bright` for running state per brand contract).
- Text: `command_text` + duration (`format_duration`) + exit symbol (`✓`/`✗`).
- Fold indicator `▾` on the right edge.

### Body Tint

Output rows get `PANEL_RAISED` background tint to visually group them under
their header.

### Running-Block Pulse (CB6)

While `block.state == Running`, `draw_block_header` paints a 2×2 pixel rect
centered in col 0 of the header row in `theme.accent_bright` with sine-modulated
alpha cycling 0.45–1.0 at 1.5 Hz.

The phase is driven by `App.running_pulse_phase: f32`, advanced
`+= 1.5 / 60.0` per tick only while any pane in the current tab has
`last_run().running == true`. Marks `dirty = true` while running; zero
cost at steady state.

The phase is threaded as a field through `draw_workspace` → `draw_viewport`
→ `CpuSink` / `GpuSink`.

### Completion Pulse (Item 23)

Distinct from the running pulse. On `CommandDone`, `Block.completed_at` is
set. For 200 ms after completion, `draw_block_header` paints a 2 px
ember-bright rect at the bottom edge of the header row with opacity
`sin(π * elapsed/200ms)`. GPU sink does not implement this (pixel rects are
CPU-only).

`app.zig` poll loop scans the focused pane for blocks completed within 250ms
and marks dirty to sustain frames during the pulse window.

### Diff Colorization

When `block.diff_kind == Unified`, `draw_viewport_into` paints a full-row
wash at 12% alpha: `theme.verified` (green) for `+` lines, `theme.failure`
(red) for `-` lines. Header lines (`+++`/`---`/`@@`) are excluded.

## Opt+Click Copy Block (CB5)

`Option+click` anywhere on a block's body rows copies the full block output
text to the system clipboard. The hit-test uses the block's content-row range
to determine which block was clicked. Source: `src/app.zig`
`anvil_mouse_event` handler checking the option modifier.

## Block-Scoped Search

`Cmd+Shift+F` opens the search bar in block scope: the query is restricted
to the OSC 133 block containing the cursor row. See [[search-system]] for
the `SearchScope::Block` implementation.

## Relationship to Shell Integration

Blocks exist only when shell integration is active (OSC 133 marks are being
emitted). Without marks, the terminal displays as a flat scrollback with no
block structure. See [[shell-integration]].
