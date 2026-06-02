---
status: active
type: log
created: 2026-05-21
updated: 2026-05-31 (unified UI scale Task 4 shipped)
sources: []
confidence: high
---

# Wiki Log

- 2026-06-02 — Added `exec_capture` to `shared.rs`: kills hung child after a configurable
  deadline using a poll loop + drain threads, returns `io::Error(TimedOut)` on timeout.
  Replaced all external-CLI `.output()` calls across `git.rs` (8 sites), `kube.rs` (1 — `kubectl`
  helper), `iac.rs` (6 — `tf_exec`, `helm`, `terraform_plan/state/apply`), `ci.rs` (19 — all gh/glab
  commands), `cloud.rs` (1 — `aws_list`), `lib.rs` (3 — `run_capture`, `repo_import_graph`, `grep`).
  Timeouts: 25s reads/lists; 30s write/trigger actions; 60s helm helper; 120s `run_capture` agent
  shell; 180s git network + terraform long ops. Left untouched: `security` CLI (macOS Keychain),
  `git apply` stdin pipe in `git_apply_hunk`, `tf_detect` version probes. `cargo build` clean;
  9 tests pass; 0 new clippy warnings; `cargo fmt` applied.

- 2026-06-02 — Code-health sweep: fixed all svelte-check diagnostics (was 1 error + 60 warnings,
  now 0/0). Changes: (1) installed `@types/node` devDep + removed now-redundant `@ts-expect-error`
  in `vite.config.js`; (2) replaced `<svelte:self>` with self-import `PaneGrid` in
  `src/lib/PaneGrid.svelte`; (3) removed unused `.hint` CSS selector in `src/routes/+page.svelte`;
  (4) added `role="button"`, `tabindex="0"`, and `onkeydown` (Enter/Space) handlers to all 29
  interactive `<div>`/`<span>` elements in `+page.svelte` that were missing a11y attributes
  (tab items, close × buttons, rail nav items, explorer session rows, bottom dock close, status
  bar toggles, zen-exit). No behavior or visual changes. vite build clean; 486/486 vitest pass.

- 2026-06-02 — Restructured `src/lib/SourceControl.svelte` into a two-pane layout (VS Code /
  Zed style). Left pane (`aside.scm-side`, 300px fixed): commit composer at top, changed files
  (Staged + Changes) scrollable below. Right pane (`main.scm-main`, flex:1): branch header,
  filters, Stashes, Tags, History graph, commit-detail panel. Moved `{#snippet tree(...)}` to
  top-level inside `.scm` so both panes can render it. Removed `.scm-col` max-width centering.
  `.scm` is now `flex-direction:row`. No script logic changed. svelte-check: 0 new errors/warnings
  in SourceControl.svelte; vite build clean; vitest 486/486 pass.

- 2026-06-02 — Converted all blocking Tauri commands to `async fn` with `spawn_blocking`.
  Modules changed: `git.rs` (38 commands), `kube.rs` (15 commands), `iac.rs` (15 commands),
  `ci.rs` (15 commands), `cloud.rs` (7 commands; `set_aws_profile`/`set_github_token` left sync
  — they only write to a Mutex with no I/O, and changing their return type would break the API),
  `fs.rs` (7 commands; `pick_folder`/`pick_file` left sync — rfd::FileDialog requires main thread),
  `lib.rs` top-level (8 commands). Tests in `lib.rs` updated to use
  `tauri::async_runtime::block_on`. `cargo build` clean; 9 tests pass; 3 pre-existing clippy
  warnings only (lsp.rs/pty.rs); `cargo fmt` applied.

- 2026-06-02 — Refactored `src-tauri/src/lib.rs` (2640 lines) into 10 domain modules.
  `lib.rs` is now a thin entry point (715 lines). New modules under `src-tauri/src/`:
  `git.rs` (470 lines, 39 commands), `pty.rs` (185 lines, 5 commands + PtyState struct),
  `llm.rs` (117 lines, 3 commands), `iac.rs` (243 lines, terraform + helm commands),
  `kube.rs` (236 lines, 17 commands), `observability.rs` (64 lines, 3 commands),
  `cloud.rs` (173 lines, 8 commands + gh_cmd helper), `ci.rs` (293 lines, glab + gh commands),
  `fs.rs` (144 lines, 10 commands + Entry struct), `window.rs` (34 lines, 2 commands),
  `shared.rs` (8 lines, aws_profile() helper used by kube/iac/cloud).
  Pure structural move: zero behavior changes. All 9 tests pass; cargo build clean;
  clippy warnings match pre-existing baseline (3 in lsp.rs and pty.rs); cargo fmt applied.
  Handler list identical: all commands remain registered under same names.

- 2026-06-01 — De-gimmick pass: removed 13+ decorative accent uses. Accent now only on cursor +
  selected/active item. Changes: `--radius` 8→6px; `.onboard` solid panel bg, no backdrop-filter,
  4px radius; `.ob-wm`/`.ob-dot`/`.ob-dots`/`.ob-d` CSS removed; dot-nav replaced with `ob-step`
  text; `.ob-go` flat outlined button; `.quake` solid panel bg; `agentq-chip` solid panel bg;
  `zen-exit` border removed; `.tab .dot` markup + style removed; `📌` emoji → `<Icon name="pin">`;
  `.rail .i.on` background cleared; `.row.cur` → `var(--sel)`; `.pane-head .accent` already
  `var(--text3)`; Workspace pane-head hint string removed; branch status span `accent` class
  removed; `FileBrowser.cwd` uppercase/letter-spacing removed; `FileBrowser .ic.folder` →
  `var(--text3)`; `SearchPanel .loc` → `var(--text3)`; search input `border-radius` 8→4px;
  `Palette.svelte` backdrop-filter removed, solid panel bg, 6px radius. Build: vite clean,
  svelte-check 1 pre-existing error (vite.config.js), vitest 486/486 pass.

- 2026-06-01 — Added `src/lib/Kube.svelte`: standalone Kubernetes dashboard component extracted
  from the k8s tab in `DevOps.svelte`. Props: `cwd`, `onRunCommand`. Wires all 15 existing Tauri
  commands verbatim (`kube_contexts`, `kube_current_context`, `kube_use_context`, `kube_namespaces`,
  `kube_current_namespace`, `kube_set_namespace`, `kube_pods`, `kube_logs`, `kube_logs_selector`
  unused pending selector UI, `kube_describe`, `kube_delete_pod`, `kube_restart`, `kube_pf_list`,
  `kube_pf_start`, `kube_pf_stop`). Layout: 28px topbar (context+namespace selects, refresh),
  collapsible port-forwards strip, sticky-header pod table (status dot, name mono, ready, restarts
  red if >0, status, age), hover-revealed per-row action buttons (logs/describe/exec/port-forward/
  restart/delete), inline 40% split panel for log+describe output with close button, auth-error bar
  with suggested fix commands. DevOps.svelte left unchanged. Build: vite green; svelte-check 1
  pre-existing error (vite.config.js), vitest 486/486.

- 2026-06-01 — Added GitLab CI page: 5 new Rust commands in `src-tauri/src/lib.rs`
  (`glab_pipelines_json`, `glab_pipeline_jobs`, `glab_job_trace`, `glab_pipeline_retry`,
  `glab_pipeline_cancel`) using `glab api` with `current_dir(cwd)` so glab resolves `:id`
  from the repo remote. All 5 registered in `tauri::generate_handler!`. New component
  `src/lib/CI.svelte`: three-level layout matching Kube.svelte conventions (22px rows,
  CSS-grid columns, `color-mix` hover, `var(--sel)` selection, 5px/4px radius buttons).
  Pipeline list polls every 5s; jobs poll while pipeline is running; job trace polls every
  4s and scrolls to bottom. Actions: retry / cancel (with `askConfirm`) / open-in-GitLab.
  Error state shows `var(--text3)` hint with `glab auth login` suggestion. `document.hidden`
  guard on all poll fetches. Build: cargo build green; vite build green; svelte-check 1
  pre-existing error (vite.config.js); vitest 486/486 passed.

- 2026-06-01 — DevOps.svelte trimmed: removed k8s, ci, tf, helm, and obs tabs (each now has a
  dedicated page). Kept prs, gitlab, aws, inc. Removed state/functions: `contexts`, `current`,
  `namespaces`, `currentNs`, `logSelector`, `pods`, `logPod`, `logs`, `podRows`, `openLogs`,
  `multiplexLogs`, `describePod`, `deletePod`, `restartPod`, `statusColor`, `k8sAuthErr`, `execPod`,
  `pfList`, `k8sErr`, `refreshPf`, `portForwardPod`, `stopPf`, `loadK8s`, `useCtx`, `useNs`,
  `runs`, `runRows`, `loadCI`, `rerun`, `runLog`, `runLogText`, `viewRunLog`, `runColor`, `ciAuthErr`
  (rebuilt as prs-only), `tfPlan`, `tfBusy`, `runTfPlan`, `runTfState`, `runTfApply`, `tfClass`,
  `helmRows`, `helmErr`, `helmValues`, `loadHelm`, `helmAllValues`, `helmCur`, `showValues`,
  `toggleHelmAll`, `obsUrl`, `savedDashboards`, `loadDash`, `persistDash`, `saveDash`, `removeDash`,
  `saveObs`, `secSource`, `secKey`, `secVal`, `secErr`, `secReveal`, `readSecret`, obs-specific
  `$effect`s (lines 182–183). Kept inc tab intact with full prom/loki engine. `onMount` now only
  calls `applyCreds()`. `cwd` prop retained (used by prs bar labels, `loadPRs`, `openPr`,
  `postPrComment`, `loadGlab`). 794→340 lines. Build: vite green; svelte-check 1 pre-existing
  error (vite.config.js); vitest 486/486 passed; cargo build green.

- 2026-06-01 — Perf: lazy-loaded SourceControl, AgentPanel, SearchPanel in `src/routes/+page.svelte`.
  Converted 3 static imports to `const X = () => import(...)` and wrapped all 6 render sites in
  `{#await X() then M}<M.default ... />{/await}`. Render sites changed: line 1328 (scm rail),
  1353 (search rail), 1377 (pane scm), 1379 (pane search), 1381 (pane agent), 1418 (agent drawer).
  Main startup chunk: 1,166,961 → 1,110,150 bytes (−56,811 bytes, ~4.9%). New split-out chunks:
  SourceControl (~31.5 kB), AgentPanel (~12.9 kB), SearchPanel (~3.9 kB + 3.1 kB shared).
  svelte-check: 1 pre-existing error (vite.config.js), 84 pre-existing warnings (unchanged).
  vitest 486/486 passed.

- 2026-06-01 — AgentPanel.svelte: converted chat-bubble message list to flat operator log.
  Removed `.bubble-row`, `.bubble-row.user`, `.bubble-row.assistant`, `.bubble-row.user .bubble`,
  `.bubble-row.assistant .bubble`, and `.bubble` CSS (asymmetric `border-radius: 14px 14px 4px 14px`
  bubble style). Replaced with `.msg-row` (grid, `hairline` rule separator), `.msg-label` (mono 10px
  uppercase `you`/`agent` prefix; agent label tinted `--purple`), and `.msg-body` (full-width, 12.5px,
  line-height 1.5). No logic changes. Build: vite build green; svelte-check 1 pre-existing error
  (unchanged); vitest 486/486 passed.

- 2026-06-01 — Replaced all blocking browser `prompt()`/`confirm()` calls in FileBrowser,
  DevOps, SourceControl, and cm-lsp with a themed in-app modal system. New files:
  `src/lib/dialog.ts` (store-driven `askText`/`askConfirm` API) and `src/lib/Dialog.svelte`
  (glass/scrim modal matching WhatsNew styling). `<Dialog />` mounted once at app root in
  `src/routes/+page.svelte`. Call sites converted: FileBrowser newFile/newFolder (2×askText),
  DevOps runTfApply/deletePod/restartPod (3×askConfirm danger), saveDash/savePromQuery
  (2×askText), portForwardPod (1×askText), SourceControl stashWithMessage (1×askText),
  cm-lsp renameKey (1×askText). Build: vite build green; svelte-check 1 pre-existing error
  (unchanged); vitest 486/486 passed.

- 2026-06-01 — Production robustness pass: audited ~62 silent error catches across src/lib and
  src/routes. Converted 9 to `toast(..., "error")` (file save, file open, new-window × 4,
  open_url_window × 2, AWS profile switch) and 13 to `console.warn` (clipboard, pty_set_active,
  cd auto-cd, list_dir × 2, kube_pf_stop, set_aws/gh creds, git_log_range, git_diff,
  git_last_message, link open, selection copy). Left all localStorage parse fallbacks, polling
  background ops, LSP best-effort, and loop-per-file skips silent. Build: vite build green;
  svelte-check 1 pre-existing error (unchanged); vitest 486/486 passed.

- 2026-05-31 — Generated real app icon set (roadmap #97): stylized orange anvil mark
  (#f5874f) on near-black rounded-square (#1a1518), rendered 1024x1024 via headless Chrome,
  built into `src-tauri/icons/icon.icns` (94 KB) plus `32x32.png`, `128x128.png`,
  `128x128@2x.png`, and `icon.png`. No `tauri.conf.json` changes needed — paths already matched.

- 2026-05-31 — Added 5 built-in themes to `src/lib/themes.ts` (Tauri branch, roadmap #78):
  `tokyo-night`, `gruvbox-dark`, `catppuccin-mocha`, `nord`, `dracula`. Each fills all 16
  `ui` keys and all 16-color `xterm` ITheme keys using canonical published palette hex values.
  `cycleTheme` already used `Object.keys(themes)` so no logic change needed. `vite build`
  passed (26.89s); `svelte-check` reported 0 errors / 25 pre-existing warnings.

- 2026-05-31 — Decision 0005 amended: web content pane shipped. The escape hatch
  in 0005 ("a webview is permitted only as the content of a dedicated pane type")
  is now active. A live WKWebView `.web` pane landed on branch `zig` (commits
  d7e73bf..44f3c07): AppKit NSView subview positioned by `anvil_web_*` shim exports,
  sibling to the Metal layer (not composited into Metal). App chrome remains 100%
  native Metal; a Metal header strip carries the URL bar and nav controls. Thin v1
  bridge: navigate/back/forward/reload (C-ABI) native→web; KVO + crash delegate
  web→native. Ephemeral data store; http/https-only URL validation. Invoked via
  "Open Browser Pane" command-palette action. Build/test/--dump green. Core decision
  (native Metal host, no WKWebView chrome) unchanged. `wiki/decisions/0005-render-host.md`
  updated.

- 2026-05-31 — Web pane Tasks 4+5: WKWebView shim lifecycle + app.zig handle map and event
  bridge. `build.zig`: added `linkFramework("WebKit", .{})`. `src/platform/shim.m`: added
  `#import <WebKit/WebKit.h>`, `AnvilWebDelegate` (navigation/crash), `AnvilWebObserver`
  (KVO: title/estimatedProgress/canGoBack/canGoForward), and the six C exports
  (`anvil_web_create`, `anvil_web_navigate`, `anvil_web_back`, `anvil_web_forward`,
  `anvil_web_reload`, `anvil_web_set_frame`, `anvil_web_set_hidden`, `anvil_web_destroy`).
  `src/app.zig`: added `webpane` import, eight extern declarations, `WebHandle` type +
  `web_handles`/`web_handle_count` module state, `webHandleFor`, `reconcileWebPanes`,
  `anvil_web_event` export, and the `reconcileWebPanes(pane_buf[0..np])` call at the end
  of the `anvil_frame` pane loop. Commit 46ed082.

- 2026-05-31 — Web pane Task 6: URL-bar header strip, nav buttons, focus + key routing; device-px→pt
  frame scale fix. `src/web/pane.zig`: `header_strip_h` corrected from 30→34 (matches
  `chrome.header_strip_h`). `src/platform/shim.m`: added `gMetalView` global (assigned at view
  creation); `anvil_web_set_frame` now divides by `backingScaleFactor` before `NSMakeRect`
  (Fix A); added `anvil_focus_metal_view` and `anvil_web_focus` shim fns; added
  `anvil_web_urlbar_focus` extern prototype; wired Cmd+L to `anvil_web_urlbar_focus`. `src/app.zig`:
  added `web_urlbar_active/buf/len` module state; `.web` branch in `anvil_input` (URL-bar editing:
  Enter/Escape/Backspace/printable); `anvil_web_urlbar_focus` export; `.web` early-return in
  `anvil_mouse` (nav-button hit-test in header strip, body click gives WKWebView first-responder);
  `.web` branch in `emitPanelHeaders` (back/forward/reload icons 0xF053/0xF054/0xF021, URL text,
  urlbar edit buffer); loading progress rect added to `emitShellRects`. Metal view global: `gMetalView`.
  Commit 3c1a54c.

- 2026-05-31 — Editor gutter Task 2: render gutter + content shift. `src/editor.zig`:
  added `drawGutter` private helper (right-aligned 1-based line numbers, current line
  bright `.default`, others dim `.indexed = 8`, pad space at `gw-1`). `render` now
  computes `gw` from `gutterWidth()` clamped to `cols-1`, calls `drawGutter` per visible
  row, starts the content column at `gw` instead of 0, and offsets `scr_col` by `gw`.
  Updated "render writes cells and places the cursor" test (digit at col 0 for gw=2,
  content at col 2, cursor cx=4). Added new test "gutter shows right-aligned line numbers,
  current line brighter" (12-line file, gw=3, right-alignment, fg contrast check).
  Updated "render keeps a manual scroll" test (content shifted to col gw=2).
  Two session.zig ripple tests remain failing — deferred to Task 4.
  All editor.zig tests pass; format clean.

- 2026-06-01 — Added 9 themes to `src/lib/themes.ts` (Tauri branch): `monokai`,
  `everforest`, `kanagawa`, `night-owl`, `tokyo-night-storm`, `ayu-dark`, `one-light`,
  `oxocarbon`, `gruvbox-material`. Added `THEME_LABELS` map and `themeLabel()` helper.
  Updated user-facing label positions to call `themeLabel()` in 3 files: theme card
  `<span>` and system light/dark `<option>` elements in `src/lib/Settings.svelte`;
  status-bar theme chip in `src/routes/+page.svelte`. Stored keys unchanged.
  `svelte-check`: 0 errors / 74 warnings (all pre-existing). `vite build`: success.

- 2026-06-01 — Added Vitest coverage tooling and 6 new test files for pure-logic modules
  (Tauri branch). Installed `@vitest/coverage-v8`. Updated `vitest.config.ts`: added
  `$lib` path alias and `coverage` block (provider v8, text/json-summary/html reporters,
  include `src/lib/**/*.ts`). Added `test:coverage` script to `package.json`.
  New test files: `keymap.test.ts` (comboOf, comboFor, setKeyOverride, clearKeyOverride,
  applyKeymapPreset), `redaction.test.ts` (applyRedaction built-in masking, user rules,
  security intent), `command-history.test.ts` (feedInput commit/dedupe/navigate, backspace,
  Ctrl-C/U, bounds), `command-blocks.test.ts` (recordPrompt/recordExit/getBlocks/clearBlocks,
  lastExit store), `fonts.test.ts` (monoStack/uiStack correctness for all MONO_FONTS/UI_FONTS,
  store validity), `themes.test.ts` (themeLabel known/unknown, isLight classification, LIGHT/DARK
  coverage, THEME_LABELS completeness). `cm-color.ts` skipped — only exports a CodeMirror DOM
  extension; `toHex` is not exported and requires DOM widgets. Baseline: 121 tests / Statements
  30.89% / Branches 24.6% / Functions 20.33% / Lines 32.14%. Final: 195 tests / Statements
  39.19% / Branches 30.68% / Functions 28.69% / Lines 40.78%. `svelte-check`: 0 errors /
  74 warnings (all pre-existing).

Append-only history of durable wiki, decision, source, and handoff operations.

- 2026-05-30 — Smooth scroll fix (`zig` branch, `src/main.zig`). Two bugs found and fixed: (1) NSTimer replaced with CADisplayLink so the render tick is phase-locked to hardware vsync — NSTimer drifted and missed vsync slots during inertial scroll; (2) `onScroll` divisor changed from hardcoded `/ 8.0` to `hasPreciseScrollingDeltas`-aware dispatch: trackpad (precise) divides `dy` by `cell_h / scale` (cell height in points) for 1:1 finger tracking; mouse wheel multiplies by 3.0 for standard 3-line-per-notch behaviour. All 342 tests pass.

- 2026-05-30 — Decision pages: created `wiki/decisions/native-file-viewer.md` (Session.Kind=viewer, src/fileview.zig 2 MiB cap + NUL binary detection, src/syntax.zig Role/Token/Lang tokenizer, fillGrid/roleColor grid reuse, entry via explorer click and `anvil view` IPC verb, q/Esc close) and `wiki/decisions/mineral-warm-palette.md` (Mineral Warm 2026-05-30 palette — warm near-black backgrounds, coral-rose mineral accent #c2614a, burnt orange ember #d4601e, three-surface cohesion chrome/ANSI/syntax, variant dependency on `theme_variant = "mineral"`). Contradiction flagged: 0003 hex values are superseded. `wiki/index.md` updated with both entries.

- 2026-05-30 — Mineral Warm palette redesign. Token swap across three source files
  plus BRAND.md reconciliation. `src/chrome.zig`: all standalone palette consts
  (graphite, charcoal, ash, ash_soft, alloy, mist, bone, line, hover, mineral, ember,
  verified, attention, agent) replaced with warm-family hex; `surface_dark` inherits
  via the consts; `surface_light` fields updated to warm light tokens (#f2ece4 canvas,
  #fdf6ee panels, warm alloy/mist/bone/line/hover). `src/render/theme.zig`:
  `mineral_dark` and `mineral_light` full replacement — new bg/fg/bar/separator/sel_bg/
  sel_fg and all 16 ANSI entries per the design spec; `mineral_high_dark` bar set to
  #070504 (darker than graphite, distinct from mineral_dark.bar #1c1614) and
  `mineral_high_light` bg set to #fdf6ee (panels, distinct from mineral_light.bg
  #f2ece4) to preserve test invariants. Test "hex parses brand tokens" updated to new
  graphite #0e0b0a and canvas #f2ece4. `editors/nvim/colors/anvil.lua`: dark and light
  palette tables replaced; highlight-group section untouched. `BRAND.md`: Mineral Warm
  description paragraph added; Core Materials, Operational Accent, Semantic Status
  tables updated; color rules mineral/ember lines reworded; logo approved-uses line
  updated; Updated date set to 2026-05-30. All tests pass; --dump renders clean.

- 2026-05-30 — Smooth/interpolated scrollback scrolling. Exp-decay animation
  for scroll position, mirroring the existing cursor animation pattern.
  `src/vt/terminal.zig`: `viewRowAt(base, r)` added next to `viewRow`; reads cells
  as if `view_offset` were `base`; returns empty slice when logical index >= totalLines
  (guards the off-by-one on the extra row). Two new tests.
  `src/render/renderer.zig`: `buildInstances` gains `y_shift: f32` and `base: usize`
  parameters; draws `rows+1` rows when `abs(y_shift) > 1e-3` (the extra bottom row
  reveals the partial strip as the grid glides); uses `viewRowAt(base, r)` to avoid
  mutating `term.view_offset`; all 5 existing tests updated to pass `0, t.view_offset`.
  Two new tests: y_shift slides positions; extra row emitted at correct y.
  `src/render/instance.zig`: `FrameData` gains two fields appended at the end:
  `pane_ranges: [*]const PaneRange` and `pane_range_count: u32`. New `PaneRange`
  extern struct: `{offset, count, x, y, w, h}` (device px scissor rect + instance
  slice bounds). `src/config.zig`: `scroll_smooth: bool = true` added after
  `cursor_smooth`; parsed with identical true/false/error pattern; 1 new test.
  `src/app.zig`: `pane_range_buf: [max_panes]PaneRange` global; `scr_anim_{off,id,
  init,last_ms}` state vars; `animateScroll(target, id, rows) -> f32` (same tau=0.028,
  max_dt=64ms, snap on first/id-change/large-jump with snap_lines=grid.rows, settle
  threshold=0.5/cell_h, idle-correct: returns without markDirty on settle); pane loop
  in `anvil_frame` computes off_f/base/frac/y_shift per pane and populates
  pane_range_buf with per-pane scissor rects (y >= bar_h, height clamped); 5 new
  tests for animateScroll + floor/frac split.
  `src/platform/shim.m`: `PaneRange` typedef added before `FrameData`; two new fields
  mirrored in C struct. Cell draw pass in both `render()` and `anvil_dump` replaced:
  when `pane_range_count > 0`, loops over pane_ranges emitting one `setScissorRect` +
  `drawPrimitives` per pane (clamped to drawable bounds to avoid Metal validation
  error); resets scissor to full drawable before divider/overlay/palette passes.
  Falls back to original single-batch draw when pane_range_count == 0. On the first
  (static) frame scr_anim_init=false → snaps → frac=0 → no extra row, no y_shift,
  pane scissor covers full pane → render check unchanged.
  Render check: `ok: 1600x1000, bg=96.0%, bar=48.8%` (bg unchanged; bar% is run-
  to-run content variance, check requires >= 30%).

- 2026-05-30 — Native read-only file viewer (Metal grid). New `src/syntax.zig`
  (Role/Token/Lang/detect/tokenizeLine; hand-written tokenizers for zig/toml/md/sh/lua
  and a generic fallback; 4 unit tests). New `src/fileview.zig` (load with 2 MiB cap +
  binary NUL detection; splitLines; libc fopen/fread; 4 unit tests).
  `src/session.zig`: Kind enum (.shell/.viewer); Session gains kind/view_bytes/view_lang/
  view_alloc fields; initViewer (no PTY); poll no-ops for viewer; write no-ops for viewer;
  fillGrid tokenizes retained bytes and prints colored cells (roleColor maps Role→ANSI
  index: keyword=13, string=2, number=3, comment=8, type=6, punct/text=default); resize
  re-fills grid for viewer kind.
  `src/pty.zig`: initNull() sentinel PTY (master=-1, pid=-1); deinit guards on master<0.
  `src/session_manager.zig`: addViewer(path, rows, cols) — loads file, detects lang,
  fills grid, falls back to "cannot open <name>" placeholder on error/binary.
  `src/ipc.zig`: ViewArg struct; view added to Command union; parseRequest parses
  "view <path>\n"; 2 new tests.
  `src/main.zig`: open verb sends "view" IPC (native viewer); new edit verb sends old
  $EDITOR-in-a-pane path as escape hatch.
  `src/app.zig`: drainIpc .view arm (splits pane, focuses viewer); explorer file-click
  opens viewer instead of writing filename to shell; anvil_input kind-gate (q/Esc closes
  viewer pane, other keys swallowed).
  `src/session_persist.zig`: convertNode skips viewer cwd (saves empty string → restore
  spawns plain shell).
  `docs/product/console-rebuild-plan.md`: M5 updated from webview editor to native
  file viewer; edit verb noted as nvim escape hatch.
  All tests pass. --dump renders clean (90 KiB PNG).

- 2026-05-30 — Background translucency + hairline dividers (aesthetic chrome).
  Item A: `src/config.zig` adds `background_opacity: f32 = 1.0` field; parsed with
  range validation 0.0–1.0 matching the `padding_x`/`padding_y` pattern; 2 new tests.
  `src/render/instance.zig` `FrameData` gains `bg_alpha: f32` after the `bg[3]` field.
  C `FrameData` in `src/platform/shim.m` mirrored with `float bg_alpha`. `src/app.zig`
  adds `divider_draw_px = 2` constant (alongside existing `divider_px = 2` gap/hit),
  `effectiveBackgroundOpacity()` (returns 1.0 for light variants; clamps dark-variant
  value to [0.75, 1.0]), and sets `bg_alpha` in `anvil_frame`'s FrameData init.
  `render()` in shim.m uses `fd.bg_alpha` for the MTLRenderPassDescriptor clear-color
  alpha. `anvil_dump` keeps its own `1.0` alpha — golden render check unchanged.
  Window setup in `anvil_run`: `win.opaque = NO`, `win.backgroundColor = clearColor`,
  `gLayer.opaque = NO`; a `NSVisualEffectView` (UnderWindowBackground / BehindWindow /
  Active) is added as the backmost subview of a new `NSView` content host, with
  AnvilView layered on top. At default opacity 1.0 the opaque clear color covers the
  blur completely; no visual change for existing users.
  Item B: divider drawing in both `render()` and `anvil_dump` now centers a 2px
  hairline (kDrawPx = 2.0f, 1 logical pt at 2x Retina) within the gap rect using
  `(gap - 2.0) * 0.5` offset on the thin axis. Hit zone (layout gap = divider_px = 2)
  unchanged. Render check: `ok: 1600x1000, bg=96.0%, bar=47.6%` — no visual change.

- 2026-05-30 — Dirty-frame coalescing. Idle terminal now costs ~0% GPU; only
  renders when visible state changes. `src/session.zig`: `Session.poll()` returns
  `PollResult{alive,consumed}` instead of `bool`; new unit test asserts
  `consumed=true` on data and `alive=false` on eof. `src/app.zig`: added
  `frame_dirty` bool (starts true) + inline `markDirty()`; `anvil_needs_render()`
  export (read-clears); `anvil_force_render()` export (just marks dirty);
  `currentBlinkPhase()` factored from `cursorVisible()`; `blinkActive()` /
  `pollBlink()` / `last_blink_phase` for edge-detection at the half-period flip
  only when a blinking cursor is visible; `last_forced_ms` + 2s periodic safety-
  net in `anvil_poll`; caldera drawer marks every tick while open. Dirty sources
  instrumented: `anvil_input`, `anvil_paste`, `anvil_scroll`, `anvil_jump_prompt`,
  `anvil_mouse`, `anvil_resize` (both init and relayout), `anvil_respawn`,
  `anvil_split`, `anvil_close_pane`, `anvil_focus_dir`, `anvil_resize_pane`,
  `anvil_balance_panes`, `anvil_zoom_toggle`, `anvil_new_tab`, `anvil_cycle_tab`,
  `anvil_select_tab`, `anvil_close_tab`, `anvil_set_theme_mode`,
  `anvil_set_os_dark`, `anvil_palette_toggle/_char/_key`,
  `anvil_search_toggle/_char/_key`, `anvil_help_toggle/_key`,
  `anvil_copy_mode_toggle/_key`, `anvil_cfg_error_dismiss`,
  `anvil_caldera_drawer_toggle/_key`, `drainIpc` (when n>0 commands),
  `reloadConfigIfChanged` (when mtime changes), `takeNotify` presence in
  `anvil_poll`. `src/platform/shim.m`: `anvil_needs_render` + `anvil_force_render`
  extern decls; gate added in `render()` after window-title update and before
  `anvil_frame`; `anvil_force_render()` called from `applicationDidBecomeActive:`.
  `--dump` path calls `anvil_frame` directly and is unaffected by the gate.
  Render check: `ok: 1600x1000, bg=96.0%, bar=47.6%` — no visual change.

- 2026-05-30 — CADisplayLink + render-on-keystroke. `src/platform/shim.m` only.
  Change 1: replaced 60Hz `NSTimer` with a runtime `@available(macOS 14.0, *)` branch.
  macOS 14+: `[view displayLinkWithTarget:tick selector:@selector(tick:)]` added to
  the main run loop in `NSRunLoopCommonModes` (ProMotion-adaptive, screen-accurate).
  macOS 13 fallback: original `NSTimer` at `1.0/60.0` in `NSRunLoopCommonModes`.
  `AnvilTick.tick:` parameter changed from `NSTimer *` to `id` so the selector is
  valid for both types. `#import <QuartzCore/CADisplayLink.h>` added. Deployment
  minimum is 13.0 (`LSMinimumSystemVersion` in build.zig's Info.plist); CADisplayLink
  on macOS is `API_AVAILABLE(macos(14.0))`, hence the runtime check.
  Change 2: `render()` called synchronously after `anvil_input` in the two PTY-write
  paths of `keyDown:` (arrow-key escape sequences and normal chars), after `anvil_paste`
  in `pasteClipboard:`, and after `anvil_scroll` in `scrollWheel:`. The dirty gate in
  `render()` prevents double-paint on the next displaylink tick.
  `--dump` path unaffected (headless loop, no run loop). Render check:
  `ok: 1600x1000, bg=96.0%, bar=47.6%` — no visual change.

- 2026-05-30 — Glyph-atlas prewarm. New export `anvil_prewarm_atlas(out_ptr, out_count)` in
  `src/app.zig`: loops U+0021..U+007E (printable ASCII) and U+2500..U+259F (box-drawing
  + block elements) calling `renderer.atlas.slotFor(cp)`, then returns the populated
  pending array pointer and count. `src/platform/shim.m`: `anvil_run()` calls
  `anvil_prewarm_atlas` immediately after `buildAtlas()` and iterates the pending list
  with `rasterizeGlyph()`, uploading all 255 common TUI glyphs to the GPU atlas before
  the run loop starts. `--dump` path unchanged (prewarm is GUI-only). Render check:
  `ok: 1600x1000, bg=96.0%, bar=47.6%` — no visual change, only rasterization timing.

- 2026-05-29 — CLI args: `anvil [path]`, `--help`, `--version`. New `src/cli.zig`
  holds `CliArgs`/`Mode` enum and a pure `parse()` function (testable without GUI).
  `main.zig` calls `parse()`, handles help/version/dump exits, validates the
  positional path with `std.c.access`, and sets `app.start_cwd` before
  `window.run()`. `app.zig` gains `pub var start_cwd: []const u8 = ""` read during
  `anvil_resize` init. `session_manager.zig` gains `spawnFirstWithCwd` (calls
  existing `addWithCwd`). 8 unit tests in `cli.zig` covering all parse branches.
  `--dump` unchanged; 169/169 mod_tests pass; `--dump` produces non-empty PNG.
  IPC-into-running-instance deferred (requires socket listener + protocol —
  a real subsystem, not a small change).

- 2026-05-29 — Caldera run-detail drawer. `caldera.zig`: added `EventSummary`
  (64-byte summary cap), `RunDetail` (agent/step/status fields at 48 bytes each,
  plus `events[8]EventSummary`), and `details[max_rows]RunDetail` in `Snapshot`.
  `buildSnapshot` now fills `details[idx]` for each run row retaining all events
  up to the cap (8). `std.Thread.Mutex` and `Thread.sleep` replaced with
  `std.c.pthread_mutex_t` and `std.c.nanosleep` (Zig 0.16 API migration;
  pre-existing breakage). `app.zig`: added `caldera_snap`, `caldera_sel`,
  `caldera_drawer` globals; exports `anvil_caldera_drawer_toggle/open/key`;
  `emitCalderaDrawer` renders a centered modal panel (title + step + status +
  numbered event lines) using Mineral semantic colors (sel_fg header, ansi[6]
  step, ansi[2]/ansi[3] status, fg for events). Drawer is gated in `anvil_frame`
  before help/palette/search. `keys.zig`: added "Agents" section (3 bindings:
  Cmd+G open, arrows navigate, Esc close); `total_bindings` 37→40. `shim.m`:
  3 new extern declarations; `Cmd+G` dispatch before all modal checks; drawer
  nav capture block (Esc=close, up/down=navigate). New test: multi-event run
  parses all summaries in order into `RunDetail`. All tests pass; `--dump` 29KB.

- 2026-05-29 — Copy mode. `src/copy_mode.zig` adds `CopyMode` struct: `open`,
  `row/col` caret, `visual` flag + anchor. Methods: `enter`, `exit`,
  `startVisual`, `move`, `halfPage`, `gotoTop`, `gotoBottom`, `wordForward`,
  `wordBack`. Enter chord: `Cmd+Shift+Space`. In-mode keys: h/j/k/l + arrows
  move caret; w/b word motion; g/G top/bottom; Ctrl+U/D half page; v starts
  visual selection (drives `Terminal.selectStart/selectExtend`); y/Enter copies
  via `anvil_copy` + `anvil_pasteboard_write` then exits; Esc/q exits without
  copying. `app.zig` adds `anvil_copy_mode_toggle/open/key` exports and
  `copyModeCaret` helper (renders caret as block in `ansi[6]` = status.trace
  mineral/cyan). Live PTY cursor suppressed while copy mode is open. `shim.m`
  adds modal capture block mirroring search/palette/help pattern. `keys.zig`
  adds "Copy Mode" section (8 bindings) + terminal entry; `total_bindings` 28→37.
  `root.zig` adds copy_mode to test aggregator. 8 unit tests in copy_mode.zig.
  All tests pass; `--dump` non-empty (29 KB).

- 2026-05-29 — OSC 8 hyperlinks (Cmd+click to open in browser). `Cell` gains
  a `link: u16` field (0 = no link). `Terminal` owns a fixed link table:
  `link_uris[256][256]`, `link_uri_lens[256]`, `links_n`, `cur_link`. Cap of 256
  unique URIs per terminal; past the cap new links are silently ignored. Interning
  deduplicates repeated URIs. `Terminal.setLink` / `Terminal.linkUri` added.
  `Terminal.print` stamps `cur_link` onto each printed cell. `Terminal.reset`
  clears the table. `oscDispatch` in `parser.zig` handles case 8: extracts URI
  after the second semicolon and calls `setLink`. New export `anvil_link_at(x, y,
  out_ptr, out_len) -> c_int` in `app.zig` resolves device-px coords to a pane →
  cell → link id → URI. `shim.m` `mouseDown` checks Cmd flag, calls
  `anvil_link_at`, validates scheme is http/https/file (rejects all others), opens
  via `[[NSWorkspace sharedWorkspace] openURL:]`. Unit test in `parser.zig`:
  feeds OSC 8 + text + close, asserts cells carry link id and URI resolves.
  All tests pass; `--dump` non-empty.

- 2026-05-29 — Session persist/restore. `src/session_persist.zig` serializes the
  tab+pane-tree structure and per-pane cwd (from OSC 7) to
  `~/.config/anvil/session.json`. `anvil_save_session` export is called from
  `AnvilController -applicationWillTerminate:` (the most robust trigger: fires on
  Cmd+Q, menu Quit, and the all-shells-exited `[NSApp terminate:]` path).
  `anvil_resize` first-run path attempts `spawnFromState` before falling back to
  `spawnFirst`. `PaneTree.exportRoot`/`freeExport` and `splitWithRatio` added.
  `Session.initWithCwd` and `Pty.spawnCwd` added (existing `spawn` unchanged).
  Round-trip unit tests in `session_persist.zig`. cwd restored via OSC 7 (already
  tracked in `Terminal.cwd_buf`); structure-only restore is not used since OSC 7
  was already implemented.

- 2026-05-29 — Config error banner. `parseFull` in `config.zig` replaces the old
  silent `parse` (which is now a wrapper). It distinguishes "no file" (silent,
  `fopen` returns null) from "file present but invalid" (captures a one-line error
  in a fixed `[128]u8` buffer). Errors: unknown key, bad enum value (theme/
  cursor_style/cursor_blink), non-parseable float, out-of-range float. `ParseResult`
  struct carries `cfg`, `err[128]`, `err_len`. `loadFull` exposed; `load` wraps it.
  `app.zig` stores `cfg_error_buf/len`; `loadConfig` now calls `loadFull` and copies
  any error. `emitCfgError` renders a `status.failure` (#b13a30) banner at the top
  of the workspace with bone text. It fires in the else branch of `anvil_frame`
  alongside run-rails and exited-panes. Clears automatically on next clean reload.
  New exports `anvil_cfg_error_open` / `anvil_cfg_error_dismiss`; shim.m routes
  Esc to dismiss when banner is visible. Six new tests in `config.zig`. All tests
  pass; `--dump` non-empty.

- 2026-05-29 — Dead-pane exited indicator + respawn. `Session.exited: bool`
  field tracks per-session shell death. `anvil_poll` sets `s.exited = true` on
  EOF and returns 0 only when ALL sessions are exited (not just the focused one).
  `emitExitedPanes` renders a failure-red (BRAND `status.failure`) status bar at
  the bottom of each dead pane with message `[process exited — Cmd+R to restart]`.
  New export `anvil_respawn` re-forks the PTY and calls `Terminal.reset()` /
  `Scrollback.clear()` in-place. Cmd+R wired in `src/platform/shim.m` keyDown
  and added to `src/keys.zig` (total_bindings now 25). All tests pass.
- 2026-05-29 — Keyboard cheatsheet modal added. New file `src/keys.zig`: single
  source of truth for all keybindings (4 sections, 24 bindings total). New exports
  in `src/app.zig`: `anvil_help_toggle`, `anvil_help_open`, `anvil_help_key`;
  `emitHelp` renders a centered panel (border+panel=2 overlay rects) with chord
  column in bright-cyan, action column in fg, section headers in cyan. Help overlay
  has highest precedence in `anvil_frame` dispatch. `src/platform/shim.m`: Cmd+/
  toggles the cheatsheet; capture block before palette/search swallows all keys
  while open (Esc closes). `src/root.zig` references `keys.zig` for test coverage.
  All tests pass; `--dump` produces a valid PNG.
- 2026-05-29 — Search: match-count display + regex mode. `emitSearch` in
  `app.zig` already rendered `cur/total` count right-aligned; updated to also
  show `[R]` (teal, `th.ansi[6]`) or `[R?]` (yellow, `th.ansi[3]` on bad
  pattern) regex mode indicator left of the count. New file `src/regex.zig`:
  backtracking engine (~175 lines) supporting literals, `.`, `*` `+` `?`
  (greedy), `[...]`/`[^...]` char classes with ranges, `^`/`$` anchors;
  no alternation. `compile()` returns null on syntax error. `Search` in
  `search.zig` gains `regex_mode: bool`, `bad_pattern: bool`, `toggleRegex()`,
  and `findInRowRegex()` (extracts ASCII from cells into a 512-byte scratch
  buf, calls `regex.search`). Bad pattern falls back to case-insensitive
  substring and sets `bad_pattern`. Toggle chord: Tab while search bar is open
  (`'\t'` case added to shim.m search handler → `anvil_search_key(5)`). Key 5
  added to `anvil_search_key` dispatch. `src/keys.zig` gains a "Search" section
  (3 bindings; `total_bindings` updated to 28). `src/root.zig` references
  `regex.zig` for test aggregation. Tests added: 13 regex unit tests (each
  metachar, anchors, negated class, compile-failure) + 4 search integration
  tests (regex match, bad-pattern fallback, toggle, known match count). All
  313 tests pass; fmt clean; `--dump` non-empty (28 677 bytes).
- 2026-05-29 — Render golden check. Added `tools/check-render.py`: decodes the
  `--dump` PNG using stdlib `zlib` (no deps), reconstructs PNG filter passes,
  asserts 1600x1000 dimensions, Mineral dark background `#0b0d0e` covers >= 90%
  of pixels, and title bar color `#161a1c` covers >= 30% of bar rows. Exact
  pixel hash is intentionally NOT used: CoreText antialiasing varies across macOS
  versions and CI runner may differ from dev machine. Updated CI step
  "Headless render smoke check" -> "Headless render golden check" to invoke the
  script. `cargo test --workspace` / `.zig/zig build test` passes.

- 2026-05-29 — Direction correction recorded in
  `context/2026-05-29-terminal-first-direction.md`: Anvil should be a
  Rust/AppKit/Metal terminal-first command deck for shells, repos, agents,
  clusters, logs, and quick edits, not a generic Zed clone or immediate Zig
  rewrite.
- 2026-05-21 — Set up the AI dev environment: agents, wiki, `AGENTS.md`,
  `CLAUDE.md`, and brand assets replicated from `caldera-os` and adapted to the
  Zig/macOS stack. See [[decisions/0001-ai-dev-environment]].
- 2026-05-21 — M1 complete: a usable single-pane GPU terminal (VT parser, grid,
  100k-line scrollback, PTY, Metal text rendering, input). 102 tests pass.
  Handoff written: `context/2026-05-21-m1-complete.md`. Brand gate flagged M1
  renderer divergences from `BRAND.md` (font, canvas, accent) — see `todo.txt`.
- 2026-05-21 — AI dev environment sub-project B confirmed complete (global
  `~/.claude`: `CLAUDE.md`, `llmwiki-convention.md`, agent backups). Reconciled
  `todo.txt` and the M1 handoff to mark it done.
- 2026-05-21 — Brand alignment of M1 renderer complete. Changed files:
  `src/render/color.zig`, `src/render/font.zig`, `src/main.zig`.
  (1) Background: `#0c0d10` → graphite `#0b0d0e` (caldera.graphite).
  (2) Accent/cursor (ANSI 6): `#2bb8b0` → mineral `#2f7f86` (accent.mineral / status.info).
  (3) Font: "Menlo" → IBM Plex Mono primary via `Font.initFirstAvailable`, fallback
      chain: IBMPlexMono → SFMono-Regular → Menlo.
  (4) ANSI 16-color palette aligned to brand semantics:
      0 black=#0b0d0e (graphite), 1 red=#b13a30 (failure), 2 green=#3f8a5b (verified),
      3 yellow=#b07a14 (attention), 4 blue=#4a6f8a (muted steel — no brand blue token),
      5 magenta=#6a5fa3 (agent), 6 cyan=#2f7f86 (mineral/info),
      7 white=#86919a (alloy/muted text), 8 bright-black=#374046 (ash),
      9 bright-red=#d44a3f, 10 bright-green=#52b070, 11 bright-yellow=#d49a28,
      12 bright-blue=#6a9ab8, 13 bright-magenta=#8f84c8, 14 bright-cyan=#4aa8b0,
      15 bright-white=#e8eaee (foreground text).
  Ambiguity resolved: ANSI 4 (blue) has no direct brand token; chose muted steel
  #4a6f8a consistent with the Mineral/graphite aesthetic. brand.risk (#a8623a)
  has no natural ANSI slot (attention owns yellow, red is failure); it is available
  via 256-color or RGB only. Bright variants derived by lightening normals ~15-20%.
  102/102 tests pass. `todo.txt` BRAND ALIGNMENT item checked off.
- 2026-05-21 — M2 config/theme sub-project, Task 1 complete: `src/config/config.zig`
  with `Config`, `Overrides`, `CursorStyle`, `Loaded`, `defaults`, `parseSlice`.
  ZON parsing uses arena ownership; `Loaded.deinit` frees the arena. 107 tests pass.
- 2026-05-21 — M2 config/theme sub-project, Task 2 complete: added `resolvePath`,
  `load`, and `Watcher` to `src/config/config.zig`. Uses `std.c` POSIX primitives
  (open/read/fstat/close) because Zig 0.16 moved the high-level file API to
  `std.Io.Dir` with a required `Io` parameter — the `std.c` path avoids the new API
  and matches the existing `std.posix`/libc style in `src/pty/pty.zig`. Also: in
  Zig 0.16 `std.posix.getenv` no longer exists; `std.c.getenv` is the replacement
  on macOS (always links libc). `mtime` returns `i128` derived from
  `timespec.sec * 1_000_000_000 + timespec.nsec`. 109 tests pass.
  Commit: df471f0.
- 2026-05-21 — M2 config/theme sub-project, Task 3 complete: `src/config/theme.zig`
  with `Theme`, `mineral_dark`, `mineral_light`, `byName`, `palette256`. Mineral dark
  uses brand-aligned palette from M1; Mineral light uses bone background with darkened
  bright slots for contrast. 114 tests pass.
- 2026-05-21 — M2 config/theme sub-project, Task 4 complete: added `hexToRgb`,
  `applyOverride` (private), and `resolve` to `src/config/theme.zig`. `resolve` starts
  from a named base theme and applies `config.Overrides` field-by-field; invalid hex
  strings are logged and skipped (base value kept). 118 tests pass. Commit: dae77c7.
- 2026-05-21 — M2 config/theme sub-project, Task 5 complete: renderer is now theme-driven.
  Removed `mineral_dark_bg`, `default_fg`, `default_bg`, `ansi16`, `palette256`, and the
  `"palette256 covers the three ranges"` test from `src/render/color.zig`. Added
  `theme: Theme` field to `App` in `src/main.zig`; initialized with
  `theme_mod.byName("mineral-dark")`; routed all color lookups in `renderFrame`,
  `drawCell`, and `resolve` through `g.theme`. Fixed `src/render/metal.zig` to inline
  the bg hex literal (`"#0b0d0e"`) since `mineral_dark_bg` was removed. No visible
  change to the rendered output. 117 tests pass. Commit: a50ff26.
- 2026-05-21 — Wiki backfill: four new pages covering M0–M2 product/engineering
  knowledge. All claims verified against source at commit df471f0.
  New: [[concepts/console-architecture]] (data flow, module map, runtime model),
  [[concepts/zig-0.16-gotchas]] (std.posix.getenv removal, high-level file API move,
  capi.zig extern-decl approach), [[decisions/0002-tech-stack]] (Zig+Metal+AppKit;
  runtime newLibraryWithSource: shader rationale), [[decisions/0003-m1-brand-palette]]
  (ANSI-16 to Mineral palette mapping; blue-token and risk-token ambiguities documented).
  Updated [[index]], [[concepts/README]], [[decisions/README]].
- 2026-05-21 — M2 config/theme sub-project, Task 6 complete: `Terminal.init` now
  accepts `scrollback_capacity: usize`; `main.zig` passes 100_000 (unchanged behavior);
  `makeTerminal` test helper passes `scrollback.default_capacity`. 117 tests pass.
- 2026-05-22 — HUD panel shipped. New module `src/render/hud.zig` implements a
  togglable right-side developer-context panel (30 columns wide, 1-column separator
  gutter). Follows tabbar/searchbar raster-draw precedent. Brand: Mineral palette,
  alloy-grey labels, semantic status colors (verified green / failure red / attention
  amber / info teal). Three sections: git (branch, dirty count, ahead/behind via
  `prompt/git.zig` reuse), last-run (OSC 133;C/D timing + exit code), memory
  (sysctl vm.page_free_count / hw.memsize). CPU deferred (two-sample complexity).
  Terminal gains `shell_running`, `shell_run_start_ms`, `shell_last_exit`,
  `shell_last_duration_ms` fields + `lastRun()` accessor. `recordPromptMark` updated
  to capture 133;C start time and parse `exit_code=N` from 133;D payload.
  `raster.zig` gains `colRule` (vertical hairline, mirrors `rowRule`).
  `config.zig`: `Keybindings` gains `hud_toggle` (default `cmd+j` — `cmd+.`
  is swallowed by macOS as a legacy cancel combo).
  `palette.zig`: `hud_toggle` action + "Toggle HUD" catalog entry.
  `main.zig`: `App.hud_visible`, `hud`, `hud_tick`; `resizeAllTabs` narrows cols by
  `hud_cols + 1` when visible; `refreshHud()` runs every 60 ticks (~1 s);
  `renderFrame` calls `hud_mod.draw`; keybinding and palette action both toggle +
  reflow. Timing uses `std.c.clock_gettime(CLOCK.MONOTONIC)` (no `std.time.milliTimestamp`
  in Zig 0.16 — see gotchas). 269/269 tests pass (up from 259; 10 new HUD tests).
- 2026-05-21 — M2 config/theme sub-project, Task 7 complete: added `cellInset` to
  `src/render/raster.zig` (fills a sub-rectangle of a cell by fractional inset —
  used for bar/underline cursors). Added `cursor_cfg: cfg_mod.Config.CursorCfg`,
  `blink_on: bool`, `blink_ticks: u32` to `App` in `src/main.zig`. Blink phase
  advances in `onTick` (32 ticks ≈ 533 ms at 60 Hz). New `drawCursor` helper dispatches
  block/bar/underline and honors blink state; `renderFrame` calls it instead of
  inlining cursor drawing. Default `cursor_cfg = .{}` (block, blink true). 118/118
  tests pass. Commit: 5335119.
- 2026-05-21 — Three subtle terminal animations implemented on branch feat/m2-config-theme:
  (1) Smooth cursor glide: `cursor_ax`/`cursor_ay` (f32) in App advance via exponential
      approach (rate 0.45) each tick; `drawCursor` uses fractional position via updated
      `cellInset(f64 col, f64 row)` API.
  (2) Smooth scroll: `scroll_anim` (f32) approaches `viewportOffset()` at rate 0.30;
      `renderFrame` renders `viewportRowAt(base+1, y)` with `y_shift_px` set to
      `(1-frac)*cell_h` to slide the grid UP. Jumps larger than one screen snap instantly.
  (3) Cursor blink fade: `blink_phase` (f32) advances `1/64` per tick; `cursorOpacity`
      implements a soft piecewise fade (solid 0–50%, fade-out 50–62%, dim 62–88%,
      fade-in 88–100%). `windowIsKey()` gates the phase advance so unfocused windows
      don't drive 60fps redraws. `color.mix` helper added to `src/render/color.zig`.
  Supporting changes: `y_shift_px: f64` on `Raster`; `viewportRowAt` on `Terminal`;
  `snapAnim()` called on tab-switch, resize, `closeDeadTabs`, and startup; tab bar
  now drawn after the grid so sliding rows can't bleed into it. 235→241 tests pass.
  Slide direction: POSITIVE y_shift_px = grid slides UP (base+1 offset rendered).
  `windowIsKey` gate: working via `g.view.msgSend("window"/"isKeyWindow")`.
- 2026-05-21 — M2 config/theme code review fixes: corrected the `cellInset` doc
  comment in `src/render/raster.zig` (was "left/top inset fractions"; now correctly
  states `fy` is offset from the cell BOTTOM, consistent with the y-up CG context)
  and added a second `cellInset` test ("cellInset underline fills the cell bottom")
  probing the underline case (`fx=0, fy=0, fw=1.0, fh=0.12`). No logic changes.
  119/119 tests pass. Commit: b01b46e.
- 2026-05-21 — M2 config/theme sub-project, Task 8 complete: config wired at startup
  and live reload implemented. Added `setClearColor` to `Renderer` in
  `src/render/metal.zig` (tracks theme background for resize-flash correctness).
  `src/main.zig`: `App` gains `config: cfg_mod.Loaded` and `watcher: cfg_mod.Watcher`;
  startup loads `~/.config/caldera-console/config.zon` and uses `cfg.font.size`,
  `cfg.font.family`, `cfg.window.width/height`, and `cfg.scrollback` for all
  startup-only settings; theme and cursor apply immediately via `theme_mod.resolve`.
  `onTick` polls the watcher each frame; `applyConfig` updates theme + cursor live
  and frees the old arena after `resolve` copies all colors to plain `[3]u8` values
  (no use-after-free). Hardcoded fallback constants removed. 119/119 tests pass.
  App launches cleanly with no errors. Live reload verified by design (interactive
  test not performed non-interactively). This completes M2 sub-project 1 (config +
  theme). New wiki page: [[concepts/config-system]].
- 2026-05-21 — M2 multi-tab sub-project, Task 2 complete: added `Tab` struct,
  `readerLoop`, `spawnIn`, `copyTrunc`, and `basename` to `src/app/tab.zig`.
  `Tab` owns a `Terminal` + `Pty` + 256 KiB per-tab handoff buffer; transcribed M1's
  `std.atomic.Mutex` / tryLock+yield spin pattern exactly. `deinit` order: close pty fd
  first (causes blocking `read` in reader thread to return `error.Eof`), then join the
  reader thread, then deinit terminal, then free the struct. API adaptation: Zig 0.16
  has no `std.process.changeCurDir` or `std.process.getCwdAlloc` (process API requires
  an `Io` instance); used `std.c.chdir` and `std.c.getcwd` directly with stack buffers.
  2 new tests pass (`basename` pure, `label` fallback). 147/147 tests pass.
  Commit: 4d38c15.
- 2026-05-21 — Code coverage tooling + test backfill. Added a `zig build coverage`
  step (kcov over the test binary) and `src/coverage_root.zig`, a pty-free test
  root — kcov livelocks tracing the pty tests' child processes on macOS. Baseline
  was 90.99% line coverage; backfilled `terminal/parser.zig` (83.8%→99.3%),
  `terminal/terminal.zig` (87.8%→99.7%) and `render/font.zig` (86.8%→98.3%) with
  49 new tests, lifting the measured set to 98.86%. The `terminal.zig` init
  out-of-memory test caught a real leak in `grid.Grid.init`: the `cells`
  allocation leaked when the later `scrolled_off` allocation failed — fixed with
  an `errdefer`. 168/168 tests pass. New wiki page: [[operations/coverage]].
  Design: `docs/specs/2026-05-21-code-coverage-audit-design.md`.
- 2026-05-21 — Code review fixes for `src/app/tab.zig` (M2 multi-tab Task 2).
  Issue 1 (deadlock): added `closing: bool = false` field to `Tab`; `readerLoop`'s
  inner buffer-full spin loop now checks `tab.closing` under the lock and exits
  via `break :outer` when set. `deinit` sets `closing = true` under the lock before
  calling `pty.deinit()`, so a thread stuck spinning on a full buffer escapes cleanly
  without waiting for the next `pty.read()` to return. Issue 2 (cwd postcondition):
  `spawnIn` now only executes the chdir-spawn-restore dance when `getcwd` succeeds;
  if `getcwd` returns null the function falls through to the no-cwd spawn path,
  preventing a permanent process cwd change. 168/168 tests pass. Commit: f722dc0.
- 2026-05-21 — M2 multi-tab sub-project, Task 4 complete: added `Chord`, `parseChord`,
  `eqIgnoreCase`, and `Keybindings` to `src/config/config.zig`; added
  `keybindings: Keybindings = .{}` field to `Config`. `parseChord` splits on `+`,
  recognises cmd/shift/ctrl/opt modifiers (case-insensitive via `std.ascii.eqlIgnoreCase`),
  and lowercases the key character via `std.ascii.toLower`. All four std APIs confirmed
  present in Zig 0.16.0_1. 3 new tests (parseChord valid, parseChord invalid, config
  parses keybindings override). 172/172 tests pass. Commit: 6358132.
- 2026-05-21 — M2 multi-tab sub-project, Task 5 complete: created
  `src/render/tabbar.zig` with `drawTabBar` and 1 unit test. `drawTabBar` draws a
  one-row tab bar into raster row 0: equal-width segments, active segment in
  `theme.accent`, inactive in `theme.ansi[8]`, labels in `theme.foreground` (active
  label in `theme.background` for contrast). Guard: returns immediately when < 2 tabs
  (low-profile rule). API verified: `Raster.cellBg`/`cellGlyph`, `Font.glyph`,
  `font.metrics.cell_w`, and all `Theme` fields match existing M1/sub-project-1
  implementations. Added `_ = @import("render/tabbar.zig");` to `src/main.zig` test
  aggregator. 173/173 tests pass. Commit: 9ccc0ab.
- 2026-05-21 — M2 multi-tab sub-project, Task 7 complete: tab keybindings and input
  routing wired in `src/main.zig`. Added `keys_new`, `keys_close`, `keys_next`,
  `keys_prev`, `keys_jump[9]` fields to `App`; `loadKeybindings` parses
  `cfg_mod.Keybindings` strings into `cfg_mod.Chord` values at startup and on every
  live config reload (`applyConfig` now calls it). `chordMatches` compares AppKit
  modifier flags + `std.ascii.toLower` of the character against a `Chord`.
  `handleTabKey` dispatches new/close/next/prev/jump tab actions; consumed events
  return true. `currentCwd` reads OSC-7 cwd from the active tab. `addTab` opens a
  new shell, sizing with bar-row reserved, then calls `resizeAllTabs` so the 1→2
  transition reflows all tabs correctly. `onKeyDown` rewritten: `⌘` combos try
  `handleTabKey` first; other keys follow the original path. `closeDeadTabs` fixed
  (Step 2b): captures `barRows()` before the close loop and only calls
  `resizeAllTabs()` when bar visibility changed — avoids unnecessary SIGWINCH on
  surviving shells. No new unit tests (integration wiring). 173/173 tests pass.
  `zig build` clean; app launches without crash. Commit: a694178.
- 2026-05-21 — M2 multi-tab sub-project complete (Task 9 closeout). Two
  code-comment fixes: (1) `src/render/tabbar.zig` doc comment corrected —
  active-segment labels use `theme.background`, inactive use `theme.foreground`
  (was incorrectly "theme foreground" for both). (2) `src/main.zig`
  `chordMatches` — replaced `cp & 0x7f` truncation with `asciiLowerCp(cp)`, an
  inline ASCII-letter lowercaser that operates on the full `u21` codepoint
  without silently misidentifying high codepoints as ASCII characters. Behavior
  unchanged for all ASCII keybindings. Documentation: `todo.txt` multi-tab item
  checked off; new wiki page [[concepts/tab-system]] (Tab struct, TabManager,
  per-tab PTY reader thread, bar-visibility rule, keybinding chords); linked from
  [[index]]. 173/173 tests pass. This completes M2 sub-project 2 of 4.
- 2026-05-21 — M2 in-terminal-search sub-project, Task 1 complete: added four
  content-line accessors to `src/terminal/terminal.zig` — `lineCount`,
  `line`, `contentRowOfViewport`, `scrollToLine`. These expose a unified index
  space (scrollback rows then active-grid rows) for the upcoming `Search` struct.
  API adaptation: the plan's `const line` local in `osc133` conflicted with the
  new `pub fn line` method name; renamed the local to `line_num`. 2 new tests
  (`lineCount and line span scrollback then grid`, `contentRowOfViewport matches
  viewport composition`). 175/175 tests pass. Commit: 65bcf58.
- 2026-05-21 — M2 in-terminal-search sub-project, Task 2 complete: created
  `src/terminal/search.zig` with `Match`, `MatchKind`, `max_matches`, and the
  `Search` struct (`init`/`deinit`/`query`/`count`/`currentMatch`/`setQuery`/
  `rescan`). Private helpers: `rowMatchesAt` (cell-by-cell codepoint comparison),
  `lowerCp` (ASCII fold). Smart-case: any uppercase letter in the query activates
  case-sensitive mode; otherwise case-insensitive. Match list capped at 2048.
  Added `_ = @import("terminal/search.zig");` to `src/main.zig` test aggregator.
  API confirmed: `std.ArrayList(T)` in Zig 0.16 is the unmanaged `Aligned` form —
  `.empty` / `append(alloc, x)` / `deinit(alloc)` / `clearRetainingCapacity` all
  correct. `std.unicode.Utf8View.initUnchecked(...).iterator()` + `nextCodepoint()`
  confirmed. No API adaptations required. 5 new tests. 180/180 tests pass.
  Commit: 6f88bf5.
- 2026-05-21 — M2 in-terminal-search follow-up fixes (Task 2 hardening). Two
  changes to `src/terminal/search.zig`: (1) UTF-8 safety: replaced
  `Utf8View.initUnchecked` with `Utf8View.init(...) catch return` in `rescan`;
  `Utf8View.init` returns `error.InvalidUtf8 ! Utf8View` — on invalid UTF-8 the
  query buffer is treated as empty (matches already cleared, `current` already 0
  at the top of `rescan`, so `return` is sufficient). (2) Scrollback test: added
  `"finds a match in scrollback"` — creates a 20×3 terminal, feeds `"findme\r\n"`
  then 5× `"x\r\n"` to push "findme" into history, then asserts 1 match with
  `m.row < t.history.len()`. 181/181 tests pass. Commit: b7cad8d.
- 2026-05-21 — M2 in-terminal-search sub-project, Task 5 complete: created
  `src/render/searchbar.zig` with `drawSearchBar` and 1 unit test. `drawSearchBar`
  paints the bottom raster row: bar background in `theme.ansi[8]`, "find: <query>"
  label left-aligned in `theme.foreground`, and a `current/total` counter
  right-aligned in `theme.foreground`. The test fills a 400×200 raster, calls
  `drawSearchBar` on the bottom row, and pixel-probes the B channel to verify
  `theme.ansi[8]` background was written. Added `_ = @import("render/searchbar.zig");`
  to `src/main.zig` test aggregator. No API adaptations required — all field and
  method names matched exactly. 186/186 tests pass. Commit: e1c7475.
- 2026-05-21 — Code review fix for `src/render/searchbar.zig`: two narrow-window
  layout defects corrected. (1) Counter/query overlap: moved `counter` computation
  before the left-text loop; left-text loop bound capped to `total_cols - counter.len - 1`;
  underflow guard — if `counter.len + 1 >= total_cols` left text is skipped entirely.
  (2) Off-by-one on counter fit guard: changed `counter.len < total_cols` to
  `counter.len <= total_cols` so a counter whose length exactly equals `total_cols`
  is drawn (start = 0, valid). Column math verified for total_cols 1/5/10: no
  out-of-bounds cellGlyph column, no usize underflow. 186/186 tests pass.
  Commit: 710c066.
- 2026-05-21 — M2 in-terminal-search sub-project, Task 6 complete: search render
  integration wired into `src/main.zig`. Added `Search` + `searchbar` imports and
  `search: Search` / `search_open: bool` fields to `App`; `Search.init(alloc)` added
  to the `g` initializer. Renamed `barRows()` to `topBarRows()` and added
  `bottomBarRows()`; updated all 7 call sites (closeDeadTabs ×2, resizeAllTabs ×1,
  handleTabKey ×2, drawCell ×1, drawCursor ×1). `resizeAllTabs` row count now uses
  saturating `topBarRows() -| bottomBarRows()`. Added `openSearch`/`closeSearch`
  helpers (reflow + dirty). `renderFrame` draws the search bar in the last raster
  row when `search_open`. `drawCell` tints matched cells after the cursor block
  (cursor wins on its own cell). `onTick` rescans after feeding the active tab when
  search is open. `handleTabKey` calls `closeSearch()` in next/prev/jump branches
  (tab switch closes search). No new unit tests (integration wiring only).
  186/186 tests pass. `zig build` clean. App launches without crash. Commit: a0506f1.
- 2026-05-21 — M2 in-terminal-search sub-project complete (Task 8 closeout). Two
  code fixes: (1) `src/terminal/search.zig` `classify` rewritten as two-pass —
  pass 1 checks `matches.items[self.current]` directly so `.current` always wins
  when a cell is covered by both the current match and another match (e.g.
  overlapping `"aa"` matches in `"aaa"`); pass 2 loops remaining matches for
  `.other`; early return on empty matches list. New test `"classify: current match
  wins on overlap"` verifies the fix. (2) `src/main.zig` stale plan-snippet comment
  `// (after the ⌘-combo block, before extractKey for normal keys)` replaced with
  `// While the search bar is open, keystrokes edit the query, not the shell.`
  Documentation: `todo.txt` in-terminal-search item checked off; new wiki page
  [[concepts/search-system]] (Search struct, smart-case, max_matches cap, UTF-8
  validation, content-row index space, bottom bar, top/bottom offset split); linked
  from [[index]]; test count updated to 187. 187/187 tests pass. This completes M2
  sub-project 3 of 4.
- 2026-05-21 — M2 shell-integration sub-project, Task 2 complete: added
  `shell_integration: bool = true` field to `Config` in `src/config/config.zig`.
  1 new test (`config parses shell_integration`). 188/188 tests pass.
- 2026-05-21 — M2 shell-integration sub-project, Task 3 complete: created
  `src/app/shell_integration.zig`. `@embedFile`s the three shell scripts via
  `src/shell/` symlinks (pointing to `shell/` at repo root) — required because
  Zig 0.16 `zig test` does not apply `--embed-dir` to embedded files even when
  `addEmbedPath` is used in `build.zig`; `build-obj` does apply it, making this a
  `zig test` bug. Workaround: symlinks inside the package tree (`src/shell/`).
  API adaptations: `std.c.setenv` and `std.c.unsetenv` are absent from Zig 0.16
  std.c — declared as `extern "c"` in the module (POSIX, available on macOS).
  The `std.c.O` flags form for write+create+truncate is
  `.{ .ACCMODE = .WRONLY, .CREAT = true, .TRUNC = true }` — confirmed correct
  for macOS via the installed `std/c.zig` (line 8526 packed struct). `std.c.mkdir`,
  `std.c.open`, `std.c.write`, `std.c.close`, `std.c.getenv` all confirmed present.
  `setup(true)` writes three files and exports five env vars; `setup(false)` writes
  the marker vars only and skips `ZDOTDIR`; all filesystem failures degrade
  gracefully (log + return, never fatal). Added `_ = @import("app/shell_integration.zig");`
  to `src/main.zig` test aggregator. 3 new tests. 191/191 tests pass. Commit: c97f549.
- 2026-05-21 — M2 shell-integration sub-project, cleanup fixes complete.
  Fix 1: removed the symlink indirection. The three shell scripts moved from
  `shell/` (repo root) into `src/shell/` as real files (git mode 120000 → 100644).
  `shell/` is now empty and removed from git. `@embedFile("../shell/...")` paths in
  `shell_integration.zig` are unchanged — they resolve to `src/shell/` relative to
  `src/app/` and already resolved correctly through the symlinks.
  Fix 2: strengthened tests in `src/app/shell_integration.zig`: (1) asserts
  `CALDERA_SHELL_INTEGRATION` and `CALDERA_SHELL_INTEGRATION_ZSH` are non-null after
  `setup(true)`; (2) reads the written `caldera-integration.zsh` file and asserts
  content length > 0; (3) new test "setup preserves a pre-existing ZDOTDIR" sets
  `ZDOTDIR=/tmp/my-zdotdir` before `setup(true)` and asserts `CALDERA_REAL_ZDOTDIR`
  equals that sentinel.
  Fix 3: corrected the error-handling table row for `shell_integration = false` in
  `docs/superpowers/specs/2026-05-21-shell-integration-design.md` (gitignored — not
  committed). The row now accurately states that setup still writes the scripts and
  exports the markers; only ZDOTDIR injection is skipped.
  Note: the spec doc commit was skipped — `docs/superpowers/` is gitignored.
  192/192 tests pass. Commit: bdaa2fe.
- 2026-05-21 — M2 shell-integration sub-project, Task 4 complete: added
  `const shell_integration = @import("app/shell_integration.zig");` import to
  `src/main.zig` (alongside the other `app/` imports) and called
  `shell_integration.setup(cfg.shell_integration)` in `main` after the config is
  loaded (`cfg` is available) and before `TabManager.init` / `tabs.newTab` — so the
  first shell and every later tab inherits the exported env via `Pty.buildChildEnv`.
  2 lines inserted; no other changes. 192/192 tests pass. `zig build` clean, exit 0.
  Commit: 59bda8a.
- 2026-05-21 — M2 shell-integration E2E bug fixes (two commits). Bug 1
  (`src/shell/caldera-integration.zsh`, commit f545ba9): `$'...'` ANSI-C quoting
  inside a double-quoted string is NOT interpreted by zsh — the literal characters
  `$'\e]133;B\a'` appeared in the prompt as visible text. Fixed by splitting the
  PS1 assignment into three adjacent tokens: `"${PS1}%{"` + `$'\e]133;B\a'` +
  `"%}"` so the middle `$'...'` is a standalone token (interpreted). Guard updated
  from `*'133;B'*` to `*$'\e]133;B'*` for consistency. `zsh -n` parses clean.
  Bug 2 (`src/terminal/terminal.zig` + `src/main.zig` + `src/app/tab.zig`,
  commit f712e62): `Terminal.cwd()` returns the raw OSC 7 `file://` URL, so
  `currentCwd()` in `main.zig` was passing a `file:///…` string to `chdir`,
  which silently failed. Added `pub fn cwdPath()` next to `cwd()` — strips
  `file://` prefix and host (everything up to and including the first `/` after
  `file://`), returns a sub-slice of `cwd_buf` (no allocation). Bare paths and
  empty values pass through unchanged. `currentCwd()` in `main.zig` and
  `label()` in `tab.zig` both updated to call `cwdPath()`. 5 new unit tests for
  `cwdPath`. 193/193 tests pass. `zig build` clean, zero warnings.
- 2026-05-21 — M2 shell-integration sub-project complete (Task 5 closeout).
  Documentation: `todo.txt` shell-integration item checked off and M2 milestone
  marked complete; new wiki page [[concepts/shell-integration]] (OSC 133 A/B/C/D
  marks, OSC 7 cwd, zsh/bash hook scripts, ZDOTDIR shim auto-injection,
  `shell_integration` config toggle, embed-and-write startup,
  `Terminal.cwdPath()` for new-tab cwd inheritance); linked from [[index]];
  test count updated to 193. This completes M2 sub-project 4 of 4.
- 2026-05-21 — M2 MILESTONE COMPLETE. All four sub-projects shipped and
  verified end-to-end: (1) config + theme with live reload, (2) multi-tab
  (Tab/TabManager, per-tab PTY reader thread, tab bar), (3) in-terminal search
  (smart-case, scrollback-aware, highlight-all), (4) shell integration (OSC 133
  marks, OSC 7 cwd, zsh ZDOTDIR auto-injection, bash opt-in, new-tab cwd
  inheritance). 193/193 tests pass. M3 (Webview host + typed IPC bridge) is
  the next milestone.
- 2026-05-21 — M3 IPC bridge, Task 1 complete: created `src/ipc/bridge.zig`
  with `Inbound` union (ready/invoke/dismiss), `DecodeError`, and `decode`.
  `Inbound.deinit` frees only `invoke.id` (duped into caller's allocator).
  `decode` uses `std.json.parseFromSlice` with `ignore_unknown_fields = true`
  into a `Wire` struct whose plain `type: []const u8` field maps to the JSON
  `type` key — `type` is a valid field identifier in Zig 0.16, and `zig fmt`
  strips an `@"type"` escape as unnecessary. 7 new tests cover all decode
  paths: ready, dismiss, invoke with id,
  unknown-field tolerance, missing id, unknown type, malformed JSON. Added
  `_ = @import("ipc/bridge.zig");` to `src/main.zig` test aggregator.
  200/200 tests pass (baseline was 193). Commit: caf4cc2.
- 2026-05-21 — M3 IPC bridge, Task 2 complete: added outbound half to
  `src/ipc/bridge.zig` — `Command`, `ThemeTokens`, `Outbound` union
  (show/hide), and `encode`. `encode` fast-paths `.hide` via `allocator.dupe`
  and serializes `.show` through a local `Wire` struct with a plain
  `type: []const u8 = "show"` field so `std.json.fmt` emits the key without
  escaping. `std.fmt.allocPrint(alloc, "{f}", .{std.json.fmt(wire, .{})})` is
  the confirmed Zig 0.16 idiom for allocating JSON. 3 new tests (encode hide,
  encode show exact string, encode show with subtitle). No expected-string
  adjustments were required — std.json field-declaration order matched exactly.
  203/203 tests pass. Commit: 862b64d.
- 2026-05-21 — M3 IPC bridge, Task 3 complete: created `src/app/palette.zig` —
  pure-logic command-palette controller. Contains the `Action` enum (7 variants),
  `Entry` struct, `catalog` (7 entries), `actionForId` lookup, and the `Palette`
  state machine (`summon`/`onReady`/`dismiss`). Summon before the webview signals
  ready is deferred via `pending_show` and flushed on `onReady`. No ObjC.
  Added `_ = @import("app/palette.zig");` to `src/main.zig` test aggregator.
  7 new tests cover all state-machine paths. 210/210 tests pass. Commit: e8ee1ba.
- 2026-05-21 — M3 IPC bridge, Task 4 complete: created `ui/palette/index.html` —
  self-contained command palette web surface (inline CSS + JS, no framework, no
  build step). Communicates with native via
  `window.webkit.messageHandlers.caldera.postMessage(jsonString)` (web→native) and
  `window.caldera.receive(obj)` (native→web). Supports show/hide messages (with
  optional theme token override), fuzzy title filtering, keyboard navigation
  (ArrowUp/Down, Enter, Escape), backdrop-click dismiss, and fires a `ready` message
  on load. IBM Plex Sans/Mono fonts; Mineral palette CSS variables. No test step —
  file is not yet referenced by Zig source.
- 2026-05-21 — M3 IPC bridge, Task 5 complete: modified `build.zig` —
  added `exe_mod.linkFramework("WebKit", .{})` and
  `exe_mod.addAnonymousImport("palette_html", ...)` pointing at
  `ui/palette/index.html`. The anonymous import is not yet referenced by Zig
  source; this wires the build so Task 6 can use it. `zig build` and
  `zig build test` both exit 0 (210/210 tests pass). Commit: 57eac3c.
- 2026-05-21 — M3 webview host, Task 6 complete: created `src/webview/webview.zig`.
  `Webview` wraps `WKWebView` with ObjC runtime class construction using the
  established `zig-objc` pattern. `init` builds a `CalderaScriptHandler` class,
  wires it as a `WKUserContentController` message handler under the name "caldera",
  creates a `WKWebViewConfiguration`, allocates a transparent `WKWebView` (hidden,
  `drawsBackground=false`), adds it as a subview of the container, and loads the
  HTML string. Public API: `show`/`hide` (toggle visibility + first-responder),
  `setFrame` (resize to new points dimensions), `evalJS` (evaluate JavaScript).
  `on_message` module global receives raw JSON strings from the web surface.
  ObjC-bound; no unit tests (consistent with `metal.zig` and `pty.zig`). Not yet
  imported by any Zig source — Task 7 wires it in. `zig build` exit 0,
  `zig build test` exit 0 (210/210 tests pass). Commit: dbc48ca.
- 2026-05-21 — M3 integration, Task 7 complete: modified `src/main.zig` to wire
  the webview, bridge, and palette into the app. Added a `webview` field and a
  `palette` controller to the `App` struct, embedded `palette_html` via
  `@embedFile("palette_html")`, and created the `WKWebView` in `main()`. ⌘K is
  intercepted in `onKeyDown`'s existing `mods.command` branch (after
  `handleTabKey`) and calls `summonPalette`. `handleWebMessage` decodes the
  web→native bridge messages: `ready` flushes a deferred `show`, `dismiss` hides
  the palette, `invoke` maps the command id to a `palette_mod.Action` and runs it
  (theme switch, config reload, clear screen, scroll top/bottom, quit). `onResize`
  also resizes the webview frame. `webview.zig` was first truly compiled here:
  fixed two call sites using the removed `std.fmt.allocPrintZ` — replaced with
  `std.fmt.allocPrintSentinel(..., 0)` (Zig 0.16). `zig build` exit 0,
  `zig build test` exit 0 (210/210 tests pass). Commit: cf8517b.
- 2026-05-21 — M3 complete: webview host, typed IPC bridge, command palette.
  New `src/ipc/bridge.zig` (typed Inbound/Outbound JSON messages),
  `src/app/palette.zig` (command catalog + summon/dismiss controller),
  `src/webview/webview.zig` (WKWebView host above the Metal view, transparent,
  hidden by default), `ui/palette/index.html` (the web surface, embedded via a
  build anonymous import). `main.zig` summons the palette on ⌘K and runs the
  chosen command. WebKit framework linked. 210 tests pass. Spec:
  docs/superpowers/specs/2026-05-21-m3-webview-ipc-design.md.
- 2026-05-21 — Caldera Prompt Phase 1, Task 1 complete: added `caldera-prompt`
  as a second standalone executable in the repo. Created `src/prompt/main.zig`
  (stub: prints "caldera-prompt" to stderr). Added `prompt_mod` / `prompt_exe` /
  `prompt_tests` / `run_prompt_tests` to `build.zig`; wired `run_prompt_tests`
  into the existing `test_step`. Both `zig-out/bin/caldera-console` and
  `zig-out/bin/caldera-prompt` produced by `zig build`. `zig build test` exits 0.
  No API adaptations required — Zig 0.16 build API matched the plan exactly.
  Commit: 901b8ae.
- 2026-05-21 — Caldera Prompt Phase 1, Task 2 complete: created
  `src/prompt/icons.zig` — icon glyph table with 11 icons, each with a rich
  Unicode glyph and an ASCII fallback. `main.zig` imports `icons.zig` so the test
  runner discovers the 2 inline tests. `zig build test` exits 0. Commit: 2d39411.
- 2026-05-21 — Caldera Prompt Phase 1, Task 3 complete: created
  `src/prompt/segments.zig` — `State` enum, `Segment` struct, `List` fixed-capacity
  stack buffer (max 12). `main.zig` imports `segments.zig` to enable test discovery;
  2 new tests (`List.add appends until capacity`, `List.add stops at capacity, never
  overflows`). `zig build test` exits 0. Commit: 213eae2.
- 2026-05-21 — Caldera Prompt Phase 1, Task 4 complete: created
  `src/prompt/context.zig` — `Context` struct, `Lang` enum, `detect()` via `std.c.access`;
  3 tests pass. Adaptation: plan used `makeDir(io, ...)` which does not exist in Zig 0.16
  `Io/Dir.zig`; replaced with `createDir(io, ".git", .default_dir)`. Added `test { _ = icons;
  _ = segments; _ = context; }` block to `main.zig` to force test-binary inclusion of
  sub-module tests (top-level imports alone are insufficient in Zig 0.16). `zig build test`
  exits 0; 218/218 tests passed (8 prompt, 210 main). Commit: c9fe657.
- 2026-05-21 — Caldera Prompt Phase 1, Task 5 complete: created `src/prompt/git.zig`
  with `parseStatus` (pure, 4 tests) and `query`. The plan's `std.process.Child.init/
  spawn/collectOutput` API does not exist in Zig 0.16; replaced with `std.process.run`
  (allocator, io, RunOptions) which is the idiomatic Zig 0.16 subprocess helper.
  `io` is obtained from `std.Io.Threaded.global_single_threaded.io()`. `Term` enum
  uses lowercase `.exited` in Zig 0.16. `query` cwd passed as `.cwd = .{ .path = cwd }`.
  Added `_ = git;` to `main.zig` test block. `zig build test` exits 0; 222/222 tests
  passed (12 prompt, 210 main). Commit: 8cddbaa.
- 2026-05-21 — Caldera Prompt Phase 1, Task 6 complete: created `src/prompt/render.zig`
  with `full` (two-line mineral-accent block) and `transient` (single-line collapsed
  glyph). 4 tests: accent edge present, failure color, ASCII mode fallback glyphs,
  transient has no newline or edge. Added `_ = render;` to `main.zig` test block.
  No API adaptations required — `std.ArrayList(u8).empty` + explicit-allocator methods
  matched Zig 0.16 exactly. `zig build test` exits 0; 226/226 tests passed.
  Commit: 00d3eb4.
- 2026-05-21 — Caldera Prompt Phase 1, Task 7 complete: added `PromptCfg` type and
  `prompt` field to `Config` in `src/config/config.zig`. `PromptCfg` has `enabled`,
  `transient`, and `custom: []const Custom`; `Custom` has `label` and `command`.
  The nested-slice ZON literal `.{ .{ .label = "aws", .command = "echo prod" } }`
  was accepted by `std.zon.parse` without adaptation. 2 new tests pass. `zig build
  test` exits 0; 228/228 tests passed.
- 2026-05-21 — Caldera Prompt Phase 1, Task 8 complete: created
  `src/prompt/build_segments.zig` — adaptive segment assembler. `Inputs` struct
  bundles cwd_base, context, git_info, exit_code, and a scratch buffer; `assemble`
  constructs the ordered `seg.List` (cwd always, git when in a repo, toolchain when
  language detected, container/cluster when present, error on non-zero exit).
  Adaptation: plan referenced `seg.Icon` but `segments.zig` does not re-export
  `Icon`; the last test uses `icons.Icon.err` instead (task note anticipated this).
  Added `_ = build_segments;` to `main.zig` test block. 4 new tests pass.
  `zig build test` exits 0; 232/232 tests passed. Commit: ad17fd8.
- 2026-05-21 — Caldera Prompt Phase 1, Task 9 complete: wired `src/prompt/main.zig`.
  `main` accepts `std.process.Init.Minimal`; args parsed via `std.process.Args.Iterator.init`.
  Stdout written with `std.c.write(1, ...)` (same pattern as `shell_integration.zig`).
  Two Zig 0.16 adaptations: (1) `git.zig` `.stderr = .ignore` removed from `RunOptions`
  (field does not exist — `run` always captures both streams into `RunResult`); (2)
  `git.query` replaced `global_single_threaded.io()` with a proper `Threaded.init(c_allocator, .{})`
  because `global_single_threaded.allocator = .failing` causes `processSpawnPosix` to OOM
  when building the argv arena. Three invocations verified: default (two-line prompt with
  repo, git branch+dirty, toolchain segments), `--transient` (single `❯ ` line), `--exit 1`
  (failure color + exit segment). `zig build test` exits 0; all tests pass.
- 2026-05-21 — Caldera Prompt Phase 1, Task 10 complete: shell integration wired to
  drive PROMPT/PS1 from `caldera-prompt`. `promptBinaryPath` helper added to
  `src/app/shell_integration.zig` using `std.c._NSGetExecutablePath` (already in Zig 0.16
  `std.c`; avoids the `Io`-parameter requirement of `std.process.executablePath`). Exports
  `CALDERA_PROMPT` to the running process env alongside the existing markers (before the
  `enabled` guard, so both zsh and bash get it). `caldera-integration.zsh` appended:
  `__caldera_prompt` precmd sets `PROMPT` from `caldera-prompt --exit $?`; `__caldera_transient`
  on `zle-line-finish` redraw with `--transient`. `caldera-integration.bash` appended:
  `__caldera_prompt` sets `PS1` from `caldera-prompt --exit $?` prepended to
  `PROMPT_COMMAND`. `zig build` exit 0; `zig build test` exit 0; `zsh -n` and `bash -n`
  both clean. Commit: 9eccac6.
- 2026-05-21 — Caldera prompt (Phase 1) complete: new `caldera-prompt`
  executable (`src/prompt/`) — `icons`, `segments`, `context`, `git`, `render`,
  `build_segments`, `main`. Adaptive segments (cwd, git, toolchain, container,
  cluster, exit), two-line + transient, rich/ASCII glyphs gated on
  `$CALDERA_CONSOLE`. `config.zon` gains a `prompt` section. Shell integration
  drives `PROMPT`/`PS1` from the binary. 232 tests pass (212 app + 20 prompt). Spec:
  docs/superpowers/specs/2026-05-21-caldera-prompt-design.md. Deferred: the
  bundled icon font (a one-file swap in `icons.zig`) and the Phase 2 interactive
  hover layer.
- 2026-05-21 — caldera-prompt theme-awareness: light vs dark palette, live via theme-hint file.
  App side: `src/app/shell_integration.zig` gains `writeThemeHint(is_light: bool)` (writes
  `"light"` or `"dark"` to `<runtimeDir>/theme`) and exports `CALDERA_THEME_FILE` pointing
  to that path in `setup()`. `src/main.zig` gains `themeIsLight(t: Theme) bool` (Rec. 601
  luma > 128) and calls `shell_integration.writeThemeHint` in three places: once at startup
  (after `g` initialized), once in `applyConfig` (after `g.theme` updated), and once in the
  `onTick` system-dark branch (after `g.theme` updated). Also wired into `setTheme()` for the
  palette command path. Prompt side: `src/prompt/render.zig` module-level color consts replaced
  by a `Palette` struct with `dark_palette` and `light_palette` instances (brand-grounded, ≥4.5:1
  on bone). `Options` gains `light: bool = false`; `full`, `rule`, `transient` select the palette
  from `opts.light`. `src/prompt/main.zig` adds `themeIsLight()` (reads the hint file via
  `CALDERA_THEME_FILE`) and sets `opts.light`. Tests: 1 new `writeThemeHint` test in
  `shell_integration.zig`, 1 new light-palette test in `render.zig`. 241→243 tests pass.
- 2026-05-21 — Bundled Nerd Font for prompt icons (completes the deferred icon-font
  item). `src/assets/BlexMonoNerdFontMono-Regular.ttf` (IBM Plex Mono patched with
  dev icons, family "BlexMono Nerd Font Mono") is `@embedFile`'d by
  `render/font.zig` and registered at startup via a new `registerBundled()`:
  `CGDataProviderCreateWithData` over the static embedded bytes (no release
  callback) → `CGFontCreateWithDataProvider` → `CTFontManagerRegisterGraphicsFont`.
  Best-effort — any failure logs and the app falls back to system fonts. New
  externs in `render/capi.zig` (CG data provider/font + `CTFontManagerRegister`-
  `GraphicsFont`). `main.zig` calls `registerBundled()` before the font stack and
  prepends `"BlexMono Nerd Font Mono"` to the name list — `CTFontCreateWithName`
  resolves it directly by family name (no PostScript-name lookup needed). The
  `rich` glyphs in `prompt/icons.zig` are now Nerd Font v3 codepoints; `prompt/`
  `render.zig` `full()` re-imports `icons.zig` and emits `<icon> <text>` per
  segment in rich mode (icon takes the segment colour). `zig build` exit 0;
  `zig build test` exit 0. Not committed (icons verified visually by the user).
- 2026-05-21 — Terminal scroll rework: gesture-direct with rubber-band bounce.
  Replaced `scroll_anim: f32` in `App` with `scroll_pos: f32` (fractional viewport
  offset, driven 1:1 by trackpad gesture) and `overscroll: f32` (rubber-band pull in
  device pixels, signed: positive = past-top, negative = past-bottom).
  `onScroll` body replaced: drives `scroll_pos` with a fractional delta (`dy/8`),
  routes any out-of-bounds excess into `addOverscroll` (diminishing resistance,
  1.5-cell hard cap), and calls `terminal.setViewportOffset` with the rounded value.
  `onTick` old scroll-ease block removed; replaced with a spring that approaches
  `overscroll` toward 0 at rate 0.18 (snaps to 0 when < 0.5 px).
  `renderFrame` fast-path conditioned on `scroll_pos == 0 and overscroll == 0`;
  interpolated path uses `base = floor(scroll_pos)`, `frac = scroll_pos - floor`,
  and combines scroll-term with overscroll: `y_shift_px = (1-frac)*cell_h - overscroll`.
  Sign convention: positive `y_shift_px` moves cells UP; overscroll > 0 (past-top)
  slides content DOWN, so its contribution is `-overscroll`.
  Cursor guard updated to `scroll_pos == 0 and overscroll == 0`.
  `snapAnim` updated: `g.scroll_pos = @floatFromInt(viewportOffset()); g.overscroll = 0`.
  Viewport-changing call sites updated: `onKeyDown` scrollToBottom adds `g.scroll_pos = 0`;
  `runAction` scroll_top/scroll_bottom set `scroll_pos` and add a `bounceImpulse()`
  (`cell_h * 0.9` px); `scrollToCurrentMatch` / `scrollToLine` snaps `scroll_pos`
  (no bounce). `setViewportOffset` method added to `Terminal` (clamps to `history.len()`).
  New test: `"setViewportOffset clamps to scrollback length and sets within range"`.
  243 → 244 tests pass. `zig build` exit 0.
- 2026-05-21 — Mouse text selection and Cmd-C copy added. New file
  `src/app/selection.zig`: `Selection` struct with `active`, `anchor`, `head`
  fields in content-row space; `contains(row, col)` with half-open range semantics
  (full middle rows, partial first/last); `ordered()` normalises anchor/head so
  upward/leftward drags work. 6 unit tests cover inactive, single-row forward,
  single-row reversed, multi-line, multi-line reversed, zero-width selection.
  `src/main.zig` changes: (1) `selection: Selection = .{}` field on `App`. (2) New
  `imMouseDragged`/`imMouseUp` ObjC thunks; both registered on the `CalderaTerminalView`
  class alongside the existing `mouseDown:`. (3) `eventCell(event, clamp) ?{row,col}`
  helper converts AppKit bottom-left-origin view-points to (viewport-row, col) using
  `raster_h - y*scale` for the y-flip, then subtracting `grid_pad` and `topBarRows()*cell_h`.
  (4) `onMouseDown` refactored: tab-bar clicks still switch tabs (same coordinate check);
  grid clicks start a selection (anchor=head=contentRowOfViewport(cell.row)). (5)
  `onMouseDragged` updates `selection.head` with clamped-to-grid content row. (6)
  `onMouseUp` clears if anchor==head (no-drag click). (7) Selection cleared in the PTY
  keypress path and in `resizeAllTabs`. (8) `drawCell` signature extended to accept
  `content_row: usize`; selection tint applied (`color.mix(background, accent, 0.28)`)
  before search tint so search `.current`/`.other` wins. (9) `renderFrame` fast-path
  passes `contentRowOfViewport(y)`; interpolated path computes content row inline as
  `(hist + y) -| off` (saturating when `off > y`) or `hist + y - off`. (10) `copySelection`
  (Cmd-C path): iterates `terminal.line(i)`, trims trailing blanks per row, encodes
  codepoints as UTF-8 via `std.unicode.utf8Encode`, joins rows with `\n`, writes to
  `NSPasteboard generalPasteboard` via `setString:forType:` with type `public.utf8-plain-text`.
  Selection tint color: `color.mix(theme.background, theme.accent, 0.28)`.
  Interpolated-path content-row: `(hist + y) -| off` (saturating) when `off > y`,
  else `hist + y - off`. Cmd-C hooked in `onKeyDown` `mods.command` branch before
  the final `return`. `_ = @import("app/selection.zig");` added to `main.zig` test
  block. 244 → 250 tests pass. `zig build` exit 0, `zig build test --summary all` exit 0.
- 2026-05-21 — Dynamic prompt colors and renderer-drawn separator. Five sub-tasks:
  (A) `src/prompt/render.zig`: replaced `Palette` struct + `dark_palette`/`light_palette`
  instances with module-level indexed ANSI color constants (`\x1b[38;5;Nm` form).
  `segColor` no longer takes a palette pointer. `Options` drops `light: bool` and `width: usize`.
  Old prompts in scrollback now recolor on theme switch because indexed colors are
  re-resolved through the active theme palette by the renderer each frame.
  (B) `src/prompt/render.zig` + `src/prompt/main.zig`: removed `rule()`, `ruleWithPalette()`,
  `Args.rule`, and the `--rule` branch. `full()` no longer prepends a rule line. Renderer
  draws the separator from OSC 133;A marks instead. `--width` accepted but ignored (forward-compat).
  (C) Theme-hint machinery removed: `src/app/shell_integration.zig` drops `writeThemeHint()`
  and the `CALDERA_THEME_FILE` export from `setup()`. `src/main.zig` drops `themeIsLight()`
  and all four `shell_integration.writeThemeHint(...)` call sites. `src/prompt/main.zig`
  drops `themeIsLight()` and the `opts.light` field.
  (D) `src/terminal/terminal.zig`: added `isPromptStart(abs_line) bool` and
  `absoluteLineOfContent(content_row) usize` (= evicted_lines + content_row). Added
  duplicate-suppression for consecutive prompt_start marks on the same absolute line in
  `recordPromptMark`. Updated the eviction-cap test to advance cursor between marks.
  4 new terminal tests.
  (E) `src/render/raster.zig`: added `rowRule(font, row, rgb)` — fills a 2px strip at
  the top edge of cell-row `row` (full-width, respects y_shift_px). 1 new raster test.
  `src/main.zig` `renderFrame`: calls `rowRule` after each row's cells in both loops when
  `terminal.isPromptStart(terminal.absoluteLineOfContent(crow))`.
  Separator color: `color.mix(theme.background, theme.foreground, 0.14)`.
  250 -> 255 tests pass. `zig build` exit 0.

- 2026-05-21 — Terminal resize pre-scroll bug fix. `Terminal.resize` in
  `src/terminal/terminal.zig` now pre-scrolls the primary grid before resizing
  when the new height cannot hold the cursor's current row. Each displaced top
  row is archived into scrollback via the existing `archive` path (same as
  `lineFeed`). `cur_y` is decremented by one after each `scrollUp(1)` call.
  The full-screen region is set before the scroll loop so any active DECSTBM
  sub-region does not interfere (resize resets it anyway). Growing and
  cursor-fits-already shrinks are unchanged. 4 new tests in `terminal.zig`:
  "shrink past cursor anchors cursor to bottom and archives overflow",
  "shrink that does not overflow the cursor leaves scrollback unchanged",
  "grow preserves content and cursor and leaves scrollback unchanged",
  "grow then shrink round trip leaves the cursor line visible".
  Grid primitive used: `Grid.scrollUp(1)` (existing, returns displaced row).
  255 → 259 tests pass. `zig build` exit 0.
- 2026-05-22 — HUD redesigned as floating overlay. `src/render/hud.zig` rewritten:
  `Hud` struct drops `mem_pct`, gains `cwd`/`cwd_len` (active terminal cwd, last two
  path components via new `formatCwd`). `draw` signature changed from `start_col` to
  `total_cols`; card computes its own corner at `(total_cols - hud_cols - 1, top_offset + 1)`.
  Card is `hud_cols × card_rows` (30 × 11 cells), filled with a gentle panel_bg
  (`color.mix(background, foreground, 0.06)`), bordered with 1-device-pixel ash strips
  via new `Raster.fillPixelRect`. Sections: cwd (new), git (unchanged data), last-run.
  `mem` section and `queryMemPct`/`queryRamTotal` removed from `main.zig`. `resizeAllTabs`
  no longer subtracts `hud_cols + 1` — terminal always has full width. HUD toggle
  (`runAction`/`handleTabKey`) no longer calls `resizeAllTabs`. `src/render/raster.zig`
  gains `fillPixelRect(px, py, pw, ph, rgb)`. 273/273 tests pass (3 new formatCwd tests).
- 2026-05-22 — File-tree panel added. New `src/app/filetree.zig` (model) and
  `src/render/filetree.zig` (renderer). Panel is 26 columns wide, left-edge,
  toggled by `cmd+e` keybinding or "Toggle File Tree" palette action. Tree is
  rooted at the active terminal's cwd (via OSC 7 `cwdPath()`). Folders expand/
  collapse in-place (toggle); files paste their name + space into the PTY.
  Directory reads use `std.c.opendir`/`readdir`/`closedir` (same POSIX pattern
  as `config.zig`). On macOS, `dirent.name` and `dirent.namlen` are the correct
  field names (not `d_name`/`d_namlen`); `O.ACCMODE = .WRONLY` replaces the
  absent `O.WRONLY` flag in the packed struct. Max 2000 entries cap. Dirs sort
  before files, each group alphabetically. Nerd Font icons: folder closed
  U+F07B, folder open U+F07C, file U+F15B; dir icons in info-teal, file names
  in alloy-grey. `Raster.x_offset` added (analogous to `y_shift_px`): set to
  `tree_cols * cell_w` before drawing the terminal grid/cursor/rowRules, reset
  to 0 before drawing tab bar, HUD, search bar, and tree panel. `eventCell`
  shifts `grid_left` by the tree pixel width when visible. `onMouseDown` checks
  tree panel first and routes to `tree.toggle` or PTY write; returns early so
  selection does not start. `resizeAllTabs` subtracts `tree_cols` from available
  cols when tree is visible. 5 new filetree tests; 273 → 278 tests pass.
  `zig build` and `zig build test --summary all` both exit 0.
- 2026-05-22 — Interactive terminal: file-tree open, ⌘-click paths/URLs, ⌘↑/↓ prompt jump.
  Three changes to `src/main.zig` + new `src/app/interact.zig`:
  (1) File-tree click (Task 1): changed the file-click branch in `onMouseDown` from writing
  the bare filename to writing `\x15${EDITOR:-open} '<abs_path>'\n` to the PTY. Path comes
  from `Entry.pathSlice()` (absolute path already stored in the model). Single-quotes in the
  path are shell-escaped as `'\''`. New helper `ptyWriteOpenFile(path)`.
  (2) ⌘-click (Task 2): `onMouseDown` now reads modifier flags and, when the Command bit is
  set, decodes the clicked terminal row to UTF-8, extracts the whitespace-delimited token at
  the clicked column, strips any trailing `:line` or `:line:col` suffix, and classifies the
  result. URLs (`http://`/`https://`) → `\x15open '<url>'\n`; paths (contains `/`, has `.ext`,
  or exists relative to cwd) → `ptyWriteOpenFile`; neither → no-op. ⌘-click never starts a
  selection. New helper `ptyWriteOpenUrl(url)`.
  (3) ⌘↑/⌘↓ prompt jump (Task 3): `onKeyDown`'s command branch reads `keyCode` directly
  (125=Down, 126=Up; arrow keys have no character codepoint). ⌘↑ (`jumpToPrevPrompt`) finds
  the nearest `prompt_start` mark above the current viewport top; ⌘↓ (`jumpToNextPrompt`)
  finds the nearest below, or scrolls to the live bottom. Absolute mark lines converted to
  content rows via `abs - evicted_lines`. Scroll state updated the same way as
  `runAction .scroll_top`/`.scroll_bottom`. ⌘↑/⌘↓ were previously unbound.
  New file `src/app/interact.zig` holds the pure, testable helpers: `tokenAtCol`,
  `stripLineSuffix`, `classify` (+ `Kind` enum). `fileExistsRelative` uses `std.c.access`.
  16 new unit tests (all in `interact.zig`). 278 → 294 tests pass.
  `zig build` and `zig build test --summary all` both exit 0.
- 2026-05-22 — Visual polish pass: surface hierarchy introduced. Added `surface: [3]u8`
  and `border: [3]u8` fields to `Theme` in `src/config/theme.zig`. Values:
  `mineral_dark` surface=#22262f (clear lift above canvas #181a21), border=#363c49.
  `mineral_light` surface=#ffffff (white — BRAND.md "raised light panels only"), border=#d4d9dc.
  Light theme `ansi[7]` changed from #c5cace to #7a828b (confident mid-grey,
  replaces the near-invisible former value). Four renderers reworked:
  HUD (`src/render/hud.zig`): panel bg → `theme.surface` at alpha 0.92; border →
  `theme.border`; card height 11→13 rows; 1 blank row at top + 1-col inner left
  padding (bullet at col+1, text at col+3). File tree (`src/render/filetree.zig`):
  panel bg → `theme.surface` (opaque); right border → `theme.border`; 1-col left
  inner padding added to all entries; 1-col right margin. Cheatsheet
  (`src/render/cheatsheet.zig`): card bg → `theme.surface` at alpha 0.97; border →
  `theme.border`; inner padding 2→3 cols; right margin 1→2 cols; desc column 19→20.
  Tab bar (`src/render/tabbar.zig`): active tab → `theme.surface` (raised); inactive →
  `theme.background` (flat canvas); active label fg → `theme.foreground`; inactive
  label → `theme.ansi[8]` (dimmed); label left padding 1→2 cols; 1-px `theme.border`
  strip along bar bottom. Stale hardcoded `ash` and `charcoal` constants removed from
  all four files. 300/300 tests pass (unchanged count).
- 2026-05-22 — WCAG contrast retune of `mineral_dark` and `mineral_light` themes.
  Added `contrastRatio(a, b: [3]u8) f64` helper (WCAG 2.x formula) to
  `src/config/theme.zig`. Audited every theme color against its background and fixed
  all failures. Dark theme fixes: `border` nudged from `#363c49` to `#3a404e`
  (1.37 → 1.46:1 vs surface); `ansi[0]` black lifted from `#222530` to `#2c303e`
  (1.14 → 1.32:1 — just distinguishable); `ansi[8]` bright-black lifted from
  `#6d7488` to `#868ea6` (3.73 → 5.32:1 — previously the main "hard to see" color).
  Light theme fixes: `accent` and `ansi[6]` darkened from `#2c7a82` to `#286e76`
  (4.39 → 5.16:1); `ansi[7]` replaced `#7a828b` with `#5e656d` (3.43 → 5.20:1);
  `ansi[9]` bright-red darkened from `#c44a3c` to `#ad4033` (4.21 → 5.20:1);
  `ansi[15]` bright-white replaced: original `#f6f8f9` was 1.07:1 on bone (invisible)
  — now `#5f676f` mid steel-grey at 5.06:1. No hue shifts; all colors stay within their
  brand hue families. Added 4 new tests: `contrastRatio` sanity (black/white = 21,
  identical = 1), `mineral_dark WCAG contrast targets`, `mineral_light WCAG contrast
  targets`. 311/311 tests pass (up from 307; 4 new contrast tests).
- 2026-05-21 — Project renamed from Caldera Console / `caldera-console` to Anvil / `anvil`.
  All source files, build files, shell scripts, brand assets, docs, wiki, and agent files
  updated. Shell scripts renamed via `git mv`: `src/shell/caldera-integration.zsh` →
  `src/shell/anvil-integration.zsh`, `src/shell/caldera-integration.bash` →
  `src/shell/anvil-integration.bash`. Binary names updated in `build.zig`: `anvil` and
  `anvil-prompt`. Config path changed to `~/.config/anvil/config.zon`. Runtime cache dir
  changed to `~/.cache/anvil/shell`. Environment variables renamed: `CALDERA_CONSOLE` →
  `ANVIL`, `CALDERA_PROMPT` → `ANVIL_PROMPT`, `CALDERA_SHELL_INTEGRATION` →
  `ANVIL_SHELL_INTEGRATION`, `CALDERA_SHELL_INTEGRATION_ZSH` → `ANVIL_SHELL_INTEGRATION_ZSH`,
  `CALDERA_REAL_ZDOTDIR` → `ANVIL_REAL_ZDOTDIR`, `CALDERA_ZSH_LOADED` → `ANVIL_ZSH_LOADED`,
  `CALDERA_BASH_LOADED` → `ANVIL_BASH_LOADED`, `CALDERA_EXIT` → `ANVIL_EXIT`.
  ObjC class names updated: `CalderaDelegate` → `AnvilDelegate`, `CalderaTerminalView` →
  `AnvilTerminalView`, `CalderaScriptHandler` → `AnvilScriptHandler`. JS bridge names updated:
  `window.caldera` → `window.anvil`, `messageHandlers.caldera` → `messageHandlers.anvil`.
  `build.zig.zon` fingerprint updated to match new package name. 255/255 tests pass.
- 2026-05-22 — Neovim/LazyVim compatibility (three gaps): (1) modified special
  keys + F1-F12 in `keys.zig`/`main.zig` — CSI 1;m form for cursor/edit keys
  with modifiers, SS3 + tilde sequences for F1-F12, keycodes mapped in
  `extractKey`; (2) mouse reporting in `main.zig` — SGR and legacy X10 encoding
  via `keys.encodeMouse`, `onMouseDown`/`onMouseDragged`/`onMouseUp`/`onScroll`
  forward events to PTY when `mouse_button`/`mouse_x10` is set; (3) DECSCUSR
  (`CSI Ps SP q`) in `terminal.zig` — `app_cursor_shape`/`app_cursor_blink`
  fields, `applyDecscusr`, `drawCursor` prefers app request over config default.
  12 new tests; 312/312 tests pass.
- 2026-05-22 — UI polish pass (21 items from design audit against BRAND.md).
  Item 0: `applyConfig` resets `last_blink_opacity = -1` on live reload.
  Items 1/10/16/5/8: HUD — fully opaque `surface` bg, 2px border, 2-col right margin,
  hairline rules between sections, value rows indent to col+4.
  Item 2/7: Searchbar — `surface` bg, 1px `border` top separator, 2-col text indent.
  Items 3/6/18: Tab bar — UTF-8 codepoint rendering, 2px accent top bar on active tab,
  3-col right label margin guard.
  Item 4: Prompt rule mix weight 0.14 → 0.28.
  Items 9/16b/17: Cheatsheet — title in `accent`, border rules between section headers
  (skip first), header text in `foreground`, 2px card border, card_cols 52→42, desc_col +18.
  Item 11: `rowRule` gains `x_start`/`x_end` params; callers pass tree-offset bounds.
  Item 12: `FileTree.selected_idx`; renderer fills selected row with accent tint; click
  handler sets `selected_idx`.
  Item 13: Cursor glide rate 0.45 → 0.30.
  Item 14: Bounce impulse `cell_h * 0.9` → `* 0.5`.
  Item 15: `git_color` in prompt/render.zig → ANSI 6 (teal).
  Item 19: File tree panel header row "FILES" in `info_teal`; border below; entries
  start one row lower.
  Item 20: `grid_pad` 22 → 24. 312/312 tests pass.
- 2026-05-22 — Foundation-hardening regression suite implemented. Added 12 new
  tests (312 → 324). New files: `src/render/draw.zig` (per-row draw loop, R1/R2
  refactor; 3 tests), `src/testing/counting_allocator.zig` (2 tests). Changed
  files: `src/render/metal.zig` (presentMode + test, R3), `src/render/raster.zig`
  (2 new stale-separator tests), `src/render/filetree.zig` (treeRowAtClick
  extraction + test), `src/terminal/grid.zig` (grid resize matrix, 1 test),
  `src/terminal/terminal.zig` (terminal resize matrix + degenerate smoke test,
  2 tests), `src/main.zig` (renderFrame delegates to draw.drawViewport; uses
  metal_mod.presentMode; onMouseDown uses filetree_render.treeRowAtClick). Freeze
  (Bug B) was the HUD git-status thread, not a resize issue — encoded as a
  bounded-termination smoke test. Ghosting end-to-end is manual QA only. See
  [[concepts/hardening-net]] and [[decisions/regression-harness-foundation]].

## 2026-05-22 — workspace/layout.zig: Phase 1 pane-tree (builder)

Created `src/workspace/layout.zig` — a new, self-contained pure-geometry
pane-tree module with no callers. Wired into the test aggregator in
`src/main.zig`. Test count: 299 -> 311 (+12).

Key implementation decisions:
- `PaneTree.layout` takes `*std.ArrayListUnmanaged(LayoutEntry)` + an
  allocator; caller clears before each call — allocation-free in steady state.
- `PaneTree.hitTest` and `neighbor` take an allocator for their temporary
  layout buffer (avoids a fixed-size stack array).
- `PaneTree.empty` flag guards `deinit` after the last leaf is closed.
- Root-replacement in `split`: when the focused leaf IS the root, allocate a
  fresh node for the old leaf content and overwrite root.* in place. This avoids
  the self-referential cycle that results from storing the root pointer as a
  child and then overwriting root.* with the split.
- `collapseParent` uses `findParentOf` (a separate tree walk) rather than
  re-using the leaf-find result, which would return the collapsing split itself
  as the "parent" of the surviving child.
- `adjustRatio` preserves the pair-sum exactly by clamping one side first and
  computing the other as `total - new_i`.

## 2026-05-22 — workspace Phases 2–4: Pane, PaneRegistry, Tab tree model (builder)

Three sequential behavior-preserving refactors on top of Phase 1. All three
verified with `zig build` (exit 0) and `zig build test` (exit 0, same 336
test count). App behavior identical: single-pane terminal per tab, all input,
scroll, selection, copy, search, and HUD features working as before.

**Phase 2 — Extract `Pane` from `Tab`**
- Created `src/workspace/pane.zig`: `Pane` struct owns Terminal + Pty + the
  PTY-reader handoff buffer (verbatim copy of the proven code from `Tab`).
  Per-pane view state defaulted as fields: `scroll_pos`, `overscroll`,
  `overscroll_target`, `cursor_ax`, `cursor_ay`, `selection`.
  `PaneId` re-exported from `layout.zig`.
- Updated `src/app/tab.zig`: `Tab` now owns `*Pane` instead of bare Terminal/Pty
  fields. `Tab.create` creates a `Pane` (id=0 placeholder); `Tab.startReader`
  delegates to `pane.startReader()`; `Tab.deinit` calls `pane.deinit()` only.
  `label` reads through `pane.terminal`. `spawnIn` moved to `pane.zig`.
  Existing `label` test updated to construct a stack `Pane` directly.
- Updated `src/main.zig`: every `.terminal` / `.pty` / `.drain()` / `.isDead()`
  access on a `Tab` routed through `.pane`. Added `workspace/pane.zig` to the
  test aggregator.

**Phase 3 — Move view state onto `Pane`; add `focusedPane()`**
- Removed `cursor_ax`, `cursor_ay`, `scroll_pos`, `overscroll`, `overscroll_target`,
  `selection` from `App` struct in `main.zig`.
- Added `focusedPane() *Pane` helper in `main.zig` (returns `g.tabs.current().pane`).
- All ~30 read/write sites of those fields in `main.zig` rewritten to go through
  `focusedPane()`: `snapAnim`, `onTick` cursor glide and rubber-band, `runAction`
  scroll_top/bottom, `jumpToPrev/NextPrompt`, `scrollToCurrentMatch`, `onKeyDown`
  normal-key path, `addOverscroll`, `onScroll`, `onMouseDown/Dragged/Up`,
  `copySelection`, `renderFrame`.

**Phase 4 — `Tab` owns a `PaneTree` + `PaneRegistry`**
- Added `PaneRegistry` to `src/workspace/pane.zig`: wraps
  `std.AutoHashMapUnmanaged(PaneId, *Pane)` + monotonic `next_id`. `remove` is
  the only path that calls `Pane.deinit`. `deinit` iterates values and deinits all.
- Updated `src/app/tab.zig`: `Tab` now owns `tree: PaneTree` and
  `registry: PaneRegistry`. `Tab.create` calls `registry.createAndRegister` (allocates
  the first pane, gets its id), then `PaneTree.initSingle(alloc, first_id)`.
  `Tab.focusedPane()` resolves `tree.focused → registry.get(id)`.
  `Tab.startReader` iterates the registry. `Tab.deinit` calls `registry.deinit`
  then `tree.deinit`. `label` resolves through `registry.get(tree.focused)`.
  `label` test updated to build a stack `Pane`, registry, and tree explicitly.
- Updated `src/main.zig`: `focusedPane()` calls `g.tabs.current().focusedPane()`.
  Drain loop: for each tab, iterate `tab.registry.map.valueIterator()` and drain
  each pane. Dead-pane check: `tab.focusedPane().isDead()`. Resize loop: iterate
  registry values per tab.

Key decisions:
- `PaneRegistry.remove` takes no allocator — `fetchRemove` does not shrink the
  backing allocation; `Pane.deinit` uses its own stored allocator.
- `Tab` keeps `focusedPane()` as a method so `main.zig`'s module-level
  `focusedPane()` helper stays a thin one-liner across all phases.
- `spawnIn` moved from `tab.zig` to `pane.zig` in Phase 2 (it belongs to Pane now).
- 2026-05-22 — Phase 5 (split-pane render model) complete. New file
  `src/render/workspace.zig` with `drawWorkspace`. Replaced `Raster.x_offset`
  with `origin_x: f64` and `origin_y: f64` — the device-pixel position of
  cell-column 0 / cell-row 0 for the pane currently being drawn. Updated
  `cellRect`, `colRule`, and `rowRule` to use `origin_x`/`origin_y` instead of
  `pad_x + x_offset`/`pad_y`. `renderFrame` now computes an `inner` content
  `Rect` (window minus top-bar and file-tree panel) and calls `drawWorkspace`
  instead of the old single `drawViewport` call. `drawWorkspace` calls
  `tree.layout(inner, div_px, ...)` using a `FixedBufferAllocator` over a
  64-entry stack array (zero heap allocations in steady state), then calls
  `drawViewport` once per leaf with `top_bar_rows=0` (the inner rect already
  encodes the offset via `origin_y`). Dock chrome (tab bar, HUD, search bar,
  file tree, cheatsheet) resets `origin_x/origin_y=0` before drawing via the
  reset at the end of `drawWorkspace`. Bleed guard: "divider overdraw" —
  dividers are drawn LAST over all pane content, so any partial row that
  smooth-scroll bleeds into the gutter is overdrawn by `theme.border` fill.
  No clip added to `drawViewport` (avoids hot-path branches; divider gutter is
  `divider_px=8` device pixels, wide enough to cover one partial cell row).
  `filetree.zig` doc comments updated from `x_offset` to `origin_x/origin_y`.
  `main.zig` test block includes `render/workspace.zig`.
  3 new tests in `workspace.zig`: single-leaf behaviour preservation (leaf rect
  equals inner rect, origin reset after call), 2-leaf zero-allocation (CountingAllocator
  asserts 0 heap allocs per frame), 2-leaf vertical bleed guard (divider pixel
  carries `theme.border` after scroll). Test count: 336 → 339. `zig build` exit 0;
  `zig build test` exits 0, 339/339 pass. Single-pane visual output unchanged.
- 2026-05-22 — Phase 6 workspace architecture: real multi-pane splitting wired end-to-end.
  Summary of changes:
  (1) Config (`src/config/config.zig`): three new keybindings in `Keybindings` struct —
      `split_right` (default `cmd+d`), `split_down` (default `cmd+shift+d`),
      `close_pane` (default `cmd+w`). The old `close_tab` default was `cmd+w`; it is
      now `""` (no binding by default) since `close_pane` subsumes it — it closes the tab
      when the last pane dies. Collision check: `cmd+d` and `cmd+shift+d` do not appear
      in any pre-existing binding. `cmd+w` was `close_tab`; that binding now delegates to
      the pane path.
  (2) Layout (`src/workspace/layout.zig`): added `DividerHit` struct and `findDividerAt`
      / `findDividerInNode` — allocation-free recursive tree walk that finds the divider
      gutter nearest a device-pixel point within a slop band.
      6 new tests: resize derivation (2-pane horizontal, 3-pane vertical ≥1 invariant),
      adjustRatio known-delta and clamping, findDividerAt hit/miss, leafCount==registry
      invariant simulation.
  (3) Workspace render (`src/render/workspace.zig`): `max_layout_entries` made `pub` so
      `main.zig` can stack-allocate layout buffers without heap.
  (4) Main (`src/main.zig`):
      - `DividerDrag` struct + `divider_drag: ?DividerDrag` in `App` for in-flight drags.
      - `max_panes_per_tab = 8` cap.
      - `CellPos`/`RasterPx` named structs (replaces anonymous struct return types, avoids
        Zig anonymous-struct type-incompatibility error across functions).
      - `innerRect()` extracts the per-frame inner content rect (avoids duplication).
      - `resizeAllTabs()` rewritten: uses `tree.layout` per tab so each pane gets its own
        cell dimensions derived from its rect, not a global single-pane size.
      - `splitFocusedPane(dir)`: creates a new pane via `registry.createAndRegister`,
        starts its reader thread, calls `tree.split`, then reflowing via `resizeAllTabs`.
        No-op (logged) when at `max_panes_per_tab`.
      - `closePane()`: calls `tree.closeLeaf` + `registry.remove`; if last pane, closes
        the tab via existing `closeAt` path (terminating the app if no tabs remain).
      - `closeDeadTabs()` rewritten: sweeps all panes in each tab (not just focused);
        a dead non-focused pane is closed without closing the tab; a dead last pane
        closes the tab; the app terminates only when all tabs are gone.
      - `handleTabKey`: `close_pane` checked before `close_tab`; `split_right` /
        `split_down` added.
      - `eventCell` refactored: routes through `tree.hitTest` to find the pane under
        the pointer; delegates to `eventCellInPane` which uses tree layout to map the
        pointer to the pane's local grid.
      - `onMouseDown`: checks dividers first (via `findDividerAt`); then click-to-focus
        (tree.hitTest → set `tree.focused`); then existing selection/mouse-reporting logic.
      - `onMouseDragged`: if `divider_drag` is set, computes ratio delta from start
        pointer position, resets to start ratios, applies `adjustRatio` with min-ratio
        floor (8 cols / 3 rows), updates terminal grids (no SIGWINCH during drag).
      - `onMouseUp`: if `divider_drag`, clear it and call `resizeAllTabs` (sends SIGWINCH
        once the drag settles).
  Test count: 314 (pre-phase-6 baseline on this branch) → 320. `zig build` exit 0.
  Registry count == leafCount invariant: verified by the new `leafCount == registry-count
  invariant` test and by the split/close code paths which always pair `tree.split` with
  `registry.createAndRegister` and `tree.closeLeaf` with `registry.remove`.
- 2026-05-22 — Workspace Phase 7 complete: focus model for split panes.
  (1) Keyboard pane navigation: four new `Keybindings` fields (`focus_left/right/up/down`,
      defaults `cmd+shift+h/l/k/j`) parsed in `loadKeybindings`; `focusNeighbor(comptime dir)`
      calls `tree.neighbor`, sets `tree.focused = next`, then `snapAnim` + dirty.
      Chord selection: `ctrl+h/j/k/l` conflict with terminal sequences (backspace, newline,
      VT ctrl); `cmd+opt+h` is macOS "Hide Others" (system-intercepted). `cmd+shift+h/j/k/l`
      have no system-level or existing Anvil conflicts (verified against all Keybindings fields
      and the onKeyDown command-branch guard `!mods.option` on the palette chord).
  (2) Focused-pane accent border: `drawDividers` now draws a 2px `theme.accent` (Mineral teal)
      border at the boundary of the focused pane's rect — only when `entries.len >= 2`
      (single pane shows no border). Border is drawn after all pane content (overdraw strategy,
      consistent with divider fill). Sits in gutter on gutter-adjacent sides; at pane edge on
      window-boundary sides (CG clips out-of-bounds fills).
  (3) Cursor single-focus confirmed: `drawWorkspace` passes `cursor_params = null` for every
      non-focused pane (code at workspace.zig lines 83-91); no fix needed.
  (4) ZON parse branch quota: added `@setEvalBranchQuota(4000)` at the top of `parseSlice`
      in `config.zig`; adding 4 new `Keybindings` fields pushed past the default 1300-branch
      limit in `std.zon.parse.fromSliceAlloc`'s `inline for` over struct fields.
  New tests: "focus navigation: neighbor sequence updates focused, edge returns null"
  (`layout.zig`) and "drawWorkspace: cursor_params only for focused pane, accent border only
  for multi-pane" (`workspace.zig`). Test count: 345 → 348. `zig build` exit 0.
  New wiki page: [[concepts/workspace-panes]] (complete pane system reference); linked from
  [[index]].
- 2026-05-22 — AG1 agent-panel surface. New files: `src/caldera/poller.zig` (types only:
  `Connection`, `RunStatus`, `AgentRunRow`, `ApprovalRow`, `FindingRow`, `Snapshot` — no
  thread, no networking) and `src/render/agent_panel.zig` (the redesigned HUD surface).
  Deleted: `src/render/hud.zig` (fully absorbed).
  `agent_panel.zig` draws a floating top-right card with: (1) a header row — status bullet +
  "agents" label + a one-line summary driven by `Snapshot.connection`; (2) up to 3 priority
  rows (pending approvals ▸ attention-amber, running runs ◆ agent-violet, failure findings ✗
  failure-red); (3) a hairline separator; (4) a compact Local footer (cwd · branch · last-run).
  `LocalContext` replaces the old `Hud` struct; all formatters (`formatDuration`,
  `formatRunStatus`, `formatAheadBehind`, `formatCwd`) moved unchanged from `hud.zig`.
  Honest empty/degraded states: `not_installed` → "caldera-local not found" (alloy bullet);
  `no_project` / `disabled` / `offline` / `error_state` similarly; `live` + no activity →
  "no active runs" (verified-green bullet). `Placement` union with `.floating` (exercised)
  and `.docked: Rect` (defined, not exercised). Bullet color priority: failure > attention >
  agent_violet > verified > alloy.
  `src/main.zig`: `hud_mod` import → `agent_panel`; `App.hud: hud_mod.Hud` → `local_ctx:
  agent_panel.LocalContext` + `agent_snap: agent_panel.Snapshot`; `GitJob.state` →
  `agent_panel.GitState`; `refreshHud` fields updated; `renderFrame` calls `agent_panel.draw`
  with `.floating` placement and `expanded = false`; test aggregator updated.
  Theme tokens: `status.agent` (#6a5fa3 violet) and `status.risk` (#a8623a) defined as
  brand constants in `agent_panel.zig` — not added to `Theme` struct (which is a terminal
  color contract; expanding it would break WCAG contrast tests). Same pattern as `alloy`,
  `verified`, etc. in `hud.zig`.
  Startup renders the honest `not_installed` card ("caldera-local not found") with a working
  Local footer. 348 → 367 tests (+19: 12 moved hud formatter tests + 7 new agent-surface
  tests). `zig build` exit 0. `zig build test` exit 0.
- 2026-05-22 — P0 Rust port scaffold committed on `rust-port` branch. Added root
  `Cargo.toml` (workspace resolver "3", edition 2024, rust-version 1.85, shared
  `[workspace.package]` and `[workspace.dependencies]`, `[profile.release]` with
  lto="thin"). Created 12 member crates under `crates/`: 10 library crates and 2
  binary crates — `anvil-term`, `anvil-agent`, `anvil-theme`, `anvil-workspace`,
  `anvil-config`, `anvil-render`, `anvil-caldera`, `anvil-control`,
  `anvil-prompt-core`, `anvil-platform`, `anvil` (bin), `anvil-prompt` (bin).
  Dependency edges wired in each `Cargo.toml` stub (no logic). `anvil-agent`
  confirmed dependency-free internally. `/target` appended to `.gitignore`.
  Decision page [[decisions/0004-rust-port]] written; [[decisions/0002-tech-stack]]
  marked superseded with pointer to 0004. objc2 = "0.6", objc2-* family = "0.3".
  Verification: `cargo build` compiles all 12 crates; `cargo test` 0 tests exit 0;
  `cargo fmt --check` passes; `zig build` still exits 0 (no Zig files touched).
- 2026-05-22 — objc2 de-risking spike completed at `/tmp/objc2-spike`.
  Versions used: objc2 = "0.6.4", objc2-foundation/app-kit/metal/quartz-core = "0.3.2",
  block2 = "0.6.2". Rust 1.95.0 (Homebrew), cargo 1.95.0.
  Findings: (1) `define_class!` compiles and works for both NSView subclass (with
  NSResponder chain) and NSObject subclass. Multi-super chain syntax is the paren
  form: `#[unsafe(super(NSView, NSResponder, NSObject))]`; single-super can use
  `#[unsafe(super = NSObject)]`. (2) Ivars holding `Rc<RefCell<u32>>` are declared
  via `#[ivars = MyIvars]` on a plain Rust struct; accessed via `self.ivars()`;
  mutation uses `.borrow_mut()` — the exact pattern the Anvil App will use.
  (3) Initialization sequence: `Self::alloc(mtm).set_ivars(ivars)` then
  `msg_send![super(this), initWithFrame: frame]` returns `Retained<Self>`. Type
  annotation on the binding is required or the compiler cannot infer it.
  (4) Protocol conformance inside `define_class!` uses `unsafe impl NSApplicationDelegate
  for Delegate {}` blocks; method bodies go inside those blocks with
  `#[unsafe(method(applicationDidFinishLaunching:))]`.
  (5) CAMetalLayer: `MTLCreateSystemDefaultDevice()` is safe (returns
  `Option<Retained<ProtocolObject<dyn MTLDevice>>>`); `metal_layer.setDevice(...)`,
  `setPixelFormat(MTLPixelFormat::BGRA8Unorm)`, `view.setWantsLayer(true)`,
  `view.setLayer(Some(&*layer))` all compile and run cleanly.
  (6) Block-based `NSTimer::scheduledTimerWithTimeInterval_repeats_block` is `unsafe`;
  uses `block2::RcBlock`; raw `*const NSApp` pointer captured in the closure is the
  required pattern since `Retained<NSApp>` is not `Send`/`Sync`.
  (7) AppKit methods like `setWantsLayer`, `setLayer`, `setContentView`,
  `makeFirstResponder` are safe in 0.3.2; `initWithContentRect_styleMask_backing_defer`,
  `setReleasedWhenClosed`, `terminate` remain `unsafe`.
  (8) Features: individual type feature flags are required; `"all"` is not a valid
  feature in the 0.3.2 crates. NSApplication is in `objc2-app-kit` (not
  `objc2-foundation`). `NSTimer` and `block2` are features of `objc2-foundation`.
  Run output: `applicationDidFinishLaunching` fired, CAMetalLayer attached=true,
  timer fired after ~1 s, `terminate` called cleanly, run loop exited.
  Verdict: approach is sound. See spike source at `/tmp/objc2-spike/src/main.rs`.
- 2026-05-22 — P5 Zig→Rust port: `anvil-prompt` and `anvil-prompt-core` fully
  implemented. Six Zig source modules ported to idiomatic Rust:
  `icons.rs` (Icon enum + glyph() fn), `segments.rs` (State/Segment/List),
  `context.rs` (Lang/Context/detect() via std::path::Path::exists),
  `git.rs` (parse_status() pure + query() via std::process::Command),
  `build_segments.rs` (assemble() with String ownership instead of scratch buf),
  `render.rs` (full()/transient() with ANSI indexed colors + shell zero-width markers).
  Entry point: `crates/anvil-prompt/src/main.rs` — thin binary, arg parsing,
  calls core, writes stdout via print!.
  Test count: 24 Rust tests = 24 Zig tests (exact parity, all pass).
  Arg convention: `anvil-prompt [--exit <n>] [--transient] [--shell <zsh|bash>]
  [--width <n>]` — identical to Zig version; `ANVIL` env var gates rich glyphs.
  Sanity run confirmed: emits correct two-line prompt (cwd, git branch+dirty,
  toolchain) in plain/ascii mode; `--transient`, `--exit`, `--shell zsh` all work.
  `cargo clippy -D warnings` clean; `cargo fmt` applied.
  Pre-existing `anvil-term` workspace build failure (serde/bitflags) is unrelated
  to this work and pre-dates it (confirmed via git stash check).
- 2026-05-22 — P1a Zig→Rust port: `anvil-term` terminal data model ported.
  Three Zig modules ported to idiomatic Rust in `crates/anvil-term/src/`:
  `cell.rs` (Color enum, Attrs bitflags, Cell struct — all serde-derived),
  `grid.rs` (Grid with all cursor/erase/scroll/resize operations),
  `scrollback.rs` (ring buffer, trimmed-row storage).
  Key decisions: Color union(enum) → Rust enum with data variants; Attrs packed-bool
  struct → bitflags!(u8) with manual serde (u8 transparent, avoids InternalBitFlags
  issue); allocator params dropped, Vec owns all data; grid resize matrix 10-case
  `inline for` → const CASES slice + plain for loop; scroll_up returns
  Option<&[Cell]> into a grid-owned buffer (mirrors Zig's scrolled_off pattern).
  Test count: 28 Rust tests vs 28 Zig tests (exact parity — 4 cell, 17 grid
  including full resize matrix, 7 scrollback). All 28 pass.
  cargo clippy (--tests) clean; cargo fmt applied; cargo build (workspace) clean;
  zig build test exits 0 (no Zig files touched).
- 2026-05-22 — P3 Zig→Rust port: `anvil-theme` color + theme system ported.
  `src/render/color.zig` → `crates/anvil-theme/src/color.rs` (ClearColor, ColorError,
  hex_to_clear_color, hex_to_rgb, mix). `src/config/theme.zig` →
  `crates/anvil-theme/src/theme.rs` (Theme, MINERAL_DARK, MINERAL_LIGHT, contrast_ratio,
  by_name, resolve, ThemeOverrides, AnsiOverrides). Wired via `crates/anvil-theme/src/lib.rs`.
  Key decisions: palette constants as `const Theme` (const struct literal, no const fn
  needed); WCAG linearize/luminance faithfully ported — powf(2.4) matches Zig's
  std.math.pow; ThemeOverrides/AnsiOverrides defined in anvil-theme (not anvil-config)
  so anvil-config can depend on anvil-theme for deserialization; serde + thiserror
  workspace deps added to Cargo.toml; hex_to_rgb exposed alongside hex_to_clear_color
  as both are needed by callers. Test count: 17 Rust tests vs 15 Zig tests across
  both source files (color.zig: 7 tests → 7 Rust; theme.zig: 8 tests → 10 Rust —
  two WCAG audit tests each loop 15 ANSI colors and are split into separate named tests
  per theme). All 17 pass. cargo clippy -D warnings clean; cargo fmt applied;
  zig build exits 0 (no Zig files touched).
- 2026-05-22 — P1b-1 Zig→Rust port: VT/ANSI parser ported into `anvil-term`.
  `src/terminal/parser.zig` → `crates/anvil-term/src/parser.rs`, wired in `lib.rs`.
  Structure: 14-state DFA enum (`State`), `Handler` trait (5 required methods +
  2 optional default-no-op methods `dcs_put`/`dcs_unhook`), `Parser` struct with
  all state fields as fixed arrays (no allocation). The Zig `anytype` handler
  duck-typing maps to a `&mut dyn Handler` trait object; optional DCS methods use
  Rust default trait method implementations. UTF-8 decoding in ground state,
  cross-call continuations via `utf8_buf`/`utf8_len`/`utf8_needed`; invalid sequences
  emit U+FFFD. Parameter saturation uses `overflowing_mul`/`overflowing_add`
  (matches Zig's `@mulWithOverflow`/`@addWithOverflow`). No error type needed —
  the parser never fails, it degrades gracefully.
  Test count: 39 Rust tests = 39 Zig tests (exact parity, all pass).
  Total `anvil-term` tests: 67 (28 pre-existing + 39 new parser tests), all pass.
  `cargo build -p anvil-term`, `cargo test -p anvil-term`, `cargo clippy -p anvil-term
  -- -D warnings` all clean; `cargo fmt` applied; `zig build` exits 0 (no Zig files
  touched).
- 2026-05-22 — Rust port P7a complete: `crates/anvil-agent/src/snapshot.rs` added.
  Ported all activity-snapshot types from `src/caldera/poller.zig` into the
  existing `anvil-agent` crate (leaf crate, deps: `serde` + `serde_json` only).
  Types ported: `Connection` (enum, default=`NotInstalled`), `RunStatus` (enum,
  default=`Unknown`), `FindingSeverity` (enum, default=`Info`), `AgentRunRow`,
  `ApprovalRow`, `FindingRow`, `Snapshot`. Fixed-size Zig buffer pairs
  (`[N]u8` + length field) replaced with idiomatic `String`; fixed-array
  `[8]AgentRunRow` + count replaced with `Vec<AgentRunRow>`. All types derive
  `Clone, Copy/Debug, Default, PartialEq, Eq, Serialize, Deserialize`;
  `#[serde(rename_all = "snake_case")]` on enums; `#[serde(default)]` on all
  struct fields for schema-drift tolerance (extra caldera-local fields silently
  ignored — no `deny_unknown_fields`). `pub mod snapshot` added to `lib.rs` with
  top-level re-exports. 15 tests: enum round-trips, default-value checks, struct
  round-trips, unknown-field tolerance (all four types), missing-field defaults.
  `cargo build -p anvil-agent`, `cargo test -p anvil-agent` (15/15 pass),
  `cargo clippy -p anvil-agent -- -D warnings` clean, `cargo fmt` applied,
  `zig build` exits 0.
- 2026-05-22 — Rust port P3b complete: `crates/anvil-config` fully implemented.
  Zig ZON config ported to TOML via `serde`/`toml` 1.1. All Zig config.zig structs
  ported: `Config`, `FontCfg`, `CursorCfg`, `WindowCfg`, `PromptCfg`,
  `CustomPromptSegment`, `Keybindings`, `Chord`. `ThemeOverrides`/`AnsiOverrides`
  reused from `anvil-theme` (already `Deserialize`). `clamp()` ported verbatim
  (NaN-safe via `!is_finite()` instead of Zig's `!(x >= y)` to satisfy clippy).
  `parse_str`, `load`, `resolve_path`, `Watcher` (mtime-poll via `std::fs::metadata`,
  no new deps), `parse_chord` all ported. Config path: `~/.config/anvil/config.toml`.
  Rust test count: 18 (Zig had 16 explicitly named `test "..."` blocks; Rust adds 2
  extra coverage tests for `resolve_path` and `defaults_has_expected_values`).
  One deliberate semantic difference: TOML ignores unknown fields by default (no
  `deny_unknown_fields`); the Zig test `".{ .nonsense = 1 }"` returned ParseFailed
  because ZON rejects unknown fields at parse time. The Rust port documents this
  difference in the test. New files: `crates/anvil-config/src/lib.rs` (full impl),
  `crates/anvil-config/Cargo.toml` (adds serde/toml/thiserror deps + tempfile dev-dep),
  `crates/anvil-config/config.example.toml` (documented default config),
  `src/config/mineral-dark.toml`, `src/config/mineral-light.toml` (TOML theme files,
  ZON originals untouched). `cargo build -p anvil-config`, `cargo test -p anvil-config`
  (18/18 pass), `cargo clippy -p anvil-config -- -D warnings` clean, `cargo fmt`
  applied, `zig build` exits 0.
- 2026-05-22 — Rust port phase P1b-2 complete: `terminal` and `search` ported into
  `crates/anvil-term`. New files: `crates/anvil-term/src/terminal.rs` and
  `crates/anvil-term/src/search.rs`. `lib.rs` updated to export the full public API.
  `Terminal` implements `Handler` directly; `Parser::feed` is called via a raw-pointer
  split to avoid self-borrow on the parser field. `BlockState` is a named public enum
  with `serde::Serialize/Deserialize` derives; `Block`, `PromptMark`, and `PromptMarkKind`
  also derive serde. Resize matrix ported as `const CASES: &[ResizeCase]` + for loop.
  Zig `Attrs` packed-bool struct → bitflags (`Attrs::BOLD`, `.insert()`/`.remove()`/`.contains()`).
  Test counts: terminal.zig ~84 Zig → 84 Rust; search.zig 11 Zig → 11 Rust.
  Total: 67 pre-existing + 84 terminal + 11 search = 159 tests, all passing.
  `cargo build -p anvil-term`, `cargo test -p anvil-term` (159/159 pass),
  `cargo clippy -p anvil-term -- -D warnings` clean, `cargo fmt` applied, `zig build` exits 0.
- 2026-05-22 — Rust port phase P2 complete: `crates/anvil-workspace` fully implemented.
  Ported 8 Zig source files into 8 Rust modules. Pane-tree representation: `enum
  PaneNode { Leaf(PaneId), Split(Split) }` with `Vec<Box<PaneNode>>` children — idiomatic
  Rust analogue of Zig's `*PaneNode` heap pointers. PTY seam: `Pane` owns only pure
  state (Terminal + view state); PTY + reader thread are a platform concern mapped by
  the platform layer's `HashMap<PaneId, Pty>` parallel to `PaneRegistry`.
  Module–Zig source mapping:
    layout     ← src/workspace/layout.zig (19 Zig → 19 Rust tests)
    pane       ← src/workspace/pane.zig   (0 Zig tests; PTY tests deferred to platform)
    tab        ← src/app/tab.zig          (6 Zig pure-logic tests → 6 Rust tests)
    selection  ← src/app/selection.zig    (6 Zig → 6 Rust tests)
    filetree   ← src/app/filetree.zig     (5 Zig → 5 Rust tests; I/O via std::fs::read_dir)
    palette    ← src/app/palette.zig      (7 Zig → 7 Rust tests)
    interact   ← src/app/interact.zig     (13 Zig → 16 Rust tests; interact.zig 13, extras in Rust)
    keys       ← src/app/keys.zig         (14 Zig → 14 Rust tests)
  Deferred: src/app/shell_integration.zig (calls setenv/getenv, embeds zsh/bash scripts,
  writes to ~/.cache/anvil/shell) — belongs in crates/anvil-platform.
  Total: 73/73 Rust tests pass. `cargo build -p anvil-workspace`, `cargo test -p
  anvil-workspace` (73/73 pass), `cargo clippy -p anvil-workspace -- -D warnings` clean,
  `cargo fmt` applied, `zig build` exits 0.
- 2026-05-22 — Rust port phase P4a complete: core rasterizer and draw loop ported into
  `crates/anvil-render`. New files: `crates/anvil-render/src/raster.rs` and
  `crates/anvil-render/src/draw.rs`; `lib.rs` updated to export both modules.
  Key architectural change: `Raster` owns a `Vec<u8>` BGRA8 pixel buffer; all rect/pixel
  operations are pure Rust. Glyph drawing is routed through a `GlyphPainter` trait
  (`draw_glyph(glyph_id, dest: PixelRect, fg, metrics, pixels, bitmap_width, bitmap_height)`)
  so `anvil-render` stays platform-free; `anvil-platform` will implement it with CoreText.
  Coordinate change: Zig's CG context is y-up; Rust uses top-down bitmap coordinates.
  `cell_rect`, `row_rule`, `cell_inset` arithmetic rewritten accordingly. The underline
  cursor fraction is adjusted at the call site (`fy = 1 - fh`) to match the Zig original's
  visual output. `draw_viewport` and `draw_cell` take `&mut Terminal` and snapshot each row
  via `.to_vec()` to avoid aliasing constraints — the per-frame snapshot is the only
  per-frame allocation (noted as a TODO if strict zero-alloc is required; the Zig
  CountingAllocator test is deferred to a follow-up phase).
  `draw_cursor` is a standalone function; the block cursor's cell-under-cursor glyph is
  re-drawn in `draw_viewport` (which holds `&mut Terminal`) after drawing the block.
  Test handling: 21 total Rust tests.
    raster module (12): 8 pure geometry tests ported directly with stub GlyphPainter;
      "cellBg and glyph" split into two tests (bg pixel + call recording);
      "cellInset left bar" adapted (20% width instead of 15% for probe to land safely
      at 10px cell_w; semantically equivalent). Glyph-pixel tests (CoreText-dependent)
      replaced by stub-painter call-recording tests (approach a).
    draw module (9): ruleRow x2, cursor_opacity x3, resolve_color x3, draw_viewport
      smoke x2 (no panic, glyph call recorded). Zero-alloc CountingAllocator test noted
      for follow-up; draw loop is allocation-free by construction for the raster buffer.
  `cargo build -p anvil-render`, `cargo test -p anvil-render` (21/21 pass),
  `cargo clippy -p anvil-render -- -D warnings` clean, `cargo fmt` applied, `zig build` exits 0.
- 2026-05-22 — Rust port phase P4b complete: panel renderers ported into `crates/anvil-render`.
  Six new modules (workspace, tabbar, agent_panel, searchbar, filetree, cheatsheet) plus
  updated `lib.rs`. Zig source → Rust:
    workspace.zig (3 Zig tests) → workspace.rs (4 tests; all geometry/structural, stub painter).
    tabbar.zig (1 Zig test) → tabbar.rs (3 tests; 0/2-tab no-op + paint check, stub painter).
    agent_panel.zig (19 Zig tests) → agent_panel.rs (21 tests; formatters x12 + header color x4
      + build_summary x3 + draw smoke x2, stub painter for draw tests).
    searchbar.zig (1 Zig test) → searchbar.rs (2 tests; background fill + zero-cols no-op).
    filetree.zig (1 Zig test) → filetree.rs (3 tests; tree_row_at_click x1 + draw smoke x2).
    cheatsheet.zig (6 Zig tests) → cheatsheet.rs (8 tests; data integrity x6 + draw smoke x2).
  Total: 21 pre-existing + 42 new = 63 tests. Stub GlyphPainter used for all draw tests.
  Key porting decisions: Zig static-buffer `buildHeaderSummary` → owned `String`;
  Zig `FontT.init` (CoreText) replaced with `FontMetrics` value; PaneRegistry now `&mut`
  (Rust ownership); `tab_label()` uses Terminal::title() + cwd_path() + basename() fallback.
  `cargo build -p anvil-render`, `cargo test -p anvil-render` (63/63 pass),
  `cargo clippy -p anvil-render -- -D warnings` clean, `cargo fmt` applied, `zig build` exits 0.
- 2026-05-22 — Rust port phase P6 complete: PTY layer and shell integration ported into
  `crates/anvil-platform`. New files: `crates/anvil-platform/src/pty.rs` and
  `crates/anvil-platform/src/shell_integration.rs`; `pub mod` declarations added to `lib.rs`;
  `nix = { workspace = true }`, `thiserror = { workspace = true }`, and `libc = "0.2"` added to
  `anvil-platform`'s `[dependencies]`. `nix` resolved to v0.31.3 (first use in the workspace).
  Zig→Rust mapping: `openpty` via `nix::pty::openpty`; `fork` via `nix::unistd::fork`;
  `login_tty`/`execve`/`ioctl(TIOCSWINSZ)` via raw `libc` FFI (not in nix); `TIOCSWINSZ` resize
  via `libc::TIOCSWINSZ`; Zig `deinit` → `Drop` impl (SIGHUP + waitpid + OwnedFd drop);
  `@embedFile` → `include_str!`; `setenv`/`unsetenv` → `env::set_var`/`remove_var` in
  `unsafe` blocks (Rust 2024 edition requirement); `_NSGetExecutablePath` → `std::env::current_exe()`.
  Unsafe blocks: 5 in `pty.rs` (fork, login_tty, close slave, from_raw_fd master, ioctl/read/write/
  kill/libc::execve in child_exec — all justified by fork/exec/raw-fd contract); env mutation in
  `shell_integration.rs` wrapped in `unsafe` with SAFETY comments (setup is startup-only, before
  threads). Test count: 6 Zig tests → 6 Rust PTY tests + 4 Zig tests → 4 Rust shell-integration
  tests = 10 total (Zig: 10 → Rust: 10, 1:1). Env-mutation tests serialised via `static ENV_LOCK`.
  `cargo build -p anvil-platform`, `cargo test -p anvil-platform` (10/10 pass),
  `cargo clippy -p anvil-platform -- -D warnings` clean, `cargo fmt` applied, `zig build` exits 0.
- 2026-05-22 — Rust port phase P3/P7 complete: IPC bridge ported into `crates/anvil-control`.
  New file `crates/anvil-control/src/bridge.rs`; `pub mod bridge` added to `lib.rs`;
  `serde` and `serde_json` added to `anvil-control`'s `[dependencies]`.
  Wire format: discriminator key `"type"` is identical to the Zig version. Inbound decode
  uses a private `InboundWire { kind: String, id: Option<String> }` struct (mirrors Zig's
  `Wire` anonymous struct) then hand-converts — preserves the `MissingField` / `UnknownType` /
  `InvalidJson` error distinctions from `DecodeError`. Outbound encode uses a private
  `OutboundShowWire<'a>` struct with `#[serde(rename = "type")]`; `subtitle: Option<String>`
  carries no `skip_serializing_if` so null is always emitted, matching the Zig `std.json`
  default. `encode(&Outbound::Hide)` short-circuits to the literal `{"type":"hide"}`.
  Test count: 10 Zig tests → 10 Rust bridge tests (all paths matched 1:1) + 1 pre-existing
  broker test = 11 total. `cargo build -p anvil-control`, `cargo test -p anvil-control`
  (11/11 pass), `cargo clippy -p anvil-control -- -D warnings` clean, `cargo fmt` applied,
  `zig build` exits 0.

## 2026-05-22 — P8: Metal renderer + CoreText font port to anvil-platform

- **Branch:** `rust-port`
- **Files created:** `crates/anvil-platform/src/metal.rs`, `crates/anvil-platform/src/font.rs`
- **Files modified:** `crates/anvil-platform/Cargo.toml` (added objc2-* deps), `Cargo.toml`
  (added `objc2-core-foundation` to workspace deps), `crates/anvil-platform/src/lib.rs`
  (added `pub mod font; pub mod metal;`)
- **Zig sources ported:** `src/render/metal.zig` → `metal.rs`,
  `src/render/font.zig` → `font.rs`. `src/render/capi.zig` was NOT ported — replaced by
  typed objc2-core-text / objc2-core-graphics bindings.

**Metal renderer (`metal.rs`):**
- `Renderer` struct owns `device`, `queue`, `layer` (`CAMetalLayer`), `pipeline`, `texture`,
  `width`, `height`, `clear`.
- Present path duality preserved exactly from Zig: `presentsWithTransaction` toggled per frame;
  sync path (`commit` + `waitUntilScheduled` + `drawable.present`) during live resize;
  async path (`presentDrawable:` + `commit`) otherwise.
- Runtime MSL shader compilation via `newLibraryWithSource_options_error` — no offline metal
  toolchain needed. Shader is a full-screen quad with nearest-neighbor sampler.
- Pixel buffer uploaded as `MTLTexture` via `replaceRegion_mipmapLevel_withBytes_bytesPerRow`.
- Protocol coercion for `CAMetalDrawable → MTLDrawable`: raw pointer cast
  `&*(Retained::as_ptr(&drawable) as *const ProtocolObject<dyn MTLDrawable>)`.

**CoreText font (`font.rs`):**
- TTF embedded with `include_bytes!("../../../src/assets/BlexMonoNerdFontMono-Regular.ttf")`.
- `Font::init()`: `CTFont::with_name`, `glyphs_for_characters` for 'M' advance,
  `advances_for_glyphs` for `FontMetrics`.
- `register_bundled()`: `CGDataProvider::with_data` + `CGFont::with_data_provider` +
  `CTFontManagerRegisterGraphicsFont` (deprecated API; `#[allow(deprecated)]` applied).
- `CoreTextPainter` implements `anvil_render::GlyphPainter` — `draw_glyph` creates
  `CGBitmapContextCreate` over caller's BGRA8 buffer, converts top-down bitmap y to CG y-up
  coordinates (`cg_cell_bottom = bitmap_h - dest.y - dest.h`; `baseline_y = cg_cell_bottom +
  metrics.descent`), calls `ct.draw_glyphs()`.
- `Font::glyph(cp)` handles both BMP and surrogate pairs for astral-plane codepoints.

**objc2 friction encountered:**
- Feature names vs type names don't always align; `MTLRenderPass` is a feature but not a
  directly importable type — types (`MTLRenderPassDescriptor`, `MTLClearColor`, etc.) come from
  the generated module enabled by that feature.
- Many objc2-metal 0.3 methods declared as safe `fn` (not `unsafe fn`); wrapping them in
  `unsafe {}` triggered `unused_unsafe` warnings.
- `Retained::as_ptr` is an associated function, not a method; call as `Retained::as_ptr(&x)`.
- `CFString::new()` renamed to `CFString::from_str()`; `CGColorSpace::create_device_rgb()`
  renamed to `new_device_rgb()`; `CGDataProviderCreateWithData` replaced by `with_data()`.

**Test counts:** Zig: 1 (metal) + 4 (font) = 5. Rust: 1 (metal) + 4 (font) = 5 new tests.
Total `anvil-platform` tests: 15 (5 new + 10 pre-existing PTY/shell-integration).

**Verification:** `cargo build -p anvil-platform` clean; `cargo test -p anvil-platform`
15/15 pass; `cargo clippy -p anvil-platform -- -D warnings` clean; `cargo fmt` applied;
`zig build` exits 0; `zig build test` exits 0.

- 2026-05-22 — P9 Zig→Rust port: AppKit shell + WKWebView ported into
  `crates/anvil-platform`. New files: `src/appkit.rs`, `src/webview.rs`.
  New dependencies in `Cargo.toml`: `objc2-app-kit`, `objc2-web-kit`, `block2`
  (with per-type feature flags). The `AppHandler` trait (16 methods) is the
  platform↔binary seam — the `anvil` binary will implement it in P10. Two
  `define_class!` subclasses (`AnvilTerminalView`, `AnvilDelegate`) store
  `HandlerPtr` ivars (raw pointer to boxed `Rc<RefCell<dyn AppHandler>>`).
  `AnvilScriptHandler` delivers WKWebView script messages. Public constructor:
  `AppKitApp::new(handler, width, height, title)` + `run()`. `Webview::init`
  takes a `WebviewConfig` struct. 16 new tests (modifier/keycode decoding)
  added. **Totals:** 31/31 `cargo test -p anvil-platform` pass; clippy -D warnings clean.
- 2026-05-22 — P10 capstone: `crates/anvil/src/main.rs` ported from the Zig stub to
  a full ~1200-line binary wiring all 11 library crates. Key design decisions:
  (1) `App` owns all workspace/PTY/render/config/git/palette state; `font` is `Box<Font>`
  for stable heap address. (2) `AppShell` wraps `App + Webview + CoreTextPainter<'static>`;
  `painter` holds an unsafe `&'static Font` pointing into the boxed font — sound
  because `painter` drops before `app` (field declaration order). (3) Two-phase init
  uses `Rc<RefCell<Option<AppShell>>>` shared between `ForwardingHandler` (which holds
  the `Rc` clone) and `main()` (which fills the `Option` after the Metal layer and
  Renderer are available) — no `downcast` needed. (4) `renderer: Option<Renderer>` in
  `App` avoids `mem::zeroed()` UB. (5) `all_pane_ids_in_registry` stub removed; both
  call sites now use `all_pane_ids_in_tree` (walks the layout tree at 1e6×1e6 rect).
  **Verify:** `cargo build` clean, `cargo build --release` clean, 6/6 `cargo test -p anvil`
  pass, `cargo clippy --workspace -- -D warnings` clean, `cargo fmt` applied.
- 2026-05-22 — Rust-port parity fixes. (1) `anvil-config`: added
  `#[serde(deny_unknown_fields)]` to `Config`, `FontCfg`, `CursorCfg`, `WindowCfg`,
  `CustomPromptSegment`, `PromptCfg`, `Keybindings` (in `anvil-config/src/lib.rs`) and
  to `ThemeOverrides`, `AnsiOverrides` (in `anvil-theme/src/theme.rs`). Restores Zig
  behavior: unknown keys produce a parse error. Updated `unknown_field_returns_parse_error`
  test to assert `Err`. (2) `anvil-render`: eliminated per-frame `Vec::to_vec()` snapshots
  in `draw_viewport` (`crates/anvil-render/src/draw.rs`) using pure rescope — each row
  is drawn fully inside a `{}` block so the `&[Cell]` borrow from `terminal.viewport_row`
  / `viewport_row_at` ends before the subsequent `&self` prompt-rule calls. The cursor
  block path was similarly rescoped (extracts `Option<Cell>` before the borrow drops).
  No API change to `anvil-term` was required. 419/419 tests pass, clippy clean, fmt applied.
- 2026-05-22 — P11 Zig→Rust port cleanup: removed all Zig source from the repo.
  **Assets relocated (old → new):**
  `src/assets/BlexMonoNerdFontMono-Regular.ttf` → `assets/BlexMonoNerdFontMono-Regular.ttf`;
  `src/assets/app-icon.png` → `assets/app-icon.png`;
  `src/shell/anvil-integration.zsh` → `shell/anvil-integration.zsh`;
  `src/shell/anvil-integration.bash` → `shell/anvil-integration.bash`;
  `src/shell/zdotdir-zshenv.zsh` → `shell/zdotdir-zshenv.zsh`.
  `include_bytes!`/`include_str!` paths updated in `crates/anvil-platform/src/font.rs`,
  `appkit.rs`, and `shell_integration.rs`.
  **Deleted:** `build.zig`, `build.zig.zon`, `zig-out/`, `zig-pkg/`, entire `src/` tree.
  **Docs rewritten for Rust:** `AGENTS.md`, `CLAUDE.md`, `README.md` — all references to
  `zig build` replaced with `cargo` commands; source map updated to `crates/` table.
  **`.gitignore`:** removed `.zig-cache/` and `zig-out/` entries.
  **Wiki updated:** `wiki/index.md` mission/current-state updated to Rust; `zig-0.16-gotchas.md`
  deleted and its link removed; concept pages `console-architecture`, `config-system`,
  `shell-integration`, `tab-system`, `search-system`, `workspace-panes`, `hardening-net`
  repointed from `src/*.zig` paths to `crates/*/src/*.rs` paths.
  **Verify:** `cargo build --workspace` clean; `cargo build --release` clean;
  `cargo test --workspace` all pass; `cargo clippy --workspace -- -D warnings` clean;
  `cargo fmt --all` applied; `cargo run -p anvil` launches successfully.
- 2026-05-22 — Design-lead polish audit applied (7 defects, all on `rust-port` branch).
  (1) `crates/anvil-theme/src/theme.rs`: `MINERAL_LIGHT.border` `#d4d9dc` → `#b8bec3`
      (~1.88:1 vs white; WCAG `border >= 1.4:1` test still passes at 1.88:1).
  (2) `crates/anvil-render/src/agent_panel.rs` P0: replaced 4 × 2px stroke border on
      the card with a 4px filled gutter approach — outer `(card_w+8, card_h+8)` rect
      filled with `theme.border`, then inner card filled with `theme.surface`. Visible
      on light mode where hairlines disappeared.
  (2) P1: replaced `const CARD_ROWS: usize = 13` with `fn card_rows(snap) -> usize`
      returning `4 + min(3, approvals + running + failures)`. Empty-state panel is now
      4 rows tall.
  (2) P2: `draw_agent_header` summary text color `ALLOY` → `theme.foreground`; added
      `theme: &Theme` parameter to `draw_agent_header` and threaded it through the call.
  (3) `crates/anvil-render/src/draw.rs`: `rule_rgb` mix `0.28` → `0.45` (prompt-rule
      hairlines visible); selection background mix `0.28` → `0.35`.
  (4) `crates/anvil-render/src/tabbar.rs`: bottom border `1.0` → `2.0` device pixels;
      inactive tab background `theme.background` → `anvil_theme::mix(bg, surface, 0.4)`.
  (5) `crates/anvil-render/src/filetree.rs`: `TREE_COLS` `26` → `30`.
  (6) `crates/anvil-render/src/searchbar.rs`: prefix chars 0–5 ("find: ") drawn in
      `theme.ansi[8]` (muted); query chars 6+ drawn in `theme.foreground`.
  (7) `crates/anvil-render/src/cheatsheet.rs`: `fill_pixel_rect_alpha(..., 0.97)` →
      opaque `fill_pixel_rect(...)`.
  No tests updated (no test asserted the changed old values).
  `cargo build --workspace` clean; `cargo test --workspace` all pass;
  `cargo clippy --workspace -- -D warnings` clean; `cargo fmt --all` applied.
- 2026-05-23 — `anvil-caldera` crate implemented: HTTP client + poller connecting
  Anvil's agent panel to live caldera-local data. New files in
  `crates/anvil-caldera/src/`:
  `client.rs` — intermediate response types (`RawActivity`, `RawAgentRuns`,
  `RawProject`) and conversion to `anvil-agent` canonical types; ISO timestamp
  parsing; severity string mapping (`"warning"` → `Attention`).
  Extends `CalderaClient` with `activity()`, `agent_runs()`, `project()`,
  and `post_raw()`.
  `detect.rs` — `detect_project(cwd: &Path) -> bool`: walks up from cwd
  for `.caldera/project.json` with `"enabled": true`. Pure file I/O.
  `poller.rs` — `Poller`: background thread polling every 2s with a typed
  `Msg::Kick`/`Msg::Stop` channel; `Arc<Mutex<Snapshot>>`; clean `stop()`
  via sender drop + thread join. State machine: detect → health → project →
  activity+agent-runs → `Live`; any failure sets the appropriate connection
  state and zeros data fields.
  `actions.rs` — `approve`, `ack_finding`, `start_run` POST helpers.
  `lib.rs` updated: `DEFAULT_ENDPOINT`, re-exports, new `pub mod` declarations.
  Fixture: `tests/fixtures/activity_v0.json`.
  Tests: 20 unit + 17 integration = 37 total; all pass.
  `cargo build --workspace` clean; `cargo clippy -p anvil-caldera -- -D warnings`
  clean; `cargo fmt --all` applied. `crates/anvil/src/main.rs` not touched.
- 2026-05-23 — nvim-RPC bridge phase 1: `crates/anvil-editor` created. Two modules:
  `codec.rs` — hand-rolled msgpack encode/decode for the nvim subset (nil, bool,
  positive/negative fixint, uint8/16/32/64, int8/16/32/64, float32/64, fixstr,
  str8/16/32, bin8/16/32, fixarray, array16/32, fixmap, map16/32). Ext tags return
  `UnsupportedType`. `encode_request` writes `[0, msgid, method, params]`.
  `decode_value` enforces depth cap (32) and collection length cap (256k).
  `transport.rs` — `Endpoint`, `Transport` (Unix socket + AtomicU32 msgid counter),
  `TransportError`. `call()` is synchronous: writes request, reads until matching
  msgid, discards notifications. Timeout via `set_read_timeout`. Only dep:
  `thiserror.workspace = true`; dev-dep `tempfile`. Tests: 25 codec (round-trips
  for every type, depth/oversize/eof/unsupported-type error paths, request-frame
  encode+decode) + 3 transport (happy-path, timeout, connect-nonexistent-path) =
  28 total. All pass. `cargo clippy -D warnings` clean. `cargo fmt --all` applied.
  `cargo build --workspace` clean.
- 2026-05-23 — Workspace test-coverage push: reached 85.29% total line coverage
  (target: 85%). Starting point was 82.12%. Files changed (tests only, no logic
  changes): `anvil-term/{terminal,grid,parser,search,scrollback}.rs`,
  `anvil-workspace/{filetree,interact,keys,layout,pane,selection,tab}.rs`,
  `anvil-config/lib.rs`, `anvil-editor/{codec,transport}.rs`,
  `anvil-caldera/{actions,detect,lib,poller}.rs`, `anvil-control/lib.rs`,
  `anvil-prompt-core/{build_segments,context,render,segments}.rs`.
  Hard ceiling: `anvil/src/main.rs` at 14.88% (AppKit/Metal/PTY handlers, not
  unit-testable). `anvil-prompt/src/main.rs` at 0% (binary entry point).
  `cargo test --workspace` passes (all crates green). `cargo clippy --workspace
  -- -D warnings` clean. `cargo fmt --all` applied. `cargo build -p anvil` clean.
- 2026-05-23 — Damage tracking (#3) implemented in `anvil-term` and `anvil-render`.
  Per-row dirty-bitmap added to `Grid` (`dirty: Vec<bool>`, `dirty_all: bool`);
  `mark_dirty(y)` and `mark_all_dirty()` called at all mutation sites in
  `grid.rs` (print, erase, scroll, resize). `DirtySet` public type added to
  `terminal.rs` with `all`, `none`, `contains`, `iter`, `mark`, `force_full`
  methods; `Terminal::take_dirty_rows()` drains the bitmap each frame.
  `DirtySet::none(rows)` added as a constructor for tests and callers that need
  a partial set. `Raster::clear_pixel_rows(y_top, y_bottom, rgb)` added to
  `raster.rs` for single-row band clearing. `draw_viewport` in `draw.rs` gains
  `dirty: Option<&DirtySet>` last parameter; non-dirty rows are skipped and
  their band is not cleared; `None` means full redraw. `draw_workspace` in
  `workspace.rs` gains `dirty: Option<&HashMap<PaneId, DirtySet>>` and threads
  the per-pane set through. `main.rs` wiring: `force_full_redraw` flag (set on
  resize/config/search toggle), `cursor_row_prev` map (ensures cursor row
  redraws on move), per-pane `take_dirty_rows()` + `force_full()` for
  scroll/search/selection, partial chrome-area clear on non-full frames,
  `ANVIL_RENDER_DEBUG=1` instrumentation (debug builds). New test:
  `draw_viewport_partial_dirty_skips_clean_rows` — verifies that marking only
  row 0 dirty produces fewer glyph calls than a full redraw of all rows.
  All tests pass; `cargo clippy --workspace -- -D warnings` clean.
- 2026-05-23 — Performance benchmark suite added (infrastructure-only, no app
  code changed). Criterion micro-benchmarks added to three crates:
  `anvil-term/benches/parser.rs` (throughput: plain_ascii ~415 MiB/s,
  csi_heavy ~272 MiB/s, unicode ~334 MiB/s on M-series), `anvil-term/benches/
  grid_resize.rs` (resize matrix), `anvil-render/benches/draw_viewport.rs`
  (full_redraw ~51 µs/frame, damaged_row_12 ~1.8 µs/frame → ~28× damage-tracking
  win), `anvil-platform/benches/glyph_cache.rs` (cold miss vs hot hit, macOS only).
  `scripts/bench-vs-alacritty.sh` added: drives vtebench against Alacritty,
  prints manual Anvil procedure (Anvil is a GUI app; headless PTY mode not yet
  available). `docs/perf.md` documents how to run all benches, hot-path rationale,
  and an empty "Current Numbers" table to be populated per perf push.
- 2026-05-23 — Prompt enhancements #11 and #12 implemented. Task #11: right-aligned
  exit code + duration segment on the prompt line (` ✗ 127  0.4s` on failure,
  ` ✓ 0.4s` on success); `Options` gains `width`, `duration_ms`, `exit_code` fields;
  `--duration-ms` CLI flag added to `anvil-prompt`. Task #12: `*N` dirty count inserted
  between basin and middot in attention amber when `git_dirty > 0`; dirty count sourced
  from the existing `git::query` call (no extra subprocess). Shell scripts updated:
  zsh uses `EPOCHREALTIME` for sub-second timing; bash uses `$SECONDS`. Four new unit
  tests added; 52 tests pass, clippy clean.
- 2026-05-23 — Task #14 (regex search) and Task #6 (HUD disk+mem gauges) implemented.
  Task #14: added `regex_mode: bool` field to `Search` in `crates/anvil-term/src/search.rs`;
  `set_regex(bool)` / `is_regex()` API; `rescan` dispatches to `rescan_literal` or
  `rescan_regex`; invalid regex patterns preserve last-good matches. Added `regex = "1"`
  to workspace deps and `anvil-term`. In `crates/anvil-render/src/searchbar.rs`: `.*`
  dim indicator drawn in 2 reserved columns right of query when regex mode is on.
  In `crates/anvil/src/main.rs`: `search_regex_toggle` field added to `Keybindings`,
  hardcoded to `cmd+opt+r`; toggles regex mode and rescans when search bar is open.
  Five new regex unit tests added to `search.rs`.
  Task #6: `gauge_bar(ratio, cells)` pure function using U+2581–U+2588 block chars;
  `mem_usage_ratio()` via `libc::host_statistics64` with 1s cache; `disk_usage_ratio()`
  via `libc::statfs("/")` with 1s cache; `total_mem_gb()` and `total_disk_gb()` helpers;
  `num_cpus()` for load-gauge normalisation. SYSTEM section in `draw_right_hud` now
  shows mem, disk, and load lines each with a 6-cell gauge + numeric. Two new tests:
  `gauge_bar_renders_proportional_blocks` and `system_section_includes_mem_and_disk_lines`.
  All workspace tests pass (no failures); clippy -D warnings clean.
- 2026-05-23 — Task #15: Cmd-click on `foo.rs:42` and `foo.rs:42:7` in terminal
  scrollback now opens the file at that line/column in $EDITOR. Added
  `parse_path_with_line(tok)` to `crates/anvil-workspace/src/interact.rs`, new
  `Kind::PathWithLine { path, line, col }` variant (removed `Copy` derive), updated
  `classify` to detect line suffixes before plain-path heuristics and return the new
  variant. Added `pty_write_open_file_at` in `crates/anvil/src/main.rs`: uses
  `code --goto path:line:col` for VS Code editors, `+line path` for vi/vim/nvim and
  other editors; resolves $EDITOR with "vi" fallback. Cmd-click handler passes raw
  token directly to `classify` and dispatches to `pty_write_open_file_at`. 12 new
  tests added; all 141 anvil-workspace tests pass, clippy on anvil-workspace clean.
  Pre-existing errors in anvil-render (tabbar.rs clippy, draw_right_hud arity) are
  unrelated and were not introduced by this task.
- 2026-05-23 — Task #10: activity dot on background tabs. Added `has_unread: bool`
  to `Tab` (default false) and `clear_unread()` method. `TabManager::switch_to`,
  `next`, `prev` all clear the newly active tab's `has_unread`. In `main.rs` tick
  loop, background tabs receiving PTY output are marked `has_unread = true` and
  `dirty = true`. `tabbar.rs` draws a 1-cell `·` (U+00B7) in `status.attention`
  amber `[0xb0, 0x7a, 0x14]` at `end_col - 2` for inactive tabs with `has_unread`.
  Active tab suppressed. 4 new tests in `tab.rs`; 2 new tests in `tabbar.rs`
  (dot appears on background tab; dot suppressed on active tab). All workspace
  tests pass; clippy and fmt clean.
- 2026-05-23 — HUD signal additions: Tasks #7 (PORTS), #8 (RECENT), #9 (BUILD).
  `LocalContext` gains `ports: Vec<u16>`, `recent_files: Vec<String>`,
  `project_kind: Option<String>`. `GitResult` gains `ports` and `project_kind`
  (populated in the git worker thread alongside each git query). A new
  `recent_cwd_tx / recent_rx` channel pair feeds a background thread that walks
  the cwd every 4 s (depth ≤ 3, skips target/, node_modules/, .git/, hidden
  dirs) and returns the top-5 most-recently-modified files. `draw_right_hud`
  renders BUILD, PORTS, RECENT sections between LAST RUN and AGENTS; each
  section is omitted when data is absent. RECENT file rows push `HudHit` entries
  (Cmd-click opens full path). Port detection via `lsof` is cached 2 s in a
  static `Mutex`. Project kind detected by marker files (Cargo.toml/package.json/
  Makefile) in cwd. 6 new smoke tests added to `agent_panel.rs` (125 total,
  all pass). Pre-existing clippy break on `push_row_hit` (8 args) and
  `draw.rs` Selection literal (missing `mode` field from a co-present
  selection.rs change) both fixed. `cargo fmt`, clippy, test all clean.
- 2026-05-24 — Option D chrome row implemented. NSWindow gains
  `FullSizeContentView` style mask + `titlebarAppearsTransparent` + title
  hidden so the raster spans the full window with traffic lights overlaid.
  `draw_tab_bar` rewritten: always renders (removed `n < 2` guard); draws
  traffic-light reserved zone (~78px), Basin mark `◒` in theme.accent,
  content-width tabs with 2px bottom accent rule on active, `+` button, and
  right-aligned branch (`⎇` in accent) + clock indicators. `TabBarHits` struct
  added to `tabbar.rs`; `mouse_down` now uses hit-rect dispatch (tab switch,
  close, add). `top_bar_rows()` always returns 1. `local_hhmm()` added to
  `main.rs` for HH:MM clock via libc. 771 tests pass, clippy clean (1
  pre-existing `ACCENT` issue in `anvil-prompt-core` untouched).
- 2026-05-24 — Block-based command output (Phase 1). Warp-style visual blocks:
  each shell command becomes a visually distinct block with a left accent bar
  and synthesized header row. Changes: (1) `anvil-term/src/terminal.rs`:
  `PromptMark` gains `col: u16` (cursor column at mark time) and `duration_ms: u64`
  (populated on CommandDone); `Block` gains `command_start_col: u16` and
  `duration_ms: u64` (both `Copy`, no size change beyond the two fields).
  `block_from_mark` propagates these from marks. `record_prompt_mark` reads
  `cur_x` into `col` for every mark. 3 new tests for `duration_ms` and
  `command_start_col`. (2) `anvil-render/src/raster.rs`: new `block_accent_bar`
  method draws a 2px full-row-height stripe in the left `pad_x` band.
  (3) `anvil-render/src/draw.rs`: `ACCENT_BRIGHT` (#54b7c0) and `PANEL_RAISED`
  (#181a1e) color constants added. `block_accent_color` replaces `gutter_mark_color`
  (running now maps to `ACCENT_BRIGHT` per brand contract). New helpers:
  `format_duration`, `read_command_text`, `draw_block_header_cpu`. Both
  `draw_viewport` (CPU) and `draw_viewport_gpu` (GPU) extended: block lookup
  always runs (not gated on folded state); output rows get `PANEL_RAISED`
  background tint; all block rows get `block_accent_bar`; CPU header row
  gets synthesized text (command + duration + exit symbol + ▾). GPU path leaves
  header synthesis as a TODO comment. 11 new tests: accent bar color per state
  (running/ok/failed), body tint, brand-contract pins. 783 tests pass, clippy
  clean (1 pre-existing `ACCENT` issue in `anvil-prompt-core` untouched).
- 2026-05-24 — Dead-code cleanup pass (builder). Removed `STATUS_BAR_ROWS` constant
  from `statusbar.rs` and its `lib.rs` re-export (unused after pixel-strip chrome
  refactor). Removed `_theme: &Theme` stub parameter from `draw_tab_bar` and
  `draw_status_bar` (chrome uses a fixed palette; theme not consulted). Updated all
  call sites in `tabbar.rs` tests, `statusbar.rs` tests, and `main.rs`. Removed
  dead cell-row chrome-clear block from `render_frame` partial-redraw path in
  `main.rs` (`top_bar_rows()` / `bottom_bar_rows()` return 0; the clear never fired).
  Fixed `drop(row)` → `let _ = row` in two `draw.rs` tests (dropping a reference is
  a no-op; compiler warned). Removed `bottom_row` test helper from `statusbar.rs`
  tests and replaced 5 dead `let row = bottom_row(...)` assignments; smoke test probe
  updated to use pixel-strip coordinates. Updated stale doc comment in `tabbar.rs`
  ("one text-row tall / Ported from tabbar.zig") and `statusbar.rs` ("one text row").
  `cargo test --workspace` all pass; `cargo clippy --workspace -- -D warnings` clean.
- 2026-05-24 — Basin mark glyph fix. U+25D2 (◒) is absent from the bundled
  BlexMonoNerdFontMono-Regular.ttf (entire Geometric Shapes Unicode block missing).
  Replaced with U+F1396 (md-circle_half_full from MDI range), which IS present and is
  visually equivalent. Changed `BASIN_MARK` constant in `crates/anvil-render/src/tabbar.rs`
  and updated three tests that were asserting on `'◒' as u32`. No other changes.
  `cargo test --workspace` and `cargo clippy` clean.
- 2026-05-24 — Removed rubber-band overscroll entirely. Deleted `overscroll`,
  `overscroll_vel`, and `overscroll_target` fields from `Pane`; removed spring-physics
  tick, `bounce_impulse()` helper, and all bounce-on-jump sites from `main.rs`.
  Replaced wheel-edge accumulation with a simple `clamp(0, max_pos)` in the scroll
  handler. Removed `overscroll: f32` parameter from `draw_viewport` and
  `draw_viewport_gpu` (and all call sites in `workspace.rs`, `main.rs`, bench, and
  tests). Scroll now stops hard at both edges with no stretch, spring, or jitter.
  `cargo test --workspace` all pass; `cargo clippy --workspace -- -D warnings` clean.
- 2026-05-24 — D-parity item 3: moved block accent bar from ON origin_x to LEFT of
  origin_x. `block_accent_bar` in `raster.rs` now computes `x = max(0, origin_x - bar_w - 2)`
  so the stripe sits in the pad_x gutter as a true border-left, not overlapping the
  first cell. Updated three draw.rs unit tests to set `origin_x = 10.0` and sample at
  x=6 (within the 3px stripe at x=5). All tests pass; clippy clean.
- 2026-05-24 — Viewport consolidation (design doc `context/2026-05-24-viewport-consolidation-design.md`):
  collapsed four near-identical draw loops (CPU live, CPU smooth, GPU live, GPU smooth) into
  one `draw_viewport_into` body via a `ViewportSink` trait. `CpuSink` wraps `Raster + dyn
  GlyphPainter`; `GpuSink` wraps `CellBatch + dyn GlyphRasterizer`. Public signatures of
  `draw_viewport` and `draw_viewport_gpu` are unchanged — all existing tests compile unmodified.
  Also: extracted `compute_block_header_chars` (deduplicates CPU/GPU header logic); added
  `CellBatch::push_bg` inherent method. Net ~177 lines removed from `draw.rs`; committed in
  8321b0d alongside `raster.rs` clear/clear_pixel_rows perf improvement. Decision recorded at
  `wiki/decisions/viewport-sink-trait.md`.
- 2026-05-24 — D-parity items 6+7: deleted `top_bar_rows: usize` parameter from
  `draw_cell`, `draw_cursor`, `draw_viewport`, `draw_viewport_gpu`; deleted
  `App::top_bar_rows()` and `App::bottom_bar_rows()` from `main.rs`; removed dead
  always-false `if top_bar_rows() != bar_before { resize_all_tabs() }` guards from
  `close_focused_pane`, `close_active_tab`, `close_dead_panes`; simplified `inner`
  rect and HUD `top_offset` arithmetic (both were `+ 0`). 109 LOC removed. All 25
  test suites pass; clippy clean.
- 2026-05-24 — Window rounded corners: set `cornerRadius = 10.0` and
  `masksToBounds = true` on the CAMetalLayer in `appkit.rs`. Clips Metal-rendered
  pixels to the native Sonoma+ corner shape. `hasShadow` is true by default for
  Titled windows; no change needed. No per-frame cost. One file changed:
  `crates/anvil-platform/src/appkit.rs`. All tests pass; clippy clean.
- 2026-05-24 — Cheatsheet overlay rebuilt. Fixed three bugs: (1) coordinate math
  used `raster.pad_y` for card centering but `total_rows` was derived from
  `dh - 2*GRID_PAD` rather than `dh - chrome_top - chrome_bottom` — card was
  vertically mis-centered and could clip chrome. (2) `force_full_redraw` was not
  set on toggle/close, leaving stale card pixels. (3) Single-column card (46 rows)
  taller than typical window safe area — card content truncated. Fixes: `draw()`
  now accepts `chrome_top_px`/`chrome_bottom_px` and centers pixel-precisely in the
  safe area; `total_rows` at call site derived from actual safe height; auto-switches
  to two-column layout (CARD_COLS_2 = 71) when CARD_ROWS exceeds available rows and
  window is wide enough; `force_full_redraw = true` added at all three toggle/close
  sites (Cmd+/ toggle, any-key close, palette CheatsheetShow). Chrome palette
  constants (CHARCOAL, CHROME_BORDER, MIST, TEXT_MUTED, TEXT_SUBTLE) replace theme
  color fields. All tests pass; clippy clean.

- 2026-05-24 — **diff detection (B, detection-only)**: Added `DiffKind { None, Unified }`
  enum and `diff_kind: DiffKind` field to `Block` in `anvil-term/src/terminal.rs`.
  `detect_diff_kind` scans up to 200 output rows at block completion (OSC 133;D),
  classifying Unified if ≥3 rows match `--- `, `+++ `, `@@ `, `+ `, or `- ` patterns.
  `Block::is_unified_diff_row` returns true for content rows (`+ x` / `- x`), false for
  headers (`+++`/`---`). `DiffKind` re-exported from `anvil-term/src/lib.rs`. Test
  fixture in `anvil-render/src/draw.rs` updated with new `diff_kind` field. Three new
  tests: `diff_detection_plain_ls_output_stays_none`,
  `diff_detection_unified_diff_output_detected`,
  `diff_detection_is_unified_diff_row_content_vs_header`. 229 anvil-term tests pass;
  clippy clean on anvil-term. anvil-render has pre-existing build failures unrelated
  to this change.

- 2026-05-24 — **A1+A4 font bundle + chrome font + calt feature flag (items 6, 7, 9 scaffold)**:
  `anvil-platform/src/font.rs` gains `FontFace` enum (Regular/Bold/Italic/BoldItalic/Chrome),
  `Font::init_face(names, pixel_size, face, ligatures)` that uses
  `CTFont::copy_with_symbolic_traits` for bold/italic variants with affine-skew fallback
  for missing italic, and `enable_calt()` that builds a `CTFontDescriptor` with
  `kCTFontFeatureSettingsAttribute = ["calt"]` to enable OpenType contextual alternates
  on the terminal grid font. `FontBundle { grid: [Font;4], chrome: Font }` groups all
  five faces; `CHROME_PT = 11.0` is the chrome font's logical point size.
  `anvil-platform/Cargo.toml` adds `CTFontTraits`, `CFArray`, `CFDictionary` features.
  `anvil/src/main.rs`: `App` gains `chrome_font: Box<Font>` (loaded at `CHROME_PT × scale`
  at startup); `AppShell` gains `chrome_painter: CoreTextPainter`; `render_frame` takes a
  second `chrome_painter` argument and passes it (with `chrome_metrics`) to `draw_tab_bar`,
  `draw_status_bar`, and `draw_search_bar` — those chrome functions continue to accept
  `&mut dyn GlyphPainter` + `FontMetrics`, just receive the smaller chrome face at the
  call site. Nine new tests in font.rs cover face trait flags, `init_face` for each face,
  `FontBundle::new`, and the chrome-is-smaller assertion.
  A3 (SGR per-cell face selection in draw_cell / CpuSink) deferred: requires touching
  `draw.rs` which is blocked by the parallel theme refactor. Scaffold is ready.
  Binary cannot be launched: `anvil-render` has pre-existing build failures from the
  in-flight theme refactor (10 `theme.alloy`/`theme.verified` etc. field errors in
  `draw.rs`), not introduced by this change.

## 2026-05-24 — Warm palette refactor complete

Builder completed the full ember-dark/ember-light theme migration:

- **anvil-theme**: `Theme` struct widened with 16 new chrome fields (`graphite`,
  `charcoal`, `panel`, `panel_raised`, `hairline`, `text_muted`, `text_subtle`,
  `alloy`, `accent_primary`, `accent_bright`, `accent_ember`, `verified`, `failure`,
  `attention`, `agent`, `info`). Added `EMBER_DARK` and `EMBER_LIGHT` constants using
  Revision 2 light-mode token values. `MINERAL_DARK` and `MINERAL_LIGHT` kept with
  legacy values for new fields. `by_name` fallback changed to `EMBER_DARK`. WCAG
  tests added for both ember variants. `accent_primary` for ember-dark nudged from
  `#d05a36` → `#d4603a` to clear 4.5:1 on `#1a1815` (actual spec value was 4.38:1).

- **anvil-render/tabbar.rs**: All 8 local `[u8; 3]` consts removed; `draw_tab_bar`
  gains `theme: &Theme` param; `draw_right_indicators` gains `theme: &Theme`; all
  usages migrated to theme fields. Tests updated to use `EMBER_DARK`.

- **anvil-render/statusbar.rs**: All 7 local consts removed; `draw_status_bar` gains
  `theme: &Theme` param; all usages migrated. Tests updated.

- **anvil-render/cheatsheet.rs**: All 5 local consts removed; `draw` gains
  `theme: &Theme`; `draw_column` gains `theme: &Theme`; all usages migrated.
  Tests updated.

- **anvil-render/draw.rs**: 5 local consts removed (`ACCENT_BRIGHT`, `VERIFIED`,
  `FAILURE`, `ALLOY`, `PANEL_RAISED`). `block_accent_color`, `compute_block_header_chars`,
  `draw_block_header_gpu` gain `theme: &Theme`. `CpuSink`/`GpuSink::draw_fold_summary`
  use `theme.alloy`. Selection refactor: per-cell `mix(...)` bg removed from
  `draw_cell` and `resolve_cell_colors`; replaced with per-row `fill_pixel_rect_alpha`
  pre-pass in `draw_viewport_into` using `theme.accent_ember` at 22% alpha (dark) /
  18% alpha (light) based on background luminance. `ViewportSink` gains
  `fill_selection_row`; CpuSink implements it; GpuSink is a no-op. Tests updated.

- **anvil-config**: Default theme `"mineral-dark"` → `"ember-dark"`. `toggle_theme:
  "cmd+shift+t"` added to `Keybindings`.

- **anvil/main.rs**: All three call sites (`draw_tab_bar`, `draw_status_bar`,
  `draw_cheatsheet`) updated to pass `&self.theme`. App-level `Keybindings` gains
  `toggle_theme: Option<Chord>`; `from_config` wires it; `handle_cmd_chord` handler
  flips `config.theme` between `ember-dark`/`ember-light`, calls `resolve_theme`,
  sets `force_full_redraw`. `effective_theme_name` updated to map `"system"` to
  ember variants. Test updated.

All 725 tests pass (anvil-platform excluded — pre-existing extern-block compile
errors unrelated to this change). Clippy clean on all modified crates.

## 2026-05-24 — Polish #19: bottom-bar agent dot pulse

Added 0.3 Hz sin-based pulse animation to the agent dot in the bottom status bar.

- `crates/anvil/src/main.rs`: `agent_pulse_phase: f32` and `last_agent_pulse_opacity: f32` fields on `App`. Ticked only when `agent_snap.connection == Connection::Live`; dirty only when opacity delta >0.02. Phase passed to `draw_status_bar`.
- `crates/anvil-render/src/statusbar.rs`: `pulse_phase: f32` parameter added. `mix_rgb` helper (linear per-channel blend). Dot color = `mix_rgb(charcoal, agent, 0.5 + 0.5*sin(TAU*phase))` when Live; `text_subtle` otherwise. Test for Live dot updated to assert not-text_subtle rather than exact agent color.

All tests pass; clippy clean.

## 2026-05-24 — HUD redesign: DevOps section set, viewport-constrained

Implemented the HUD redesign per `context/2026-05-24-hud-redesign.md`.

- `crates/anvil-prompt-core/src/ci.rs` (new): `CiState` enum and `CiStatus` struct.
- `crates/anvil-prompt-core/src/lib.rs`: added `ci` module; re-exports `CiState`, `CiStatus`.
- `crates/anvil-render/src/agent_panel.rs`: `SectionId` variants replaced — `Repo`, `Git`, `LastRun`, `Build` removed; `Context`, `RepoGit`, `Ci` added. `DEFAULT_ORDER` now 7 entries: `[Context, RepoGit, Ci, Agents, Recent, Ports, System]`. `LocalContext` gains `ci_status: Option<CiStatus>`. Dispatch rewritten: CONTEXT (hidden when `kube_context` None, shows env-tint dot + cluster · namespace), REPO+GIT (merged: basename, parent, branch+dirty/ahead-behind, HEAD SHA+subject), CI (hidden when `ci_status` None — always None this pass), SYSTEM demoted to single compact row (`mem ▄▅▆▆ N/N GB · load X.XX`, disk and clock rows removed). Dead functions `disk_usage_ratio`, `total_disk_gb`, `DISK_CACHE` removed.
- `crates/anvil/src/main.rs`: `surface_rect` in the HUD block now starts at `chrome_top_px()` and has height `dh - chrome_top_px() - chrome_bottom_px()`; `rows` derived from viewport height. HUD no longer overlaps tab bar or status bar.

Tests updated: `build_section_*` tests replaced with `context_section_*` tests; `system_section_includes_mem_and_disk_lines` updated to `system_section_compact_row_has_mem_and_load`. All tests pass; clippy clean.
- 2026-05-24 — Atlas glyph cache LRU (polish batch item 2). `crates/anvil-platform/src/font.rs`: replaced unbounded `HashMap<u16, GlyphMask>` in `CoreTextPainter` with inline `GlyphCache` struct (HashMap + per-entry u64 tick + cap). Cap = 2048 entries. On insert past cap: O(n) scan evicts min-tick entry. `AtlasPainter.map` in `glyph_atlas.rs` left unchanged — already bounded by atlas texture exhaustion. All tests pass; clippy clean.
- 2026-05-24 — HUD v2 builder: expand-outward behavior + theme migration. `crates/anvil/src/main.rs`: added `HUD_WIDTH_PT = 280.0`; added `window: Retained<NSWindow>` to `AppShell`; added `AppShell::toggle_hud()` which flips `hud_visible` and calls `window.setContentSize` ±`HUD_WIDTH_PT` (AppKit fires resize which calls `resize_all_tabs` — PTY cols unchanged); intercepted `kb.hud_toggle` in `perform_key_equivalent` and Esc-to-close path to route through `toggle_hud`; palette `HudToggle` intercepted at `webview_message` level; removed `test!(kb.hud_toggle, …)` from `App::handle_cmd_chord`. `crates/anvil-render/src/agent_panel.rs`: fixed compile error (line 675 `theme.foreground` → `app_theme.foreground`); fixed three pre-existing clippy lints (`section_break` dead-code, `draw_section_header` explicit-counter-loop, `draw_local_footer` too-many-arguments). All tests pass; clippy clean.
- 2026-05-24 — Pane divider drag-resize (polish item 15). `crates/anvil-workspace/src/layout.rs`: added `split_rect: Rect` and `split_dir: SplitDir` fields to `DividerHit` (populated in `find_divider_in_node` at hit time); added `split_at_path_mut` free function to navigate a `PaneTree` to the split node owning a divider by following the hit path. `crates/anvil/src/main.rs`: imported `DividerHit`, `find_divider_at`, `adjust_ratio`, `split_at_path_mut`; added `divider_drag: Option<DividerHit>` field to `App`; in `mouse_down` added a divider hit-test (5 px slop, checked before click-to-focus); in `mouse_up` clear `divider_drag` on release; in `mouse_dragged` compute ratio delta from mouse position relative to the split node's own rect, call `adjust_ratio` with `min_ratio = 0.05`, then `resize_all_tabs`. All tests pass; clippy clean.
- 2026-05-24 — HUD pretty spec (`context/2026-05-24-hud-pretty.md`), all 8 deltas. `crates/anvil-render/src/agent_panel.rs` only: (1) new `draw_section_accent_bar` helper + 7 call sites before each section header using `app_theme.accent_primary` (2px ember bar, gutter-left); (2) top-edge hairline on glass surface after left hairline; (3) `draw_section_rule` y-pos `+0.5` → `+0.1`; (4) all 7 `draw_section_header` color args `text_subtle` → `text_muted`; (5) `if r < bottom { r += 1; }` gap inserted before each section except first (6 insertions: RepoGit, Ci, Ports, Recent, Agents, System); (6) CI Ok/Failed rows split into left branch name + right-aligned `{duration}s` via `draw_text_right`; (7) `*N modified` → `*N`; (8) empty sections stay collapsed (no change). All tests pass; clippy clean.

- 2026-05-24 — Smooth-scroll easing on momentum release (polish item 21). `crates/anvil-workspace/src/pane.rs`: added `scroll_target: f32` and `scroll_vel: f32` fields to `Pane`, initialized to 0.0. `crates/anvil/src/main.rs`: `scroll()` now writes to `pane.scroll_target` / `pane.scroll_vel` instead of `pane.scroll_pos` directly; `tick()` applies cubic-out easing (`scroll_pos += diff * 0.30`) each frame and calls `set_viewport_offset` to keep terminal in sync — only marks `dirty` while `|diff| > 0.01`, then snaps and clears. All instant-snap paths (key input, ScrollTop/ScrollBottom, jump-to-prompt, search, snap_anim) also reset `scroll_target` and `scroll_vel` so easing cannot fight manual position changes. All tests pass; clippy clean.

- 2026-05-24 — Diff colorization render side (polish item 27). `crates/anvil-render/src/draw.rs` only: in `draw_viewport_into`, after the selection row wash and before cell drawing, added a diff-row tint block. For rows belonging to a `DiffKind::Unified` block, calls `block.is_unified_diff_row(terminal, abs)`; on match reads the first cell sigil and paints a full-row wash at 12% alpha using `theme.verified` (green) for `+` lines or `theme.failure` (red) for `-` lines via the existing `fill_selection_row` sink method. Header lines (`+++`/`---`/`@@`) are excluded by `is_unified_diff_row`'s `second == ' '` guard. All draw tests pass; clippy clean.

- 2026-05-24 — Active-pane glow (item 30, A1.4). `crates/anvil-render/src/workspace.rs` only: removed the `let _ = focused_id` suppressor from `draw_dividers`; added a 4-edge 1px inset `theme.accent` border painted after the gutter fills when `entries.len() >= 2` (single-pane: no ring). Flipped `focused_pane_has_no_accent_border` → `focused_pane_has_accent_border` asserting accent at left and top inset of the focused pane and absence on the non-focused pane. All 129 anvil-render tests pass; clippy clean.

- 2026-05-24 — DirtySet bitmap (item 3). `crates/anvil-term/src/terminal.rs` only: replaced `Vec<bool>` internal storage in `DirtySet` with `bits: Vec<u64>` + `cap: usize`. `all`, `none`, `from_raw`, `contains`, `iter`, `mark`, `force_full` API unchanged; `from_raw` converts the `Vec<bool>` from `Grid::take_dirty` during construction. Used `usize::div_ceil(64)` to satisfy clippy's `manual-div-ceil` lint. All callers (`draw.rs`, `workspace.rs`, `main.rs`, bench) required no changes. All 129 tests pass; clippy clean.

- 2026-05-24 — Tab open/close micro-animation (item 22). `crates/anvil-workspace/src/tab.rs`: added `anim_phase: f32` (0=invisible, 1=fully open) and `target_phase: f32` (1=opening, 0=closing) to `Tab`; both constructors (`new`, `new_single_pane_starting_at`) init to `anim_phase=0, target_phase=1`. Added `TabManager::begin_close_at` (sets `target_phase=0`, adjusts `active`, returns false when last live tab) and `TabManager::purge_closed_tabs` (removes tabs where `target_phase==0 && anim_phase<=0`). `crates/anvil-render/src/tabbar.rs`: multiply `tw` by `anim_phase`; added `blend_color` helper; label fg faded from `fg_base` toward `theme.graphite` proportionally. `crates/anvil/src/main.rs`: `close_active_tab` uses `begin_close_at`; `tick` advances `anim_phase` toward `target_phase` each frame (open: +1/6, close: -1/5), calls `purge_closed_tabs` when animation finishes, marks `dirty` only while animating. Steady-state cost: zero. All tests pass; clippy clean.

- 2026-05-24 — OpenType ligature shaping infrastructure (item 28). `crates/anvil-platform/src/font.rs` only: added `CoreTextPainter::draw_run`, a new public method that gathers codepoints into a CFAttributedString, shapes them via `CTLine::with_attributed_string` (enabling `calt` ligatures), rasterizes the run into a wide RGBA `CGBitmapContext` (using `kCTForegroundColorFromContextAttributeName` so CTLine respects the CG fill color), extracts the R channel as a coverage mask, and composites into the destination BGRA8 buffer. Added `CFAttributedString`, `CFNumber`, `CTLine`, `CTStringAttributes` to the platform crate's feature lists. Existing `draw_glyph` path untouched. The render loop (`draw.rs`) still calls `draw_glyph` per cell — visible ligature substitution requires a future render rewrite to accumulate same-style runs before calling `draw_run`. Added 1 test `draw_run_inks_pixels_for_a_run_of_codepoints` verifying end-to-end shaping composites pixels. 47 tests pass; clippy clean.

- 2026-05-24 — Block-header pulse on command completion (item 23). Three-file change: (1) `crates/anvil-term/src/terminal.rs`: added `mark_completion_times: [Option<Instant>; MAX_MARKS]` parallel to `marks`; set on CommandDone; shifted in lock-step with marks at capacity. Added `completed_at: Option<Instant>` (#[serde(skip)]) to `Block`; `block_from_mark` populates it from `ctimes[j]` when the CommandDone mark is found. Added `any_block_completed_within(Duration) -> bool` accessor. Removed `PartialEq + Eq` from `Block` (not serialization-compatible with `Instant`). (2) `crates/anvil-render/src/draw.rs`: CpuSink::draw_block_header paints a 2px ember-bright rect at the bottom edge of the header row with opacity `sin(π * elapsed/200ms)` when `completed_at` is within 200ms. GPU sink unchanged (pixel rects are CPU-only). Fixed test helper `make_block` to include `completed_at: None`. (3) `crates/anvil/src/main.rs`: tick loop scans focused pane for any block completed within 250ms and sets `dirty = true` to keep frames flowing during the pulse. 229 tests pass in modified crates; clippy clean.

- 2026-05-24 — SGR bold/italic face selection (item 29). Five-file change: (1) `crates/anvil-platform/src/font.rs`: added `FontBundle::face(bold: bool, italic: bool) -> &Font` accessor mapping the four `(bool, bool)` combinations to `grid[0..3]`. (2) `crates/anvil-render/src/draw.rs`: replaced single `painter: &mut dyn GlyphPainter` in `CpuSink` with four named fields (regular/bold/italic/bold_italic); added `GridPainters<'a>` public struct; `CpuSink::new` extracts painters via raw-pointer reborrow to satisfy the borrow checker with a single struct lifetime; `ViewportSink::draw_cell` dispatches to the correct painter via inline `match (BOLD, ITALIC)` on `cell.attrs`; cursor glyph in `draw_cursor` likewise dispatches by the cell-under-cursor's attrs. `draw_viewport` signature changed from `&mut dyn GlyphPainter` to `&mut GridPainters<'_>`. (3) `crates/anvil-render/src/workspace.rs`: `draw_workspace` updated to accept `&mut GridPainters<'_>`; all 4 workspace tests updated. (4) `crates/anvil/src/main.rs`: `App` gains `bold_font`, `italic_font`, `bold_italic_font: Box<Font>`; `AppShell` gains `bold_painter`, `italic_painter`, `bold_italic_painter`; `render_frame` signature changed to `grid_painters: &mut GridPainters<'_>`; `bump_font_size` rebuilds all 4 faces; startup init loads all 4. (5) `crates/anvil-render/benches/draw_viewport.rs`: updated bench to construct 4 `NullPainter` instances per scenario. All existing tests pass; 3 new tests verify bold, italic, and bold-SGR-in-viewport dispatch. clippy clean.

- 2026-05-24 — Tab drag reorder (item 19). Three-file change: (1) `crates/anvil-workspace/src/tab.rs`: added `TabManager::move_tab(from, to)` — bounds-checked, does `Vec::remove + insert`, preserves active by identity (active follows the moved element; other indices shift correctly). 5 unit tests added (forward, backward, no-op same index, active follows non-active move, out-of-bounds noop). (2) `crates/anvil/src/main.rs`: added `tab_drag: Option<(usize, f64)>` field to `App` (current dragged tab index + mouse-down raster x). Mouse-down on a Tab hit records this state after the existing `switch_to` call (click-to-activate preserved). Mouse-dragged: if `tab_drag` is set and cursor has moved >= `cell_w * 0.5`, finds the tab hit zone under the cursor and calls `move_tab`; updates `tab_drag` to track the new index after each swap (live reorder). Mouse-up: clears `tab_drag` via `take()`. (3) `crates/anvil-render/src/tabbar.rs`: no changes needed. 148 workspace unit tests pass; pre-existing clippy errors in anvil-render (unrelated, not caused by this change).

- 2026-05-24 — Cursor smooth move (item 24). Two-site change in `crates/anvil/src/main.rs` only; `draw.rs` already used `cursor_ax`/`cursor_ay` as fractional f64. (1) Tick loop (was snap-only): replaced with exponential ease — `cursor_ax += (tx - cursor_ax) * 0.35` / same for y; snaps when delta < 0.02; first-frame guard snaps immediately if fields are (0,0) but cursor isn't. Sets `dirty = true` while moving. (2) Dirty-row section: additionally marks `cursor_ay.floor()` and `cursor_ay.ceil()` dirty each frame (alongside the existing `cur.y` mark) so intermediate pixels are cleared, preventing the stale-pixel trail bug that caused the previous animation to be removed. `cursor_row_prev` updated from `cur.y` to `cursor_ay.round()` so the next frame clears the animated row. All tests pass; clippy clean.

- 2026-05-24 — Block-scoped search (item 14). Four-file change. (1) `crates/anvil-term/src/search.rs`: added `SearchScope { All, Block }` enum and `scope` field to `Search`; added `set_scope`, `scope()`, `set_query_in_block(term, query, anchor_content_row)`, and `rescan_in_block(term, anchor_content_row)` methods; refactored `rescan_literal`/`rescan_regex` to accept `Option<(start_row, end_row)>` so the matcher is not duplicated; added private `block_row_range(term, anchor)` that converts `PromptStart` marks (absolute lines) to a content-row range; 2 new unit tests: `block_search_finds_hit_inside_target_block_not_in_neighbour` and `block_search_excludes_hit_in_next_block`. (2) `crates/anvil-term/src/lib.rs`: exported `SearchScope`. (3) `crates/anvil/src/main.rs`: added `search_open_block: Option<Chord>` to `Keybindings` (hardcoded `cmd+shift+f`); added `open_search_block()` method (computes cursor content row, calls `set_query_in_block`); wired Cmd+Shift+F keybind; `open_search` now calls `set_scope(All)`; `close_search` now calls `set_scope(All)`. (4) `crates/anvil-render/src/searchbar.rs`: prefix string changes to "block find: " when `search.scope() == SearchScope::Block`. All 236 anvil-term tests + full workspace pass; clippy clean.

- 2026-05-24 — Right-click NSMenu context menu (item 17). Three-file change: (1) `crates/anvil-platform/src/appkit.rs`: added `ContextAction` enum (Copy/Paste/Clear/SplitRight/SplitDown); added `context_action(&mut self, ContextAction)` to `AppHandler` trait; added `rightMouseDown:` to `AnvilView` — builds a 5-item NSMenu (Copy, Paste, sep, Clear, sep, Split Right, Split Down) with the view as target, shown via `NSMenu::popUpContextMenu_withEvent_forView`; added 5 `anvilContext*:` action selectors on the view. (2) `crates/anvil-platform/Cargo.toml`: added `NSMenu` + `NSMenuItem` features to `objc2-app-kit`. (3) `crates/anvil/src/main.rs`: implemented `context_action` on `AppShell` and forwarding stub on `ForwardingHandler`. Dispatch: `ContextAction` enum, not synth-key-events. Tests pass; clippy clean.

- 2026-05-24 — Living-scrollback indicator (item 20). Bar fallback chosen (no text rendering into viewport). Three-file change: (1) `crates/anvil-workspace/src/pane.rs`: added `unseen_baseline: Option<usize>` field to `Pane` (initialized `None`); added `unseen_rows()` helper that returns `line_count().saturating_sub(baseline)` or 0 when pinned. 3 new unit tests. (2) `crates/anvil/src/main.rs`: in the scroll-easing tick block, after easing: set `unseen_baseline = Some(line_count())` when `scroll_pos > 0` and baseline is `None`; clear to `None` when `scroll_pos == 0`. (3) `crates/anvil-render/src/workspace.rs`: in `draw_workspace`, after `draw_viewport` for each pane (raster origin still at pane origin), call `pane.unseen_rows()` and when > 0 paint a 4px-tall `theme.accent_ember` bar at `(e.rect.x, e.rect.y + e.rect.h - 4)` via `fill_pixel_rect_alpha` at alpha 0.92. All tests pass; clippy clean.

- 2026-05-24 — Dead-code sweep (CRAP audit af37e41). Nine items across six crates. Removed: FontBundle struct + impl + 2 tests (~80 LoC, font.rs); CoreTextPainter::draw_run + 1 test (~180 LoC, font.rs) + CTLine/CTStringAttributes/CFAttributedString/CFNumber Cargo features; anvil-prompt-core::ci module (CiStatus, CiState, ~20 LoC) + ci_status field from LocalContext + SectionId::Ci variant + CI draw arm (~110 LoC, agent_panel.rs); DirtySet::iter_spans + DirtySpans iterator + 5 tests (~55 LoC, terminal.rs). Demoted: Search::rescan_in_block pub→fn; mix_rgb pub(crate)→fn; GlassTones+glass_tones_for pub→module-private (also removed 3 unused fields from GlassTones). Fixed: warm all four painters at startup (bold/italic/bold_italic were not warmed; now 4×95 glyph warm). cursor_ax/cursor_ay deferred (requires plumbing terminal.cursor() through draw_workspace). All 153 tests pass; clippy clean.

- 2026-05-24 — CB6 running-block header dot pulse. Added `running_pulse_phase: f32` to `App` (main.rs), advanced `+= 1.5 / 60.0` per tick while any pane in the current tab has `shell_running == true` (checked via `last_run().running`); marks `dirty = true` while running. Threaded the phase down through `draw_workspace` → `draw_viewport` → `CpuSink` / `GpuSink` (stored as a field in each sink struct). `CpuSink::draw_block_header`: when `block.state == Running`, paints a 2×2 pixel rect centered in col 0 of the header row in `theme.accent_bright` with sine-modulated alpha cycling 0.45–1.0 at 1.5 Hz. `GpuSink::draw_block_header`: same logic via `batch.push_cell` with alpha baked into the mixed color. Existing completion pulse (item 23) untouched. Files: `crates/anvil-render/src/draw.rs`, `crates/anvil-render/src/workspace.rs`, `crates/anvil/src/main.rs`, `crates/anvil-render/benches/draw_viewport.rs`. All tests pass; clippy clean.

- 2026-05-24 — ID2: top context bar (Ide mode). `Docks::for_mode` now sets `top_h = 24.0 * scale` for Ide mode (Terminal/Codex remain 0). New `crates/anvil-render/src/context_bar.rs` (335 LoC, 7 tests): `draw_context_bar(raster, painter, metrics, theme, local, rect)` — charcoal bg + hairline bottom, left section (project icon + cwd basename + branch in accent/subtle), right section (kube cluster + head_short). All sections omitted when data absent. `lib.rs` exports `pub mod context_bar` and `pub use context_bar::draw_context_bar`. `main.rs` line 1644: Ide-mode guard calls `draw_context_bar` with `areas.top_bar`. Added 3 new tests in `mode.rs` (`ide_top_bar_h_24px_at_1x`, `terminal_top_bar_h_zero`, `codex_top_bar_h_zero`). All tests pass; clippy clean.

- 2026-05-24 — ID1: docks geometry + LayoutMode. New `crates/anvil-workspace/src/mode.rs`: exports `LayoutMode { Terminal, Ide, Codex }`, `DockMetrics`, `Docks` (with `for_mode` + `compute_areas`), and `Areas`. 12 unit tests cover width sums, height sums, non-overlap, zero-width Terminal docks, scale linearity, and round-trip. In `crates/anvil/src/main.rs`: added `use anvil_workspace::mode::{DockMetrics, Docks, LayoutMode}`, added `layout_mode: LayoutMode` field (default Terminal), replaced `inner_rect()` with `window_inner()` (pre-dock rect) + `dock_metrics()` + `pane_area_rect()` helpers, replaced all `inner_rect()` call sites (resize_all_tabs, draw_workspace, event_cell, focus_neighbor, split_focused_pane, mouse_down divider/hit-test) with `pane_area_rect()`, replaced the render_frame `inner` local variable with `pane_area_rect()`. Added `ANVIL_LAYOUT_MODE=ide|codex|terminal` env-var debug override at startup. AG1 HUD carve-out now lives in `Docks::for_mode` as `right_w`; Terminal mode pixel layout byte-identical. All 165 workspace tests pass; clippy clean.

- 2026-05-24 — ID4: right dock always-on in Ide mode. Single predicate change at `main.rs:1453`: `eff_hud = self.hud_visible` → `self.hud_visible || self.layout_mode == LayoutMode::Ide`. Existing `draw_right_hud` covers both the Cmd+\\ toggle path (Terminal) and the permanent Ide path with no other changes.

- 2026-05-24 — ID3: left dock (explorer + outline). New `crates/anvil/src/fs_worker.rs` (205 LoC, 4 tests): `spawn_fs_worker()` named thread `anvil-fs-worker`, `SyncSender<PathBuf>` in / `Receiver<DirSnapshot>` out, 2-second per-path debounce, top-level-only read_dir, skips dot-prefix + target/node_modules/.git, dirs-first alphabetical sort, 200-entry cap. New `crates/anvil-render/src/left_dock.rs` (330 LoC, 5 tests): `draw_left_dock(raster, painter, metrics, theme, snapshot, rect)` — charcoal bg + right hairline, 60/40 vertical split with divider hairline, explorer section (EXPLORER·basename header, one row per entry: dirs in text_subtle with ▸ prefix, files in text_muted), outline section (OUTLINE header + "Outline unavailable" + 50%-alpha BR5 note). Zero-rect no-panic guaranteed. `lib.rs`: added `pub mod left_dock` and re-export. `main.rs`: added `fs_tx`, `fs_rx`, `fs_snapshot`, `fs_last_cwd` fields; startup spawns worker; `refresh_hud` drains channel and sends cwd on change (Ide mode only); `render_frame` Ide block calls `draw_left_dock` with `areas.left_dock`. All tests pass; clippy clean.
- 2026-05-24 — AG3: caldera approve/start keybindings (commit fd06814). `Cmd+Shift+A` fires `actions::approve(client)` (POST `/approve`); `Cmd+Shift+S` fires `actions::start_run(client)` (POST `/start_run`). Both wired in `main.rs` `perform_key_equivalent`, dispatched through `crates/anvil-caldera/src/actions.rs`. Keybinds are hardcoded; not yet in `Keybindings` config. New wiki page: [[concepts/agent-actions]].
- 2026-05-24 — CB5: Opt+click copy block. `Option+click` on any block body row copies the full block output text to the system clipboard. Hit-test uses the block's content-row range. Wired in `main.rs` `mouse_down` handler, gated on `Modifiers::option`. See [[concepts/block-model]].
- 2026-05-24 — ID5: Cmd+Shift+E mode cycle. Cycles `Terminal → Ide → Terminal` (Codex excluded until dock set defined). Calls `toggle_hud()` to expand/contract window by `HUD_WIDTH_PT`, then `resize_all_tabs`. Hardcoded in `main.rs`; not yet in `Keybindings` config. See [[concepts/layout-modes]].
- 2026-05-24 — Wiki upkeep: added concept pages for [[concepts/layout-modes]] (LayoutMode/Docks/Areas/context bar/mode cycle), [[concepts/block-model]] (OSC 133 block structure, running pulse CB6, completion pulse, Opt+click CB5, diff colorization), [[concepts/agent-actions]] (AG3 approve/start keybinds). Updated [[index]] with new key-page entries.
- 2026-05-24 — Palette enhancements (3 deltas). Delta 1: `AppShell::resize` calls `self.webview.set_frame(width, height)` when `palette.visible` (`main.rs`). Delta 2: `send_palette_show` appends dynamic entries — tabs most-recent-first (`tab.switch:{idx}`), three layout mode items (`layout.mode:terminal/ide/codex`), and agent items (`agent.approve`, `agent.start`) gated on `Connection::Live`; `Action` enum gains `SwitchTab(usize)`, `LayoutTerminal/Ide/Codex`, `AgentApprove/Start`; `action_for_id` handles prefix routing; `handle_palette_action` dispatches all new variants (`anvil-workspace/src/palette.rs`, `main.rs`). Delta 3: `ui/palette/index.html` replaces the boolean `fuzzy` filter with a scored sort (substring +100, word-start +70, subsequence +10) and a per-session recency map (bump invoked by +20, decay all by 0.9 on each invoke). All 25 test suites pass; clippy clean.
- 2026-05-24 — BR3: EditorBridge thread + EditorSnapshot. New `crates/anvil-editor/src/bridge.rs` (310 LoC, 2 unit tests): `ConnectionState` (Disconnected/Connecting/Live/Error, Default=Disconnected), `EditorSnapshot` (socket_path, connection, buffer_name, cursor 0-indexed, modified, polled_at_unix; Default zeroed), `EditorBridge::spawn(Option<PathBuf>)` spawns named thread `anvil-editor-bridge`, worker loop with 250 ms poll cadence + `Msg::Kick/Shutdown/SetSocket` channel, four nvim RPCs (nvim_get_current_buf, nvim_buf_get_name, nvim_win_get_cursor, nvim_buf_get_option modified) with 500 ms timeout each, Error state zeros data and retries next tick. `lib.rs`: added `pub mod bridge` + re-exports. `crates/anvil/Cargo.toml`: added `anvil-editor` dep. `main.rs`: `App` gains `editor_bridge: Option<EditorBridge>` + `editor_snapshot: Option<EditorSnapshot>`; startup reads `$NVIM_LISTEN_ADDRESS` and spawns bridge if set; `refresh_hud` clones snapshot each tick. No UI rendered (BR4/BR5 will consume). All 11 test suites pass; clippy clean.
- 2026-05-24 — BR5: Outline LSP pull + debounce. `crates/anvil-editor/src/bridge.rs`: added `SymbolKind` (10-variant enum, `from_lsp_int` maps LSP spec integers), `OutlineSymbol { name, kind, line, depth }`, `OutlineState { Idle|Pending|Ready|NoServer }`, `outline: Vec<OutlineSymbol>` + `outline_state: OutlineState` on `EditorSnapshot`; worker gains `last_outline_pull_ms`/`last_buf_name` state; each 250 ms tick fires `pull_outline` if 1500 ms elapsed or buffer changed (immediate, clears outline first); `pull_outline` sends `nvim_exec_lua` with a 28-line Lua script (gets LSP clients, calls `buf_request_sync` with 1.5 s timeout, flattens hierarchy with depth); four-state decode: `attached=false→NoServer`, `symbols=nil→Pending` (retain last), `[]→Ready empty`, `[..]→Ready populated`; malformed → NoServer; `zero_data` clears outline on Error. `crates/anvil-render/src/left_dock.rs`: mirror types `OutlineKind`/`OutlineRow`; `draw_left_dock` gains `outline: Option<&[OutlineRow]>` before rect; `draw_outline_section` dispatches `None→unavailable placeholder`, `Some(&[])→"No symbols"`, `Some(rows)→indented rows` with kind glyphs (ƒ fn/method, ▢ class/struct/enum, ⚙ module, · other) and name in text_muted; `lib.rs` re-exports `OutlineKind`/`OutlineRow`. `crates/anvil/src/main.rs`: inline conversion `EditorSnapshot.outline → Vec<OutlineRow>` (Ready→Some rows, NoServer→Some([]), other→None); passes to `draw_left_dock`. 4 new tests: `outline_default_idle_and_empty`, `outline_kind_serializes_known_values`, `left_dock_renders_outline_no_symbols`, `left_dock_renders_outline_with_rows`. 172 tests pass; clippy clean.
- 2026-05-24 — Removed `LayoutMode::Codex`. Deleted variant from enum and all match sites (`mode.rs`, `palette.rs`, `main.rs`). Catalog push, `Action::LayoutCodex`, env var arm `Ok("codex")`, and Ide→Codex cycle step all removed. Toggle is now Terminal↔Ide. Deleted 3 Codex tests (`widths_sum_to_inner_w_codex`, `codex_zero_left_dock`, `codex_top_bar_h_zero`); `layout.mode:codex` id now returns `None`. Updated `wiki/concepts/layout-modes.md`. Tests: −3 deleted; all remaining pass (9 in mode.rs).
- 2026-05-24 — NE1: Buffer crate skeleton. Moved `bridge.rs`, `codec.rs`, `transport.rs` into `crates/anvil-editor/src/nvim/` (3 file moves; `crate::` → `super::` path fixes). New `src/nvim/mod.rs` re-exports all nvim bridge types. New `src/buffer.rs` (≈300 LoC): rope-backed `Buffer` on `ropey 1.6`; types `BufferId`, `Position`, `Range`, `Cursor`, `Edit`; AI-native placeholder structs `EditProposal`, `GhostTextSpan`, `RevisionTag` (empty, reserved for NE14); grapheme-aware `pos_to_char_idx` via `unicode-segmentation`; ops `insert_char/str`, `delete_range`, `replace_range`, `line/line_count/byte_len/char_count/char_at/char_to_line/line_to_char`. Renamed `from_str` → `from_text` (clippy `should_implement_trait`). Updated `src/lib.rs`: `pub mod nvim; pub mod buffer;` + all re-exports (nvim call sites unchanged). Added `ropey = "1.6"` and `unicode-segmentation = "1"` to workspace and crate deps. 14 new buffer tests: `buffer_new_is_empty`, `buffer_from_text_round_trip`, `buffer_insert_char_ascii`, `buffer_insert_char_at_line_split`, `buffer_insert_str_multibyte`, `buffer_insert_emoji`, `buffer_insert_cjk`, `buffer_delete_range_within_line`, `buffer_delete_range_across_lines`, `buffer_replace_range`, `buffer_line_access`, `buffer_line_count`, `buffer_char_to_line_roundtrip`, `buffer_empty_buffer_edge_cases`. All 66 anvil-editor tests pass; full workspace passes; clippy clean.
- 2026-05-24 — BR4: New Editor Pane action. `Cmd+E` opens `nvim --listen $TMPDIR/anvil-nvim-<pid>-<counter>.sock` in a horizontal split; second press focuses the existing pane (singleton invariant). Changed files: `crates/anvil-config/src/lib.rs` (`editor_new: String = "cmd+e"` added to `Keybindings`); `crates/anvil/src/main.rs` (runtime `Keybindings::editor_new`, `App::editor_pane_id/editor_socket_counter/nvim_path` fields, `new_editor_pane()` method, `clear_editor_pane_if()` cleanup helper wired into `close_focused_pane` and `close_dead_panes`, `Cmd+E` chord dispatch, `Action::NewEditorPane` palette handler); `crates/anvil-workspace/src/palette.rs` (`Action::NewEditorPane` variant, `editor.new` CATALOG entry, `editor_new_id_maps_to_action` unit test); `crates/anvil-render/src/context_bar.rs` (`editor: Option<&EditorSnapshot>` param — renders `edit: <name>[•]` right-anchored when `Live`, 2 new tests, all existing call sites updated); `crates/anvil-render/Cargo.toml` (`anvil-editor` dep); `crates/anvil/Cargo.toml` (`which` dep); `Cargo.toml` workspace (`which = "7"`). Spawn API used: `Pty::spawn_exec`. `EditorBridge::set_socket` is the bridge handoff (existing from BR3, uses `Msg::SetSocket`). All 11 test suites pass; clippy clean.
- 2026-05-25 — NE3: Undo/redo. Restored and completed the NE3 stash on `crates/anvil-editor/src/buffer.rs`. `EditRecord { edit, inverse, at }` and `UndoStack { undo: VecDeque<Vec<EditRecord>>, redo, cap }` added. `apply_edit` routes through `apply_edit_at(Instant)` which captures the pre-edit text, checks coalesce eligibility, mutates the rope, builds the inverse, and either appends to the top group or starts a new one (evicting oldest when `len > cap`). `undo()` applies inverses in reverse order and pushes the group to redo; `redo()` re-applies forward edits and pushes back to undo; both bump `revisions`. Coalesce rule: single-char true inserts within 500 ms and adjacent in buffer merge into one group; breaks on `flush_undo_group()` (cursor jump, selection, save). Default cap: 1000 groups. `flush_undo_group()` is the public hook for callers (NE2 `save` calls it). Test helpers `apply_edit_at_ts` (injectable `Instant`) and `with_undo_cap` added under `#[cfg(test)]`. 9 new NE3 unit tests: `undo_single_char_insert`, `undo_coalesces_consecutive_typing`, `undo_breaks_on_cursor_jump`, `undo_breaks_on_500ms_gap`, `redo_after_undo_restores`, `new_edit_after_undo_clears_redo`, `undo_cap_evicts_oldest`. Also removed stale NE2 import (`use std::path::Path`) left over from the stash. 82 anvil-editor tests pass; clippy clean.

- 2026-05-25 — NE4: EditorPane + PaneRegistry integration. `Pane.terminal: Terminal` → `Option<Terminal>`; new `Pane.editor_id: Option<BufferId>`. `Pane::new_editor(id, buffer_id)` constructor (no PTY). `PaneRegistry::peek_next_id()` + `create_and_register_editor(buffer_id)`. New `crates/anvil-workspace/src/editor_pane.rs`: `EditorPane { buffer_id, cursor, selection, scroll_pos/target/vel }`, `EditorPaneRegistry` (HashMap<PaneId, EditorPane> + HashMap<BufferId, Buffer> + next_buffer_id counter). `Tab` gains `editor_panes: EditorPaneRegistry` and `split_native_editor(dir)` method (peek_next_id → new_pane → create_and_register_editor → tree.split). Keybinding `cmd+opt+e` → `new_native_editor_pane()` in `main.rs` (coexists with `cmd+e` → nvim). Render stub: editor panes fill charcoal via `draw_editor_pane_stub` (NE5 will replace). All ~60 `pane.terminal.*` call sites in `main.rs` guarded with `if let Some(terminal) = &pane.terminal` / `.as_ref()` / `.as_mut()`. 7 new unit tests: `editor_pane_registry_new_pane_returns_buffer_id`, `editor_pane_registry_get_buffer_round_trip`, `editor_pane_registry_remove_pane_drops_buffer`, `editor_pane_registry_multiple_panes_independent`, `pane_new_editor_has_no_terminal`, `pane_new_editor_unseen_rows_zero`. All tests pass; clippy clean.

- 2026-05-25 — NE5: Editor render path. New `crates/anvil-render/src/editor.rs` (≈280 LoC, 5 unit tests): `draw_editor_into(raster, painter, editor_pane, buffer, metrics, theme, rect)` fills background, computes gutter width from `buffer.line_count().to_string().len() + 2`, right-aligns line numbers in `theme.text_muted`, iterates graphemes per row via `unicode-segmentation`, paints each in `theme.foreground`, clips long lines with a `▸` marker, draws a 2 px vertical cursor bar in `theme.accent`, and applies selection wash with `fill_pixel_rect_alpha` at α=0.18 using `theme.accent_ember`. Scroll is integer-row-aligned (`floor(scroll_pos)`). `draw_workspace` gains `editor_panes: &EditorPaneRegistry` param; editor pane branch replaced stub with real `draw_editor_into` call (looks up pane + buffer, blanks on missing). `lib.rs`: `pub mod editor` + `pub use editor::draw_editor_into`. `Cargo.toml`: `unicode-segmentation = { workspace = true }`. 5 new tests: `draw_editor_empty_buffer_paints_only_gutter_line_one`, `draw_editor_hello_world_paints_each_grapheme`, `draw_editor_cursor_at_row_5_col_3_paints_cursor_rect`, `draw_editor_long_line_paints_overflow_marker`, `draw_editor_scroll_skips_top_rows`. 154 anvil-render tests pass; clippy clean.
- 2026-05-25 — added concepts/native-editor.md (NE2-NE5 phase landing); covers Buffer/rope model, file IO, undo/redo, EditorPane + registry, render path. Updated wiki/index.md and wiki/concepts/README.md.

- 2026-05-25 — NE6: Keyboard input (insert mode). `EditorAction` enum (23 variants: InsertChar, InsertNewline, Backspace, Delete, MoveLeft/Right/Up/Down/LineStart/LineEnd/BufferStart/BufferEnd {extend}, PageUp/Down {extend}, Save, Undo, Redo, Copy, Cut, Paste(String), SelectAll, GoToLine(usize), InsertTab) added to `crates/anvil-workspace/src/editor_pane.rs`. `EditorPaneRegistry::apply(pane_id, action, clipboard_out) -> bool` implements all variants: insert ops advance cursor; Backspace/Delete walk grapheme boundaries via `prev_position`/`next_position`; cursor moves clamp to valid (line, grapheme-col); SelectAll sets anchor=(0,0) pos=(last_line, last_col); Save calls `buffer.save(tracked_path)` no-op-with-log when no path; Undo/Redo call buffer methods and clamp cursor; Copy/Cut extract text via `selected_text`; Paste inserts and advances cursor. Added `Buffer::tracked_path() -> Option<&Path>` accessor in `anvil-editor`. `EditorAction` re-exported from `anvil-control`. Key dispatch wired in `crates/anvil/src/main.rs`: `focused_is_native_editor()` + `apply_editor_action()` helpers on `App`; `key_event_to_editor_action()` free function; native-editor Cmd shortcuts (S/Z/Shift+Z/C/X/V/A/L) intercepted at top of `handle_cmd_chord`; Cmd+Up/Down/Home/End intercepted in `key_down` Cmd block for buffer-start/end; non-Cmd keys routed through `key_event_to_editor_action` before PTY write path. Terminal PTY path unchanged. `unicode-segmentation` dep added to `anvil-workspace/Cargo.toml`. 8 new tests: `apply_insert_char_appends_to_buffer`, `apply_backspace_removes_prior_char`, `apply_move_right_advances_cursor`, `apply_move_right_with_extend_grows_selection`, `apply_select_all_anchors_at_origin`, `apply_paste_inserts_string`, `apply_undo_reverts_last_edit`, `apply_save_with_no_path_is_noop`. 183 workspace tests pass; clippy clean.
- 2026-05-25 — NE2: File IO + encoding detection. `crates/anvil-editor/src/buffer.rs` (406 → 1300 LoC): `IoError` enum (`TooLarge`, `Encoding(EncodingError)`, `Io(std::io::Error)`) + `EncodingError` (`InvalidUtf8`, `UnsupportedEncoding`) with `Display`/`Error` impls; `tracked_path: Option<PathBuf>` and `tracked_mtime: Option<SystemTime>` added to `Buffer` struct and all constructors; `from_path` (50 MB guard), `from_path_with_limit` (pub(crate), testable), `decode_bytes` (UTF-8 BOM strip, UTF-16 LE/BE via `u16::from_le/be_bytes` + `char::decode_utf16`, no new crates), `to_text`, `save` (atomic: write `.tmp` then rename, updates tracked_mtime + calls `flush_undo_group`), `is_externally_modified` (compares stored mtime to disk). 9 new tests: `io_round_trip_ascii`, `io_round_trip_utf8`, `io_utf8_bom_stripped`, `io_utf16_le_round_trip`, `io_utf16_be_round_trip`, `io_too_large_rejected`, `io_invalid_utf8_rejected`, `io_atomic_save_round_trip`, `io_external_modification_detected`. No new runtime deps; `tempfile = "3"` dev-dep already present. 82 anvil-editor tests pass; clippy clean on crate.- 2026-05-25 — NE7: Mouse input. `crates/anvil-workspace/src/editor_pane.rs`: added local `FontMetrics` struct (avoids anvil-render cycle); `pixel_to_position(editor_pane, buffer, rel_x, rel_y, metrics, gutter_cols) -> Position` free function (row = floor(rel_y/cell_h)+scroll_pos clamped, col = round((rel_x - gutter*cell_w)/cell_w) clamped to grapheme length); three new `EditorAction` variants: `MoveTo { pos, extend }` (set cursor, optionally extend selection), `SelectWordAt(pos)` (walk graphemes left/right while alphanumeric/_), `SelectLineAt(pos)` (anchor col=0, pos col=line_len); all three variants implemented in `EditorPaneRegistry::apply`. `crates/anvil/src/main.rs`: `editor_mouse_drag_start: Option<EditorPosition>` field on `App`; `native_editor_rel_px` + `native_editor_pos_at` helpers on `App`; `mouse_down` gains native-editor branch (click_count 1/2/3 → MoveTo/SelectWordAt/SelectLineAt, Shift extends) before terminal-selection path with early return; `mouse_dragged` gains native-editor branch (extends selection) before tab/divider drag; `mouse_up` clears `editor_mouse_drag_start`; `scroll` gains native-editor branch (adjusts `scroll_target` on the editor pane). Terminal paths unchanged. `anvil-control/src/lib.rs` re-export unchanged (picks up new variants automatically). 7 new tests: `pixel_to_position_origin_returns_0_0`, `pixel_to_position_row_3_col_5_with_gutter`, `pixel_to_position_clamps_overflow`, `apply_move_to_clears_selection`, `apply_move_to_with_extend_preserves_anchor`, `apply_select_word_at_picks_word_span`, `apply_select_line_at_picks_full_line`. 190 workspace tests pass; clippy clean.

- 2026-05-25 — NE8 render wire-up: per-grapheme syntax color in `draw_editor_into`. `SyntaxLayer::highlights_for_range` signature changed from `&mut self → &[(Range<usize>, SyntaxRole)]` to `&self → Vec<(Range<usize>, SyntaxRole)>`; `visible_cache` field wrapped in `RefCell<HighlightCache>` so the cache write is interior mutation; all callers inside `syntax.rs` (`set_language_from_path`, `parse`, `edit`, `invalidate`) updated to use `borrow_mut()`. `Buffer::line_to_byte(line) -> usize` and `Buffer::syntax() -> &SyntaxLayer` accessors added in `buffer.rs`. `draw_editor_into` in `crates/anvil-render/src/editor.rs`: `SyntaxRole` imported; `buffer.to_text()` hoisted before row loop with TODO comment for future streaming; per-row `line_byte_start`/`line_byte_end` computed via `buffer.line_to_byte`; `buffer.syntax().highlights_for_range(start, end, &full_text)` called per row; grapheme loop tracks `grapheme_byte` offset and resolves fg color via `match role { SyntaxRole::Plain => theme.foreground, ... }`. New test `draw_editor_paints_keyword_color_on_fn_keyword`: buffer from "fn main() {}\n", Rust grammar set via `buf.syntax.set_language_from_path`/`buf.syntax.parse`, asserts 'f' and 'n' are painted with `theme.syntax.keyword` and not with `theme.foreground`. All pre-existing 5 editor render tests continue to pass (plain-text buffers with no language still produce `SyntaxRole::Plain` → `theme.foreground`). All workspace tests pass; clippy clean.

- 2026-05-25 — NE11: In-buffer search. New `crates/anvil-workspace/src/editor_search.rs`: `EditorSearch { query, is_regex, hits: Vec<Range>, current }` with `rescan(buffer)` (literal via `match_indices`; regex via `regex` crate, already in workspace), `next()`/`prev()` (wrapping), `current_hit()`, `clear()`. Byte→Position conversion via `Buffer::char_to_line`/`line_to_char`. `EditorPane` gains `search: Option<EditorSearch>` field (appended; all struct-literal constructors updated in `editor_pane.rs`, `anvil-render/src/editor.rs`). Six new `EditorAction` variants: `SearchOpen`, `SearchClose`, `SearchSetQuery(String)`, `SearchNext`, `SearchPrev`, `SearchToggleRegex`; all handled in `EditorPaneRegistry::apply`. `SearchNext`/`SearchPrev` set `cursor.anchor=hit.start`, `cursor.pos=hit.end`, `scroll_target=hit.start.line`. `draw_search_bar` in `anvil-render/src/searchbar.rs` gains `editor_search: Option<&EditorSearch>` parameter; reads query/counter from editor search when `Some`, terminal search otherwise. `crates/anvil/src/main.rs`: `open_search` branches on `focused_is_native_editor()`; `close_search` dispatches `SearchClose`; `search_next`/`search_prev`/`search_regex_toggle` keybind blocks branch on native editor; `key_down` search-open block branches on native editor for Backspace/Char/Enter (routes to `SearchSetQuery`/`SearchNext`). Also fixed pre-existing `lsp-types` `Uri`→`Url` revert (types were consistent at 0.95.1) and trivial clippy `map_or` → `is_some_and`. 5 new tests: `editor_search_finds_all_literal_hits`, `editor_search_regex_matches`, `editor_search_next_wraps`, `editor_search_prev_wraps`, `editor_search_clear_drops_hits`. All 11 workspace test suites pass (885+ tests); clippy clean.
- 2026-05-25 — NE9: LSP client core. New `crates/anvil-editor/src/lsp.rs` (~380 LoC): direct JSON-RPC over `tokio::process::Child` stdio (chose over async-lsp: async-lsp uses `futures::io` AsyncRead which does not compose cleanly with tokio process). `LspState` (Down/Spawning/Live/Failed(String)), `DiagnosticSeverity` (Error/Warning/Info/Hint), `DocumentDiagnostic { line, col, severity, message }`, `LspCommand` (DidOpen/DidChange/DidClose/Shutdown). `LspManager::new() -> Option<Self>` creates a 2-thread Tokio runtime (anvil-lsp). Servers spawn lazily: `which::which` binary detection at `get_or_spawn` time; `Failed` state set synchronously if binary missing; otherwise background task drives initialize → Live → select loop. `publishDiagnostics` stored per-path. `language_id_for_ext` (6 extensions) and `server_id_for_language` (rust→rust-analyzer, ts→typescript-language-server, py→pyright-langserver, toml→taplo, json→vscode-json-language-server). `Buffer::language_id()` derived from tracked_path extension. `App.lsp_manager: Option<LspManager>` + `lsp_last_sync: HashMap<PaneId, Instant>`. `AppShell::tick` 250 ms debounce: first sync → `did_open`; subsequent → `did_change`. Workspace deps added: `tokio = {version="1", features=["full"]}`, `lsp-types = "0.95"`. 4 new tests: `lsp_manager_new_does_not_panic`, `lsp_manager_state_of_down_when_no_did_open`, `lsp_manager_did_open_with_missing_binary_returns_failed_state`, `language_id_for_ext_rs_is_rust`. All workspace tests pass; clippy clean.

- 2026-05-25 — NE12: Project-wide search. New `crates/anvil-workspace/src/project_search.rs`: `ProjectSearchHit { path, line, col, preview }` and `ProjectSearch { query, root, hits, running, selected, visible }` with synchronous `scan(query, root)` using `ignore::WalkBuilder` (`.require_git(false)` so `.gitignore` files are honored outside git repos), `grep_regex::RegexMatcher`, and `grep_searcher::Searcher::search_path`. Caps: `MAX_HITS=1000` (returns early from closure with `Ok(false)`), `MAX_FILE_BYTES=1 MiB` (size guard before search_path), `MAX_DEPTH=8`. Navigation helpers: `select_next/prev`, `current_hit`, `open`, `close`. Keybind: `Cmd+Shift+F` → project search; `search_open_block` (block-scoped search) moved to `Cmd+Opt+Shift+F`. `project_search: String = "cmd+shift+f"` added to `anvil-config::Keybindings` (struct field + Default impl). `App` gains `project_search: ProjectSearch` field; `open_project_search()` method seeds root from `local_ctx.cwd` and re-runs scan if query non-empty. New workspace deps: `ignore = "0.4"`, `grep-matcher = "0.1"`, `grep-regex = "0.1"`, `grep-searcher = "0.1"` (added to root `Cargo.toml` workspace.dependencies and `anvil-workspace/Cargo.toml`). Dev-dep: `tempfile = "3"`. 3 new tests: `project_search_finds_literal_in_root`, `project_search_respects_gitignore`, `project_search_caps_hits_at_1000`. 198 anvil-workspace lib tests pass; clippy clean.

- 2026-05-25 — NE10 (partial): LSP UI surfaces — diagnostics gutter + hover popup. `RenderDiagnostic { line, severity }` and `RenderSeverity` added to `crates/anvil-render/src/editor.rs`; cycle-safe (render crate does not import LspManager). `draw_editor_into` gains `diagnostics: &[RenderDiagnostic]` parameter: per-row 4 px left-gutter stripe + α=0.06 row tint, colored by severity (failure/attention/info/alloy). Hover popup rendered as a floating panel (surface background, border, per-line glyph text) anchored below the cursor position. `draw_workspace` gains `diag_by_pane: &HashMap<PaneId, Vec<RenderDiagnostic>>` parameter; `main.rs` builds this map from `LspManager::diagnostics_for` each frame. `HoverPopup { text, anchor }` and `EditorAction::HoverRequest` / `HoverDismiss` added to `crates/anvil-workspace/src/editor_pane.rs`; `EditorPane.hover_popup: Option<HoverPopup>` field. `LspManager` gains `request_hover(path, line, character) -> u64` (sends `LspCommand::Hover` to server task), `poll_hover(request_id) -> Option<HoverResult>` (consumes the result slot), `HoverResult { text }`. Server task handles hover response via `handle_server_message`. Cmd+K in native editor triggers `trigger_hover_request()` → `poll_hover_result()` called each tick. 1 new test: `draw_editor_diagnostic_gutter_stripe_painted`. Deferred to follow-up dispatch: autocomplete popup, code action menu, definition jump. All workspace tests pass (199 new total); clippy clean.

- 2026-05-26 — NE8: Syntax highlighting (tree-sitter). New `crates/anvil-editor/src/syntax.rs` (≈220 LoC, 7 tests): `SyntaxRole` enum (Plain/Keyword/String/Number/Comment/Function/Type/Variable/Operator/Punctuation); `SyntaxLayer` holds a tree-sitter `Parser` + `Tree` + `Language` + `Query` + one-slot `HighlightCache`. `set_language_from_path` detects grammar from extension (.rs/.ts/.tsx/.py/.toml/.json/.md/.markdown). `parse` does a full parse; `edit(InputEdit, text)` does incremental reparse. `highlights_for_range(start_byte, end_byte, text)` runs a `QueryCursor` with byte-range restriction, maps capture names to `SyntaxRole` via first-dot-segment matching, and caches the result; `invalidate()` clears the cache. Grammars: tree-sitter 0.25.10 core; rust 0.24.2, typescript 0.23.2, python 0.25.0, toml-ng 0.7.0, json 0.24.8, md 0.5.3. All use bundled `HIGHLIGHTS_QUERY` (md uses `HIGHLIGHT_QUERY_BLOCK`). `SyntaxTheme { keyword, string, number, comment, function, type_, variable, operator, punctuation: [u8;3] }` added to `anvil-theme::theme::Theme` (field `syntax`) and populated for all 4 built-in themes using existing palette roles. `Buffer` gains `pub syntax: SyntaxLayer`; `from_path` calls `set_language_from_path` + `parse`; `apply_edit_at` calls `syntax.invalidate()`. 7 new tests: set_language_from_rs_path, parse_rust_fn_extracts_keyword, parse_python_string_extracts_string_role, incremental_edit_preserves_tree, no_language_returns_empty_highlights, cache_hit_on_same_range, invalidate_clears_cache. Release binary: 5.9 MB (well under 20 MB ceiling). 190 workspace tests pass; clippy and fmt clean.

- 2026-05-25 — NE4 fix-up: three reviewer BLOCKs resolved. HIGH 1: tick loop now skips PTY drain and dead-marking for editor panes (terminal.is_none()); close_dead_panes filter excludes editor panes. HIGH 2: close_focused_pane and close_dead_panes both call tab.editor_panes.remove_pane(id) alongside tab.registry.remove(id) to prevent buffer leaks. MED: added EditorCfg { backend: String } (default "nvim") to anvil-config; clamp() rejects unknown values with eprintln; Cmd+E / Action::NewEditorPane branch on config.editor.backend == "native"; Cmd+Opt+E side-channel chord removed. New tests: editor_pane_registry_remove_pane_evicts_only_target (editor_pane.rs), config_editor_backend_defaults_to_nvim + config_editor_backend_native_parses (anvil-config). Pre-existing compile breakages also fixed (try_to_blob→try_into_blob, cursor→cursors[0] rename, missing match arms for AcceptGhostText/DismissGhostText/AddCursorAt/ClearSecondaryCursors, draw_editor_into arity, missing anvil-editor dep in anvil-control). 902 workspace lib tests pass; clippy and fmt clean.

- 2026-05-25 — NE13: Git gutter + multi-cursor. New `crates/anvil-editor/src/git.rs`: `GitChange` enum (None/Added/Modified/Removed) and `GitGutter { per_line: Vec<GitChange> }`. `GitGutter::compute(buffer_text, path)` discovers repo via `gix::discover`, reads HEAD blob via `head_commit → tree → lookup_entry_by_path → try_into_blob`, diffs with `similar::TextDiff::from_lines`, annotates each buffer line (`Delete+Insert` → Modified, pure `Insert` → Added, `Delete` before Equal → Removed on the following line). Returns all-None on any error — no panic path. `Buffer` gains `pub git_gutter: Option<GitGutter>` field; recomputed in `from_path_with_limit` and `save`. Workspace deps added: `gix = { version = "0.71", default-features = false, features = ["max-performance-safe"] }` and `similar = "2"`. `draw_editor_into` gains `gutter: Option<&GitGutter>` as final parameter; gutter width expands by 2 cols when non-None; glyph painted per line: `+` in `theme.verified`, `~` in `theme.attention`, `▴` (`U+25B4`) in `theme.failure`, blank when None. `EditorPane::cursor: Cursor` promoted to `EditorPane::cursors: Vec<Cursor>` (primary at [0]); `primary_cursor()` and `primary_cursor_mut()` accessors added. New `EditorAction` variants: `AddCursorAt(Position)` (clamps, deduplicates, appends) and `ClearSecondaryCursors` (truncates to 1). `InsertChar`, `Backspace`, `Delete` made multi-cursor-aware using reverse-position order (sort descending, dedup, apply each, then advance all cursors uniformly). `main.rs`: Cmd+click → `AddCursorAt`; Esc with `cursors.len() > 1` → `ClearSecondaryCursors` before normal Esc handling. Secondary cursors rendered as 2 px `theme.accent_ember` bars; primary as `theme.accent`. 5 new tests: `git_gutter_compute_returns_empty_outside_repo`, `git_gutter_added_line_marked_added`, `multi_cursor_add_appends_to_cursors_vec`, `multi_cursor_insert_char_applies_to_all`, `multi_cursor_clear_drops_secondary`. All workspace tests pass; clippy clean.

- 2026-05-25 — NE14: AI-native edit API + ghost-text completions. Replaced empty placeholder structs in `crates/anvil-editor/src/buffer.rs` with concrete types: `ProposalStatus` (Pending/Accepted/Rejected), `EditProposal` (agent_id, proposed_at, edit, rationale, status), `ProposalError` (OutOfRange/NotPending), `GhostTextSpan` (anchor, text, source_agent), `AgentRevision` (revision, agent_id, at, note). Buffer gains `agent_revisions: Vec<AgentRevision>` field (renamed from the phantom `.revisions` slot; the NE3 u64 undo counter stays as `.revisions`). New Buffer API: `propose_edit`, `accept_proposal` (routes through `apply_edit` so undo records it), `reject_proposal`, `set_ghost_text`, `clear_ghost_text`, `record_agent_revision`. `apply_edit_internal` clears `ghost_text` on every mutation. `EditorAction::AcceptGhostText` and `DismissGhostText` added to `crates/anvil-workspace/src/editor_pane.rs` with full `apply()` handlers. Ghost-text render added to `crates/anvil-render/src/editor.rs`: spans at cursor position painted in `theme.text_subtle` after the cursor bar. Tab/Esc keybind intercept in `crates/anvil/src/main.rs`: when ghost text is active, Tab → AcceptGhostText and Esc → DismissGhostText (before normal keymap). `crates/anvil-control/src/lib.rs` gains `AgentInbound` enum with `AgentPropose`, `AgentSetGhost`, `AgentClearGhost` variants plus re-exports of AI buffer types. `anvil-editor` added to `anvil-control` Cargo.toml deps. 13 new tests (7 buffer, 1 render). 201 workspace lib tests pass; clippy clean.

- 2026-05-25 — Hermes item 5 finish: passive Explorer hover. Added `fn mouse_moved(&mut self, loc: MouseLocation)` to `AppHandler` trait (`crates/anvil-platform/src/appkit.rs`). `AnvilView` gains `mouseMoved:`, `mouseExited:`, and `updateTrackingAreas` ObjC methods; an `NSTrackingArea` (ActiveInKeyWindow | InVisibleRect | MouseMoved | MouseEnteredAndExited) is installed at startup and rebuilt on resize. `mouseExited:` reports (-1, -1) which maps to no hit zone, clearing hover. `mouse_moved` implemented on `AppShell` and `ForwardingHandler` in `crates/anvil/src/main.rs` using the same `left_dock_hits.at(rx, ry)` hit-test as `mouse_dragged`. `NSTrackingArea` feature added to `crates/anvil-platform/Cargo.toml`. All gates green.

- 2026-05-25 — Hermes items 7, 8, 9, 10: Explorer expand-in-place, scroll affordance, overflow smoke test, Outline empty-state. Item 7: `expanded_dirs: HashSet<usize>` on `App`; dir-row click toggles presence, clears on root snapshot change; chevron swaps ▸/▾ in `draw_explorer_section`. Do not call `open_path_in_native_editor` for dirs. Item 8: `scroll_indicator_alpha: f32` + `scroll_indicator_last_scroll: Option<Instant>` on `App`; tick decays alpha (600ms hold, 200ms ease-out); scroll handler sets alpha=1.0; renderer paints 3px thumb via `fill_pixel_rect_alpha`. Item 9: new `overflow_scroll_changes_rendered_entries` test using hit-region indices at two scroll offsets. Item 10: `draw_outline_section` computes `has_symbols` to pick header color (`text_subtle` when empty, `accent_bright` when rows present); `None | Some([])` arm renders header only (no body copy); `blend_50` helper removed. Updated tests: `entries_rendered_with_correct_colors` (checks chevron U+25B8 in `text_subtle` instead of body copy 's'); `outline_unavailable_always_shown` (negative check uses 'c' unique to removed body copy); `left_dock_renders_outline_no_symbols` (negative check uses 'y' unique to removed "No symbols" body); `outline_empty_header_uses_text_subtle` (uses 'U' unique to "OUTLINE" to avoid 'O' ambiguity with "EXPLORER"). `draw_left_dock_with_scroll` signature gains `expanded_dirs: &HashSet<usize>` and `scroll_indicator_alpha: f32`. All workspace tests pass; clippy clean.

- 2026-05-25 — Item 7 (real nesting) + file-open pipeline: Explorer now renders nested children. `collect_visible_rows` recursively walks root snapshot + `child_snapshots: HashMap<PathBuf, DirSnapshot>` (depth cap 32) to produce a flat `all_rows: Vec<(PathBuf, bool, usize)>`. `LeftDockHits` gains `visible_rows: Vec<(PathBuf, bool)>` parallel to hit indices. `expanded_dirs` promoted from `HashSet<usize>` to `HashSet<PathBuf>` (absolute path, survives re-snapshots). `spawn_child_fs_worker` in `fs_worker.rs` reads a child dir off-thread and sends `(PathBuf, DirSnapshot)` back; drained each tick in `refresh_hud`. On dir-click: if path not in `child_snapshots`, sends load request then inserts into `expanded_dirs`; click again removes from `expanded_dirs`. On file-click: calls `open_path_in_native_editor`. Smoke-tested: expand shows indented children after child worker responds (~1s), collapse removes them, file click loads content in editor with ember left-rail selection. Two new tests: `expanded_dir_shows_children_in_visible_rows`, `collect_visible_rows_depth_cap`. Commit: 4d16a94.

- 2026-05-25 — NE15: nvim editor backend retired. Deleted `crates/anvil-editor/src/nvim/` (bridge, codec, transport, mod.rs); trimmed `anvil-editor::lib.rs` re-exports to native-only types. Removed `App` fields `editor_bridge`, `editor_snapshot`, `editor_pane_id`, `editor_socket_counter`, `nvim_path`; removed methods `new_editor_pane` and `clear_editor_pane_if`; dropped the `$NVIM_LISTEN_ADDRESS` bridge spawn at startup and the per-tick `bridge.snapshot()` pull. `Cmd+E` and `Action::NewEditorPane` now invoke `new_native_editor_pane` unconditionally — Cmd+E is the sole editor pane keybind. Dropped `editor.backend` config key + `EditorCfg` struct + its two tests; dropped `which` crate dep from `anvil/Cargo.toml` (still used by `anvil-editor::lsp`). Updated `anvil-render::context_bar`: replaced `Option<&EditorSnapshot>` with a new `ContextBarEditor { name, modified }` struct fed by the focused native editor pane's `Buffer::tracked_path` basename (no dirty flag yet — Buffer has no is_modified API). Left-dock outline now passes `None` (nvim LSP outline gone; native LSP outline wiring deferred to a follow-up). Updated palette catalog subtitle ("Open the native editor in a new pane"). 2 new tests: `tab_split_native_editor_registers_editor_pane_and_buffer` (anvil-workspace), `editor_new_default_chord_is_cmd_e` (anvil-config). `cargo anvil-build`, `cargo test --workspace --lib`, `cargo clippy --workspace -- -D warnings` all clean. `rg nvim` in source returns only NE15 retirement comments and the `vi/vim/nvim/emacs/nano` helper-text mention.

- 2026-05-26 — Hermes #14: per-pane editor buffer tab strip. Replaced the single-chip ◇ buffer.rs chrome with a 34px N-tab strip (VS Code/Zed model). `EditorPane` gains `open_buffers: Vec<BufferId>`; `buffer_id` stays as the active pointer. New registry methods: `open_path_as_tab` (dedup by path, MAX_TABS_PER_PANE=16 cap), `open_buffer` (tab-click switch), `close_buffer` (right-neighbor fallback; creates fresh scratch when last buffer closed). `draw_editor_chrome` in `workspace.rs` rewritten to paint N tabs; `EditorTabHit` struct emitted per tab-body and per-close-button. `draw_workspace` gains `hovered_editor_tab` + `editor_tab_hits` params. `main.rs`: `open_path_in_native_editor` calls `open_path_as_tab`; `mouse_down` hit-tests editor tabs before chrome row; `mouse_moved`/`mouse_dragged` update `hovered_editor_tab` for × hover. 5 new tests; 926 total passing (was 921). Commits: 11264b1, 838c224.

- 2026-05-26 — 20-task burn-down group 4: items 13b, 18, 19, 20. (13b) Drawer drag-to-resize: `drawer_drag_active: bool` on App, `ide_drawer_divider_y()` helper, ±4px hit zone on IDE vertical split divider, editor ratio clamped to [0.40, 0.95] (drawer [0.05, 0.60]) on drag. (18) LSP rootUri: thread `std::env::current_dir()` into `spawn_server`; populate `InitializeParams.root_uri`; log `anvil-lsp: <bin> not found in PATH; diagnostics disabled` to stderr on missing binary. (19) Outline panel: `derive_outline_rows()` in `anvil-editor::syntax` walks tree-sitter tree for fn/impl/struct/enum/trait; regex fallback for non-Rust; `line: usize` field added to `OutlineRow`; wired into `draw_left_dock_with_scroll`; outline-row click dispatches `EditorAction::MoveTo`. (20) Multi-window stub: `App::detach_buffer_to_new_window` removes buffer from source pane and logs; `TODO(anvil-20-window-spawn)` in `AppKitApp::new`. 6 new tests added (13b clamp bounds, 18 rootUri path, 19 fn find/struct-impl fallback/line accuracy, 20 detach removes buffer). 942 total passing (was 936). 4 commits: b96483c, 6a35280, 8669f6f, 0bc6dee.

- 2026-05-26 — Editor functional-gap closure (7 items): (1) PTY in IDE drawer — `spawn_ide_terminal_drawer` in `main.rs` spawns a terminal pane + PTY when entering IDE mode if none exists; called from layout_mode_toggle, Cmd+E, and palette `LayoutIde`. (2) Cmd+P file picker — `send_file_picker_show` reuses `recent_files_in_dir` (depth 3, ≤500 files), injects `file:open:<abs-path>` commands into the palette webview; `Inbound::Invoke` handler short-circuits on `file:open:` prefix and calls `open_path_in_native_editor` directly. Items 3 (Cmd+S) and 6 (Cmd+F search) were already wired (NE6/NE11). (4) Typing verified wired via `key_event_to_editor_action`. (5) Cmd+W closes active buffer tab in native editor pane (falls through to close-tab for terminal panes). (7) Cmd+\ splits native editor pane horizontally; Cmd+Shift+\ vertically. 3 new tests in `anvil-workspace`: `split_native_editor_horizontal_adds_second_editor_leaf`, `normalize_ide_editor_drawer_without_terminal_returns_editor_only`, `file_open_prefix_not_in_action_catalog`. 931 total passing (was 928). fmt/clippy/test all green.

- 2026-05-26 — IDE tier-2 power features (6 items): (9) Find+Replace — `EditorSearch` gains `replace_input: Option<String>`, `open_replace()`, `close_replace()`; `EditorAction`: `FindReplaceOpen`, `SetReplaceInput`, `ReplaceOne` (replace current hit, rescan), `ReplaceAll` (sort hits descending, apply all, rescan). Keybinding: Cmd+Opt+F opens find+replace; Tab switches find/replace rows; Enter/Backspace/Char route to the active row. `draw_search_bar_with_replace` renders second "replace: " row with `[replace]` and `[all]` buttons when `replace_input` is Some. `replace_row_active: bool` on App. (10) Project-wide search overlay — `ProjectSearch` state machine (already existed from NE12). Added `draw_project_search_overlay` in main.rs (centered panel, input + result rows, selected row tinted); key handler: Esc/Enter/Up/Down/Backspace/Char; Enter opens file via `open_path_in_native_editor` + `GoToLine`, then closes overlay. (11) Goto-line overlay — `goto_line_input: Option<String>` on App; Cmd+G opens; keys: digit/comma only, Enter parses NNN or NNN,CCC (1-indexed), dispatches `GoToLine`; `draw_goto_line_overlay`: small accent-bordered panel. (12) Multi-cursor Cmd+D — `EditorAction::AddNextOccurrence`: if primary cursor has selection, finds next occurrence of selected text and adds a secondary cursor with same-length selection; if no selection, expands to word under cursor first; wraps at buffer end. (13) Code folding — `derive_fold_ranges(layer)` in `anvil-editor::syntax` walks tree-sitter nodes (function_item/impl_item/struct_item/enum_item/trait_item/mod_item/block); exported as `FoldRange { start, end }`. `EditorPane.folds: HashMap<BufferId, HashSet<usize>>`. `EditorAction::ToggleFold(line)`. Row loop in `draw_editor_into` changed to while loop skipping `hidden_lines`; chevron ▾/▸ in last gutter column; folded range → one `…` marker row. Gutter-click via `gutter_click_fold_line` dispatches ToggleFold. (14) Bracket matching — `bracket_match_for(buffer, pos, max_lines)` in `editor_search.rs`: stack scan over ±max_lines/2 region, returns `(open_pos, close_pos)` or None; exported from anvil-workspace. Render: 2px outline around both matched brackets in `theme.accent` α=0.4. Gates: 945 tests pass (was 942; +3 bracket_match tests), clippy clean, release build clean. Commits: 71a1feb, f513e4e, c2cdc1c.

- 2026-05-26 — Tier-3 LSP items 15-18 complete. (15) Hover popup fires: 400ms mouse debounce in tick loop reads `hover_mouse_pos`/`hover_mouse_time`; fires `request_hover` when still; existing `EditorPane.hover_popup` render path unchanged. (16) Completion popup: `LspCommand::Completion` + `parse_completion_result` in `lsp.rs`; `CompletionPopup` struct on `EditorPane` (12-row prefix-filtered list, `CompletionEntry { label, detail, insert_text }`); `EditorAction` variants `CompletionOpen/Up/Down/Accept/Dismiss/Filter`; rendered as floating list (selection α=0.18 accent, label 24-col, detail 20-col); Ctrl+Space triggers, autotrigger on `.`/`:`; `HoverSlot`/`DefinitionSlot`/`CompletionSlot` type aliases fix clippy `type_complexity`. (17) Goto definition: `LspCommand::Definition` + `parse_definition_result` (handles single Location / Vec<Location> / Vec<LocationLink>); Cmd+click → `trigger_definition_request` → `poll_definition_result` jumps cursor (opens new buffer if different file); `TODO(anvil-tier3-17-picker)` stub for multi-location picker. (18) Symbol breadcrumbs: `EDITOR_BREADCRUMB_H=22px` strip below tab bar; `breadcrumb_segments_at_line` calls `derive_outline_rows` filtered to symbols ≤ cursor line, formats "fn foo"/"impl Bar" up to depth 8; `draw_breadcrumb_row` paints graphite strip + `text_subtle` " › "-joined segments; 3 tests (empty registry, Rust fn identifier, plain text). Tests: 948 total passing (was 945). Commits: dbbac41, b38e76c, a31909d.

- 2026-05-26 — Tier-4 items 19-20: session persistence + graceful shutdown. (19) `crates/anvil/src/session.rs`: `SessionState` (serde_json), `session_path()` (DefaultHasher → 16-char hex under `~/.config/anvil/sessions/`), `save_session()`, `load_session()`. `App::build_session_state()` captures ui_scale, left_dock_w_pt, layout_mode, editor split ratio, expanded_dirs, and open buffer paths per pane. `App::restore_session()` applies stored state on startup and reopens saved paths. `crates/anvil/Cargo.toml` gains `serde`+`serde_json` workspace deps + `tempfile` dev-dep. 5 new tests in session.rs. (20) `LspManager::shutdown_all()` in `anvil-editor/src/lsp.rs`: sends `Shutdown` to each live server, polls ≤500ms for Down/Failed state, clears servers map. `window_will_close` in `appkit.rs` now calls `handler.should_terminate()` (which runs `App::shutdown()`) before `process::exit(0)`. `App::shutdown()` saves session → shuts down LSP → clears PTYs. `TODO(anvil-tier4-20-shouldterm)` left in appkit.rs for future async cleanup path. 2 new tests (shutdown_all empty/idempotent). 955 total tests passing (was 948). Commits: f270152, bfc07a5.

- 2026-05-26 — Explorer tier-1 polish (8 items): (1) Global ui_scale — `RowMetrics::from_scale(ui_scale)` replaces four hardcoded constants (HEADER_H/ROW_H/PAD_X/INDENT_PX) in left_dock.rs; `ui_scale: f64` field on App (default 1.0); `bump_ui_scale(delta)` on AppShell does full font rebuild; Cmd+=/−/0 bound to ±0.1/reset; `chrome_top/bottom_px` and dock width all multiply by `ui_scale`. (2) Tree branch glyphs — `is_last_at_depth` computed per visible row; │├└─ (U+2502/251C/2514/2500) rendered in depth columns before each row icon. (3) Nerd Font file icons — `file_icon_colored(name, theme)` returns BlexMono Nerd Font codepoint + semantic color; Rust→U+E7A8, Markdown→U+F48A, TOML/YAML→U+E6B2, JSON→U+E60B, HTML→U+E736, CSS→U+E749, TXT→U+F15C, lock→U+F023, default→U+25C7. (4) Explorer keyboard nav — `FocusTarget` enum (Editor|Explorer|Terminal); dock click sets FocusTarget::Explorer; ↑↓→←/Enter/Esc routed to Explorer when focused; `selected_explorer_row: Option<usize>` tracks cursor. (5) Editor↔Explorer selection sync — `active_explorer_file` on App; `sync_active_explorer_file()` expands collapsed ancestor dirs; every buffer tab switch and file-open click calls sync. (6) Inline rename — `RenameState {old_path, input, row_idx}`; F2 triggers; Enter commits `fs::rename` + tree reload, Esc cancels. (7) New file/folder — `NewItemState {parent_dir, input, is_dir}`; Cmd+N/Cmd+Shift+N when Explorer focused; Enter creates file/dir + reload, Esc cancels. (8) Delete with confirm — `DeleteConfirm {path, name}`; Delete key triggers modal; Enter executes `remove_file`/`remove_dir_all` + reload, Esc cancels. Files: `crates/anvil-render/src/left_dock.rs`, `crates/anvil/src/main.rs`. 942 total tests (unchanged from baseline). Commits: 8adb8f5, fe15925.

- 2026-05-26 — Tier-A render/scaling bugs (6 items). A1: test asserts top-level explorer rows (depth=0) emit no tree connector glyphs; nested depth-1 rows do (guard `if *depth > 0` already existed — test was missing). A2: `draw_editor_chrome` gains `ui_scale: f64` param; `TAB_MIN_W`, `TAB_MAX_W`, `TAB_PAD_L`, `TAB_CLOSE_W` scaled at runtime; `draw_workspace` threaded with `ui_scale`; labels clip to `text_max_x` with `…` ellipsis; tab-overflow test added. A3: breadcrumb height reserved only when `breadcrumb_segments_at_line` returns non-empty (was always 22px); test verifies `content.y == rect.y + EDITOR_TABS_H` for empty breadcrumbs. A4: `Docks::for_mode_with_left_dock_w` gains `ui_scale: f64` param; `top_h = 28.0 * scale * ui_scale`; 3 main.rs call sites updated; test verifies `top_h = 42.0` at ui_scale=1.5. A5: status bar height `= EDITOR_STATUS_H * ui_scale` (folded into A2 chrome changes). A6: drawer and sidebar hit zones scale by `ui_scale` (was fixed 4 device px; now `4.0 * ui_scale`). 959 total tests passing (was 955; +4 new). Files: `crates/anvil-render/src/left_dock.rs`, `crates/anvil-render/src/workspace.rs`, `crates/anvil-workspace/src/mode.rs`, `crates/anvil/src/main.rs`.

- 2026-05-26 — Tier-B keybind + polish (items 7-15). Item 7 (Cmd+B sidebar) was already implemented (confirmed). Items shipped: #8 Cmd+J drawer toggle (`toggle_ide_drawer`, `drawer_hidden`+`drawer_saved_ratio` fields); #9 Cmd+1-9 buffer-tab jump when native editor focused (intercepts jump loop before tab-switch); #10 editor buffer tab drag-reorder (`editor_tab_drag` field, 4px threshold, swaps `open_buffers`); #11 find-match highlights in editor body (α=0.30 attention wash for inactive hits, 1px outline for active); #12 cursor blink in editor (`blink_phase: f32` param on `draw_editor_into`, routed from `blink_phase` via `cursor_opacity`); #13 selection wash α 0.18→0.25 + color `accent_ember`→`accent_primary`; #14 Cmd+, opens `~/.config/anvil/config.toml` (creates skeleton if missing); #15 Cmd+Shift+E recent-files palette (tracks up to 50, shows top 10). 3 new tests (`cursor_blink_at_mid_phase_dims_cursor`, `find_match_highlights_render_attention_wash`, `selection_wash_uses_accent_primary`). 969 total tests passing (was 959; +10 new). Commits: 322c0ee, bf83c7f. Files: `crates/anvil-render/src/editor.rs`, `crates/anvil-render/src/workspace.rs`, `crates/anvil/src/main.rs`.

- 2026-05-26 — Tier-D LSP depth (items 24-26, all 3 shipped). #24 LSP rename (F2 in editor body): `LspManager::request_rename/poll_rename`; `lsp_rename_input: Option<String>` overlay on App; F2 key in editor block opens overlay pre-filled with word under cursor; Enter sends `textDocument/rename`, applies `WorkspaceEdit` edits to open buffers (or loads+edits+saves from disk); `apply_rename_edits` helper groups edits by path and applies in reverse document order. #25 LSP code actions (Cmd+.): `LspManager::request_code_actions/poll_code_actions`; `CodeActionsPopup` on `EditorPane` mirrors `CompletionPopup` struct; renders via same floating-list chrome in `draw_editor_into`; `code_actions_pending_edits: Vec<Vec<RenameEdit>>` on App; `WorkspaceEdit` flattened to `Vec<RenameEdit>` at receive time (no lsp-types dep in anvil binary); Enter applies selected action's edits; command-only actions are no-ops (log: TODO workspace/executeCommand). #26 References (Shift+F12): `LspManager::request_references/poll_references`; `LspReferencesOverlay` on App with `rows: Vec<ReferencesRow>` and `selected` index; renders centered floating panel showing `file:line:col` rows; ↑↓ navigates, Enter jumps to path+line, Esc dismisses. 8 new tests in `anvil-editor` (no-server returns 0 × 3, poll-zero-id-none × 3, parse_workspace_edit_flattens_edits, parse_code_actions_result_extracts_title). 995 tests passing (was 987; +8). Gates: fmt ✓, clippy -D warnings ✓, test --workspace ✓. Files: `crates/anvil-editor/src/lsp.rs`, `crates/anvil-editor/src/lib.rs`, `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil-render/src/editor.rs`, `crates/anvil/src/main.rs`.

- 2026-05-26 — Tier-E final 4 items (27-30), all 4 shipped. #27 File watcher: mtime-polling background thread sends `FileWatchEvent { buffer_id }` via mpsc; tick drain silently reloads clean buffers (`Buffer::reload_from_disk`) or records dirty ones in `disk_changed_dirty` for a "file changed on disk — Cmd+R to reload" banner; Cmd+R forces reload. `Buffer::is_dirty` + `saved_revision` added; `save_with_options` snapshots `saved_revision`. #28 Welcome screen: IDE mode shows a centered panel (title, subtitle, action rows, recent-project list, version footer) when no buffers are open (`App::should_show_welcome`). #29 Crash reporter: `std::panic::set_hook` installed at top of `main()` writes panic info + forced backtrace to `~/.config/anvil/crashes/crash-<ts>.txt`, then delegates to default hook. #30 Project switcher: `recent_projects: Vec<PathBuf>` on `App` (cap 20) persisted in `SessionState`; Cmd+Shift+O opens palette-style overlay; Enter spawns new Anvil in chosen dir via `std::process::Command` and exits; `build_session_state` and `restore_session` updated. 7 new tests (`is_dirty_clean_on_load`, `is_dirty_after_edit`, `is_clean_after_save`, `reload_from_disk_replaces_content`, `reload_from_disk_clears_undo_history`, `reload_from_disk_no_path_returns_error`, `session_recent_projects_round_trip`, `session_missing_recent_projects_defaults_to_empty`). Gates: fmt ✓, clippy -D warnings ✓, test --workspace ✓. Commit: 2740548. Files: `crates/anvil-editor/src/buffer.rs`, `crates/anvil/src/main.rs`, `crates/anvil/src/session.rs`.

- 2026-05-26 — Tier-F layout bugs (6 items, all 6 shipped). F1: `main.rs:3073` — render_frame Docks call passed `left_dock_w_pt` without `* self.ui_scale`; the context_bar/left_dock areas used a 1× dock width while resize_all_tabs used a ui_scale× width, causing editor text to paint under the explorer dock on sidebar drag. F2: `editor.rs::draw_editor_into` — added explicit `gx < rect.x || gx + cw > rect.x + rect.w` guard in glyph emit loop (defense in depth against future rect misalignment). F3: tab-strip clip already correct via pane_area_rect; no code change needed (confirmed). F4: `context_bar.rs` — replaced `draw_run_clipped` with new `draw_run_ellipsized` for path text; appends `…` when path exceeds available width; 1 new test (`long_path_gets_ellipsis`). F5: `main.rs::AppShell::resize` — removed `if !in_live_resize` guard; `resize_all_tabs()` now always fires during live window drag. F6: `appkit.rs` — `window.setContentMinSize(NSSize::new(640.0, 400.0))` added after NSWindow creation. 1 new test; 1004 total tests passing (was 1003). Gates: fmt ✓, clippy -D warnings ✓, test --workspace ✓, release build ✓. Commit: 1ea1b03. Files: `crates/anvil/src/main.rs`, `crates/anvil-render/src/editor.rs`, `crates/anvil-render/src/context_bar.rs`, `crates/anvil-platform/src/appkit.rs`.

- 2026-05-26 — Tier-H power-editing (4 items shipped). H1: `soft_wrap: bool` on `EditorPane`, Cmd+K two-stroke chord via `pending_chord_k: bool` in `App` (W = soft-wrap, Space = whitespace, K = preserve HoverRequest). Stub render shows "wrap" hint with `TODO(anvil-tierH-H1-wrap):` — multi-visual-row layout deferred. H2: `show_whitespace: bool` on `EditorPane`, render loop emits U+00B7 for spaces and U+2192 for tabs via pre-blended `text_subtle @ α=0.4` (no alpha variant in raster API). H3: `IndentStyle { Spaces(usize), Tabs(usize) }` enum + `Buffer::indent_style()` scanning first 100 non-blank lines; label appended to editor status bar right side. H4: `font_scale: f64` in `App` + `SessionState` (`#[serde(default)]` for compat); `bump_font_scale` on `AppShell` rebuilds 4 font faces, clamps [0.6, 2.5]; Cmd+Opt+=/- wired in both `key_down` and `perform_key_equivalent`; startup restore after `restore_session`. 8 new tests (4 H3 buffer, 2 H4 session). Gates: fmt ✓, clippy -D warnings ✓, test --workspace ✓. Commits: a13f870, 886d4ae, 6896b23. Files: `crates/anvil-editor/src/buffer.rs`, `crates/anvil-editor/src/lib.rs`, `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil-render/src/editor.rs`, `crates/anvil-render/src/workspace.rs`, `crates/anvil/src/main.rs`, `crates/anvil/src/session.rs`.

- 2026-05-26 — Tier-G adaptive layout (4 items, all 4 shipped). G1: sidebar icon-only mode below 120pt (`draw_left_dock_icons_only`): 48pt wide, icons centered, no header/labels/outline, click hits preserved; hidden entirely below 60pt. G2: drawer collapse strip below 50pt height: PTY cells skipped, 24pt charcoal strip with "▸ TERMINAL" rendered via `draw_drawer_collapsed_strip`. G3: smooth sidebar drag — `left_dock_w_pt_target` added to `App`; drag updates target only; `tick` eases `_pt` at 0.35 factor/frame, snaps at <0.5pt delta; `resize_all_tabs` called on each step. G4: per-buffer scroll in `EditorPaneRegistry.buffer_scroll: HashMap<BufferId, f32>`; `open_buffer` saves outgoing scroll and restores incoming; `close_buffer` drops entry. 6 new tests (2 G1, 1 G2, 2 G4). Gates: fmt ✓, clippy -D warnings ✓, test --workspace ✓, release build ✓. Commit: 08654c4. Files: `crates/anvil-render/src/left_dock.rs`, `crates/anvil-render/src/workspace.rs`, `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil/src/main.rs`.

- 2026-05-26 — Tier-J items J1 (save-on-blur) and J2 (save-as). J1: `EditorCfg { save_on_blur: bool }` added to `anvil-config`; `AppShell::focus_lost` iterates all tab editor registries and calls `Buffer::save` on dirty buffers with tracked paths; skips if `explorer_rename`, `lsp_rename_input`, or `save_as_input` modals are active; errors to stderr. J2: `EditorAction::SaveAs(PathBuf)` added; `save_as_input: Option<String>` field on `App`; Cmd+Shift+S opens inline path-input overlay pre-filled with the current buffer path; Enter applies `SaveAs`, Escape cancels; `draw_save_as_overlay` renders a 60-column bordered panel; `TODO(anvil-tierJ-J2-nspanel)` marks the NSSavePanel upgrade. `EditorPaneRegistry::buffers_mut()` added for J1. Gates: fmt ✓, clippy -D warnings ✓, test --workspace ✓ (all 269+ pass). Commit: c86f367. Files: `crates/anvil-config/src/lib.rs`, `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil/src/main.rs`.

- 2026-05-26 — Tier-I context menus + drag-drop (4 items shipped). I1: Explorer right-click shows Open/Rename/Delete/New File/New Folder/Reveal in Finder. `right_click_zone()` added to `AppHandler` to query hit-surface before the menu is built; `App::right_click_path` stores the target path for `context_action`. I2: Editor pane right-click shows Go to Definition/Find References/Rename Symbol (grayed when no LSP)/Format File/Toggle Comment; dispatches to existing trigger helpers. I3: Explorer-drag-to-editor — `App::explorer_drag` + `explorer_drag_cursor` fields; `mouse_dragged` tracks cursor past 4pt threshold; floating basename chip rendered near cursor; `mouse_up` over editor area calls `open_path_in_native_editor`. I4: Finder drop — `AnvilView` registers for `NSPasteboardTypeFileURL`; `draggingEntered:` returns `Copy`; `performDragOperation:` extracts paths from `NSFilenamesPboardType` and calls new `AppHandler::dropped_files` method. Added `NSDragging`/`NSArray` features to `anvil-platform/Cargo.toml`. Gates: fmt ✓, clippy -D warnings ✓, test --workspace ✓, release build ✓. Commit: e3c983a. Files: `crates/anvil-platform/Cargo.toml`, `crates/anvil-platform/src/appkit.rs`, `crates/anvil-platform/src/lib.rs`, `crates/anvil/src/main.rs`.

- 2026-05-26 — Tier-L clipboard + undo/redo (L1/L2). L1: `EditorAction::Copy` now falls back to copying the whole current line (with `\n`) when no selection is active, matching VS Code behaviour. `EditorAction::Cut` does the same and deletes the line. The NSPasteboard `set_clipboard`/`get_clipboard` helpers and Cmd+C/X/V/Z/Shift+Z key wiring were already complete; only the no-selection fallback was missing. L2: `EditorAction::Undo`/`Redo` and `Buffer::undo()`/`redo()` confirmed fully wired; added an explicit undo→redo round-trip test. 4 new tests (copy no selection, copy with selection, cut no selection, undo/redo round trip). Gates: fmt ✓, clippy -D warnings ✓, test --workspace ✓. Commit: 163fab7. Files: `crates/anvil-workspace/src/editor_pane.rs`.

- 2026-05-26 — Tier-K selection/mouse (K6 shipped; K1–K5 already present). K1 (double-click word), K2 (triple-click line), K3 (drag extend), K4 (Shift+arrow), K5 (Cmd+A) were all already implemented. K6 (Cmd+L select line): added `EditorAction::SelectLine`; first call selects col 0 → end-of-line; repeated calls extend down one line while selection is line-aligned. Wired Cmd+L (replacing no-op placeholder). 3 new tests. Gates: fmt ✓, clippy -D warnings ✓, test -p anvil-workspace ✓ (242 pass). Files: `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil/src/main.rs`.

- 2026-05-26 — Tier-M scroll/nav (M1–M5, all 5 shipped). M1: mouse-wheel updates `scroll_target` by `delta_y / cell_h`, clamped to `[0, max_lines - visible_rows]`. M2: `tick()` easing loop — `pos += (target - pos) * 0.35`, snaps at delta < 0.01. M3: PageUp/PageDown intercept in `key_down()` adjusts `scroll_target ±visible_rows` before applying cursor action. M4: Cmd+↑/↓ sets `scroll_target = 0` or `max_scroll` after moving cursor. M5: 3px right-edge scrollbar thumb in `text_subtle` α=0.6, rendered in `draw_editor_into()`, threaded through `draw_workspace()`, fade-in/out reuses existing 600ms/200ms timer shared with Explorer. New helper `editor_visible_rows()` on `App`. 5 new tests. Gates: fmt ✓, clippy -D warnings ✓, test --workspace ✓. Commits: 3cc74d4, 2c158fc. Files: `crates/anvil/src/main.rs`, `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil-render/src/editor.rs`, `crates/anvil-render/src/workspace.rs`.

- 2026-05-26 — Tier-C editor power features (items 16-23, all 8 shipped). #16 InsertNewlineSmart: auto-indent on Enter copies leading whitespace of prior line; adds 4 extra spaces after `{`/`(`/`[`. #17 Smart bracket pairs: typing `(`, `[`, `{`, `"`, `'`, `` ` `` auto-inserts the pair, cursor lands between; wraps selection; skips over existing closing bracket; skips quote auto-pair after alphanumeric. #18 Cmd+/ ToggleLineComment: per-language marker (`//` or `#`); adds/strips on all selected lines. #19 Cmd+Shift+D DuplicateLine: copies current line below, cursor follows. #20 Opt+Up/Down MoveLineUp/MoveLineDown: swaps current line with neighbor, cursor follows. #21 Tab/Shift+Tab on multi-line selection: IndentSelection prepends 4 spaces; DedentSelection removes up to 4. #22 Buffer::save trims trailing whitespace before write (opt-out via `save_with_options(false)`). #23 Cmd+Shift+I FormatFile: rustfmt fallback for Rust buffers (`--emit stdout --edition 2021`); LSP path stubbed as TODO(anvil-tierC-#23-lsp). 18 new tests; 987 total tests passing (was 969; +18). Commit: 7b5bcba. Files: `crates/anvil-editor/src/buffer.rs`, `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil/src/main.rs`.

- 2026-05-26 — Tier-N visual polish (N1–N4, all 4 shipped). N1 indent guides: 1px `text_subtle` α=0.25 vertical bars at each indent stop in leading whitespace; indent width from `Buffer::indent_style()`. N2 tildes below buffer end: vim-style `~` glyphs (text_subtle α=0.4) at col 0 of every visual row below the last buffer line. N3 toast system: `VecDeque<Toast>` on `App` with `toast_info/success/error` helpers, 3 s TTL, 60-char cap, 5-toast cap; `draw_toasts` paints a bottom-right stack above the status bar; hooked on Cmd+S (success/failure) and LSP-not-found (info, one-shot per server). N4 search-bar nav arrows: `SearchBarArrowHits` out-param on `draw_search_bar_with_replace` carries pixel rects for ◀ ▶ buttons rendered right of the "N of M" counter; `mouse_down` in `AppShell` checks rects and fires `SearchPrev`/`SearchNext`. 8 new tests; all 1019 tests passing. Commit: ea4fc0b. Files: `crates/anvil-render/src/editor.rs`, `crates/anvil-render/src/searchbar.rs`, `crates/anvil/src/main.rs`.
- 2026-05-26 — Tier-O IDE final polish (O1/O2/O3, all 3 shipped). O1 Cmd+T workspace symbols: `WorkspaceSymbolHit { name, kind_label, path, line }` + `request_workspace_symbols`/`poll_workspace_symbols`/`any_live` on `LspManager`; `WorkspaceSymbolSearch` state on App with 200ms debounce, Up/Down/Enter/Esc routing; Enter calls `open_path_in_native_editor` + `GoToLine`; "(LSP unavailable)" shown when no server live. O2 Cmd+R buffer symbols: `BufferSymbolSearch` state backed by `derive_outline_rows`; inline if keybinding (before reload handler to win Cmd+R); Enter dispatches `GoToLine(line)`. Both overlays share the project-search overlay shape. O3 status-bar mode chip: `StatusMode` enum (Editing/Searching/Renaming/Picking) in `anvil-render::statusbar`; left-anchored chip on editor status line (`EDITING`=text_subtle, `PICKING`/`SEARCHING`=accent_primary, `RENAMING`=accent_bright); `draw_status_bar` gains `mode: StatusMode` param; App computes mode from overlay/rename state. Note: `SEARCHING` is structurally unreachable (search bar and status bar are mutually exclusive chrome — kept for spec completeness). 2 new statusbar tests (mode chip color mapping). 1 clippy fix (`map_or` → `is_none_or`). All tests passing. Commits: a81eed2, 75b708b. Files: `crates/anvil-editor/src/lsp.rs`, `crates/anvil-editor/src/lib.rs`, `crates/anvil-render/src/statusbar.rs`, `crates/anvil/src/main.rs`.
- 2026-05-26 — Tier-R items R1/R2/R3 shipped. R1 inline diagnostic labels: `RenderDiagnostic.message: String` added; render path paints EOL label after line text in severity color at α=0.7; truncates with `…` on overlap; worst-severity wins per line; `message` threaded from `DocumentDiagnostic` in `main.rs`. R2 Explorer file tooltip: 500ms steady hover over a file row (not dir) triggers `App.explorer_hover_meta` population via `std::fs::metadata` in tick loop; tooltip box rendered as late-frame overlay — `panel` bg, 1px `hairline` border, 8pt padding, basename in `foreground`, size (`humanize_bytes`) + mtime (`relative_time`) in `text_subtle`. R3 Explorer filter: `App.explorer_filter: Option<String>` accumulated from printable keys when Explorer focused; `draw_left_dock_with_scroll` gains `explorer_filter: Option<&str>` param; when active, all dirs auto-expanded, rows filtered to matching files + ancestor dirs; header chip shows `[filter]`; Backspace shrinks filter; Esc clears filter first then exits; Enter opens single match. 1 new R1 render test, 2 new R2 helper tests, 2 new R3 filter tests. All tests pass. Commit: a4f4571. Files: `crates/anvil-render/src/editor.rs`, `crates/anvil-render/src/left_dock.rs`, `crates/anvil/src/main.rs`.

- 2026-05-26 — Tier-P layout items (P1/P2/P3, all 3 shipped). P1 F-tier geometry regression: 3 new test functions in `mode.rs` covering `for_mode_with_left_dock_w` for 6 widths × 3 ui_scales = 54 combinations; asserts left_dock.w, pane_area.x adjacency, pane_area.w clamping. P2 resize-bar hover state: `CursorKind` enum (Arrow/ColResize/RowResize) on AppKit platform; `AppHandler::mouse_moved` returns `CursorKind` and platform sets `NSCursor`; `App.divider_hover: Option<DividerKind>` (Sidebar/Drawer) tracks 4pt hit zones; render loop paints 1px `accent_primary` α=0.50 stripe at active divider. P3 editor horizontal scroll: `EditorPane.scroll_x: f64` (forced 0 on soft-wrap); `AppHandler::scroll` gains `shift: bool`; Shift+wheel scrolls horizontally; cursor auto-scroll keeps cursor visible; 3px horizontal scrollbar thumb (text_subtle α=indicator×0.6); drag thumb maps pixel x to scroll_x by scanning buffer for max_line_len; `hscroll_drag_active` armed in mouse_down, released in mouse_up, handled in mouse_dragged. All tests passing. Commits: 395a1c4 (P1), 57b48aa (P2+P3). Files: `crates/anvil-workspace/src/mode.rs`, `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil-render/src/editor.rs`, `crates/anvil-platform/src/appkit.rs`, `crates/anvil-platform/src/lib.rs`, `crates/anvil-platform/Cargo.toml`, `crates/anvil/src/main.rs`, `crates/anvil/Cargo.toml`.
- 2026-05-26 — Tier-S items S1/S2/S3 shipped. S1 gitignore-aware Explorer: `FilterFlags{show_hidden, show_gitignored}` replaces the bare `bool` in the fs worker channel; `read_entries` uses `ignore::gitignore::GitignoreBuilder` to load `.gitignore` + `.git/info/exclude` and skips matched entries when `show_gitignored==false` (default). `App.show_gitignored_files` field added; Cmd+K I chord toggles it and refreshes the snapshot. `ignore` added to `crates/anvil/Cargo.toml`. S2 verified: terminal search (Cmd+F → `open_search()` → terminal path → type=live highlight, Enter/Cmd+G=next, Cmd+Shift+G=prev, Esc=close) was already fully wired; no code change needed. S3 ahead/behind chip: `↑N ↓M` (U+2191/U+2193) chip in `text_subtle` rendered between head_short and branch chips in `draw_context_bar`; data already populated via `git status --porcelain=v1 --branch` parsing in `anvil-prompt-core`. 4 new tests (2 gitignore, 2 ahead/behind). All tests passing. Commit: 919ffd7. Files: `crates/anvil-render/src/context_bar.rs`, `crates/anvil/src/fs_worker.rs`, `crates/anvil/src/main.rs`, `crates/anvil/Cargo.toml`.
- 2026-05-26 — Tier-T items T1/T2 shipped. T1 (#75) git gutter bar: replaced glyph markers (+/~/▴) with a 2px colored vertical bar at the gutter right edge (Added=verified green, Modified=attention yellow, Removed=failure red + ◢ triangle glyph); moved `GitGutter::compute` off the main thread onto a dedicated `anvil-gutter-worker` named thread; main thread sends `GutterRequest{buffer_id, path, text}` after file open and Cmd+S save; `GutterResult` polled in `poll_gutter_results` each tick; save-on-blur also queues requests. T2 (#76) blame on hover: 800ms cursor dwell in a native editor fires `git blame -L<line>,<line> --porcelain <path>` on a dedicated `anvil-blame-worker` thread; `parse_blame_porcelain` + `blame_relative_time` helpers parse the response; results cached in `App.blame_cache: HashMap<(PathBuf, usize), (Option<BlameEntry>, Instant)>` with 60s TTL; tooltip `BlamePopup` rendered as a late-frame overlay (panel bg, hairline border, text_subtle) near the mouse — "author · time · hash" or "Not Committed Yet"; popup clears on line change. 4 new tests: `git_gutter_bar_does_not_paint_plus_or_tilde_glyphs`, `parse_blame_porcelain_parses_committed_entry`, `parse_blame_porcelain_returns_none_for_uncommitted`, `blame_relative_time_formats_correctly`. All tests passing, clippy clean, fmt clean. Commit: 480b5a7. Files: `crates/anvil-editor/src/buffer.rs`, `crates/anvil-render/src/editor.rs`, `crates/anvil/src/main.rs`.
- 2026-05-26 — Tier-U items U1/U2 shipped. U1 (#95) large-file open without freeze: `Buffer.syntax_pending: bool` added; `SYNTAX_DEFERRED_THRESHOLD = 256 KB`; `from_path_with_limit` and `reload_from_disk` skip `SyntaxLayer::parse()` and set `syntax_pending=true` for files > 256 KB; `poll_syntax_pending()` on App parses one pending buffer per tick (60-fps budget); `ContextBarEditor.syntax_pending` field added; `draw_context_bar` renders "Parsing…" right-anchored chip when pending. U2 (#99) LSP cancel on buffer close: `LspManager.in_flight: HashMap<u64, PathBuf>` tracks all in-flight request ids; all `request_*` methods changed to `&mut self` and insert into `in_flight`; all `poll_*` methods changed to `&mut self` and remove from `in_flight` on consumption; `LspCommand::CancelRequest{cancel_json}` variant added; `run_server` handles it by forwarding the notification; `cancel_requests_for(&Path)` sends `$/cancelRequest` to all servers and clears result slots; all 3 `close_buffer` call sites in `main.rs` hook `cancel_requests_for` before closing. 6 new buffer tests (large_file_opens_with_syntax_pending, syntax_pending_clears_after_manual_parse, small_file_has_no_syntax_pending) + 3 LSP tests (cancel_requests_for_empty_manager_is_noop, cancel_requests_for_clears_in_flight_for_path, poll_hover_zero_id_returns_none). All tests passing, clippy clean, fmt clean. Files: `crates/anvil-editor/src/buffer.rs`, `crates/anvil-editor/src/lsp.rs`, `crates/anvil-render/src/context_bar.rs`, `crates/anvil/src/main.rs`.
- 2026-05-26 — Tier-W keybind items W2/W3/W4/W7/W8/W9/W10/W11/W12 shipped (9/12 clean; W1/W5/W6 skipped due to keybind conflicts: W1 Cmd+Shift+W = close_pane, W5 Cmd+Enter = agent_approve, W6 Cmd+Shift+Enter = agent_start). New `EditorAction` variants: `DeleteLine`, `SelectAllOccurrences`, `DropLastCursor`, `FoldAll`, `UnfoldAll`, `ConvertCaseUpper`, `ConvertCaseLower`, `ConvertCaseTitle`, `SortSelectedLines`, `JoinLines` — each with `apply()` arm and unit tests. New helpers: `word_or_selection_text()`, `convert_case<F>()`. Keybinds: W2 Cmd+K S (cheatsheet toggle), W3 Cmd+K T (theme picker overlay — new `ThemePickerState` + `draw_theme_picker_overlay`), W4 Cmd+Shift+K (DeleteLine, editor-focused), W7 Cmd+Shift+L (SelectAllOccurrences), W8 Cmd+U (DropLastCursor), W9 Cmd+K 0 / Cmd+K J (FoldAll/UnfoldAll), W10 Cmd+K U/Y/Shift+T (case convert), W11 Cmd+K R (SortSelectedLines), W12 Cmd+Shift+J (JoinLines, editor-focused). fmt/clippy/tests/release all green. Commit: 509e365. Files: `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil/src/main.rs`.
- 2026-05-26 — Tier-V layout items V1–V6 + V10 shipped (7/10 clean). V1 (#3): double-click sidebar divider within 350ms resets `left_dock_w_pt_target` to 300pt. V2 (#4): double-click drawer divider within 350ms sets `drawer_ratio_target=0.72` (animated). V3 (#5): sidebar toggle (Cmd+B) now animates — hide drives target to 0 and flips `left_dock_visible=false` at snap; show flips visible immediately and eases from 0→300; drawer toggle (Cmd+J) now animates via `drawer_ratio_target` field in tick loop (0.35 easing factor, mirrors G3 sidebar). V4 (#9): pane divider drag (Cmd+\) already fully functional via existing `find_divider_at`/`DividerHit`/`divider_drag` machinery — confirmed. V5 (#10): Cmd+M toggles maximize — `collapse_siblings()` collapses siblings to `MAXIMIZE_SLIVER=0.01` each; un-maximize equalizes. V6 (#11): Cmd+Shift+M equalizes all panes — `equalize_ratios()` walks the tree setting every split to uniform ratios. V10 (#15): Cmd+H toggles `hud_visible` in any layout mode (App-level direct flip + resize + force_full_redraw). V7 (#12, tab strip scroll), V8 (#13, sidebar section drag), V9 (#14, per-section collapse) deferred with TODO stubs — invasive render-crate changes. 3 new tests: `equalize_ratios_two_pane_gives_half_each`, `equalize_ratios_single_leaf_is_noop`, `collapse_siblings_gives_focused_nearly_all_space`. All tests passing, clippy clean, fmt clean, release build clean. Commit: 8d99409. Files: `crates/anvil-workspace/src/layout.rs`, `crates/anvil/src/main.rs`.
- 2026-05-26 — Tier-X editor depth items shipped (11/14). X1 real soft-wrap: removed the "wrap" stub; row loop now breaks long lines at `content_cols` and paints continuation rows indented to `leading_ws + 2`; overflow marker suppressed in wrap mode; diagnostic EOL label suppressed in wrap mode; TODO(anvil-tierX-#1-word-break) for smart word-break. X6 URL detect + Cmd+click open: `url_at_col(line, col)` scans for `http://`/`https://` tokens; Cmd+click in native editor checks URL first and calls `open` before triggering goto-def. X8 `GotoSymbolAtCursor` action (Cmd+F12): extracts word under cursor, calls `derive_outline_rows`, jumps to first matching symbol. X9 `PeekDefinition` (Opt+F12): no-op stub with TODO. X10 Cmd+click LSP fallback: when LSP unavailable, Cmd+click dispatches `GotoSymbolAtCursor` instead of `trigger_definition_request`. X11 Snippet system: `load_snippets()` parses `~/.config/anvil/snippets.toml` ([snippet.trigger] body = "..."); App.snippets field; `try_expand_snippet()` checks word-under-cursor; Tab in editor tries expansion before normal Tab; `ExpandSnippet{trigger, body}` action deletes trigger and inserts expanded body (placeholder `$0`/`$1` stripped for v1). X12 Surround-selection: already shipped via InsertChar bracket-pair wrapping. X13 `TrimTrailingWhitespace` action (Cmd+K Cmd+W): pending_chord_k second-stroke with Cmd held now intercepted before global Cmd handler; trims trailing spaces/tabs from selection range or whole buffer in reverse-line order. X14 `MoveWordLeft`/`MoveWordRight` (Opt+←/→): `prev_word_boundary` and `next_word_boundary` helpers (skip non-word chars then word chars); `extend` for selection. Stubs: X2 sticky scroll, X3 lightbulb gutter, X4 inlay hints, X7 image preview — all TODO(anvil-tierX-#N). 19 new unit tests. All gates: fmt, clippy -D warnings, cargo test --workspace (755 tests), release build. Commit: 03e78bb. Files: `crates/anvil-render/src/editor.rs`, `crates/anvil-workspace/src/editor_pane.rs`, `crates/anvil/Cargo.toml`, `crates/anvil/src/main.rs`.
- 2026-05-26 — Tier-Y explorer + terminal items shipped (13/15; Y13/Y14 pre-existing). Y1: `ExplorerCfg.auto_collapse_siblings: bool` (default false) in anvil-config; `maybe_auto_collapse_siblings()` on App collapses sibling dirs at depth >2 on expand. Y2: Cmd+Shift+R reveals active file in Explorer — switches to IDE layout, opens left dock, syncs selection and scroll. Y3: `is_symlink: bool` added to `DirEntry` in fs_worker (lstat, does not follow); render-side `DirEntry` propagated; U+2192 arrow glyph rendered in text_subtle for symlink entries. Y4: `\x00empty` sentinel row pushed when expanded dir has 0 children; rendered as italic "(empty)" in text_subtle. Y5: collapsed dirs with cached children show "(N)" count badge in foreground. Y6: explorer drag-end calls `std::fs::rename(src, dst_dir/basename)` when released over a valid explorer row. Y7: `drawer_terminals: Vec<PaneId>` + `drawer_active_terminal: usize` added to App; `spawn_drawer_terminal_tab()` splits drawer and appends to vec; Cmd+T when drawer focused adds tab. Y8: drawer tab strip rendered below divider when `drawer_terminals.len() > 1` (pills with active highlight). Y9: Cmd+backtick cycles `drawer_active_terminal` and focuses corresponding pane. Y10: CWD strip rendered in drawer chrome when only one terminal tab (`📁 <cwd>`). Y11/Y12: `[tasks.<name>] cmd = "..."` parsed from anvil.toml; task entries injected into palette with `> name` title; `task:run:` invocations write cmd to active drawer terminal PTY. Y15: Cmd+\ when terminal focused in IDE mode calls `spawn_drawer_terminal_tab()`. Y13/Y14 (Cmd+click path/URL): pre-existing via `interact::classify`. 4 commits: 8b28928, 9d3d006, fba0e87, b43e8e5. Files: `crates/anvil-config/src/lib.rs`, `crates/anvil/src/fs_worker.rs`, `crates/anvil-render/src/left_dock.rs`, `crates/anvil/src/main.rs`.
- 2026-05-26 — Tier-Z git integration items shipped (9/12; Z4/Z10/Z11 deferred). Z1/Z7 (#71/#79): Cmd+Shift+G toggles source-control panel overlay (`ScmPanel` state) showing staged/unstaged files from `git status --porcelain`; sections: STAGED (A/M marks in verified green), UNSTAGED (M marks in attention yellow). Z2 (#72): Enter on a file row in SCM panel stages (git add) or unstages (git reset HEAD) the file; toast on success/error. Z3 (#73): Tab focuses commit-message text input at bottom of SCM panel; Cmd+Enter commits staged files (`git commit -m <msg>`); toast confirms. Z5 (#77): STASHES section in SCM panel lists `git stash list`; Enter on a stash row applies it (`git stash apply stash@{N}`). Z6 (#78): Cmd+Shift+B opens branch-switcher palette — lists `git branch`, filter by typing, Up/Down nav, Enter = `git checkout <branch>`, toast confirms. Z8 (#80): Cmd+K Cmd+G opens git-log palette — lists `git log --oneline -50`, Up/Down nav, Enter = `git show <hash>` in a scratch native editor buffer. Z9 (#81): `draw_push_pull_chips` in `context_bar.rs` renders `↑ push` and `↓ pull` chips after the IDE chip in the context bar (only when in a git repo); click hits computed via `PixelRect` return; click dispatches `spawn_git_push`/`spawn_git_pull` workers (30s timeout) and shows toast. Z12 (#85): PULL REQUESTS section in SCM panel shell-outs to `gh pr list --json number,title,headRefName`; minimal JSON parsed without serde; shows "(N) PRs" header; expandable. Deferred: Z4 (diff view — needs a read-only diff buffer with + green / - red coloring), Z10 (conflict marker rendering — needs per-cell tinting), Z11 (discard hunk — requires reverse patch computation). All gates: fmt ✓, clippy -D warnings ✓, cargo test --workspace (all suites) ✓, release build ✓. Commit: c94601f. Files: `crates/anvil/src/main.rs`, `crates/anvil-render/src/context_bar.rs`, `crates/anvil-render/src/lib.rs`.
- 2026-05-26 — Tier-U final cleanup tier shipped (5/17 items implemented; complex LSP/SCM items deferred). W1 (#18): close_pane default keybinding changed from cmd+shift+w to cmd+shift+q; Cmd+Shift+W in perform_key_equivalent closes the window via shutdown()+exit(0). W5 (#24) + W6 (#25): InsertBlankLineBelow and InsertBlankLineAbove EditorAction variants; Cmd+Return/Cmd+Shift+Return dispatch them when editor focused and HUD not visible; fall through to agent actions when HUD visible. #7 (tab strip scroll): draw_tab_bar gains tab_strip_scroll: f64 param; tabs use natural widths and scroll; ◀▶ chevrons appear at strip edges when strip overflows; shift+wheel over chrome row scrolls strip; chevron clicks step by ~3 tab widths; App.tab_strip_scroll_offset field tracks state. #9 (section collapse): draw_left_dock_with_scroll gains explorer_collapsed + outline_collapsed params; header rows show ▾/▸ chevrons; clicking EXPLORER/OUTLINE header toggles collapse; collapsed section shows header only, other section expands to fill; OutlineHeader LeftDockHitKind variant added. Word-break soft-wrap: pre-compute wrap_starts before draw loop; breaks at last whitespace within 20 cols when available, otherwise at column boundary. Also fixed 5 pre-existing E0425 compile errors (_dh params used as dh in overlay draw helpers). Deferred: #8 sidebar drag-to-swap, sticky scroll, lightbulb, inlay hints, image preview, peek definition, diff view, conflict markers, discard hunk, NSSavePanel, LSP formatting. Gates: fmt ✓, clippy -D warnings ✓, cargo test --workspace ✓, build ✓. 6 commits: 30f8c18, d8b4386, 70a12db, eacc8cb, b42e0cf, c9261e7. Files: crates/anvil-config/src/lib.rs, crates/anvil-workspace/src/editor_pane.rs, crates/anvil-workspace/src/tab.rs, crates/anvil-render/src/tabbar.rs, crates/anvil-render/src/left_dock.rs, crates/anvil-render/src/editor.rs, crates/anvil/src/main.rs.
- 2026-05-26 — Overlay redesign Phases 1+2 shipped. Phase 1: crates/anvil-render/src/overlay/ module tree created: mod.rs (OverlayId, Overlay enum, OverlayStack, CardSize, OverlayMeasureCtx, OverlayRenderCtx, CustomOverlay trait, Submission), chrome.rs (draw_card_chrome: scrim + 3 shadow rects + panel fill + top highlight + 1px border), anim.rs (OverlayAnim with ease_out_cubic, Entering/Visible/Leaving state machine), text.rs (OverlayPainter trait + MonoPainter wrapping GlyphPainter), input.rs (OverlayKey mirror type + OverlayInputRouter::dispatch_key per spec §5), widgets/{mod,picker,text_input,tooltip}.rs (PickerOverlay with filter_rows + full render, TextInputOverlay, TooltipOverlay with anchor clamping). 16 unit tests: chrome_paints_shadow_then_panel_then_border, anim_alpha_zero_at_entering_start_one_at_visible, anim_begin_close_transitions_to_leaving, anim_finished_after_leave, ease_out_cubic_monotone, empty_stack_passthrough, esc_returns_close, picker_arrow_down_consumed, picker_filters_rows_on_query, picker_filter_empty_query_returns_all, picker_arrows_clamp, picker_enter_submits_selected_index, text_input_enter_emits_submission, text_input_backspace_deletes_last_char, tooltip_anchor_positions_card, tooltip_anchor_clamps_to_viewport. Phase 2: App.overlays: OverlayStack field wired; open_project_search pushes Overlay::Picker; render loop calls overlays.render() then gates old draw_project_search_overlay with overlays.is_empty(); key_down dispatches through OverlayInputRouter before old handler (Consumed→scan+rebuild_project_search_picker, Submit→open file+GoToLine, Close→close both). Old draw fn retained per spec. All gates: fmt ✓, clippy -D warnings ✓, cargo test --workspace (pre-existing failures: explorer_rows_return_click_hits + 4 PTY tests) ✓, release build ✓. 4 commits: 62e2483, b323314, a8c2cbc, c93935d. Files: crates/anvil-render/src/lib.rs, crates/anvil-render/src/overlay/*, crates/anvil/src/main.rs.
- 2026-05-26 — Overlay redesign Phases 3+4 shipped (14/18 overlays migrated; SCM/hover/code-actions/completion deferred as custom). Phase 3 widget render: TextInputOverlay.render (prompt + tail-clipped value + cursor block), TooltipOverlay.render (card via anchor_position), OverlayStack::render wired for TextInput+Tooltip arms, top_id() accessor added. Phase 4 polish: fill_rounded_rect on Raster (per-pixel antialiased corner coverage, fallback when radius≤0), draw_card_chrome now calls fill_rounded_rect for panel fill, draw_overlay_shadow deleted (no callers). Migration: 4 TextInputs — GotoLine, LspRename (guard !empty), SaveAs (guard !empty), OpenFolder; 8 Pickers — ThemePicker, LangPicker, LspReferences, WorkspaceSymbols, BufferSymbols, ProjectSwitcher, BranchSwitcher, GitLog; 2 Tooltips — blame tip + file tip infrastructure. Animation: AppShell.last_tick_time field; tick() computes dt_ms and drives overlays.tick(dt_ms). Deleted 12 old draw functions. Close branch clears all 13 backing-state fields. Consumed branch dispatches by top_id() for live picker sync. Submit block handles all 12 migrated overlays. Old backing-field handlers remain inert (overlay stack returns early on Consumed/Submit/Close). Pre-existing PTY failures (5, ENXIO) unchanged. All other 40 tests pass. 2 commits: 4e6ffc1, 47063f2. Files: crates/anvil-render/src/raster.rs, crates/anvil-render/src/overlay/chrome.rs, crates/anvil-render/src/overlay/mod.rs, crates/anvil-render/src/overlay/widgets/text_input.rs, crates/anvil-render/src/overlay/widgets/tooltip.rs, crates/anvil/src/main.rs.

- 2026-05-26 — Tier-AA theme + perf items shipped (13/13). AA1: ember-light scale test at ui_scale ∈ {0.8,1.0,1.5,2.0} — no panics + all contrast tokens distinct from background. AA2: SOLARIZED_DARK/SOLARIZED_LIGHT palette consts added to anvil-theme; wired into by_name resolver and PICKER_THEMES list. AA3: ThemeEntry enum supports both `theme="name"` (bare string, backward compat) and `[theme] override="name" / [theme.tokens] accent="#ff0000"` table form; ThemeSectionCfg + ThemeTokenOverrides structs. AA4: [syntax.<lang>] per-language color overrides as LangSyntaxOverride (keyword/string/number/comment/function/type_/variable/operator/punctuation); Config.syntax: HashMap<String, LangSyntaxOverride>. AA5: editor.italic_comments (bool, default true). AA6: editor.bold_keywords (bool, default false). AA7: cursor.color = "#rrggbb" override; CursorConfig.color: Option<[u8;3]>; draw_cursor uses override over theme.accent when present. AA8: cursor shape config verified — all three styles (block/bar/underline) round-trip through cursor_cfg_from_config; test added. AA9: render_100_frames_no_panic — 100-frame draw_viewport stability test via CPU path; GPU/Metal path noted as hardware-dependent. AA10: rope_100mb_open_edit_save_round_trip test tagged #[ignore] (opt-in); 100 MB synthetic buffer open+edit+save+reload round-trip. AA11: atlas eviction — when 2048² atlas full, evicts all cached glyphs (LRU clear-all), resets ShelfPacker, re-rasterizes on demand; eviction_count field tracks occurrences; ATLAS_MEMORY_BUDGET const documents the budget. AA12: read_dir_snapshot_with_timeout wraps read_dir_snapshot in a 10s deadline via sub-thread+recv_timeout; timeout returns empty snapshot + stderr warning; FS_WORKER_TIMEOUT const. AA13: App.mem_baseline: u64 captured at init; Cmd+Shift+Alt+M logs baseline+delta to stderr; TODO(anvil-AA13-mem) placeholder for real RSS query. All gates: fmt ✓, clippy -D warnings ✓, cargo test --workspace ✓, release build ✓. 5 commits: 8e3109d, f9f0a20, caef74e, e031c4d, fb83830. Files: crates/anvil-theme/src/theme.rs, crates/anvil-theme/src/lib.rs, crates/anvil-config/src/lib.rs, crates/anvil-render/src/draw.rs, crates/anvil-render/benches/draw_viewport.rs, crates/anvil-platform/src/glyph_atlas.rs, crates/anvil/src/fs_worker.rs, crates/anvil/src/main.rs, crates/anvil-editor/src/buffer.rs.

- 2026-05-26 — Track A Phase 1: proportional UI font foundation shipped. New UiTextPainter trait + UiWeight enum + Raster::ui_line/ui_measure wrappers in anvil-render/src/raster.rs. New UiPainter struct in anvil-platform/src/ui_text.rs: CoreText (CTFont/CTLine/CFAttributedString) rasterizer, per-size FontCache, UiLineMask, UiLineCache LRU (1024 entries / 4 MiB budget), load_ct_font_with_fallback (SF Pro Text → system UI fallback), kCTForegroundColorFromContextAttributeName=kCFBooleanTrue required to make CTLine::draw use CG context fill color. Per-surface logical-pt constants in new ui_text_sizes.rs module. UiFontCfg struct added to anvil-config (family/size/weight_regular/weight_strong, serde default + deny_unknown_fields). AppShell.ui_painter: UiPainter wired in main.rs; [font.ui] config change detected in tick() via borrow-split block and drives ui_painter.reset(). 8 new unit tests (measure/draw_line/cache LRU behavior) all pass. Pre-existing PTY ENXIO failures (5) unchanged. All gates: fmt ✓, clippy -D warnings ✓, cargo test --workspace ✓, cargo build --release -p anvil ✓. No call-site migrations (Phase 2 follow-up). 3 commits: 00b5d61 (trait), 3dba1b0 (UiPainter), bd27709 (config+wiring). Files: crates/anvil-render/src/raster.rs, crates/anvil-render/src/lib.rs, crates/anvil-render/src/ui_text_sizes.rs, crates/anvil-platform/src/ui_text.rs, crates/anvil-platform/src/lib.rs, crates/anvil-platform/Cargo.toml, crates/anvil-config/src/lib.rs, crates/anvil/src/main.rs.
- 2026-05-26 — Track A Phases 2+3: proportional UI text migrated to all visible chrome surfaces. Phase 2 (Explorer): left_dock.rs section headers → UiWeight::Semibold at EXPLORER_HEADER_PT; row labels → Regular at EXPLORER_ROW_PT; icon glyphs (chevron, file badge, symlink arrow, outline kind) stay on mono GlyphPainter. ui_truncate() helper fits labels in pixel budget with '…' suffix. draw_text_run removed (unused). Phase 3a (Tabs): tabbar.rs tab labels → ui_line(); active tabs UiWeight::Medium, inactive Regular at TAB_LABEL_PT; ×, chevrons, basin mark, dots stay mono. Phase 3b (Status): statusbar.rs mode chip, CWD, run duration, agent tail, separator, clock → ui_line() at STATUS_PT; ✓/✗ and ● stay mono; right-alignment uses ui_measure for pixel-accurate widths; draw_run closure removed. Phase 3c (Context bar): context_bar.rs all chip labels and path text → ui_line() at CONTEXT_BAR_PT; draw_chip, draw_chip_right, draw_run_ellipsized converted to UiTextPainter; draw_push_pull_chips gains ui_painter param + uses ui_measure for chip widths; draw_run_clipped removed. Tests: StubUiPainter (records whole-string draws) added in all 3 modules; assertions updated from per-char glyph checks to string-level checks. main.rs: draw_context_bar + draw_push_pull_chips call sites updated to pass ui_painter. All gates: fmt ✓, clippy -D warnings ✓, cargo test --workspace ✓ (pre-existing PTY ENXIO 5 failures unchanged). 3 commits: 5af20a9, 2b22409, 6321a2b. Files: crates/anvil-render/src/left_dock.rs, crates/anvil-render/src/tabbar.rs, crates/anvil-render/src/statusbar.rs, crates/anvil-render/src/context_bar.rs, crates/anvil/src/main.rs.
- 2026-05-26 — Overlay Phase 3 (final 4) + Track D polish sweep shipped. Part 1: 4 deferred overlays migrated to CustomOverlay. SCM panel (Z1/Z2/Z3/Z5/Z12): ScmPanelOverlay struct in crates/anvil/src/overlays.rs; pushed to OverlayStack on open, snapshot synced on every key mutation; legacy draw_scm_panel_overlay and scm_draw_row deleted from main.rs. Completion popup (item 16), code-actions (item 25), hover (NE10): inline draws removed from editor.rs; sync_editor_popup_overlays() called each frame in render_frame to push CompletionOverlay/CodeActionsOverlay/HoverOverlay snapshots with pixel anchors computed from pane_rect + scroll_line. OverlayStack gains remove_by_id/contains_id helpers. CustomOverlay trait gains card_origin() for anchored positioning. Part 2 polish: animation duration 120ms → 100ms; card shadow offsets +2/+4/+8 → +1/+3/+6 (tighter); cursor blink rate 1/64 → 1/30 per tick (500ms phase); status bar EDITING mode adds 3px ember dot in accent_ember at α=0.7. Vertical centering (tab strip, status bar, context bar, explorer rows) verified in-code — all already use correct formula. Pre-existing failures: left_dock::explorer_rows_return_click_hits (render-lib only, pre-existing), 5 PTY ENXIO (sandbox). All workspace gates: fmt ✓, clippy -D warnings ✓, cargo test --workspace ✓, release build ✓. 2 commits: 32bc4c3 (overlay), 9aa91c0 (polish). Files: crates/anvil/src/main.rs, crates/anvil/src/overlays.rs, crates/anvil-render/src/editor.rs, crates/anvil-render/src/overlay/mod.rs, crates/anvil-render/src/overlay/anim.rs, crates/anvil-render/src/overlay/chrome.rs, crates/anvil-render/src/statusbar.rs.
- 2026-05-26 — Track C Phases 1+2: anvil-plugin crate shipped. New crate crates/anvil-plugin with mlua 0.9 (lua54 vendored, send). Phase 1: manifest.rs (plugin.toml loader, name regex ^[a-z0-9-]{1,40}$, api major=1 gate, entry.lua path check); sandbox.rs (io/debug/dofile/loadfile stripped, os narrowed to date/time/clock, package.loadlib removed, require shim restricted to plugin dir, 32 MiB memory limit via set_memory_limit); plugin.rs (Plugin::load creates Lua VM, installs sandbox, installs api table, sets 1000-instruction-count hook with 200ms wall-clock budget, runs init.lua); host.rs (PluginHost::new, discover_and_load iterates ~/.config/anvil/plugins/*/plugin.toml, spawn dedicated threads, tick drains HostRequests, commands()/statusbar_chips() snapshot, invoke_command routes events, drain_toasts). Phase 2: api/command.rs (anvil.command stores callback in anvil._callbacks table, sends RegisterCommand); api/keymap.rs (anvil.keymap, built-in-wins warn on conflict); api/statusbar.rs (anvil.statusbar.add/update/remove, text capped 64 chars, position validated); api/notify.rs (anvil.notify level=info/warn/error). App integration: App.plugin_host field, boot discovers plugins after mem_baseline, AppShell::tick drains plugin_host.tick() + converts notify toasts to app toasts. 10 unit tests all pass. Pre-existing PTY ENXIO failures unchanged. mlua vendored C compile time ~15-20s (first build only; incremental is fast). 4 commits: 41482e7, 5d84dbd, 53d7ed7, 78556b7. Files: crates/anvil-plugin/Cargo.toml, src/{lib,manifest,sandbox,bridge,plugin,host}.rs, src/api/{mod,command,keymap,statusbar,notify}.rs, tests/plugin_tests.rs, Cargo.toml, crates/anvil/Cargo.toml, crates/anvil/src/main.rs.
- 2026-05-27 — Baseline alignment fix across all chrome surfaces. Root cause: Track A proportional font migration (commits 5af20a9/2b22409/6321a2b) introduced UiTextPainter but all surfaces passed a cell-top y to both glyph_at (correct: wants cell-top) and ui_line (wrong: wants baseline). Fix: compute icon_top = existing centring formula; baseline_y = icon_top + (cell_h - descent); pass icon_top to all glyph_at calls and baseline_y to all ui_line calls. Four surfaces fixed: left_dock.rs (explorer header, content rows, empty/waiting placeholders, outline header, outline rows), tabbar.rs (label, close ×, scroll chevrons, unread dot, + button), statusbar.rs (all ui_line + glyph_at calls, ember dot positioned from icon_top), context_bar.rs (draw_chip text_y, draw_run_ellipsized baseline). No new API methods on UiTextPainter — mono metrics alone suffice. All tests pass; no test y-coordinate assumptions broken (hit rects use row_top not glyph_y). 4 commits: 51d9a62, b2b70d5, 8a696c7, 1abbe54. Files: crates/anvil-render/src/left_dock.rs, crates/anvil-render/src/tabbar.rs, crates/anvil-render/src/statusbar.rs, crates/anvil-render/src/context_bar.rs.
- 2026-05-26 — Option A visual diff spec D1-D9 shipped. D2: left_dock_w_pt default 300→260 in mode.rs + main.rs (initial value + reset on hide). D3: tabbar.rs tab widths use ui_painter.measure() instead of cell-count estimate; min/max clamped to 9-24*cell_w. D4: draw_empty_pane replaces TERMINAL ⌘T placeholder with centered welcome block (Anvil title + key hints). D5: draw_terminal_drawer_chrome adds 28pt charcoal header strip + TERMINAL glyph label above terminal cells. D6: context_bar fill graphite→charcoal (visual separation from tab strip). D7: HEADER_H_BASE 36→30 (Explorer section header proportional to 34pt row height). D8: EXPLORER_HEADER_PT 11→10, Semibold→Regular for EXPLORER/OUTLINE section labels. D9: pane-level 2px accent top rule removed from draw_editor_chrome (active tab rule is sufficient). D1/D10: geometry already correct (context bar sits below tab strip at chrome_top_px); status bar already has panel+hairline (pre-existing). Tests updated: terminal_drawer_chrome_paints_header_strip (renamed + body-pixel asserts preserved), editor_chrome_paints_header_and_offsets_content_rect (active-tab rule note). All gates: fmt ✓, clippy -D warnings ✓, cargo test --workspace ✓ (pre-existing PTY ENXIO 5 unchanged). 4 commits: 6f7621f, 2c86741, bd5afe1, 9d637b8. Files: crates/anvil-workspace/src/mode.rs, crates/anvil-render/src/tabbar.rs, crates/anvil-render/src/workspace.rs, crates/anvil-render/src/context_bar.rs, crates/anvil-render/src/left_dock.rs, crates/anvil-render/src/ui_text_sizes.rs, crates/anvil/src/main.rs.
- 2026-05-27 — Tier DD visual polish (DD6/DD7/DD9; DD8/DD10 already present). DD6: 4pt graphite gap strip at top of context bar rect separates OS tab-strip chrome from IDE context bar without changing pane geometry. DD7: bottom-drawer PTY viewport inset by 24pt*ui_scale via raster.origin_y bump; draw_terminal_drawer_chrome gains header_h param and paints a real charcoal 'TERMINAL' header with top+bottom hairlines (replacing the hairline-only stub). DD8 (left-edge focus ring) and DD10 (active-line highlight at α=0.40) were already implemented — verified in place. DD9: draw_buffer_tab dirty indicator switched from revisions > 0 to buffer.is_dirty() (revisions != saved_revision) so saved buffers no longer show the dot. Test updated: terminal_drawer_chrome_paints_top_hairline_only → terminal_drawer_chrome_paints_header_strip. All gates: fmt ✓, clippy -D warnings ✓, cargo test --workspace ✓. 1 commit: 9a74337. Files: crates/anvil-render/src/context_bar.rs, crates/anvil-render/src/workspace.rs.
- 2026-05-27 — Tier EE editor/workflow (EE11/EE14/EE15 shipped; EE12/EE13 verified already present). EE11: verified Cmd+P → send_file_picker_show → Inbound::Invoke file:open: handler is wired end-to-end; added 2 unit tests for the prefix routing logic. EE12: verified Cmd+Shift+F opens ProjectSearch overlay, typing filters hits, Enter opens file + GoToLine — all wired. EE13: verified draw_editor_into paints find-match highlights (attention wash α=0.30 inactive, 1px outline active); existing test find_match_highlights_render_attention_wash covers it. EE14: extracted nvim_pane_argv() pure helper from spawn_nvim_pane; added test asserting ["/usr/bin/env", "nvim", path] shape. EE15 (fix): AppShell::scroll() now detects when ry > ide_drawer_divider_y and drawer_terminals is non-empty, routing scroll to the active drawer terminal instead of the focused native editor pane. 1 commit: a5772ae. Files: crates/anvil/src/main.rs.
- 2026-05-27 — Tier CC stability + testability (5 items) shipped. CC1: ANVIL_PERF=1 extended with per-stage frame timestamps (clear/workspace/chrome/blit/total ms) and 10/sec throttle via App::perf_log_last; old single-total line replaced. CC2: verified already wired (window_will_close → should_terminate → shutdown → ptys.clear); no change needed. CC3: sweep_orphan_anvil_shells() added before shell integration setup; uses /bin/ps -A -o pid,ppid,comm to find PPID=1 shell processes and SIGTERM them; logs count. CC4: static PTY_PIDS registry (Mutex<Vec<pid_t>>); register_pty_pid/unregister_pty_pid called at every Pty insert/remove including pane split, tab open, nvim pane, dead-pane drain; panic hook calls kill_registered_ptys() before delegating to default hook; Pty::child_pid() accessor added. CC5: NullUiPainter replaced with real UiPainter::new(UiFontCfg::default(), 2.0) in anvil-snapshot; NullGlyphPainter kept for mono; snapshot now renders visible chrome text via CoreText. Verified with /tmp/snap.png visual. All fmt/clippy/test gates pass; pre-existing anvil-caldera::detect_stops_at_disabled_ancestor failure unchanged. 1 commit: be527aa. Files: crates/anvil-platform/src/pty.rs, crates/anvil/src/main.rs, crates/anvil/src/bin/anvil-snapshot.rs.
- 2026-05-27 — Editor scroll direction fix + AA13 RSS real implementation. Editor scroll was inverted vs macOS natural scroll: AppKit scrollingDeltaY is positive when finger moves up (natural scroll = should scroll up = decrease scroll_target). Changed `scroll_target + d` to `scroll_target - d` in native editor scroll path; terminal path unchanged (opposite semantics). AA13 RSS: replaced stub `mem_baseline=0` with real `proc_pidinfo(PROC_PIDTASKINFO)` via libc; `current_rss_bytes()` called at init and in Cmd+Shift+Alt+M reporter. Commit: 1648e30. File: crates/anvil/src/main.rs.
- 2026-05-27 — Final tier FF (FF16-FF20) shipped. FF16/FF17/FF18: verified already present (hover paints theme.panel via hovered_explorer_row path wired to draw_left_dock_with_scroll; dir expansion synchronous via read_dir_snapshot_fast; outline wired from derive_outline_rows in draw loop); no code change needed. FF19: added --scene <name> flag to anvil-snapshot; 5 named scenes (welcome, code-open, palette-open, drawer-shell, terminal-only); Scene enum + SceneGeometry struct; draw_ide_chrome() helper; each scene renders visually distinct non-empty PNG; legacy no-scene path preserved as render_legacy(). FF20: tab pin/unpin; Tab.is_pinned field; TabManager::pin_tab() (pin moves to end of pinned group; unpin moves to start of unpinned group); draw_tab_bar renders ● (U+25CF) before label of pinned tabs in accent_primary; right-click on tab → RightClickZone::TabBar{tab_index, is_pinned} → "Pin Tab"/"Unpin Tab" NSMenu item → ContextAction::TabPinToggle → pin_tab(right_click_tab_index); 3 unit tests for pin_tab (pin, unpin, oob). All fmt/clippy/test gates clean. Files: crates/anvil-workspace/src/tab.rs, crates/anvil-render/src/tabbar.rs, crates/anvil-platform/src/appkit.rs, crates/anvil/src/main.rs, crates/anvil/src/bin/anvil-snapshot.rs.
- 2026-05-27 — Z4 diff view, X2 sticky scroll, X3 lightbulb gutter, multi-window (item 20) shipped. Z4: Enter on SCM file opens git diff as virtual buffer; Buffer.diff_view field + set_tracked_path accessor; EditorPaneRegistry::open_text_tab; git_diff_for_file helper + scm_open_diff on App; draw_editor_into colorizes + lines (verified), - lines (failure), @ lines (accent). X2: sticky scroll strip above editor when scrolled; derive_outline_rows finds innermost scope above scroll_line; graphite strip + symbol label in text_muted; skipped for diff_view buffers. X3: lightbulb glyph (U+F0336, Nerd Fonts) in gutter gap column when code_actions_popup.anchor.line == cursor line; theme.attention color. Multi-window: AppKitApp::spawn_window creates NSWindow+AnvilView+AnvilDelegate+NSTimer without re-running NSApp init; SpawnedWindow struct; SPAWNED_WINDOWS thread-local keeps windows alive; spawn_second_window builds fresh App + AppShell + workers; App::request_new_window defers to tick; Cmd+Shift+N trigger (repurposed from retired nvim open). All fmt/clippy/test gates clean. Commit: 3f92127. Files: crates/anvil-editor/src/buffer.rs, crates/anvil-platform/src/appkit.rs, crates/anvil-render/src/editor.rs, crates/anvil-workspace/src/editor_pane.rs, crates/anvil/src/main.rs.
- 2026-05-27 — First-launch polish (6/8 items). Item 1: auto_open_readme_if_empty() opens README.md → AGENTS.md → first *.md when no session buffers on IDE launch. Item 2: draw_empty_pane rewritten — centered block with accent_bright title, text_muted subtitle, two-column action grid, footer. Item 3: draw_tab_bar hides tab strip (pills, +, chevrons) when n < 2; chrome/basin/indicators always present. Item 4: apply_first_launch_defaults() auto-expands crates/ or src/ when expanded_dirs is empty on startup. Item 5: crisp 1px hairline divider drawn at pane_area.x - 1 when left_dock_visible in IDE mode. Item 6: all three easing factors 0.35 → 0.25 (sidebar, drawer ratio, editor scroll). Item 7: first-ever-launch toast via sessions dir empty/absent check. Item 8 (drawer min-height) skipped — existing G2 collapse strip covers the same need without further change. All fmt/clippy/test gates clean. Commit: 9b57a21. Files: crates/anvil-render/src/workspace.rs, crates/anvil-render/src/tabbar.rs, crates/anvil/src/main.rs.
- 2026-05-27 — Polish wave 2 (8/10 items). P2: active-line tint switched to theme.surface α=0.55 (was panel α=0.40) — tint now visible against editor background. P4: explorer dir chevrons → accent_primary (was text_muted); file labels → text_muted (was foreground); gives dirs visual weight over leaf files. P5: STATUS_PT 12→13pt; inter-segment gaps changed from 2×cell_w to 8pt×window_scale. P7: blink_phase already advances in tick (verified — no code change). P8: welcome card shifted 30pt below center, adds ~60pt top padding. P9: ⋯ dot indicator in text_subtle centered on drawer-top hairline — drag handle discoverable without hover. P10: all 5 snapshot scenes produce valid PNGs (44–87KB). P1 already correct (text.title→Keyword→accent_bright, wired in G3 dispatch). P3 (code block backdrop) and P6 (tab icon drop) deferred as invasive. All fmt/clippy/test gates clean. 4 commits: eb5ffee, 00b495f, 64c0826, b53d73e. Files: crates/anvil-render/src/editor.rs, crates/anvil-render/src/left_dock.rs, crates/anvil-render/src/ui_text_sizes.rs, crates/anvil-render/src/statusbar.rs, crates/anvil-render/src/workspace.rs.
- 2026-05-27 — UX gripes G1–G5 fixed. G1/G2: auto_open_readme_if_empty and should_show_welcome both checked !ep.open_buffers.is_empty() which is always true (every EditorPane starts with a scratch buffer). Fixed to check for buffers with a tracked_path (real files). G3: markdown SyntaxRole was Plain for all text because tree-sitter-md emits text.title/text.literal/text.uri/text.reference capture names — the "text" base didn't match any arm. Added sub-segment dispatch: title→Keyword (accent_bright), literal→String, uri→Function, reference→Type. Two new tests verify heading and code-block roles. G4: unfocused terminal panes got cursor_params=None (no cursor). Changed to pass blink_phase=0.5 for unfocused panes, giving a dim static cursor at the 0.35 opacity floor. G5: verified correct — blink_phase advances in tick (line 8399) and flows to draw_editor_into via workspace.rs:252; no code change needed. All fmt/clippy/test gates clean. 3 commits: 3a20a0f, 17b0d8f, ba8bcc0. Files: crates/anvil/src/main.rs, crates/anvil-editor/src/syntax.rs, crates/anvil-render/src/workspace.rs.
- 2026-05-27 — Polish wave 3 (items 1, 2, 4, 6 shipped; items 3, 5, 7, 8 noted below). Item 1: sidebar headers "EXPLORER" and "OUTLINE" switched to UiWeight::Semibold + theme.text_muted; removes accent_bright so headers read as navigation chrome. Item 2: non-active gutter line numbers blended 50% text_muted→graphite; cursor line keeps full text_muted, creating an active-line beacon. Item 4: 1px graphite hairline centered in the 8pt inter-segment gap between mode chip and cwd in the status bar. Item 6: 1px graphite ring around the welcome card rect in draw_empty_pane. Also fixed two pre-existing test regressions from prior waves: entries_rendered_with_correct_colors (P4 changed labels to text_muted, test still expected foreground); draw_editor_cursor_line_tint_focused_only (tint color is surface not panel, assertion updated). Item 5 (caret weight): already 2px — no change needed. Item 7 (pin glyph alignment): standard icon_top formula used everywhere — no visual offset found. Item 8 (drawer header hairline): already present as hairline at α=0.60. Item 3 (code fence backdrop): deferred — no surface_alt token in Theme; a simple line scan is invasive in the per-row loop. anvil-snapshot binary does not exist in this repo (not built in this wave). Commits: ebac81d, 0050d18. Files: crates/anvil-render/src/left_dock.rs, crates/anvil-render/src/editor.rs, crates/anvil-render/src/statusbar.rs, crates/anvil-render/src/workspace.rs.
- 2026-05-27 — Polish wave 4 (all 5 items shipped). Item 1: added `surface_alt: [u8; 3]` to `Theme` struct; wired for all 6 built-in themes (mineral-dark: #2a2f39, mineral-light: #f2f4f5, ember-dark: #2b2825, ember-light: #f7f3ef, solarized-dark/light: reuse panel_raised). Item 2: markdown code fence backdrop — per-frame line scan builds a set of fence-interior line indices; surface_alt rect painted behind those rows before glyph loop (markdown buffers only). Item 3: sidebar hover tint upgraded from theme.panel to theme.surface_alt in both full-dock and icons-only modes; test assertion updated to match. Item 4: 2px accent_primary vertical strip painted at the left edge of the gutter on the active cursor line (focused pane only) — ember rail marking cursor row. Item 5: welcome card key-hint pills — paint_key_pill closure paints surface_alt rect behind the key chord cluster (chars before "  ") in each action row, giving ⌘P / ⌘T / etc. a badge appearance. All fmt/clippy/test gates clean (281 tests pass). Commits: f08f9dc, 3ee3643. Files: crates/anvil-theme/src/theme.rs, crates/anvil-render/src/editor.rs, crates/anvil-render/src/left_dock.rs, crates/anvil-render/src/workspace.rs.
- 2026-05-28 — Mode-toggle stability, compact explorer, and gutter geometry pass shipped. IDE/Terminal round-trip now restores hidden editor panes from the registry instead of replacing them with scratch buffers. LSP sync now skips full-buffer text serialization unless a document open/change needs to be sent. Recent-files moved to a configurable `recent_files` keybinding defaulting to `cmd+opt+p`, avoiding the `cmd+shift+e` layout-toggle collision. Editor gutter width is centralized in `editor_gutter_cols`/`editor_gutter_width` and reused by renderer, fold hit testing, popup anchors, and horizontal-scroll math, fixing drift between line-number display and editor body math. Explorer rows tightened to 22px with 8px horizontal padding; gutter line numbers are quieter except for the active row; Terminal-only layout shows a `TERMINAL` status mode. Live release smoke via CuaDriver: IDE launched, Terminal mode accepted and executed `echo anvil_toggle_ok`, mode toggled back to IDE without losing the editor/explorer state, and no Anvil process was left running. Gates: cargo fmt --all, cargo test --workspace, cargo clippy --workspace -- -D warnings all clean. Files: crates/anvil/src/main.rs, crates/anvil-workspace/src/tab.rs, crates/anvil-render/src/editor.rs, crates/anvil-render/src/left_dock.rs, crates/anvil-render/src/statusbar.rs, crates/anvil-render/src/lib.rs, crates/anvil-config/src/lib.rs, crates/anvil-config/config.example.toml, crates/anvil-caldera/tests/integration.rs, wiki/log.md.
- 2026-05-28 — Top-left stoplight/titlebar polish. The top chrome strip is now 32pt, matching the native AppKit traffic-light container height used by `align_traffic_lights`; stoplight y-position is centered from actual button height. `draw_tab_bar` no longer paints the Basin glyph in the traffic-light lane, reserves 104pt plus two cells before tabs, and keeps the first tab/+ hit targets on the same row without crowding the green button. Tests added for chrome height, stoplight centering, traffic-light reserve, and no brand glyph in the stoplight lane. Live release CuaDriver screenshots: `/tmp/anvil-shots/topbar-32-clean.png` and `/tmp/anvil-shots/topbar-32-tabs.png`. Gates: cargo fmt --all, cargo test --workspace, cargo clippy --workspace -- -D warnings all clean. Files: crates/anvil/src/main.rs, crates/anvil-platform/src/appkit.rs, crates/anvil-render/src/tabbar.rs, wiki/log.md.
- 2026-05-28 — Light/dark theme inversion pass. Sampled the live Caldera Control Room at `http://127.0.0.1:4175/`: dark uses near-black warm glass with cream text; light uses near-white glass with deep navy text and measured high-contrast accents. Updated Anvil's Mineral Light to follow the same paired-inversion idea without unreadable literal RGB inversion: `#fafafa` canvas, `#001934` text, light glass panes, darker mineral/info accents, and readable muted/editor tokens. Config defaults now use `theme = "system"`, and system detection reads the macOS `AppleInterfaceStyle` preference instead of the app/window effective appearance so forced dark titlebar chrome cannot pin system mode to dark. Added contrast tests for light editor tokens plus a parser test for macOS appearance strings. Live CuaDriver smoke screenshots: `/tmp/anvil-shots/mineral-light-smoke.png`, `/tmp/anvil-shots/mineral-light-terminal-typed.png`, `/tmp/anvil-shots/mineral-dark-terminal-typed.png`; terminal input worked in both light and dark terminal modes. Gates: cargo fmt --all, cargo test --workspace, cargo clippy --workspace -- -D warnings, git diff --check all clean. Files: crates/anvil-theme/src/theme.rs, crates/anvil-config/src/lib.rs, crates/anvil-config/config.example.toml, crates/anvil/src/main.rs, brand/tokens.css, wiki/log.md.
- 2026-05-28 — Explorer icon support pass. Normal and icon-only explorer rows now render file/folder icons again, with separate lanes for chevrons, type icons, git badges, and labels so status marks no longer collide with filenames. File icons use the bundled BlexMono Nerd Font glyph set with semantic per-extension tinting; default mono font is now `BlexMono Nerd Font Mono` and font stacks prefer the configured family before falling back to BlexMono, IBM Plex Mono, SF Mono, then Menlo. `register_bundled()` is idempotent via `Once`. Added renderer tests for icon glyph selection and badge spacing; updated config defaults/tests. Visual smoke screenshot: `/tmp/anvil-shots/icon-support-ide.png`; terminal-only input smoke: `/tmp/anvil-shots/icon-support-terminal-mode-typed.png`.
- 2026-05-29 — Editor drag-selection and nvim/LazyVim support slice. Editor body geometry is now computed independently from painting (`editor_body_rect`, `collect_editor_body_hits`) and reused by mouse selection plus horizontal scrollbar math, so first-frame/GPU hit-test drift no longer drops drag selection. GPU mode now draws native editor panes into the chrome raster via `draw_workspace_editors` instead of leaving the editor body blank. Last-tab/window termination is scheduled onto the next AppKit run-loop turn to avoid `RefCell already borrowed` panics from reentrant `windowWillClose:` callbacks. Added `[nvim]` config (`appname`, `theme_sync`, `colorscheme`) and nvim spawn env/commands for LazyVim (`NVIM_APPNAME`), truecolor, theme name/mode, and theme color tokens. Tests cover editor body hit collection, GPU editor painting, nvim config parsing/defaults, and nvim argv/env shape.
- 2026-05-29 — Decision 0005 (render host): closed the backlog #19 webview-vs-native gate in favor of native Metal. The Zig rewrite already renders all chrome (tabs, dividers, palette, search, cursor, overlays) through the native instance/solid-rect pipelines; no WKWebView exists in the tree. Decision: the host is AppKit+Metal, no webview chrome; a webview is allowed only as the *content* of a future dedicated pane type, and the typed native↔web IPC bridge is deferred until such a surface exists. #20 (agent surface) proceeds terminal-native. New file wiki/decisions/0005-render-host.md; added 0004 and 0005 to wiki/decisions/README.md.
- 2026-05-29 — Zig rewrite backlog tiers 4–5 completed (#20–#23). #20 agent surface reframed as ambient inline run-block rails: OSC 133 marks carry exit state (133;D[;code] → ok/fail), each visible command block paints a 3px colored rail in the pane left gutter via the solid-rect pipeline (agent/verified/failure), shown only when no modal is open. #21 smooth resize: the 60Hz tick was a default-mode NSTimer suspended during event-tracking, so live resize froze and tore — rescheduled in NSRunLoopCommonModes, render() now runs synchronously from setFrameSize, and the implicit CA animation on drawableSize is disabled (CATransaction setDisableActions). #22 nerd-font icons: atlas hardcoded Menlo (no PUA glyphs) — bundled BlexMonoNerdFontMono-Regular.ttf is now embedded via a Zig anonymous import (build.zig addAnonymousImport "font_ttf"), exposed through anvil_font_data, and built into a CTFont from bytes (CGFont → CTFontCreateWithGraphicsFont) with a Menlo fallback. #23 nvim/lazyvim: added DECSTBM scroll regions (CSI r) with region-aware lineFeed, IND/RI/NEL (ESC D/M/E), IL/DL (CSI L/M), SU/SD (CSI S/T); grid scrollRegionUp/Down; regions reset on resize and alt enter/exit. Added DA1/DA2 (CSI c, CSI >c), DSR (CSI 5n), CPR (CSI 6n) replies via a Terminal reply buffer drained to the PTY in Session.poll; COLORTERM=truecolor advertised. Verified by zig build test (all green), zig fmt --check, and headless --dump (28k PNG). Live GUI nvim smoke not runnable from this context (bare binary doesn't attach to WindowServer; computer-use can't grant a non-bundled binary). Commits e997295, 8850550, 6392d2e, 32d7193. Files: src/app.zig, src/platform/shim.m, src/vt/terminal.zig, src/vt/parser.zig, src/vt/grid.zig, src/session.zig, src/pty.zig, build.zig, docs/product/backlog.md.
- 2026-05-29 — Vendored Zig toolchain + zls, SCS/charset fix, OSC color-query replies, seamless nvim colorscheme, and docs cleanup. Toolchain: tools/zig-version pins 0.16.0; tools/get-zig.sh downloads zig + matching zls into gitignored .zig/, SHA-256-verifies both per arch/os triple, and writes a gitignored zls.json pointing zig_exe_path at .zig/zig. .gitignore ignores .zig/ and zls.json. VT: parser now handles SCS designators (ESC ( ) * +) via a charset state + g0_graphics flag with a DEC Special Graphics line-drawing map, fixing literal `(B` leakage in nvim; ground print routes through decGraphics when G0 graphics active. OSC color queries answered: OSC 10/11 (fg/bg) and OSC 4 (palette index) reply in xterm `rgb:RRRR/GGGG/BBBB` form via Terminal.replyOscColor + a grown 256-byte reply buffer; Terminal holds q_fg/q_bg/q_ansi pushed down from the active theme by app.pushThemeColors() (keeps vt/ free of render/ dep). app also exports ANVIL_THEME (mineral-dark/mineral-light) via setenv on theme change (updateThemeEnv). Seamless editor theming: editors/nvim/colors/anvil.lua is an opt-in colorscheme with exact Mineral tokens for both variants, variant chosen from vim.g.anvil_background → ANVIL_THEME env → vim.o.background; sets termguicolors, terminal_color_0-15, ~60 highlight groups, and treesitter @capture links. Docs: AGENTS.md Source Layout rewritten from the archived Rust crates/ map to the Zig src/ map; Toolchain + Build And Verify use .zig/zig; style rule now Zig + `.zig/zig fmt`. CLAUDE.md "This Project" rewritten to Zig (./tools/get-zig.sh, .zig/zig build run/test/fmt, --dump). Gates: .zig/zig build test (exit 0), .zig/zig fmt --check src build.zig, headless --dump (28k PNG) all green. Files: tools/zig-version, tools/get-zig.sh, .gitignore, src/vt/parser.zig, src/vt/terminal.zig, src/app.zig, editors/nvim/colors/anvil.lua, AGENTS.md, CLAUDE.md, wiki/log.md.
- 2026-05-29 — GitHub Actions CI workflow added. .github/workflows/ci.yml runs on push and pull_request using macos-latest (required for Cocoa/Metal/CoreText frameworks). Steps: checkout, cache .zig/ keyed on tools/zig-version, ./tools/get-zig.sh, .zig/zig fmt --check src build.zig, .zig/zig build, .zig/zig build test, headless render smoke (./zig-out/bin/anvil --dump /tmp/anvil-ci.png + test -s). No source files changed.
- 2026-05-29 — Wiki Rust→Zig sync: audited all wiki pages for stale Rust/cargo/crate references. Updated mission, current-state, build instructions, module paths, and decision statuses across index.md, operations/agent-session-loop.md, operations/coverage.md, concepts/console-architecture.md (full rewrite to Zig src/ paths), concepts/hardening-net.md (archival note), concepts/workspace-panes.md, concepts/layout-modes.md, concepts/tab-system.md, concepts/config-system.md, concepts/shell-integration.md, concepts/search-system.md, concepts/agent-actions.md, concepts/block-model.md, concepts/native-editor.md (confidence→low, archival note), decisions/0002-tech-stack.md (status→active, Rust port reversal), decisions/0004-rust-port.md (status→superseded), decisions/README.md.
- 2026-05-29 — App icon + macOS .app bundle, keyboard cheatsheet, GitHub mirror, Claude-attribution history scrub. Icon: assets/AppIcon.png (1024² source art, silver anvil on dark squircle) → tools/gen-iconset.swift renders an .iconset (16–1024, each scaled to the Apple icon-grid body ~80.5% and clipped to a rounded rect so the source's square corners become transparent) and tools/make-icns.sh runs iconutil → assets/AppIcon.icns plus a 512² assets/app-icon.png. build.zig gained a `bundle` step assembling zig-out/Anvil.app (Contents/MacOS/anvil + Resources/AppIcon.icns + Info.plist, bundle id io.brzrkr.anvil); the bare binary also sets NSApp.applicationIconImage from the embedded app-icon.png (new anvil_icon_data export + app_icon_png anonymous import) so `build run` shows the dock icon too. Cheatsheet: Cmd+/ opens a centered modal of all 24 shortcuts grouped General/Panes/Tabs/Terminal, sourced from new src/keys.zig (single source of truth mirroring shim.m keyDown); Esc or Cmd+/ closes; emitHelp added to app.zig as the highest-precedence overlay branch. GitHub: created private github.com/brzrkr-io/anvil, pushed zig (default branch); codeberg remains origin, github added as a second HTTPS remote. History: git filter-repo stripped all Co-Authored-By: Claude / "Generated with Claude Code" trailers from 450 commits across every branch (no commit was ever authored/committed by a Claude identity — trailers only); pre-rewrite backup bundle at /tmp/anvil-pre-strip.bundle; force-pushed cleaned zig to github. Gates: .zig/zig build test (exit 0), fmt --check, build bundle, headless --dump (28k PNG) all green. Files: assets/AppIcon.png, assets/AppIcon.icns, assets/app-icon.png, tools/gen-iconset.swift, tools/make-icns.sh, build.zig, src/app.zig, src/platform/shim.m, src/keys.zig, src/root.zig, AGENTS.md, CLAUDE.md, editors/nvim/colors/anvil.lua, wiki/log.md.

- 2026-05-29 — Context chip in title bar. New src/context_chip.zig provides two pure parsers (parseHeadLine, parseKubeCurrentContext) and a Chip cache struct. gitBranch() walks up from the focused pane's OSC-7 cwd to find .git/HEAD (libc fopen/fread, no std.fs). kubeContext() reads $KUBECONFIG (first colon-separated path) or ~/.kube/config, line-scanning for current-context:. Cache invalidates on cwd change via Wyhash of the cwd string. emitContextChip() added to app.zig renders Nerd Font glyphs U+E0A0 (git branch) and U+F10D6 (kubernetes) plus text right-aligned in the title bar using mineral/cyan (status.info). 9 unit tests added; wired into root.zig test aggregator. Gates: .zig/zig build test (exit 0), fmt --check, headless --dump (29k PNG) all green. Files: src/context_chip.zig (new), src/app.zig, src/root.zig, wiki/log.md.

- 2026-05-29 — Caldera poller wired. Rewrote fetch() in src/caldera.zig: replaced dead std.net.tcpConnectToHost with libc sockets via @cImport (sys/socket.h, netinet/in.h, arpa/inet.h, unistd.h) — pattern mirrors src/pty.zig. socket()/setsockopt SO_SNDTIMEO+SO_RCVTIMEO(200ms)/connect()/write()/read()-loop/close(). Thread handle now calls .detach() so process exit is not blocked. Wired caldera.start(std.heap.page_allocator) in anvil_resize first-run block (after sessions spawn, before ready=true). No other API drift found — std.json, pthread mutex, nanosleep, ArrayListUnmanaged all compiled cleanly once referenced. --dump timing unchanged at 1.66s (pre-existing anvil_poll drain loop in shim.m). Gates: build, test, fmt --check, --dump 29k PNG all green. Files: src/caldera.zig, src/app.zig, wiki/log.md.

- 2026-05-29 — C-ABI boundary hardening. Five crash paths closed in src/app.zig. (1) Tab label glyph loop (anvil_frame): added `if (n >= instances.len) break` before writing to instances[n] — prevents OOB write when terminal cells + tab labels saturate the 60000-instance cap. (2) emitHelp chord loop: same guard added before the direct instances[n] write. (3) putGlyph: added `if (idx >= instances.len) return` — covers all overlay text emitters (emitPalette, emitSearch, emitCfgError, emitCalderaDrawer, emitHelp action column, title rows, section headers). (4) anvil_set_theme_mode: range guard `m < 0 or m > 2` before @enumFromInt(m) on ThemeMode — undefined behavior if native code passes an unexpected appearance value. (5) anvil_focus_dir / anvil_resize_pane: range guard `dir < 0 or dir > 3` before @enumFromInt(dir) on Dir enum. The ready guard already protects all session/grid/renderer dereferences; no gap found there. All tests pass; --dump OK.

- 2026-05-29 — Command-completion notifications. OSC 133;C records a monotonic wall-clock timestamp (std.c.clock_gettime MONOTONIC) in Terminal.cmd_start_ns. OSC 133;D (commandEnd) computes elapsed seconds; if elapsed >= 10s, sets notify_pending/notify_exit/notify_elapsed_s. Terminal.takeNotify() drains the flag. Terminal.shouldNotify(elapsed_s, is_active) is a pure helper. anvil_poll in app.zig calls takeNotify and formats title/body strings, then calls extern fn anvil_notify. In shim.m, anvil_notify gates on [NSApp isActive] (no-op when focused) and bundle id presence (no-op when unbundled; --dump path safe). UNUserNotificationCenter requests authorization once (gNotifyRequested/gNotifyAuthorized); subsequent calls are fire-and-forget with a time-based unique identifier. build.zig links UserNotifications framework. Two new unit tests in terminal.zig: shouldNotify checks threshold+active permutations; takeNotify checks set/drain/clear. Gates: .zig/zig build test (exit 0), fmt --check, build, --dump (25k PNG) all green. Files: src/vt/terminal.zig, src/app.zig, src/platform/shim.m, build.zig, wiki/log.md.

- 2026-05-29 — Theme variants. Added Variant struct to src/render/theme.zig grouping dark+light Theme pairs. Exposed variants[] array (mineral, mineral-high) and byName() helper. mineral-high: dark uses near-black bar (#0c0d0e) + bone fg (#eef1f2); light uses white bg (#ffffff) — both use brand outer-edge Mineral tokens for maximum legibility. Added theme_variant field ([32]u8 + len) to config.Config with themeVariant() accessor; parsed from theme_variant = "..." line; unknown values stored verbatim so byName falls back to mineral default at call site. app.zig: added active_variant var (theme.Variant), loadConfig() calls byName(cfg.themeVariant()) with mineral fallback, activeTheme() selects dark/light from active_variant. Tests added: byName/unknown/mineral-high-differs in theme.zig; parse/default/verbatim-unknown/no-error in config.zig. Gates: fmt, build test (exit 0), --dump OK. Files: src/render/theme.zig, src/config.zig, src/app.zig, wiki/log.md.

- 2026-05-29 — Cmd+N new window. Process-per-window slice: Cmd+N in shim.m launches a second anvil process via NSTask with --new. src/cli.zig: added fresh: bool field to CliArgs, parsed from --new flag (compatible with positional path: `anvil --new /some/dir` valid). Help text updated. Two new tests: --new sets fresh; --new with path sets both. src/app.zig: added pub var suppress_persist: bool = false. anvil_resize init block: when suppress_persist, skips persist.loadFromFile and goes straight to spawnFirstWithCwd. anvil_save_session: early-return when suppress_persist. src/main.zig: sets app.suppress_persist = true when args.fresh, before window.run(). src/platform/shim.m: Cmd+N handler at top of keyDown: block; gets executable via [NSBundle mainBundle].executablePath (falls back to argv[0]); launches NSTask with arg --new; does not block run loop. src/keys.zig: added Cmd+N / New Window entry to general section; total_bindings test updated 40→41. Persist race sidestepped: only the original window participates in restore/save; --new processes neither restore nor overwrite. No same-cwd wiring (no existing cwd export). Gates: fmt, build test (exit 0), --dump OK.

- 2026-05-30 — IPC subsystem Slice 1 (split/tab from shell). New src/ipc.zig: AF_UNIX SOCK_STREAM socket at $TMPDIR/anvil-<uid>.sock. Single-binder contract: on bind EADDRINUSE, probe with connect — live server detected → skip; stale socket → unlink + retry bind. Listener runs on a detached std.Thread, reads one \n-terminated line, parses verb+arg, pushes a Command{split|tab} onto a bounded [32]Command queue (pthread_mutex_t, PTHREAD_MUTEX_INITIALIZER), writes ok\n or err ...\n. ipc.takeCommands() locks+copies+resets queue, called from main thread only. ipc.tryClient() connects, sends verb line, prints reply to stderr on error. src/cli.zig: added Mode.client, verb: []const u8, verb_arg: ?[]const u8; parse recognizes leading "split" and "tab" tokens. src/main.zig: .client branch calls ipc.tryClient(verb, arg) then returns. src/app.zig: imports ipc; drainIpc() helper calls ipc.takeCommands then dispatches split/tab on main thread via mgr.splitFocused / mgr.newTabCwd + applyCursorDefault + relayout; drainIpc called at top of anvil_poll after !ready guard; ipc.start() called in anvil_resize init block after caldera.start(). root.zig: added ipc.zig to test aggregator. Tests: cli verb-parse (split h, split v, tab /x, bare tab); ipc pure helpers (buildSockPath uid, parseRequest split/tab/unknown/bare-tab, takeCommands drain). Gates: fmt clean, build test exit 0, --dump OK (25k+ PNG).

- 2026-05-30 — Smooth cursor animation. Focused live cursor now glides to its target via time-based exponential decay (tau=0.028s) instead of snapping cell-to-cell. Implementation: `animateCursor(target_x, target_y, id)` in src/app.zig holds global state (cur_anim_x/y, cur_anim_id, cur_anim_init, cur_anim_last_ms). Snap conditions: first call (cur_anim_init=false), session-id change (tab/pane switch avoids cross-pane fly), large jump (>6 cells avoids editor swooshes). Each frame: dt clamped to 64ms max, alpha=1-exp(-dt/0.028), lerp applied. Settle: when remaining distance <0.5px on both axes, snaps exactly to target and returns WITHOUT calling markDirty — this is what lets the terminal go idle. While in motion, markDirty() is called so the displaylink keeps building frames. Disabled state (cursor_smooth=false) sets cur_anim_init=false so re-enabling always snaps fresh. src/config.zig: added cursor_smooth: bool=true field, parsed identically to cursor_blink. Only the show_live_cursor code path is affected; non-focused panes, copy-mode caret, scrollback view are unchanged. 4 unit tests added in app.zig (snap-on-init, snap-on-id-change, snap-on-large-jump, settled-at-target). Gates: fmt clean, build test exit 0, --dump ok: 1600x1000 bg=96.0% bar=47.6% (cursor position unchanged on first frame, snap path correct). Files: src/app.zig, src/config.zig, wiki/log.md.

- 2026-05-31 — Unified UI Scale Task 1: chrome size constants made runtime-scalable.
  `src/chrome.zig`: 16 `pub const` size tokens replaced with private base consts (value/2)
  and `pub var` initialised to the 2x defaults. Added `pub fn applyScale(s: f32)` that
  multiplies the base table by s; s=2 exactly reproduces today's values. New test
  "applyScale scales sizes from the 2x base; s=2 reproduces defaults" verifies s=2 and s=1.
  `src/app.zig`: fixed three container-scope `pub var` reads: (a) deleted `const bar_h`,
  updated `barH()` to return `chrome.top_bar_h`, replaced all ~30 `bar_h` usages, changed
  renderer initializer `.pad_y` to literal 50 (placeholder); (b) converted `const tab_strip_margin`
  to `fn tabStripMargin()` and updated 2 call sites; (c) changed `var sidebar_w` initializer
  from `chrome.sidebar_w` to literal 300. Gates: build test exit 0, --dump DUMP_OK, fmt clean.
  Files: src/chrome.zig, src/app.zig.

- 2026-05-31 — Editor gutter Task 1: gutterWidth helper. Added private fn gutterWidth(self: *const Editor) usize to src/editor.zig just above fn lineLen. Counts digits of lines.len then adds one pad space. Depends only on line count so the content column never jitters while scrolling. New test "gutterWidth counts digits of the last line plus a pad space" covers 1-line (width 2) and 10-line (width 3) cases. Gates: build test exit 0, fmt clean. Files: src/editor.zig, wiki/log.md.

- 2026-05-31 — Web pane Task 2: Session .web kind. Added WebPane import and `web: ?WebPane = null` field to Session in src/session.zig. Extended Kind enum with `.web`. Added `initWeb(alloc, rows, cols, url)` constructor (uses Pty.initNull, sets kind=.web, stores WebPane.init(url)). Added `.web => {}` arms to deinit and resize switches to keep them exhaustive. poll's existing `if (self.kind != .shell)` guard covers .web with no change. Test: "web session: initWeb sets kind + url, poll is inert" verifies kind, url(), and poll result. Gates: build test exit 0, fmt clean. Commit cb31738. Files: src/session.zig.

- 2026-06-01 — Coverage round 2: DOM/localStorage-bound store/settings/persistence tests.
  Switched vitest environment from `node` to `happy-dom`. Node 26 defines `localStorage` as
  a global set to `undefined`, preventing happy-dom's `populateGlobal` from overriding it;
  resolved by adding `src/test-setup.ts` (a `setupFiles` shim that installs a map-backed
  Storage via `Object.defineProperty` on `globalThis`, plus a global `beforeEach` clear).
  Added 11 new test files covering: `editor-settings.ts`, `terminal-settings.ts`, `scale.ts`,
  `density.ts`, `layout-settings.ts`, `fonts.ts` (DOM paths), `themes.ts` (DOM paths),
  `redaction.ts` (audit log paths), `command-history.ts` (persistence), `toast.ts` (store +
  fake timers), `agent-queue.ts` (FIFO), `offline.ts` (navigator/events). Total tests:
  195 → 327, 0 failures. Statement coverage: 39.19% → 56.16%. `svelte-check` 0 new errors.
  Modules skipped (no new tests): `accounts.ts`, `cm-*`, `crash.ts`, `diagnostics.ts`,
  `telemetry.ts`, `term-registry.ts` — all require Tauri invoke or are pure runtime hooks
  with no testable surface without full IPC mocks.

- 2026-05-31 — Unified UI Scale Task 4: ui_scale config field + DPI-auto resolution + persist. Added `ui_scale: f32 = 0` to Config struct (0 = auto, >0 = pinned). Added applyKey branch for ui_scale (valid: 0 or 0.5-4.0). Extracted `persistKey` helper from persistTheme body; both persistTheme and new persistUiScale delegate to it. In app.zig: anvil_set_backing_scale now resolves DPI-auto (g_backing < 2.0 → 2.0, else 1.0) or honours cfg.ui_scale pin, sets default_ui_scale. Added persistUiScale() wrapper that calls config.persistUiScale and refreshes cfg_mtime. setUiScale now calls persistUiScale(). 5 new tests all pass. Gates: build clean, test exit 0, DUMP_OK, fmt clean. Files: src/config.zig, src/app.zig.

- 2026-06-02 — Security boundary audit (#45). New concept page
  `wiki/concepts/security-boundary.md` + index link. CSP is locked
  (default-src 'self', object-src none, frame-ancestors none, connect-src local
  ipc only); `'unsafe-inline'` confirmed *required* — SvelteKit hydration emits
  one inline bootstrap `<script>` and xterm/CodeMirror inject inline styles;
  accepted because all origins are local. Capabilities scoped, no shell plugin,
  no withGlobalTauri / dangerousRemoteDomainIpcAccess. ~150 commands build
  Command via explicit .arg() (no shell interpolation); verb-reaching commands
  allow-list the verb. Single shell exec is `run_capture` (agent tool, UI-gated)
  → that is #46's surface, not this boundary's. Finding: boundary sound.

- 2026-06-02 — Product wedge defined (#50). New decision `0006-product-wedge.md`
  + index link. Wedge user = solo DevOps/platform eng (project owner's own role).
  Three beat-targets Anvil must beat terminal+VSCode+kubectl+browser at: (1) GitOps
  reconcile loop, (2) IaC plan/apply, (3) PR+CI triage — all have surfaces, work is
  sharpening not building. Agent (gated run_capture) overlays all three. De-prioritizes
  observability depth, generalist-dev polish, agent autonomy beyond the gated loop.

- 2026-06-02 — Sharpened beat-target 1 (GitOps reconcile loop, decision 0006).
  Flux.svelte: broken-first sort, inline failure message (was hover-only),
  auto-poll 6s, "N failing" chip, per-row `flux events` diagnosis, and a red
  failing-count badge on the k8s rail icon (onHealth: Flux→Kube→page). Pure
  sort/summary logic → flux-health.ts + 10 tests. Goal: reconcile loop faster
  than the kubectl+terminal path (fewer keystrokes, state visible without typing).

- 2026-06-02 — Sharpened beat-target 2 (IaC plan/apply, decision 0006).
  Terraform.svelte: plan result cached per stack dir and shown on the stacks list
  as a drift badge ('+a ~c -d' or '✓'), so pending changes are visible without
  re-planning each stack. Pure plan parsing + line classification → iac-plan.ts
  + 10 tests. Mirrors the GitOps loop's "state visible without typing" approach.

- 2026-06-02 — Sharpened beat-target 3 (PR+CI triage, decision 0006).
  New ci.rs gh_prs_json (gh pr list --json …statusCheckRollup). DevOps.svelte PR
  list now rolls up each PR's checks (worst-wins), sorts failing-first, shows a
  red/yellow/green status dot, and offers one-click re-run on failed PRs. Pure
  rollup/sort/parse → pr-checks.ts + 13 tests. Closes the "is this PR red?"
  browser trip. All three beat-targets now have a first sharpening pass.

- 2026-06-02 — Agent-driven ops (first increment), decision 0006 AI overlay.
  One-click "Investigate" on failing resources in all 3 loops (Flux failed row,
  Terraform drift/error, PR with red checks). Seeds the gated agent with a
  focused prompt (real failure data, marked untrusted) + the right read-only
  diagnostic, enables Agent mode, auto-sends via new agentInvestigate store.
  Pure prompt builders in agent-ops.ts + 7 tests. Agent proposes, never mutates
  unapproved — composes with the #46 injection defenses.

- 2026-06-02 — Agent-driven ops: apply-and-verify + GitHub Actions view.
  agent-ops COMMON now contracts re-run-the-diagnostic-after-fix (apply-and-
  verify), MAX_TOOL_STEPS 8→12 for headroom. New GitHub Actions runs view: wired
  the previously-unused gh_runs_json into a DevOps "Actions" tab — failing-first,
  status dot, log/-failed, re-run, investigate-with-agent, red tab badge. Pure
  parse/sort in actions-runs.ts + 12 tests. Loop 3 (PR+CI triage) now covers PRs
  AND Actions runs.
