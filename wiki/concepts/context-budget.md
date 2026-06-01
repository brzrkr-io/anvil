---
status: active
type: concept
created: 2026-05-21
updated: 2026-05-21
sources:
  - ../sources/karpathy-llm-wiki
  - ../sources/multica-claude-guidelines
confidence: high
---

# Context Budget

Context budget is the discipline of preserving reasoning room by loading only
the smallest useful set of files.

## Rules

- Start with `AGENTS.md`, `wiki/index.md`, and any relevant handoff.
- Use `rg` before opening files.
- Prefer wiki summaries over raw sources.
- Open raw sources only for provenance, nuance, or unresolved ambiguity.
- Write handoff packs before context becomes hard to transfer.
