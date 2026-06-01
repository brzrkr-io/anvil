---
status: active
type: source
created: 2026-05-21
updated: 2026-05-21
sources:
  - https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f
confidence: high
---

# Karpathy LLM Wiki

## Summary

Karpathy describes an LLM-maintained wiki as a compounding knowledge artifact
between immutable raw sources and chat-time answers. Instead of re-deriving
synthesis on every question, an agent ingests sources into structured Markdown
pages, updates entities and concepts, flags contradictions, maintains an index,
and appends a chronological log.

## Useful For Anvil

- Let agents write and maintain `wiki/`.
- Treat `AGENTS.md` and `wiki/index.md` as the schema for wiki rules.
- Read `wiki/index.md` before drilling into pages.
- Append `wiki/log.md` after ingest, query, lint, and handoff operations.
- File valuable answers back into the wiki when they should outlive chat.

## Constraints

Keep the implementation simple: Markdown files, frontmatter, `rg`, an index, and
a log before adding search engines, MCP servers, or plugins.

## Links

- [[../concepts/llm-wiki|LLM Wiki]]
- [[../operations/source-ingest|Source Ingest]]
- [[../operations/wiki-lint|Wiki Lint]]
