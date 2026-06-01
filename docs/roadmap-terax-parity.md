# Anvil → Terax Parity & Beyond — 100-item Roadmap

Stack: Rust + Tauri v2 + SvelteKit (Svelte 5) + vanilla TS. xterm.js terminal,
Monaco editor, `portable-pty`. Branch `tauri`. Goal: the AI-native console for
100% of a DevOps engineer's work — terminal, editor, source control, AI agent,
DevOps integrations — cross-platform, blazingly fast.

Legend: [ ] todo · [~] partial · [x] done. P0 = parity foundation, P3 = polish/edge.

## A. Terminal (core UX)
1. [~] Real PTY terminal (xterm + portable-pty) — DONE base
2. [ ] Multiple terminal tabs (new/close/switch, ⌘T/⌘W)
3. [ ] Split panes (horizontal/vertical), focus nav, resize
4. [ ] Per-tab persistent shell + scrollback restore on relaunch
5. [ ] Terminal search (⌘F) with match highlight + next/prev
6. [ ] Copy-mode / keyboard scrollback nav (vim keys)
7. [ ] OSC 8 hyperlinks (⌘-click to open)
8. [ ] OSC 7 cwd tracking (drives tab title + SCM + breadcrumb)
9. [ ] OSC 133 shell-integration prompt marks + jump prev/next
10. [ ] Right-click context menu (copy/paste/clear/split)
11. [ ] WebGL renderer addon + ligature support + font config
12. [ ] Bracketed paste + multiline paste guard

## B. Editor (Monaco, IDE-grade)
13. [~] Monaco open-from-Explorer + save (⌘S) — DONE base
14. [ ] Multiple editor tabs + dirty tracking + reopen-on-launch
15. [x] LSP client (Rust backend ↔ rust-analyzer / gopls over stdio) — src-tauri/src/lsp.rs; Go verified end-to-end vs gopls; Rust works after `rustup component add rust-analyzer`
16. [x] Diagnostics (squiggles) — publishDiagnostics → Monaco markers
17. [x] Completions / hover / signature help via LSP — all three registered (src/lib/lsp.ts)
18. [x] Go-to-definition / references / rename symbol — all three registered
19. [x] Format-on-save (LSP / prettier / rustfmt) — Monaco format-on-save wired
20. [ ] Find/replace across file + across workspace (ripgrep)
21. [ ] File watcher → reload-on-external-change + unsaved conflict prompt
22. [ ] Breadcrumbs + symbol outline panel
23. [ ] Inline git blame + gutter change markers (added/modified/deleted)
24. [ ] Diff editor (side-by-side) for SCM + file history
25. [x] Bracket-pair colorization + indent guides + minimap toggle
26. [x] Editor settings: font, tab size, word wrap, theme map

## C. Source Control / Git
27. [~] Commit graph + status view — DONE base
28. [ ] SCM follows active terminal cwd (and repo-root detect)
29. [x] Real commit-graph lanes (parent edges, colored branches) — buildGraph() swimlanes + SVG, visually verified
30. [ ] Stage / unstage / discard hunks + files
31. [ ] Commit (message box) + amend
32. [ ] Push / pull / fetch with auth + progress (gated UX)
33. [ ] Branch create/checkout/delete/merge/rebase
34. [ ] Diff view per commit + per file (uses diff editor)
35. [ ] Conflict resolver UI
36. [ ] Stash list + apply/drop/create
37. [ ] Blame view + file history timeline
38. [ ] Remotes management + PR list (GitHub/GitLab API)
39. [ ] Tag list + create
40. [ ] Commit detail panel: changed files + colored icons + Copy SHA

## D. AI Agent (the differentiator)
41. [ ] Agent side panel (chat) with streaming responses
42. [ ] Provider seam: Anthropic / OpenAI / local (Ollama/LM Studio) config
43. [ ] Tool use: run shell command in a pane (with approval)
44. [ ] Tool use: read/edit files (diff preview + approve)
45. [ ] Context: attach open file / selection / terminal output / repo
46. [ ] Inline "explain this error" from terminal/diagnostics
47. [ ] Agent edits → editor diff review + accept/reject per hunk
48. [ ] Command-from-NL: "deploy staging" → proposes shell, you confirm
49. [x] Multi-step agent runs with a visible plan + step status
50. [x] Agent run history + replay + cost/token meter
51. [x] @-mentions (files, symbols, commands) in chat
52. [x] Background agents (long tasks) with notifications
53. [x] Caldera bridge (revive: AI control plane HTTP API) — src-tauri/src/lib.rs `caldera_snapshot` (GET /health, /api/project, /api/agent-runs, /api/activity on localhost:4175) + src/lib/Caldera.svelte rail view, polls every 4s, degrades to "offline". Code verified against the LIVE daemon via `cargo test caldera_tests::reach` → online=true, 86 runs, 3 attention parsed. In-app live render needs the macOS Local Network grant / a signed build (network.client entitlement added).
54. [x] Secrets-safe: never send creds; redaction pass on context

