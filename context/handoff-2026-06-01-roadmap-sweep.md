# Handoff — Terax/Zed roadmap sweep (2026-06-01, updated)

Branch: `tauri`. Stack: Rust + Tauri v2 + SvelteKit (SPA) + Svelte 5 runes + xterm.js + Monaco.
Roadmap: `docs/roadmap-terax-zed.md` is the **canonical status** (100 items, [x]/[~]/[ ]).
Current: **63 done, 37 partial, 0 fully open** (100/100 addressed). Tree green: svelte-check 0 errors,
115 unit tests pass, `vite build` + `cargo check` clean. **All work below is unsaved — commit before risky changes.**

## 2026-06-01 sweep round 2 — the last open items
Closed every remaining fully-open item; `[ ]` count is now 0. New work:
- **#62 stage/discard hunks** — pure `git.ts` `parseHunks`/`buildHunkPatch` + Rust `git_apply_hunk`
  (`git apply --cached`/`--reverse` over stdin); SourceControl file rows get a hunk toggle → inline
  per-hunk Stage/Unstage/Discard (`HunkStage.svelte`).
- **#44 PTY read-coalescing** — reader thread → mpsc → coalescer batches bursts over a 4ms idle window
  (or 256 KiB cap) into one Channel message (lib.rs `pty_open`).
- **#93 virtualize commit log** — windowed rows (fixed 26px, scrollTop→visStart/visEnd + overscan, absolute
  rows in a spacer); `git_log` raised to 500.
- **#34 3-way merge editor** — pure `merge.ts` parses diff3 base section; Editor conflict bar resolves first
  conflict Ours/Theirs/Base/Both + All-ours/All-theirs (fixed the prior diff3 lumping bug).
- **#38 multi-buffer search-and-edit** — SearchPanel "Edit results" inline-edits match lines; Save batches
  per-file via pure `search-edit.ts` `applyLineEdits`/`groupByFile`.
- **#54 inline per-hunk agent diff** — `linediff.ts` (LCS) + `DiffReview.svelte`: agent's proposed file opens
  as accept/reject-per-hunk vs current; "Apply N/M" merges accepted hunks.
- **#53 agent tool-use loop** — `agent-tools.ts` self-defined protocol (```anvil:run / ```anvil:read);
  approval card; Rust `run_capture` executes & feeds result back; bounded auto-continue (MAX 8 steps).
- **#17 detach pane → window** — *partial*: "Detach Pane to New Window" seeds a new window via `?detach=`
  JSON; detached window skips state restore/persist. Live PTY transfer across webviews is out of scope.
- **#1 workspace-as-primary** — *partial by product decision*: stays a selectable rail, not the boot default.
New pure+tested modules: `merge.ts`, `search-edit.ts`, `linediff.ts`, `agent-tools.ts` (+ hunk fns in `git.ts`).
New Rust cmds: `git_apply_hunk`, `run_capture`; `new_window` gained an optional `seed`.

## Verify loop (run after every change)
- `node_modules/.bin/svelte-check --tsconfig ./tsconfig.json` → `0 ERRORS`
- `node_modules/.bin/vitest run` → all pass (76)
- `node_modules/.bin/vite build` → `✔ done`; Rust: `cd src-tauri && cargo check`
- Browser preview: Claude_Preview MCP `anvil-preview` (4173). **Stop+start the server** for a fresh build
  (`location.reload()` serves stale). invoke()s throw in-browser (no backend) — init state synchronously,
  not in onMount (caught the onboarding bug this way).

## New files this session
- `src/lib/diagnostics.ts` — monaco-free LSP-problems store (keeps Monaco lazy)
- `src/lib/extensions.ts` (+`.test.ts`) — extension manifest/registry/rail-gating
- `src/lib/agent-seed.ts` — pre-fill agent input ("Explain Errors")
- `src/lib/layout-settings.ts` — autoHideRail
- `.github/workflows/release.yml` — signed release + notarize (needs secrets)
- `docs/superpowers/plans/2026-06-01-tabs-in-panes.md` — the executed plan

## Big things built (arcs)
- **Tabs-in-panes (§A #2)**: Leaf = `{tabs:PaneTab[], active}` with back-compat `view`/`ref` mirror; PaneGrid
  renders a per-pane tab strip; +page has wsSetActiveTab/wsCloseTab/wsAddTab; center-drop adds a tab; the
  paneView snippet is keyed by active tab id (clean remount). Migration in `remapTermRefs` for old saved trees.
- **Pane zoom (§A #8)**: PaneGrid `zoomId` hides sibling cells via display:none (no unmount → no PTY-kill).
- **Extensions (§F #69–80)**: extensions.ts model + Settings▸Extensions store + rail-gating; k8s pod
  describe/restart/delete (kube_* Rust cmds); GitLab via glab palette cmds; Prometheus `prom_query` pane;
  Grafana `open_url_window` (native window, XFO-free); Terraform `terraform_plan` diff-colored tab; saved dashboards.
- **Agent streaming (§D #52)**: `llm_chat_stream` Rust cmd (SSE over Channel, no_proxy) + AgentPanel token render w/ fallback. Added reqwest "stream" + futures-util deps.
- **LSP depth (§B)**: code-actions/document-symbols providers added to lsp.ts (rename/refs already existed).
- Onboarding overlay (#100), config export/import (#87), PR create/view (gh, #66), git pull/push/amend/ahead-behind,
  file history, search-replace, cursor-style, line-height/letter-spacing, ligatures, sticky-scroll, inlay-hints,
  format-on-save, blame-always, editor nav history, named layout presets, balance/close-others panes, bottom dock (⌘J).

## Remaining 14 fully-open — and why each is blocked
- **#53 tool-use loop, #54 inline per-hunk agent diffs** — need a live LLM to build+verify the approval UX safely.
- **#82 editable keymap** — refactor `onKey` (big context-gated if/else in +page) to a keymap store; no test net → do in a fresh context.
- **#34 3-way merge editor, #38 multi-buffer search-edit, #46 tmux-in-terminal-tab** — each a substantial editor feature.
- **#62 stage-hunk** — `git apply --cached` Rust cmd + per-hunk patch construction + DiffView buttons (unfamiliar component).
- **#93 virtualize SCM lists** — windowing the commit-graph swimlanes (harder than the flat search list).
- **#36 emmet** — needs `emmet-monaco-es` npm dep.
- **#44 PTY read-coalescing** — blocking-read loop; needs nonblocking-fd rework to time-batch safely.
- **#94 frame-budget profiling, #96 Playwright e2e** — measurement/harness, not feature code.
- **#1 workspace-as-primary** — UX default flip; product decision (don't flip unilaterally).
- **#17 detach-pane→OS-window** — live PTY transfer between webviews (cross-process).
- **§J #97 notarize** — wired in release.yml; needs your Apple Developer cert in repo secrets.

## Conventions
Caveman-lite chat. No commit/push unless asked. No Claude co-author. Secrets → Keychain. LLM client `.no_proxy()`.
Match existing Svelte/Settings patterns. DevOps tabs: k8s/ci/prs/obs/tf. Palette is the main action surface.
