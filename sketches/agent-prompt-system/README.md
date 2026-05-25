# Anvil Agent Prompt System

Cohesive visual system based on the dense + wild prompt directions.

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/agent-prompt-system/index.html
```

## Recommendation

Use dense as the base language and wild as state-specific treatments.

- Shell prompt: `01 Thin Trace Arrow`
- Activated agent prompt: `operator-frame` hybrid of Recommended Agent Prompt + Box-Draw Frame
- Verbose mode: Full Header Agent
- Approval/scope: Bounded Lease
- Execution: Forge Runner
- Memory/context: Pixel Memory
- Multi-agent/Caldera: Route Trace
- Evidence: Evidence Complete
- Blocked/policy: Takeover Guard

## Shared grammar

- Same arrow everywhere: `›`
- Same agent sigil everywhere: `◈`
- Same bracket language: `[hermes]`, `[lease]`, `[forge]`, `[mem]`, `[route]`
- Same color semantics:
  - violet = agent identity
  - mineral = prompt arrow / trace
  - ember = active execution
  - amber = approval / attention
  - green = verified evidence
  - red = blocked / failure

## Product shape

```toml
[prompt.shell]
skin = "thin-trace-arrow"
glyph = "›"

[prompt.agent]
skin = "operator-frame"
agent = "hermes"
sigil = "◈"
arrow = "›"
show_tokens = true
show_context_percent = true
show_tools = "compact"

[prompt.agent.states]
execution = "forge-runner"
approval = "bounded-lease"
memory = "pixel-memory"
route = "route-trace"
evidence = "evidence-complete"
blocked = "takeover-guard"
```
