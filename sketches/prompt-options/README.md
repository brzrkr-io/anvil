# Anvil Prompt Options

Visual companion for prompt iteration. Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/prompt-options/index.html
```

## Design stance

Twenty compact prompt directions that keep the Anvil prompt semantic, palette-native, and operator-grade. The prompt should not become decorative powerline chrome; glyphs and colors should encode meaningful shell, repo, agent, execution, and evidence state.

## Palette mapping

- Mineral / info / trace: normal ready prompt, provenance, focused operational state.
- Ember: active execution only.
- Verified green: evidence-backed checks only.
- Attention amber: reviewable dirty/stale/pending state.
- Failure red: failed command or invalid state.
- Agent violet: automation/model activity.

## My recommendation

Start from option 20, Adaptive Minimal:

- Default: bare mineral `❯`
- Failure: red `❯` with right-side exit code
- Dirty repo: amber `*` prefix only when needed
- Agent active: violet `◈` prefix
- Command running: ember `▶` or ember arrow only during execution
- Verification passed: green `✓` only for real check commands

This keeps the prompt quiet by default and makes it feel smarter when real state appears.

## Trade-offs

- Strongest minimal defaults: 01, 02, 20
- Strongest repo-aware states: 05, 06, 08
- Strongest agent-aware states: 09, 11, 12
- Strongest verification states: 13, 15, 16
- Most visually distinctive: 04, 11, 19
- Riskiest/noisiest: 07, 08, 18 if shown too often
