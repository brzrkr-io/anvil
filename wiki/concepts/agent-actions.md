---
status: active
type: concept
created: 2026-05-24
updated: 2026-05-29
sources:
  - ../../src/caldera.zig
  - ../../src/app.zig
confidence: high
---

# Agent Actions and Keybindings (AG3)

Anvil exposes keyboard shortcuts for common agent lifecycle operations that
post directly to the caldera-local HTTP API via `anvil-caldera`.

## Shipped Actions (AG3, commit fd06814)

| Keybind | Action | HTTP endpoint |
|---------|--------|---------------|
| `Cmd+Return` | Approve the pending agent action | `POST /approve` |
| `Cmd+Shift+Return` | Start a new agent run | `POST /start_run` |

Both are wired in `src/app.zig` key-event dispatch and
call helpers in `src/caldera.zig`:

- `approve()` — fires a fire-and-forget POST to `/approve`. Response
  is logged but not surfaced to the UI.
- `start_run()` — fires a fire-and-forget POST to `/start_run`.

The Caldera client is held in `App` state in `src/app.zig` via `src/caldera.zig`.

## `ack_finding`

`actions.rs` also exports `ack_finding(client, id)` but this is not yet wired
to a keybind. It is invoked from the HUD panel click handler when the user
clicks a finding row (low confidence — implementation not verified in this pass).

## Keybinding Configuration

`agent_approve` (default `cmd+return`) and `agent_start` (default
`cmd+shift+return`) are defined in the `Keybindings` config struct in
`src/config.zig`. Users can remap via their TOML config.

## Relationship to Caldera

The Caldera poller (2s poll interval, in `src/caldera.zig`) drives the agent
panel snapshot that the HUD displays. The action helpers are write-side
operations on the same HTTP API.

## Contradictions / Low-confidence

- `ack_finding` dispatch path (HUD click) is low confidence — not confirmed
  from code in this wiki pass.
- Keybind config promotion: planned but no timeline in the log.
