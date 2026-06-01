---
status: active
type: operation
created: 2026-05-21
updated: 2026-05-29
sources:
  - ../sources/karpathy-llm-wiki
  - ../sources/multica-claude-guidelines
confidence: high
---

# Agent Session Loop

## Purpose

Keep the wiki and handoff state current as agents work.

## Start Gate

- Read `AGENTS.md` and `wiki/index.md`.
- Read the latest relevant handoff in `context/`.
- State the intended output and the verification check.

## During-Work Gate

- Update wiki pages when durable knowledge changes.
- Use `rg` before opening long files.
- Write a handoff when context grows or delegation is likely.

## Closeout Gate

- Append `wiki/log.md` for durable operations.
- Run `.zig/zig build test`.
- Report changed files, verification output, and remaining work.

## Rule

A task is not complete if repo state, wiki state, and handoff state disagree.
