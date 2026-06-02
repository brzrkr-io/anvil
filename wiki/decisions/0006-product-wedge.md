---
status: active
type: decision
created: 2026-06-02
updated: 2026-06-02
sources: []
confidence: high
---

# 0006 — Product Wedge & Beat-Targets

Decision on who Anvil wins first and what it must be unambiguously better at.
Closes roadmap #50. Supersedes the "single console for 100% of work" north star
as the *near-term* focus — that remains the endgame, this is the wedge into it.

## Wedge user

**The solo DevOps / platform engineer** who owns CI + Kubernetes + IaC for a
team. Lives in terminal + kubectl all day; juggles flux, terragrunt, terraform,
helm, gh. This is the project owner's own role — so the design oracle is
in-house and feedback latency is zero.

Why this wedge over the alternatives:
- **vs on-call SRE** — stickier but needs metrics/dashboard depth Anvil lacks; a
  build, not a sharpen.
- **vs AI-first builder** — differentiated but unproven market + high trust bar.
- **vs generalist dev** — broadest market, weakest wedge; they won't leave VSCode.

## The promise

Collapse the kubectl / flux / terragrunt / gh context-switching that today
sprawls across many terminal tabs + a browser + VSCode into **one cockpit**.
Breadth is not the pitch — nobody switches consoles for "slightly better
everything." The pitch is three loops that are *measurably* faster here.

## The three beat-targets

Anvil must beat `terminal + VSCode + kubectl + browser` at exactly these. All
three already have surfaces — the work is sharpening (fewer keystrokes, zero
context switch, state visible without a command), not building from zero.

1. **GitOps reconcile loop** — flux drift / failed HelmRelease.
   - Today: `kubectl get` + `flux describe` + `kubectl logs` across 4 tabs.
   - Anvil: k8s page, live Flux status, one-click reconcile/suspend, logs→terminal.
   - Surface: `Kube.svelte`, `Flux.svelte`, `flux.rs`.

2. **IaC plan → apply** — terragrunt/terraform across mixed stack/run-all repos.
   - Today: cd around, remember run-all vs stack, squint at scrollback.
   - Anvil: kind-classified discovery, plan/apply streamed to terminal, kind badges.
   - Surface: `Terraform.svelte`, `iac.rs`.

3. **PR + CI triage** — "what's failing."
   - Today: browser for Actions + `gh` in terminal + VSCode for the diff.
   - Anvil: PR list, inline diff, approve/request-changes, re-run CI, pod logs.
   - Surface: `DevOps.svelte`, `ci.rs`.

The **approval-gated agent** (`run_capture` loop, hardened in #46) overlays all
three: investigate-then-propose, human approves each mutation.

## Success test

For each loop, the Anvil path must be fewer keystrokes / fewer context switches
than the terminal+browser path, and the relevant state must be visible without
typing a command. If a loop can't clear that bar, it's not a beat-target yet.

## What this de-prioritizes (for now)

Observability/metrics depth, generalist-dev editor polish, and net-new agent
autonomy beyond the gated loop. Revisit once the three beat-targets demonstrably
win.
