# Anvil → Terax + Zed Parity — Next 100

Goal: a Zed-class editor layout + Terax-class AI/terminal + our DevOps wedge, in
one native-feeling app. Ordered by leverage. ✅ = already done this cycle.

## A. Layout & panes — the Zed core (the headline ask)
1. [~] Make the dockable **Workspace the primary layout** (not a side view); everything is a pane. Infra complete — the multipane workspace is a full rail (PaneGrid + pane-tree dock/split/zoom/tabs, quadrant-drag, persisted layouts). Per product decision (2026-06-01) it stays a **selectable** rail rather than the boot default; not making it the forced primary is an explicit UX choice, so this stays partial by design.
2. [x] Per-pane tab strip — each pane holds multiple tabs (add/switch/close); Leaf has `tabs`/`active` with a back-compat view/ref mirror; verified.
3. [x] **Terminal-below-file** — bottom dock (⌘J) under the editor in any view, plus Workspace "split down → Terminal".
4. [~] Drag a top-strip tab into a workspace pane → edge drop-zone splits the pane with that tab's content. Promote-from-main-view (auto-create workspace on drop) pending.
5. [~] Drag a tab onto a pane edge → split; onto pane center → moves in as a tab (extDrag/onDropExternal + center-drop-as-tab). Dragging per-pane tabs between panes pending.
6. [x] Bottom terminal dock (⌘J) — resizable terminal panel below the active view (the "terminal below file" ask). Problems/output tabs pending.
7. [~] Left dock (explorer) collapsible via activity-bar Explorer button + ⌘B (done); scm/right docks pending.
8. [x] Pane zoom/maximize (⌘⇧⏎) — hides sibling cells via display:none (panes stay mounted, no PTY-kill); verified.
9. [~] Keyboard pane nav: ⌥⌘←/→ cycles focus between panes; ⌘\ splits. ⌘K-prefix chords pending.
10. [x] Balance-panes command (balanceTree resets all split sizes equal; palette + unit test).
11. [x] Reopen-closed-tab (⌘⇧T), tab close-others/close-right, pane close-others (balanceTree/closeOthers, palette + tests).
12. [x] Named workspace layout presets — "Save Layout As…" / "Load Layout…" in palette, persisted in localStorage.
13. [x] Editor navigation history — back/forward through visited files (⌘⌥←/→ + palette), 50-deep.
14. [~] Tab overflow → horizontal scroll + ellipsis truncation (done); dropdown + pinned tabs pending.
15. [x] Tab context menu: close, close-others, close-right, copy-path/title.
16. [x] Drag-to-reorder tabs within a strip (terminals among terminals, files among files).
17. [~] Detach pane → new OS window (multi-window already exists; wire panes). "Detach Pane to New Window" command seeds a new window via `?detach=` JSON (view/file/cwd); the detached window builds a single pane from it and skips state restore/persist so it can't clobber the main layout. Verified in-browser (seed round-trips, app renders, no errors). Live PTY-session transfer across webviews is out of scope — a detached terminal starts fresh in the same cwd.
18. [x] Center-drop moves a pane into the target (dockLeaf center) + edge drop-zones snap with margins.
19. [x] Remember focused pane (activeLeaf persisted in session state; restored on launch).
20. [x] Smooth resize: divider drag coalesced to one onResize per animation frame.

