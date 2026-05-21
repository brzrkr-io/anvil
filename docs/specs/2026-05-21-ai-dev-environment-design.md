# AI Dev Environment — Design Spec

Status: approved design
Date: 2026-05-21
Author: Parker Anderson (with Claude Code)
Scope: two sub-projects — (A) the `caldera-console` AI dev environment, (B) the global `~/.claude` overhaul.

## Purpose

Replicate the agent/wiki/brand/instruction setup from the private `caldera-os`
workspace into the standalone `caldera-console` repo, adapted to this app's real
stack, and standardize the global `~/.claude` setup on the Karpathy/multica
`CLAUDE.md` behavioral standard plus the LLM-wiki convention.

`caldera-console` is a native macOS app: Zig, `zig build`, Metal, AppKit.
It must not depend on `caldera-os` at runtime or reference Linear, the `anvil`
CLI, Forge, or the Agent-OS "bootstrap" framing.

## Sources

- `caldera-os/agents/` — 7 plain-Markdown role docs (no frontmatter).
- `caldera-os/AGENTS.md`, `caldera-os/CLAUDE.md`.
- `caldera-os/BRAND.md`, `caldera-os/brand/` — 9 portable assets.
- `caldera-os/wiki/` — wiki system + reusable pattern pages.
- `multica-ai/andrej-karpathy-skills` — `CLAUDE.md` behavioral standard
  (https://github.com/multica-ai/andrej-karpathy-skills).
- `caldera-os/wiki/sources/karpathy-llm-wiki.md` — LLM-wiki concept.

## Key finding

The `caldera-os/agents/*.md` files are role *documentation*, not Claude Code
subagents — they have no YAML frontmatter and no system-prompt body. To make
them function in `/agents` and via the `Agent` tool they must be converted:
add frontmatter (`name`, `description`, `model`, `tools`) and rewrite the body
as a system prompt.

---

## Sub-project A — `caldera-console` AI dev environment

### A.1 Repo layout (new files)

```
caldera-console/
├── CLAUDE.md              @AGENTS.md include + Zig/macOS build notes
├── AGENTS.md              adapted from caldera-os — Zig/macOS, no Linear/anvil/Forge
├── BRAND.md               copied verbatim
├── brand/                 9 assets copied verbatim
├── .claude/
│   └── agents/            7 subagents (frontmatter + system prompt)
│       ├── builder.md
│       ├── reviewer.md
│       ├── systems-architect.md
│       ├── orchestrator.md
│       ├── librarian.md
│       ├── design-lead.md
│       └── product-strategist.md
├── wiki/
│   ├── index.md           fresh, scoped to caldera-console
│   ├── log.md             seeded with the setup entry
│   ├── concepts/
│   │   ├── README.md
│   │   ├── llm-wiki.md
│   │   └── context-budget.md
│   ├── operations/
│   │   ├── README.md
│   │   ├── agent-session-loop.md
│   │   ├── source-ingest.md
│   │   └── wiki-lint.md
│   ├── decisions/
│   │   ├── README.md
│   │   └── 0001-ai-dev-environment.md
│   └── sources/
│       ├── README.md
│       ├── karpathy-llm-wiki.md
│       └── multica-claude-guidelines.md
└── context/
    └── README.md          handoff-pack convention
```

`agents/` lives under `.claude/` (not repo root as in caldera-os) so Claude
Code's `/agents` discovers them.

### A.2 The 7 agents

Each caldera-os role doc becomes a real subagent:

- YAML frontmatter: `name` (kebab-case), `description` (when-to-use, with
  trigger examples for auto-dispatch), `model`, `tools`.
- Body rewritten as a system prompt, keeping the Purpose / Inputs / Outputs /
  Done Criteria / Context Limits structure.
- Adapted to Zig / `zig build` / Metal / AppKit / macOS. Removed: Rust, Cargo,
  `anvil check`, Linear, Forge, "bootstrap" gating.

| Agent | `name` | `model` | `tools` |
|---|---|---|---|
| Builder | `builder` | sonnet | Read, Edit, Write, Bash, Grep, Glob |
| Reviewer | `reviewer` | opus | Read, Grep, Glob, Bash |
| Systems Architect | `systems-architect` | opus | Read, Grep, Glob, Bash |
| Orchestrator | `orchestrator` | opus | Read, Grep, Glob, Bash |
| Librarian | `librarian` | sonnet | Read, Edit, Write, Grep, Glob |
| Design Lead | `design-lead` | sonnet | Read, Grep, Glob |
| Product Strategist | `product-strategist` | opus | Read, Grep, Glob, WebSearch, WebFetch |

Reviewer is read-only (reports findings, does not edit). Models/tools are
tunable post-implementation.

### A.3 `AGENTS.md` (adapted)

Slimmed from caldera-os. Keep:
- Start-here list (read `AGENTS.md`, `wiki/index.md`, active handoff).
- Brand gate: read `BRAND.md` before any UI / window / icon / user-facing work.
- Work rules: state assumptions, surgical changes, define success criteria,
  verify before claiming done.
- Wiki rules: frontmatter convention, update `index.md` + `log.md` on durable
  changes, ingest one source at a time.
- Session loop: start / during / closeout gates.

Remove: Linear, `anvil check`, Forge portability/contracts, parallel-agent
Linear coupling, the "do not build product code until bootstrap passes" rule.
Closeout verification command becomes `zig build test`.

### A.4 `CLAUDE.md` (adapted)

Short file:
- `@AGENTS.md` include.
- Project-specific Claude notes: `zig build run`, `zig build test`, minimum Zig
  version (`build.zig.zon`), macOS + Xcode CLT requirement.

The Karpathy behavioral standard is NOT duplicated here — it lives in the global
`~/.claude/CLAUDE.md` (sub-project B) and applies automatically.

### A.5 Wiki

The wiki *system*, not caldera-os history:
- `index.md` — fresh, describing this app (Zig console rebuild, M0/M1 status,
  read-first list, frontmatter convention, section links).
- `log.md` — append-only; seeded with the environment-setup entry.
- `concepts/llm-wiki.md`, `concepts/context-budget.md` — copied, de-Linear'd.
- `operations/agent-session-loop.md` — copied, `anvil check` → `zig build test`,
  Linear references removed.
- `operations/source-ingest.md`, `operations/wiki-lint.md` — copied (already
  generic).
- Section `README.md` files — copied, anvil-specific bullet lists removed.
- `decisions/0001-ai-dev-environment.md` — records this setup as the repo's
  first decision record.
- `sources/karpathy-llm-wiki.md`, `sources/multica-claude-guidelines.md` —
  copied with relative paths fixed; they document why this setup exists.

No `products/`, no Anvil/Forge operation logs, no `entities/`.

### A.6 Brand

`BRAND.md` and all 9 `brand/` files copied **verbatim**. `BRAND.md` is already
written as portable ("Product repos may copy this file") and already names
"Caldera Console." Its one caldera-os-specific reference
(`docs/design/shared-core-surface-modes.md`) is already conditioned on "when
working inside caldera-os," so verbatim copy is correct.

---

## Sub-project B — global `~/.claude` overhaul

### B.1 New `~/.claude/CLAUDE.md`

The multica/Karpathy `CLAUDE.md` standard, verbatim — the four principles:
Think Before Coding, Simplicity First, Surgical Changes, Goal-Driven Execution.
One-line header crediting the source URL. Non-destructive: no global `CLAUDE.md`
exists today. Applies to every project once created.

### B.2 New `~/.claude/llmwiki-convention.md`

A ~1-page reusable guide for the LLM-wiki pattern: frontmatter fields, the roles
of `index.md` and `log.md`, ingest discipline, "query the wiki before raw
sources." Generic — no Caldera specifics. Referenced by a single pointer line in
the global `CLAUDE.md` so the standard itself stays short.

### B.3 Refresh the 3 global agents

`file-finder`, `gitlab-tracker`, `senior-platform-engineer` — light-touch
alignment with the standard: trim anything that nudges toward speculative
abstraction or non-surgical edits; ensure each defers to the four principles.
Minimal diffs, shown to the user before applying. `senior-platform-engineer`
gets the most attention since it writes production code.

### B.4 `settings.json` — review only

Report, do not change. Note `skipDangerousModePermissionPrompt: true` (disables
permission prompts in dangerous mode) for the user's awareness. Any change is a
separate, explicitly-approved step.

### B.5 Safety

`~/.claude/` is not a git repo. Before modifying the 3 agents or any existing
file, copy them to `~/.claude/backups/` with timestamps for rollback.

---

## Success criteria

- `/agents` in `caldera-console` lists the 7 new subagents; each is dispatchable.
- `caldera-console/AGENTS.md` and `CLAUDE.md` contain no Rust, Cargo, Linear,
  `anvil`, or Forge references; build/test commands are `zig build`.
- `caldera-console/wiki/` contains the system + reusable pages only — no
  Anvil/Forge operation logs.
- `BRAND.md` + 9 `brand/` assets present and byte-identical to caldera-os.
- `~/.claude/CLAUDE.md` exists with the verbatim Karpathy standard.
- `~/.claude/llmwiki-convention.md` exists; global `CLAUDE.md` points to it.
- The 3 global agents are backed up before edit; diffs reviewed before applying.
- `zig build test` still passes after the changes (the setup adds docs/config
  only — no source code changes).

## Out of scope

- No changes to `caldera-console` application source code.
- No new Claude Code plugins or marketplaces installed.
- No changes to `settings.json` (review only).
- No replication into other repos (anvil-console, finance, k8s-platform, etc.).
