# Changelog

## v0.1.0 — first release

Anvil is an AI-native macOS console for DevOps work: terminal, editor, git, and
the k8s / IaC / CI surfaces in one native window, with an approval-gated agent
that can drive them.

### The wedge: three loops, sharpened
- **GitOps reconcile** — the Kubernetes/Flux view sorts broken-first, shows the
  failure reason inline (no hover), auto-polls so a reconcile is watched to
  green, surfaces a failing-count badge on the rail, and offers one-click
  `flux events` diagnosis.
- **IaC plan/apply** — Terraform/Terragrunt discovery is kind-aware
  (unit / stack / run-all); plan results persist per stack as a drift badge
  (`+a ~c -d` / `✓`) so pending changes are visible without re-planning.
- **PR + CI triage** — the PR list rolls up CI checks and sorts failing-first
  with a status dot + one-click re-run; a new GitHub **Actions** tab lists
  workflow runs failing-first with log / re-run.

### Agent-driven ops
- One-click **Investigate** on any failing resource (Flux reconcile, drifted
  plan, red PR/CI) seeds the agent with the real failure data and the right
  read-only diagnostic, runs it, and proposes a minimal fix you approve.
- **Apply-and-verify**: after an approved fix, the agent re-runs the diagnostic
  to confirm it's resolved.
- Every command and edit stays **approval-gated**; tool output is treated as
  untrusted (prompt-injection defense), with risky-command warnings on the
  approval card.

### Platform & distribution
- Signed + notarized release pipeline; **auto-update** end-to-end (quiet check
  after launch + on-demand "Check for Updates", signature-verified install).
- Multi-monitor: a window restored off-screen is recentered.
- Native macOS menu bar; per-window session isolation; crash-safe session
  restore; window size/position persists across relaunches.

### Security
- Locked CSP (local origins only), scoped capabilities, no shell plugin, command
  verbs allow-listed. See `wiki/concepts/security-boundary.md`.

### Quality
- 545 unit tests; coverage gated at 90% lines/statements; Playwright e2e in CI.
