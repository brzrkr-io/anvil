# Anvil Agent Prompt — Bolt Mode

Cyberpunk/Hermes/Honcho-inspired prompt sketch that applies the corrected grammar:

the lightning bolt is the agent-mode prompt caret. Do not render the shell arrow in agent mode.

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/agent-prompt-cyberpunk-hermes/index.html
```

## Hard rule

Bad:

```text
⌁ › ask
⌁ ⚡ › ask
```

Good:

```text
⚡ ask
⌁ ⚡ ask
[⌁ ctx 73 · tok 141k] ⚡ ask
```

Meaning:

- `⌁` = agent signal / attached channel
- `⌙` = Anvil scoped/gated mode
- `⚡` = live agent prompt caret

## Recommended grammar

Shell mode keeps a normal shell prompt/arrow.

Agent mode replaces the arrow with the bolt:

```text
⌁ ⚡ ask agent
```

Expanded telemetry:

```text
[⌁ ctx ▮▮▮▮░░ 73 · tok 141k]
⚡ ask agent
```

Anvil gated/scoped state:

```text
[⌙ gated · repo:anvil]
⚡ confirm edit
```

## Best candidates

- `01 Pure Bolt` — if the pane already clearly says agent mode.
- `02 Hook Bolt` — best all-around default.
- `03 Bracketed Hook Bolt` — best bracket identity.
- `07 Tiny Bracket Meter` — best reference-meter version.
- `25 Hook + Bolt Family` — recommended implementation family.
- `28 Stateful Agent Prompt` — best Honcho-inspired long-running session variant.
