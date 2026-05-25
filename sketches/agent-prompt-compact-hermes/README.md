# Anvil Compact Hermes-Inspired Agent Prompts

Smaller/cleaner pass inspired by Hermes CLI prompt behavior.

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/agent-prompt-compact-hermes/index.html
```

## Design shift

The previous hook/notch board was too loud. This pass follows the Hermes CLI discipline:

- prompt first
- tiny symbol
- state prefix only when needed
- compact/narrow mode matters
- metadata is secondary chrome, not the prompt itself

## Best direction

Default:

```text
⌁ ›
```

Named:

```text
⌁ hermes ›
```

Useful telemetry:

```text
[⌁ ctx ▮▮▮▮░░ 73 · tok 141k]
⌁ ›
```

Compact bracket identity:

```text
[⌁] ›
```

## Anvil Notch role

Use `⌙` for:

- Anvil-branded skin
- scoped edit mode
- policy/gated state
- blocked/revise state

Example:

```text
[⌙ anvil · gated · ctx 73]
⌙ ›
```

## Recommendation

Ship a small default:

```text
⌁ ›
```

Then expose an expanded line only when the agent mode needs telemetry:

```text
[⌁ ctx ▮▮▮▮░░ 73 · tok 141k]
⌁ ›
```

Keep the big bracket meters out of the permanent prompt.
