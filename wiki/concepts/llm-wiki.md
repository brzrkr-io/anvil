---
status: active
type: concept
created: 2026-05-21
updated: 2026-05-21
sources:
  - ../sources/karpathy-llm-wiki
confidence: high
---

# LLM Wiki

An LLM wiki is a persistent Markdown knowledge base maintained by agents. It
compiles raw sources into linked pages so future answers can start from existing
synthesis instead of re-reading everything.

## Anvil Version

- Agent-written summaries and synthesis live in `wiki/`.
- `wiki/index.md` routes agents to the right pages.
- `wiki/log.md` records chronological operations.
- Frontmatter records page type, status, sources, dates, and confidence.

## Required Operations

- Ingest one source into a source summary plus linked wiki updates.
- Query the wiki first, raw sources second.
- Lint for contradictions, stale pages, orphans, missing concepts, and bloat.
- Hand off through compact packs in `context/`.
