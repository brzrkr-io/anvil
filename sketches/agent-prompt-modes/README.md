# Anvil Agent Prompt Modes

Visual companion for the question: should Anvil have a separate agent prompt, or should the normal prompt signify agent state?

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/agent-prompt-modes/index.html
```

## Recommendation

Use a hybrid:

1. Keep option 2 / Thin Trace Arrow as the canonical prompt.
2. Signify lightweight agent state inline with the prompt.
3. Add a toggleable agent companion/drawer for richer Hermes + Caldera OS state.

This preserves the operator prompt as the trusted command surface while still giving Hermes/Caldera agent work a strong visual identity.

## Why not a totally separate permanent agent prompt?

A separate permanent agent prompt is clear, but it fragments the terminal mental model. It can make the operator wonder which prompt owns the shell. That is risky in a local-first enterprise control plane.

A toggleable companion works better:

- default prompt stays stable: `›`
- agent observing: violet `◇` or `◈`
- agent needs approval: amber `?`
- active execution: ember `▶`
- verified evidence: green `✓`
- policy/risk/failure: scoped amber/red labels

## Best options in this sketch

- 01 Thin Trace Arrow: base prompt
- 02 Observer Sigil: low-noise agent awareness
- 08 Collapsed Companion: shows the agent layer is summonable
- 09 Open Companion Drawer: best toggle model
- 13 Caldera Route Trace: best Caldera OS cross-agent visual
- 17 Collaboration Lock: useful when another Hermes agent owns files
- 20 Context Staleness: Honcho-inspired but honest about stale memory
- 24 Thin Arrow + Agent Companion: recommended hybrid

## Aesthetic source notes

This sketch uses:

- Anvil graphite/charcoal dark canvas
- Mineral trace/focus color
- Violet for Hermes/agent automation
- Ember only for active execution
- Amber/red/green only for semantic state
- Honcho-like dark grid, sparse terminal cards, pixel/circuit motifs, and code-console density

It intentionally does not copy Honcho branding, logo, wording, exact diagrams, or layout.
