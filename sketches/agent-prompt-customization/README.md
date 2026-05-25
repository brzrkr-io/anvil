# Anvil Agent Prompt Customization

Visual companion for the activated agent prompt, separate from the regular shell prompt.

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/agent-prompt-customization/index.html
```

## Direction

Keep the regular shell prompt as the clean option-01 Thin Trace Arrow:

```text
~/caldera/anvil
› cargo test
```

When agent mode is activated, swap to a visibly different agent prompt with bracket identity and telemetry:

```text
[hermes] ctx 73% · 141k/192k
anvil · context fresh · tools gated
◈ › make this prompt cooler
```

## What the gallery explores

30 activated-agent prompt variants with:

- bracket identity
- token counter
- context percentage
- model/backend label
- toolbelt/capability state
- policy/approval gates
- bounded execution lease
- evidence state
- Caldera route / peer agent state
- dashboard/control-plane sync
- compact/minimal vs dense hacker-console modes
- customizable skins/glyphs/telemetry slots

## Best options

- 02 Token Counter Rail: clean token/context telemetry.
- 03 Full Header Agent: dense operator prompt with model/tokens/tools.
- 06 Approval Gate: shows agent power without takeover risk.
- 18 Box-Draw Frame: strong hacker-console aesthetic.
- 22 Bounded Lease Prompt: best safety model for agent actions.
- 26 Compact Pro: daily-driver compact mode.
- 30 Recommended Agent Prompt: best default activated-agent prompt.

## Recommended product model

Two prompt modes:

1. Shell prompt:
   - `›`
   - Enter executes shell command.
   - Can be agent-aware, but remains human-owned.

2. Agent prompt:
   - `[hermes] ... ◈ ›`
   - Enter sends instruction to agent.
   - Agent can propose commands.
   - Shell execution remains explicit/approved.

Customization should be profile-based:

```toml
[prompt.shell]
skin = "thin-trace-arrow"
glyph = "›"
show_agent_awareness = true

[prompt.agent]
skin = "bracket-dense"
agent = "hermes"
glyph = "◈"
arrow = "›"
show_model = true
show_tokens = true
show_context_percent = true
show_tools = "compact"
show_policy = true
show_evidence = true
compact_below_width = 100
```