## E. Files / Workspace
55. [~] Explorer file browser (list_dir) — DONE base
56. [ ] Tree view (nested expand/collapse, not flat cwd)
57. [ ] Open folder / workspace concept + recent workspaces
58. [ ] File ops: new/rename/delete/move (with confirm) + drag-drop
59. [ ] Fuzzy file finder (⌘P) over workspace (ripgrep/fd)
60. [ ] Global content search (⌘⇧F) results panel (ripgrep)
61. [ ] .gitignore-aware listing + file-type icons
62. [ ] Quick-open recent files + symbols (⌘⇧O)

## F. Command & Navigation
63. [ ] Command palette (⌘K) — all actions, fuzzy, keybind hints
64. [ ] Keybinding system + editable keymap + presets (vscode/vim)
65. [ ] Status bar: cwd/git/branch/LSP/encoding/line-col chips
66. [ ] Breadcrumb bar (workspace › folder › file)
67. [ ] Quick switcher between tabs/panes (⌘1-9)
68. [ ] Zen / focus mode + zoom in/out
69. [ ] Notifications/toasts center
70. [ ] Welcome / empty-state with recent + actions

## G. Panes / Layout / Tabs
71. [ ] Unified tab model (terminal | editor | scm | web | agent) draggable
72. [ ] Split any pane with any content type
73. [ ] Drag-to-reorder + drag-to-split + detach to window
74. [ ] Multiple windows (⌘N) + per-window state
75. [ ] Resizable sidebar + drawer + persisted layout
76. [ ] Right context drawer (runs / trace / agent) toggle (⌘J)

## H. Themes / Appearance
77. [~] Theme system + Solarized dark/light + Amber — DONE base
78. [ ] More built-in themes (Tokyo Night, Gruvbox, Catppuccin, Nord, Dracula)
79. [ ] System light/dark follow + per-OS accent
80. [ ] User custom theme (JSON) + live reload
81. [ ] Theme picker UI (preview swatches) in command palette
82. [ ] Font + density + chrome-opacity/blur settings

## I. Settings / Config / Persistence
83. [ ] Settings UI + on-disk config (TOML/JSON) + live reload
84. [x] Per-workspace settings override
85. [ ] Session/layout persistence across relaunch (tabs, cwds, open files)
86. [ ] Config schema validation + in-app error banner
87. [ ] Import settings from VS Code / iTerm where sensible

## J. Performance / Quality
88. [x] Stream PTY via binary channel (drop base64) for throughput
89. [x] Virtualized long lists (commit graph, search results)
90. [x] Startup time budget + lazy-load Monaco/heavy panes
91. [ ] Crash safety + panic capture + auto-restore
92. [ ] Test suite: Rust unit (git/pty parsers) + frontend component + e2e smoke

## K. Platform / Distribution
93. [ ] Cross-platform verified (macOS / Linux / Windows) CI builds
94. [~] Signed + notarized macOS build — config wired: entitlements.plist + bundle.macOS in tauri.conf.json; docs/RELEASE.md. Needs only the Apple Developer cert (env secret), no code left.
95. [x] Auto-update (Tauri updater) + release channel — plugin wired (tauri-plugin-updater), signing keypair generated, pubkey + endpoint in tauri.conf.json, `check_update` command + "Check for Updates" palette action. Verified: checks endpoint, degrades gracefully (no crash) on placeholder. Serving real updates needs a release host (docs/RELEASE.md).
96. [ ] CLI verb `anvil` (open files/dir, split, pipe into pane)
97. [ ] App icon + brand pass for the new stack

## L. DevOps Integrations (the wedge for our user)
98. [x] kubectl context/namespace chip + pod log pane
99. [x] CI status pane (GitHub Actions / GitLab) + re-run
100. [x] Observability quick-pane (Prometheus/Grafana/Loki) via embedded dashboard

---

## Execution order (top-down clusters)
1. **Terminal UX** (2,3,5,8) + **Command palette/finder** (59,63) — daily-driver feel
2. **Editor depth** (14,15,16,17,24) — IDE-grade
3. **SCM real ops** (28,29,30,31,34) — the Terax headline
4. **AI agent** (41–47) — the differentiator
5. Workspace/tabs/layout (56,57,71,74,85)
6. Themes/settings/perf/platform/devops — breadth + polish