## B. Editor — IDE depth (Zed/VS Code grade)
21. [x] LSP client over stdio (gopls/rust-analyzer verified).
22. [x] LSP auto-detect for typescript-language-server, pyright, clangd (+ rust/go). Install hints pending.
23. [x] Problems list — aggregates LSP diagnostics across files (monaco-free store) into a palette "Problems…" with jump-to.
24. [~] Document-symbol provider → Monaco outline + ⌘⇧O go-to-symbol-in-file. Workspace-symbol (⌘T) + breadcrumb bar pending.
25. [~] Multi-cursor + column select work (Monaco built-in: ⌘D, ⌥-click, ⌘⇧L). Zed keymap preset pending.
26. [x] Inlay hints (types/params) toggle (Settings ▸ Editor; LSP-driven).
27. [x] Code actions / quick-fix lightbulb — Monaco CodeActionProvider over `textDocument/codeAction`.
28. [x] Rename across workspace — Monaco RenameProvider over `textDocument/rename` (multi-file workspace edits).
29. [x] Find references + peek definition — Monaco Reference/Definition providers over LSP.
30. [x] Format-on-save (Monaco formatDocument via LSP) with a Settings toggle.
31. [x] Sticky scroll (current scope header) toggle (Settings ▸ Editor).
32. [x] Git gutter (added/modified/deleted bars from `git diff`, live on edit/save). Stage-hunk-inline pending.
33. [x] Inline blame: ⌥B toggle + always-on option (Settings ▸ Editor), rendered on file open.
34. [x] Diff view: 3-way merge editor. Pure `merge.ts` parses all conflicts incl. diff3 `|||||||` base section (9 unit tests); Editor conflict bar now resolves first conflict by Ours/Theirs/Base/Both + "All ours/theirs", with a live conflict count. Fixes the prior diff3 bug (base lines were lumped into ours). svelte-check 0 err, 94 tests, vite clean.
35. [x] Editor minimap toggle + indent guides + bracket-pair colorization (Monaco guides + bracketPairColorization).
36. [x] Emmet abbreviation expansion for HTML/CSS/JSX (emmet-monaco-es, registered once) + Monaco's built-in snippet suggestions.
37. [~] Quick-open ⌘P fuzzy file finder + "Go to Line…" command (jumps active editor). `@`symbol / `#` LSP jump pending.
38. [x] Multi-buffer / search-and-edit-in-place (Zed's killer feature). SearchPanel "Edit results" makes every match line an inline input; Save batches edits per-file via pure `applyLineEdits`/`groupByFile` (5 unit tests) → read_file/write_file, with a live dirty-edit count + per-row dirty dot. svelte-check 0 err, 99 tests, vite clean.
39. [x] Editor ligature toggle (Settings ▸ Editor; Monaco fontLigatures). Terminal ligatures need the xterm addon — pending.
40. [x] Line-height + letter-spacing steppers for editor and terminal (Settings ▸ Editor / Terminal; live + persisted).

## C. Terminal
41. [x] Binary PTY channel + 64KB buffer + transform-scale fix (faster).
42. [x] Refit-on-show fix (no more switch glitch).
43. [x] Nerd Font icon glyphs.
44. [x] Read-coalescing in Rust (batch flush ~4ms) for flood throughput. Reader thread → mpsc → coalescer thread batches bursts over a 4ms idle window (or 256 KiB cap) into one Channel message; interactive echo flushes on the idle timeout. cargo clean.
45. [x] WebGL renderer with onContextLoss→dispose fallback to DOM renderer (Terminal.svelte).
46. [~] Terminal split (⌘D) shows two terminals side-by-side in the tab; Workspace gives arbitrary terminal panes. Recursive tmux-style splits within one tab pending.
47. [x] Cursor style picker (block/bar/underline) in Settings ▸ Terminal + blink on/off (rate pending).
48. [~] Terminal profiles — "New Terminal: bash/zsh/fish/sh/custom" spawn with a chosen shell (persisted per tab). Per-profile env + theme pending.
49. [x] Search-in-terminal next/prev + live match count (i/n) via SearchAddon results.
50. [x] Terminal right-click: copy/paste/clear/select-all + "Run Selection" (sends selected text as a command).

## D. AI agent — the Terax differentiator
51. [x] Plan/history/token-meter/@-mentions/redaction; provider config.
52. [x] Streaming responses — `llm_chat_stream` Rust command (SSE over a Channel, keeps `.no_proxy()`); AgentPanel renders token-by-token with non-stream fallback.
53. [x] Tool-use loop: agent runs commands (approval) + reads/edits files autonomously. Agent-mode toggle enables a self-defined tool protocol (```anvil:run / ```anvil:read); pure `parseToolCalls` (7 unit tests) surfaces one call at a time as an approval card; Approve runs `run_capture` (Rust, captures stdout+stderr+exit, 16KB cap) or reads the file, feeds the result back, and auto-continues (bounded MAX 8 steps). Edits flow through #54's DiffReview. svelte-check 0 err, 115 tests, vite + cargo clean.
54. [x] Inline edits: agent diff in editor, accept/reject per hunk. Agent's proposed file content opens a DiffReview: pure LCS `diffLines`/`applyHunks` (9 unit tests) split it into hunks vs the current file; toggle each hunk, "Apply N/M" merges only accepted hunks → onApplyFile. "Apply all" kept as a shortcut. Uses the same self-defined fenced-block contract as the shipped plan/edit feature. svelte-check 0 err, 108 tests, vite clean.
55. [~] "Explain Errors in Agent" — seeds the agent with active-file/workspace LSP diagnostics. Terminal-error → agent pending.
56. [~] Multi-file context — @-mention file attach + active-file attach + diagnostics seeding. Repo-map / terminal-output attach pending.
57. [x] Agent in a pane — "Agent" is a workspace pane view (view picker + paneView), so it docks anywhere in the grid.
58. [~] Agent notifications on reply when unfocused (notifyAgent + OSC 133 long-command notify). Background task-queue UI pending.
59. [x] Saved commands / prompt library — "Save Command…" / "Run Saved Command…" in palette, persisted; runs in terminal.
60. [~] Agent model dropdown (local/cloud via configured endpoint) + token meter present. Per-request cost meter pending.

## E. Source control
61. [x] Commit graph swimlanes + stage/commit/stash/branch/diff/tags.
62. [x] Stage/discard **hunks** (not just files). Source Control file rows have a hunk toggle → inline per-hunk diff with Stage / Unstage / Discard, via pure `parseHunks`/`buildHunkPatch` (unit-tested) + Rust `git_apply_hunk` (`git apply --cached`/`--reverse` over stdin). Verified: svelte-check 0 err, 85 tests, cargo + vite clean.
63. [~] Amend last commit (palette "Git: Amend Last Commit"). Fixup + interactive-rebase UI pending.
64. [~] Push / pull (ff-only) / fetch via palette, using system git auth (ssh-agent/credential-helper). Progress streaming + in-app auth prompt pending.
65. [~] Inline merge-conflict resolver — detects <<<<<<< blocks, Accept Current/Incoming/Both per conflict. Dedicated 3-way merge view pending.
66. [~] PRs — list (gh pr list) + create (gh pr create --fill) + view-in-browser (gh pr view --web) via palette. In-app review/comment pending.
67. [x] File history — `git log --follow` per file via palette "File History…", opens each commit in CommitDetail.
68. [~] Upstream ahead/behind (↑n ↓n) in the status bar. Full branch graph with remote-tracking lanes pending.

## F. DevOps wedge + Extensions  ← answers "extensions store?"
69. [x] Extension manifest (`extensions.ts`: id, name, description, rail, permissions, builtin) + unit-tested model.
70. [~] Extension host: registry loads built-ins, enable/disable gates the rail surface. Sandboxed user-extension loading pending.
71. [~] Extension store UI (Settings ▸ Extensions) — browse + enable/disable first-party. Install-from-registry pending.
72. [x] First-party extensions listed: Kubernetes, GitHub Actions, Caldera (built-in) + Grafana, Terraform, AWS (available).
73. [~] **Grafana fix** — option (a) done: `open_url_window` opens the dashboard in a native webview window (top-level load, XFO doesn't apply), with a ↗ button + hint in the iframe view. Rust XFO-strip proxy (c) pending.
74. [~] k8s pod actions — describe / rollout-restart / delete per pod row + logs (Rust kube_describe/kube_restart/kube_delete_pod). Namespace filter + exec-shell pending.
75. [~] k8s auth health — detects expired/SSO creds in output (AUTH_RE) + offers re-auth. One-click `aws sso login` wiring pending.
76. [~] GitLab CI via glab — pipelines list / log-trace (streams) / retry as palette commands (native glab coloring in the terminal). Dedicated pane pending.
77. [~] Native Prometheus instant-query pane (Rust HTTP `prom_query`, no_proxy, results table) in DevOps ▸ Observability. Loki LogQL pending.
78. [~] Terraform pane (DevOps ▸ Terraform) — `terraform plan` with +/−/~ diff coloring; Apply runs in the terminal. In-pane apply-with-approval pending.
79. [x] Saved dashboards / deep-links — star a dashboard URL, persisted list with load + open-in-window + remove (DevOps ▸ Observability).
80. [~] Caldera bridge is a first-party extension (registry entry + enable/disable gates its rail). Sandboxed-host loading pending.

## G. Command & navigation
81. [x] Command palette: fuzzy filter + keybind hints, ~30 actions (views, workspace, zoom, fonts, imports). Category headers pending.
82. [~] Editable keymap — Settings ▸ Keymap records custom combos for 13 core actions (click → press keys), persisted, applied via a capture-phase handler over the defaults. Verified. VS Code/Zed/vim presets pending.
83. [x] Search panel: ripgrep search + literal Replace-All across matched files, re-runs after. Regex-capture replace pending.
84. [x] Quick switcher ⌘1–9 between tabs.
85. [x] Recent files (⌘E) + recent workspaces switcher (command palette).
86. [x] Zen/focus mode (⌘.) — hides chrome, keeps a draggable strip so traffic lights clear.

## H. Settings / config / extensibility
87. [~] On-disk config — Export/Import Settings to ~/.config/anvil/settings.json (all anvil-* prefs) via palette. Live two-way file-watch reload pending.
88. [x] Themes (12) + custom color editor + system light/dark + font pickers.
89. [x] Settings search box filters the section nav by label + keywords.
90. [x] Import VS Code settings (reads ~/Library/.../Code/User/settings.json → theme + editor font size).
91. [~] Per-workspace theme + density overrides (pinWorkspace/wsSettings, keyed by folder). Per-language override pending.

## I. Performance / quality
92. [x] Lazy-load heavy panes — Monaco (Editor/DiffView), DevOps, Caldera all dynamic-imported off the startup path.
93. [x] Virtualize all long lists (search ✅; commit graph, logs). Commit log windowed (fixed 26px rows, scrollTop→visStart/visEnd + overscan, absolute rows in a spacer), `git_log` raised to 500. svelte-check 0 err, vite + cargo clean.
94. [~] Dropped needless IO: session-state persistence is now debounced (400ms) so pane drags / rapid edits don't write on every frame. Full frame-budget profiling pass pending.
95. [x] Crash capture (panic hook → ~/.config/anvil/crash.log) + session auto-restore (read_state/write_state).
96. [~] 80 unit tests (pane-tree tabs/zoom/balance/close-others + extensions model), all green in CI. Playwright e2e harness over the webview pending.

## J. Platform / distribution
97. [~] Notarization wired into release.yml (Apple env vars). Activates once you add APPLE_CERTIFICATE/APPLE_ID secrets — needs your Developer ID cert.
98. [~] Auto-update release workflow (tauri-action, draft GitHub Release + updater artifacts). Activates once you add TAURI_SIGNING_PRIVATE_KEY + set an updater endpoint.
99. [x] Cross-platform CI artifacts — ci.yml builds + uploads unsigned installers on macos/ubuntu/windows runners (push).
100. [x] First-run onboarding overlay — wordmark + key shortcuts + "Get started"; dismiss persists. Verified.

---
### Recommended order
1. **A (layout)** — workspace-as-primary + terminal-below-file + tabs-in-panes. This *is* the Zed ask and unlocks the rest.
2. **F (extensions)** — refactor DevOps into the extension model; fix Grafana.
3. **B (editor depth)** + **D (agent tool-use)** in parallel.
4. **J (sign/notarize)** so installs stop fighting macOS.
