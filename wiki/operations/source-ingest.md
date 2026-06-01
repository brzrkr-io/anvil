---
status: active
type: operation
created: 2026-05-21
updated: 2026-05-21
sources:
  - ../sources/karpathy-llm-wiki
confidence: high
---

# Source Ingest

## Workflow

1. Read one raw source.
2. Create one summary in `wiki/sources/`.
3. Update linked concept, decision, or operation pages.
4. Update `wiki/index.md` if routing changed.
5. Append `wiki/log.md`.

## Checks

- The raw source was not modified.
- New wiki pages have required frontmatter.
- Claims cite raw paths, source pages, or URLs.
- Contradictions and low-confidence claims are visible.
