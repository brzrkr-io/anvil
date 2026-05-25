# Anvil Agent Prompt — Hook Caret Bars

Corrected prompt exploration: `⌁` is the agent-mode input caret.

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/agent-prompt-hook-caret/index.html
```

## Corrected rule

Shell mode may use a normal arrow.

Agent mode does not.

```text
⌁ ask agent
```

Do not use:

```text
⚡ ask
⌁ › ask
⌁ ⚡ ask
```

## Visual language

This board pushes:

- bracket telemetry
- pipes: `|`, `│`, `╎`
- compact loading bars
- block/dotted-tail meters inspired by the provided reference image
- Hermes/Honcho-style state chips
- hacker/operator rails

## Recommended family

Default:

```text
⌁ ask agent
```

Normal with identity:

```text
[hermes|ctx73|tok141] ⌁ ask agent
```

Expanded:

```text
│ ctx ▮▮▮▮░░ │ tok ▮▮▮░░░ │
⌁ ask agent
```

Color rule: `ctx` meter is ember/orange, `tok` meter is violet/purple. Do not use both-purple double meters.

Cooler operator version:

```text
hermesd | state:warm | repo:anvil
ctx ▮▮▮▮░░ | tok ▮▮▮░░░
⌁ continue from handoff
```

## Best cards

- 02 Hermes Pipe Chip — best everyday prompt with identity.
- 03 Inline Load Chip — best compact bar version.
- 04 Pipe Rail Stack — selected expanded prompt.
- 14 Twin Reference Bars — screenshot-style expanded telemetry, with ctx orange and tok purple.
- 25 Pipe Rail Stack Family — implementation direction.
- 26 Rail Operator Prompt — coolest hacker/operator variant.
- 27 Compact Reference Bar — best visual callback to the screenshot.
- 28 Ultra Compact Chip — best narrow mode.
