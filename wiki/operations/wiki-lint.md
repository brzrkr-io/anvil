---
status: active
type: operation
created: 2026-05-21
updated: 2026-05-21
sources:
  - ../sources/karpathy-llm-wiki
confidence: high
---

# Wiki Lint

## Checks

- Missing required frontmatter.
- Empty or near-empty pages.
- Orphan pages not linked from `wiki/index.md` or a directory index.
- Contradictions or stale naming.
- Pages too large for easy agent loading. Append-only `type: log` pages are
  exempt from the normal page budget.
- Concepts mentioned repeatedly without their own page.

## Output

Report findings with file paths and recommended fixes. Update pages only when
the lint task includes fixing.
