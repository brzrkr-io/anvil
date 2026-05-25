# Anvil Fresh Agent Prompt Symbols

Fresh symbol/shape exploration for activated agent mode prompts.

Open:

```sh
open /Users/pjanderson/projects/caldera/anvil/sketches/agent-prompt-symbols/index.html
```

## Correction preserved

Shell prompt and agent prompt are separate modes. This board shows the shell prompt only as reference; all variants are activated-agent prompts.

## Why the old ones felt generic

Most agent UIs converge on:

```text
◈ ◆ ✦ ✧ ✨ [] ▰
```

They are easy to see in monospace and imply “magic agent,” but they now feel generic.

## Fresh symbol families explored

- Hook / pulse: `⌁`
- Basin half / Anvil-native: `◒`
- Socket / endpoint: `⊚`
- Trace node / provenance: `⦿`
- Caret / modifier: `⌃` or `^`
- Command lens: `⌕`
- Kernel/process: `⧉`
- Route ticks: `╴`
- Local daemon grammar: `anvild:hermes`
- Stack pointer grammar: `sp:hermes`

## Strongest candidates

1. Hook Prompt

```text
⌁ hermes · ctx 73 · tok 141k
⌁ › make prompt smarter
```

2. Basin Half

```text
◒ anvil-agent · ctx 73% · tok 141k
◒ › inspect repo state
```

3. Socket Prompt

```text
⊚ hermes.sock · model routed
ctx 73% · tok 141k/192k · tools gated
⊚ › open command channel
```

4. Trace Node

```text
⦿ trace://hermes · ctx 73%
tok 141k/192k · evidence on · repo anvil
⦿ › trace this decision
```

5. Local Daemon

```text
anvild:hermes  ctx=73 tok=141k
policy=armed tools=gated evidence=on
:› do agent work
```

## Recommendation

Best fresh family:

- default agent: `⌁` Hook Prompt
- Anvil-branded skin: `◒` Basin Half / Basin Rail
- systems/internal skin: `⊚` Socket Prompt
- evidence/provenance state: `⦿` Trace Node
- compact fallback: `››` Double Prompt or `^›` Ember Caret

This is fresher than diamonds/sparkles while still giving each symbol a product reason.
