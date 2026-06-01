---
date: 2026-05-24
kind: routing-note
goal: Redesign right-docked HUD to match new chrome palette and DevOps + AI-native purpose
---

# HUD Redesign Routing

## Goal
Rebuild `draw_right_hud` so it (a) visually matches the graphite/charcoal chrome
just landed in tabbar/statusbar, and (b) surfaces what a DevOps engineer using
an AI-native console actually needs (k8s context, CI status, infra, agent state).

## Scope Boundaries
- In: `crates/anvil-render/src/agent_panel.rs::draw_right_hud`, possibly a new
  `crates/anvil-render/src/palette.rs`, possibly new worker modules under
  `crates/anvil-platform/` or a sibling crate for kubectl/gh polling.
- Out: chrome strips (done), `draw.rs` (item 8 builder still in flight),
  prompt/block rendering, Cmd+\ toggle behavior.

## Visual Constraints
- Reuse `GRAPHITE`, `CHARCOAL`, `CHROME_BORDER`, `TEXT_MUTED`, `ASH` from
  tabbar/statusbar — extract to `palette.rs` only if cleaner.
- Hairline + fixed-pixel-strip discipline (no cell-grid alignment).
- Status dots/badges follow BRAND semantic colors.

## Routing
1. [design] design-lead — Produce spec at `context/2026-05-24-hud-redesign.md`:
   section list (k8s ctx, CI, IaC drift, observability hooks, Caldera agent,
   repo/git, system) with order + rationale; palette token usage; spacing,
   typography, hairline rules, dot/badge conventions; one mockup at
   `docs/design/hud-mockup.html`. Depends on: none. Blocks: 2, 3.
2. [architecture-gate] systems-architect — Read design spec; decide whether new
   sections need background workers (kubectl-context poller, gh-CLI CI poller,
   terraform-state probe). If yes, design worker lifecycle + caching + crate
   placement. If no new data, skip and note "no architecture pass needed."
   Depends on: 1. Blocks: 3 only if workers required.
3. [implementation] builder — Implement design + any approved workers. Touches
   `agent_panel.rs::draw_right_hud`, possibly `palette.rs`, possibly new worker
   modules. Verify `cargo test --workspace` + `cargo clippy --workspace -- -D
   warnings`. Visual verification via `scripts/run.sh` + Cmd+\. Depends on: 1
   (always), 2 (if workers).

## Recommended Execution Order
1 (design-lead) → 2 (systems-architect, may be a no-op) → 3 (builder).
