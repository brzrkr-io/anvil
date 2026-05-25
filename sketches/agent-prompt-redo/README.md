# Anvil Agent Prompt Redo

Redo after correcting the core interaction model.

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/agent-prompt-redo/index.html
```

## Correction

The shell prompt and agent prompt are separate modes.

Do not implement a live split prompt where shell input is left and agent input is right.

Side-by-side / together display is only valid on a design comparison board.

## Actual behavior

Shell mode:

```text
~/caldera/anvil
› cargo test
```

Enter runs shell.

Agent mode:

```text
[hermes] ctx 73% · tok 141k/192k · tools gated
◈ › make the prompt smarter
```

Enter sends instruction to the agent.

## Strongest candidates

- 01 Agent Block Header
- 03 Operator Strip
- 06 Sigil Deck
- 12 One-line Agent
- 15 Canonical Candidate

## Recommendation

Default agent prompt:

```text
[hermes] ctx 73% · tok 141k/192k · tools gated
repo anvil · evidence on · policy armed
▰▰▰▰▰▰▱▱
◈ › make the prompt smarter
```

Compact:

```text
[h 73% 141k] ◈ › ask
```

State variants:

```text
[approval] bounded edit requested
? › approve once / deny / inspect diff

[forge] active execution · 00:42
▶ › running approved command

[evidence] test pass · clippy clean
✓ › summarize changes
```
