---
status: active
type: decision
created: 2026-05-21
updated: 2026-05-21
sources:
  - ../sources/karpathy-llm-wiki
  - ../sources/multica-claude-guidelines
confidence: high
---

# 0001 — AI Dev Environment

## Status

Active. Decided 2026-05-21.

## Context

Anvil needed a consistent agent and knowledge setup. The private
`caldera-os` workspace already had a usable pattern: role-based agents, an
LLM wiki, a brand contract, and shared agent instructions.

## Decision

Replicate that setup into this repo, adapted to the Zig/macOS stack:

- Seven subagents in `.claude/agents/`, rewritten as Claude Code subagents.
- A wiki under `wiki/` using the LLM-wiki pattern.
- `AGENTS.md` and `CLAUDE.md` with the shared work rules.
- `BRAND.md` and `brand/` adapted from `caldera-os`.

Specifics from caldera-os — Linear, the old CLI tooling, Forge, Rust — were dropped.
Verification is `zig build test`.

## Consequences

- `/agents` lists seven dispatchable subagents.
- Durable knowledge has a home in `wiki/`.
- This repo stays standalone — no runtime dependency on `caldera-os`.
