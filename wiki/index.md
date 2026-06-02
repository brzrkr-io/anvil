---
status: active
type: index
created: 2026-05-21
updated: 2026-05-31
sources: []
confidence: high
---

# Anvil Wiki Index

## Mission

Build Anvil as a native macOS control plane for software and devops
work: Zig, Metal, AppKit (thin Obj-C shim). This wiki is the durable, agent-maintained knowledge
base for the project.

## Read First

1. `AGENTS.md`
2. This index
3. `BRAND.md` before any user-facing work
4. The latest relevant handoff in `context/`
5. Specific wiki pages found with `rg`

## Frontmatter Convention

Every wiki page starts with:

```yaml
---
status: active
type: index
created: 2026-05-21
updated: 2026-05-21
sources: []
confidence: high
---
```

Allowed `type`: `index`, `concept`, `operation`, `decision`, `source`, `log`.
Allowed `status`: `active`, `draft`, `superseded`.
Allowed `confidence`: `high`, `medium`, `low`. Dates use `YYYY-MM-DD`.

## Sections

- [[concepts/README|Concepts]] — durable ideas and patterns.
- [[operations/README|Operations]] — agent workflows and process rules.
- [[decisions/README|Decisions]] — decision records.
- [[sources/README|Sources]] — summaries of ingested raw sources.
- [[log|Log]] — append-only operation history.

## Key Pages

- [[concepts/llm-wiki|LLM Wiki]] — the wiki pattern this repo follows.
- [[concepts/context-budget|Context Budget]] — selective loading rules.
- [[concepts/console-architecture|Console Architecture]] — codebase map: data flow, module list, runtime model.
- [[concepts/config-system|Config System]] — TOML config, arena ownership, live/startup-only split, Watcher, built-in themes.
- [[concepts/tab-system|Tab System]] — Tab struct, TabManager, per-tab PTY reader thread, bar-visibility rule, keybinding chords.
- [[concepts/search-system|Search System]] — Search struct, smart-case, max_matches cap, UTF-8 validation, content-row index space, bottom bar, top/bottom bar-row offset split.
- [[concepts/shell-integration|Shell Integration]] — OSC 133 A/B/C/D marks, OSC 7 cwd, zsh/bash hook scripts, ZDOTDIR shim auto-injection, `shell_integration` config toggle, embed-and-write startup, `cwd_path()` for new-tab cwd inheritance.
- [[concepts/workspace-panes|Workspace Panes]] — PaneTree layout engine, split/close/focus model, divider drag, focused-pane accent border, keyboard nav chords.
- [[operations/agent-session-loop|Agent Session Loop]] — start/during/closeout gates.
- [[operations/source-ingest|Source Ingest]] — raw source to wiki workflow.
- [[operations/wiki-lint|Wiki Lint]] — health checks for wiki growth.
- [[operations/coverage|Code Coverage]] — `.zig/zig build test` and the coverage workflow.
- [[decisions/0001-ai-dev-environment|0001 AI Dev Environment]] — this setup.
- [[decisions/0002-tech-stack|0002 Tech Stack]] — Zig + Metal + AppKit; runtime MSL shader rationale (superseded by 0005).
- [[decisions/0003-m1-brand-palette|0003 M1 Brand Palette]] — ANSI-16 → Mineral palette mapping and ambiguity resolution.
- [[decisions/regression-harness-foundation|Regression Harness Foundation]] — design decisions for the Bug A–E regression suite.
- [[decisions/native-file-viewer|Native File Viewer]] — Session.Kind=viewer, no PTY, syntax.zig tokenizer, fillGrid reuse; entry via explorer click and `anvil view` IPC verb.
- [[decisions/mineral-warm-palette|Mineral Warm Palette]] — 2026-05-30 palette evolution: warm near-black backgrounds, coral-rose accent; three-surface cohesion (chrome/ANSI/syntax). Supersedes 0003 hex values.
- [[decisions/viewport-sink-trait|Viewport Sink Trait]] — trait-object sink over generics/enum for the unified viewport draw loop; xy-is-pre-shift contract.
- [[concepts/hardening-net|Hardening Net]] — each fixed bug and its test hook(s).
- [[concepts/layout-modes|Layout Modes]] — LayoutMode (Terminal/Ide/Codex), Docks::Areas geometry, context bar (ID1/ID2), mode-cycle keybind (ID5).
- [[concepts/block-model|Block Model]] — OSC 133 block structure, header row, running-block pulse (CB6), completion pulse, Opt+click copy (CB5), diff colorization.
- [[concepts/agent-actions|Agent Actions]] — caldera approve/start keybindings (AG3), `anvil-caldera` action helpers.
- [[concepts/native-editor|Native Editor]] — Buffer/rope model (NE1), file IO (NE2), undo/redo (NE3), EditorPane + registry (NE4), render path (NE5).
- [[concepts/security-boundary|Security Boundary]] — CSP + IPC audit: locked CSP (local-only, unsafe-inline required by hydration), scoped capabilities, no shell plugin, allow-listed verbs; agent `run_capture` is the live surface.
- [../BRAND](../BRAND.md) — Anvil brand contract for all user-facing work.

## Current State

- Anvil is a native macOS app: Zig, Metal, AppKit (thin Obj-C shim). Active branch: `zig`. The Rust port (`rust-port`) was archived on tag `rust-port-archive`.
- Status: ground-up Zig rewrite. GPU terminal with native Metal chrome (tab bar, search, dividers, palette). A `.web` pane type (WKWebView, NSView subview) shipped 2026-05-31; app chrome stays 100% native Metal — see [[decisions/0005-render-host]].
- See `docs/product/console-rebuild-plan.md` for the full rebuild plan.
- This repo is standalone. It must not depend on `caldera-os` at runtime.

## Agent Roles

Subagents live in `.claude/agents/`: builder, reviewer, systems-architect,
orchestrator, librarian, design-lead, product-strategist.
