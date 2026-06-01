---
status: active
type: source
created: 2026-05-21
updated: 2026-05-21
sources:
  - https://github.com/multica-ai/andrej-karpathy-skills/blob/main/CLAUDE.md
confidence: high
---

# Multica CLAUDE Guidelines

## Summary

A compact set of behavioral rules for coding agents, derived from Andrej
Karpathy's observations on LLM coding pitfalls: state assumptions, surface
tradeoffs, prefer simple solutions, make surgical edits, define success
criteria, and verify before claiming completion.

## Useful For Anvil

- These guidelines are installed globally at `~/.claude/CLAUDE.md`.
- Bias toward simple, scoped changes over speculative flexibility.
- Require every changed line to trace to the active request.
- Convert vague tasks into verifiable success criteria.
- Stop and ask when ambiguity materially changes the result.

## Constraints

These are behavior rules, not project architecture. They stay short enough to
guide agents without crowding repo-level instructions.

## Links

- [[../concepts/context-budget|Context Budget]]
