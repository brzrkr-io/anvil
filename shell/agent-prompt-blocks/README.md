# Anvil Agent Prompt Blocks

Alternative visual direction after rejecting the heavy box-draw frame.

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/agent-prompt-blocks/index.html
```

## Direction

Less frame, more bars/blocks/rails.

- regular prompt stays left
- activated agent prompt/telemetry moves to the right
- block meters replace the box frame
- wildness comes from context bars, memory blocks, route blocks, forge bars
- semantic colors remain brand-correct

## Best candidates

- 01 Right Agent Rail
- 02 Horizontal Telemetry Bars
- 03 Context Block Stack
- 10 Prompt Duet
- 12 Recommended: Block Rail Duet
- 18 Best Flow: Block Rail Duet

## Recommendation

Use `block-rail-duet` as the next concept:

```text
~/caldera/anvil                                      [hermes] ◈ ›
› cargo test                         ctx 73% · tok 141k/192k · tools gated
▰▰▰▰▰▰▱▱
mem ▣▣□▣□▣▣□                         repo anvil · evidence on · peer none
```

It keeps the shell prompt and agent prompt as a left/right pair, removes the frame, and preserves the dense/wild agent aesthetic through blocks and bars.

## Config sketch

```toml
[prompt.shell]
skin = "thin-trace-arrow"
glyph = "›"

[prompt.agent]
skin = "block-rail-duet"
placement = "right"
sigil = "◈"
show_tokens = true
show_context_percent = true
meter_style = "blocks"
frame = false

[prompt.agent.states]
execution = "forge-bars"
approval = "approval-bars"
memory = "context-blocks"
route = "route-blocks"
```
