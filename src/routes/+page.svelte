<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import Terminal from "$lib/Terminal.svelte";
  import Problems from "$lib/Problems.svelte";
  const SourceControl = () => import("$lib/SourceControl.svelte");
  import FileBrowser from "$lib/FileBrowser.svelte";
  // Editor + DiffView pull in Monaco (~4 MB); load them lazily on first use
  // so app startup stays fast (#90).
  const Editor = () => import("$lib/Editor.svelte");
  const FileView = () => import("$lib/FileView.svelte");
  const MarkdownPreview = () => import("$lib/MarkdownPreview.svelte");
  const RunbookView = () => import("$lib/RunbookView.svelte");
  let mdPreview = $state(false);
  let runbook = $state(false);
  const isMarkdown = (p: string) => /\.(md|markdown)$/i.test(p);
  const NONTEXT = new Set(["png","jpg","jpeg","gif","webp","bmp","ico","svg","avif","pdf","zip","tar","gz","mp4","mov","mp3","wav","woff","woff2","ttf","otf","exe","bin","wasm","so","dylib","o","a"]);
  const isNonText = (p: string) => NONTEXT.has(p.split(".").pop()?.toLowerCase() ?? "");
  const DiffView = () => import("$lib/DiffView.svelte");
  const SearchPanel = () => import("$lib/SearchPanel.svelte");
  import CommitDetail from "$lib/CommitDetail.svelte";
  const AgentPanel = () => import("$lib/AgentPanel.svelte");
  import { readTerminal, broadcastInput } from "$lib/term-registry";
  import { telemetryEnabled, toggleTelemetry, getEvents, clearEvents, logEvent } from "$lib/telemetry";
  import { online } from "$lib/offline";
  import { terminalOpenPath } from "$lib/terminal-open";
  import { lastExit } from "$lib/command-blocks";

  // #72 PTY flood bench — write a large-output command and time how long the
  // terminal takes to drain it, using the OSC 133 exit mark (#12) as the signal.
  let floodT0 = 0;
  let floodArmed = false;
  function ptyFloodBench() {
    if (!activeTerm) { toast("Open a terminal first", "info"); return; }
    floodArmed = true;
    floodT0 = performance.now();
    toast("Flooding terminal (seq 1 1000000)…", "info");
    invoke("pty_write", { id: activeTerm, data: "seq 1 1000000\r" }).catch((e) => console.warn("ptyFloodBench pty_write failed", e));
    rail = "term";
  }
  import { getHistory, clearHistory } from "$lib/command-history";
  import { agentQueue, enqueueAgent, dequeueAgent, removeQueued, clearQueue } from "$lib/agent-queue";
  import { llmCreds } from "$lib/accounts";

  // #42 Agent-written PR body from the branch's commits + diff → clipboard.
  async function generatePrBody() {
    let commits = "";
    try { commits = await invoke<string>("git_log_range", { cwd, range: "origin/main..HEAD" }); } catch (e) { console.warn("git_log_range failed", e); }
    if (!commits.trim()) { toast("No commits ahead of origin/main", "info"); return; }
    let diff = "";
    try { diff = await invoke<string>("git_diff", { cwd, path: ".", staged: false }); } catch (e) { console.warn("git_diff failed", e); }
    toast("Drafting PR body…", "info");
    try {
      const { base, apiKey } = await llmCreds();
      const models = await invoke<string[]>("llm_models", { base, apiKey }).catch(() => [] as string[]);
      const util = (typeof localStorage !== "undefined" && localStorage.getItem("anvil-util-model")) || "";
      const model = (util && models.includes(util) ? util : models[0]) ?? "";
      const prompt = `Write a concise GitHub pull-request description in Markdown for these commits. Include a one-line summary, a "## What changed" bullet list, and a "## Why" sentence. Output ONLY the markdown.\n\nCommits:\n${commits}\n\nDiff (truncated):\n${diff.slice(0, 10000)}`;
      const reply = await invoke<string>("llm_chat", { model, messages: [{ role: "user", content: prompt }], base, apiKey });
      const body = reply.replace(/^```[\w]*\n?|```$/g, "").trim();
      if (!body) { toast("Empty response", "error"); return; }
      await navigator.clipboard.writeText(body);
      toast("PR body copied to clipboard", "success");
    } catch (e) { toast(String(e).slice(0, 80) || "PR body generation failed", "error"); }
  }
  import { installCrashHandlers, getCrashes, clearCrashes, diagnosticsReport } from "$lib/crash";
  import { redactionRules, addRedactionRule, removeRedactionRule, getAuditLog, clearAuditLog } from "$lib/redaction";
  import { getUserSnippets, addUserSnippet, removeUserSnippet } from "$lib/cm-snippets";
  // Settings is a large surface and never the startup view — load on demand (#74).
  const Settings = () => import("$lib/Settings.svelte");
  // DevOps (kubectl/CI) and Caldera panes aren't the startup view; load them
  // lazily so initial render stays cheap (#92).
  const DevOps = () => import("$lib/DevOps.svelte");
  const Kube = () => import("$lib/Kube.svelte");
  const CI = () => import("$lib/CI.svelte");
  const Terraform = () => import("$lib/Terraform.svelte");
  const Helm = () => import("$lib/Helm.svelte");
  const Observability = () => import("$lib/Observability.svelte");

  // Keep-alive for the DevOps rail views: mount each once visited, then toggle
  // with display instead of unmount/remount. Re-mounting re-ran kubectl/glab/etc
  // on every page switch — that round-trip was the switching lag.
  const KEEPALIVE_RAILS = ["k8s", "ci", "terraform", "helm", "obs", "devops", "scm", "search"];
  let mountedRails = $state<Record<string, boolean>>({});
  $effect(() => { if (KEEPALIVE_RAILS.includes(rail) && !mountedRails[rail]) mountedRails = { ...mountedRails, [rail]: true }; });
  function sendToTerm(cmd: string) {
    invoke("pty_write", { id: activeTerm, data: cmd + "\n" });
    rail = "term";
    toast("Sent to terminal", "info");
  }

  // Prewarm the language server when a project folder opens, so the server
  // (gopls especially) indexes in the background instead of cold-starting on the
  // first file open. Detect the project type by its root marker file.
  const LSP_MARKERS: Record<string, string> = {
    "go.mod": "go", "Cargo.toml": "rust", "tsconfig.json": "typescript",
    "package.json": "typescript", "pyproject.toml": "python", "setup.py": "python",
    "requirements.txt": "python", "CMakeLists.txt": "cpp", "compile_commands.json": "cpp",
  };
  let prewarmedCwd = "";
  async function prewarmLsp(dir: string) {
    if (!dir || dir === prewarmedCwd) return;
    prewarmedCwd = dir;
    let entries: { name: string }[] = [];
    try { entries = await invoke("list_dir", { path: dir }); } catch { return; }
    const names = new Set(entries.map((e) => e.name));
    const langs = new Set<string>();
    for (const [marker, lang] of Object.entries(LSP_MARKERS)) if (names.has(marker)) langs.add(lang);
    for (const lang of langs) ensureLsp(lang, dir).catch(() => {});
  }
  $effect(() => { if (cwd) prewarmLsp(cwd); });
  import PaneGrid from "$lib/PaneGrid.svelte";
  import { leaf, splitLeaf, closeLeaf, resizeSplit, dockLeaf, setView as setLeafView, paneId, remapTermRefs, firstLeaf, findLeaf, balanceTree, closeOthers as closeOtherPanes, leafIds, addTab, setActiveTab, closeTab, type PaneNode, type Leaf, type ViewKind, type Edge } from "$lib/panes";
  import Palette, { type Item } from "$lib/Palette.svelte";
  import Toasts from "$lib/Toasts.svelte";
  import Dialog from "$lib/Dialog.svelte";
  import Welcome from "$lib/Welcome.svelte";
  import WhatsNew from "$lib/WhatsNew.svelte";
  import Keymap from "$lib/Keymap.svelte";
  import MergeView3 from "$lib/MergeView3.svelte";
  import RebasePlan from "$lib/RebasePlan.svelte";
  import { toast } from "$lib/toast";
  import { get } from "svelte/store";
  import { activeTheme, initTheme, cycleTheme, applyTheme, themeLabel } from "$lib/themes";
  import { density, initDensity, toggleDensity, applyDensity, type Density } from "$lib/density";
  import { initScale, bumpScale, resetScale } from "$lib/scale";
  import { initOpacity } from "$lib/window-opacity";
  import { initFonts } from "$lib/fonts";
  import { autoHideRail, focusDimming, toggleFocusDimming, terminalAutoCd, toggleTerminalAutoCd } from "$lib/layout-settings";
  import Icon from "$lib/Icon.svelte";
  import { bumpEditorFontSize, setEditorFontSize, editorGoto, editorFormatOnSave, editorTabSize, editorWordWrap, editorGhostText, toggleGhostText, editorGhostSource, setGhostSource } from "$lib/editor-settings";
  import { lspLang, ensureLsp } from "$lib/lsp";
  import { fetchSymbols, searchWorkspaceSymbols } from "$lib/cm-lsp";
  import { problems } from "$lib/diagnostics";
  import { railEnabled, extEnabled } from "$lib/extensions";
  import { agentSeed } from "$lib/agent-seed";
  import { keyOverrides, comboOf, KEY_PRESETS, applyKeymapPreset } from "$lib/keymap";

  // Per-workspace theme/density overrides (#84), keyed by folder path.
  let wsSettings = $state<Record<string, { theme?: string; density?: Density }>>({});
  function pinWorkspace() {
    wsSettings = { ...wsSettings, [cwd]: { theme: get(activeTheme), density: get(density) } };
    toast(`Pinned theme + density to ${cwd.split("/").pop()}`, "success");
  }

  function importThemeJson() {
    const s = prompt('Paste theme JSON (token→hex), e.g. {"bg":"#101418","accent":"#ff6a3d"}');
    if (!s) return;
    try {
      const obj = JSON.parse(s) as Record<string, string>;
      for (const [k, v] of Object.entries(obj)) document.documentElement.style.setProperty(`--${k}`, v);
      localStorage.setItem("anvil-custom-theme", s);
      toast("Custom theme applied", "success");
    } catch { toast("Invalid theme JSON", "error"); }
  }
  function importVSCodeTheme() {
    const s = prompt("Paste a VS Code color theme JSON (the file with a \"colors\" map):");
    if (!s) return;
    let vc: { colors?: Record<string, string> };
    try {
      const cleaned = s.replace(/\/\/.*$/gm, "").replace(/,\s*([}\]])/g, "$1");
      vc = JSON.parse(cleaned);
    } catch { toast("Invalid theme JSON", "error"); return; }
    const c = vc.colors ?? {};
    const pick = (...keys: string[]) => { for (const k of keys) if (c[k]) return c[k]; return undefined; };
    const map: Record<string, string | undefined> = {
      bg: pick("editor.background"),
      panel: pick("sideBar.background", "editorGroupHeader.tabsBackground", "panel.background"),
      panel2: pick("input.background", "dropdown.background", "panel.background"),
      border: pick("panel.border", "editorGroupHeader.tabsBorder", "contrastBorder", "focusBorder"),
      text: pick("editor.foreground", "foreground"),
      text2: pick("tab.inactiveForeground", "descriptionForeground", "foreground"),
      text3: pick("disabledForeground", "editorLineNumber.foreground"),
      sel: pick("editor.selectionBackground", "list.activeSelectionBackground"),
      accent: pick("focusBorder", "button.background", "activityBarBadge.background", "textLink.activeForeground"),
      accent2: pick("textLink.foreground", "terminal.ansiBrightRed"),
      green: pick("terminal.ansiGreen", "terminal.ansiBrightGreen"),
      red: pick("terminal.ansiRed", "errorForeground"),
      blue: pick("terminal.ansiBlue", "terminal.ansiBrightBlue"),
      purple: pick("terminal.ansiMagenta", "terminal.ansiBrightMagenta"),
      teal: pick("terminal.ansiCyan", "terminal.ansiBrightCyan"),
      yellow: pick("terminal.ansiYellow", "terminal.ansiBrightYellow"),
    };
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(map)) if (v) out[k] = v;
    if (!out.bg && !out.text) { toast("No recognizable colors in that theme", "error"); return; }
    for (const [k, v] of Object.entries(out)) document.documentElement.style.setProperty(`--${k}`, v);
    localStorage.setItem("anvil-custom-theme", JSON.stringify(out));
    toast(`Imported VS Code theme (${Object.keys(out).length} colors)`, "success");
  }
  async function importVSCode() {
    const home = await invoke<string>("home_dir");
    const p = `${home}/Library/Application Support/Code/User/settings.json`;
    let raw = "";
    try { raw = await invoke<string>("read_file", { path: p }); } catch { toast("VS Code settings not found", "error"); return; }
    try {
      const cleaned = raw.replace(/\/\/.*$/gm, "").replace(/,\s*([}\]])/g, "$1");
      const obj = JSON.parse(cleaned) as Record<string, unknown>;
      const ct = String(obj["workbench.colorTheme"] ?? "");
      if (/light/i.test(ct)) applyTheme("solarized-light");
      else if (/dark/i.test(ct)) applyTheme("solarized-dark");
      const fs = Number(obj["editor.fontSize"]);
      if (fs >= 8 && fs <= 32) setEditorFontSize(fs);
      toast("Imported VS Code settings", "success");
    } catch { toast("Could not parse VS Code settings", "error"); }
  }

  // On-disk settings config file (#87): export/import every anvil-* preference.
  async function exportSettings() {
    try {
      const home = await invoke<string>("home_dir");
      const data: Record<string, string> = {};
      for (let i = 0; i < localStorage.length; i++) {
        const k = localStorage.key(i);
        if (k?.startsWith("anvil-")) data[k] = localStorage.getItem(k) ?? "";
      }
      const path = `${home}/.config/anvil/settings.json`;
      await invoke("create_path", { path, isDir: false }).catch(() => {});
      await invoke("write_file", { path, contents: JSON.stringify(data, null, 2) });
      toast(`Exported settings → ${path}`, "success");
    } catch (e) { toast(`Export failed: ${String(e).slice(0, 60)}`, "error"); }
  }
  async function importSettings() {
    try {
      const home = await invoke<string>("home_dir");
      const raw = await invoke<string>("read_file", { path: `${home}/.config/anvil/settings.json` });
      const data = JSON.parse(raw) as Record<string, string>;
      for (const [k, v] of Object.entries(data)) if (k.startsWith("anvil-")) localStorage.setItem(k, v);
      toast("Imported settings — reloading…", "success");
      setTimeout(() => location.reload(), 600);
    } catch { toast("No valid ~/.config/anvil/settings.json", "error"); }
  }

  let paletteOpen = $state(false);
  let paletteItems = $state<Item[]>([]);
  let palettePlaceholder = $state("");
  let diffTarget = $state<{ path?: string; staged?: boolean; rev?: string } | null>(null);

  let rail = $state("term");
  let cwd = $state("");

  // Detach-pane → new OS window (#17). A detached window carries a `?detach=`
  // seed (URL-encoded JSON: { view, ref?, file?, cwd? }); it seeds a single pane
  // from it and never restores/persists shared state (so it can't clobber the
  // main window's layout). Live PTY transfer is out of scope — a detached
  // terminal starts fresh in the same cwd.
  const detachSeed: { view?: string; file?: string; cwd?: string } | null = (() => {
    if (typeof location === "undefined") return null;
    const raw = new URLSearchParams(location.search).get("detach");
    if (!raw) return null;
    try { return JSON.parse(decodeURIComponent(raw)); } catch { return null; }
  })();
  const isDetached = !!detachSeed;

  let settingsOpen = $state(false);
  let zen = $state(false);
  let zenPrevRail = "term";
  // Zen is always a distraction-free terminal: entering forces the terminal view
  // (creating one if none exists) regardless of the current page; exiting restores
  // wherever you were.
  function toggleZen() {
    if (zen) {
      zen = false;
      rail = zenPrevRail;
    } else {
      zenPrevRail = rail;
      rail = "term";
      if (!activeTerm) newTerm();
      zen = true;
    }
  }
  function openSettings() { settingsOpen = true; rail = "settings"; }
  // Explorer is a persistent left panel, independent of the main view (#74): it
  // stays open while you're in the editor/terminal instead of being a `rail`
  // mode that other views replace.
  let explorerOpen = $state(false);
  function toggleSide() { explorerOpen = !explorerOpen; }

  // Activity rail is hideable on demand (⌘⇧B), persisted.
  let railHidden = $state(false);
  function toggleRail() { railHidden = !railHidden; }

  // First-run onboarding (#100): shown until dismissed. Read synchronously so it
  // doesn't depend on onMount (which can bail early on a failed invoke).
  let onboarded = $state(typeof localStorage !== "undefined" ? localStorage.getItem("anvil-onboarded") === "1" : true);
  function dismissOnboard() { onboarded = true; try { localStorage.setItem("anvil-onboarded", "1"); } catch { /* ignore */ } }
  // #74 Startup-timing harness — measure shell mount + first paint.
  const bootStart = typeof performance !== "undefined" ? performance.now() : 0;
  let bootMs = $state(0);
  let firstPaintMs = $state(0);
  function startupReport() {
    let nav = "";
    try {
      const e = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined;
      if (e) nav = ` · DCL ${Math.round(e.domContentLoadedEventEnd)}ms · load ${Math.round(e.loadEventEnd)}ms`;
    } catch { /* ignore */ }
    toast(`Startup: shell ${bootMs}ms · first paint ${firstPaintMs}ms${nav}`, "info");
  }

  // #77 Frame-budget profiler overlay (dev). A rAF loop samples FPS + frame ms.
  let fpsOn = $state(false);
  let fps = $state(0);
  let frameMs = $state(0);
  function toggleFps() {
    fpsOn = !fpsOn;
    if (fpsOn) {
      let last = performance.now();
      let frames = 0;
      let acc = 0;
      const tick = (now: number) => {
        if (!fpsOn) return;
        const dt = now - last; last = now; frames += 1; acc += dt;
        if (acc >= 500) { fps = Math.round((frames * 1000) / acc); frameMs = Math.round((acc / frames) * 10) / 10; frames = 0; acc = 0; }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    }
  }

  // #96 Onboarding tour v2 — a short stepped intro instead of one wall of tips.
  let obStep = $state(0);
  const TOUR = [
    { title: "Welcome to Anvil", body: "The AI-native console for 100% of your work — terminal, editor, git, and DevOps in one native surface.", tips: ["Press <kbd>⌘K</kbd> any time — every action lives in the command palette."] },
    { title: "Code & terminal together", body: "Open files alongside live shells.", tips: ["<kbd>⌘O</kbd> open file · <kbd>⌘⇧O</kbd> open folder", "<kbd>⌘T</kbd> new terminal · <kbd>⌘J</kbd> terminal under your editor", "<kbd>⌘\\</kbd> split workspace panes · drag a tab onto a pane edge"] },
    { title: "Git & DevOps built in", body: "Source Control has a Terax-style commit panel, swimlane history, and per-hunk staging. The DevOps tab fronts k8s, CI, Helm, Terraform, and observability.", tips: ["The <kbd>gen</kbd> button writes your commit message from the staged diff."] },
    { title: "Your AI agent", body: "The agent reads files, terminal output, and the repo map — all redacted, all local-first.", tips: ["<kbd>⌘I</kbd> ask the agent · <kbd>⌘,</kbd> Settings for themes, fonts, keymap."] },
  ];
  function tourNext() { if (obStep < TOUR.length - 1) obStep += 1; else dismissOnboard(); }
  function tourBack() { if (obStep > 0) obStep -= 1; }

  // #97 What's-new: auto-show once per release, reopenable from the palette.
  const WHATS_NEW_VERSION = "0.1.0";
  const WHATS_NEW_NOTES = [
    { title: "Editor", items: [
      "Migrated to CodeMirror 6 — faster, cleaner, palette-matched syntax colors.",
      "Find/replace panel (⌘F) with regex, whole-word, and in-selection toggles.",
      "Format-on-save, inline blame heatmap (⌥B), bookmarks (⌘⌥K), color swatches, minimap.",
    ] },
    { title: "Git", items: [
      "Terax-style commit panel with co-author picker and click-through diffs.",
      "Word-level inline diffs and per-hunk staging.",
    ] },
    { title: "DevOps", items: [
      "k8s context/namespace switcher, log multiplex, exec, port-forward.",
      "GitLab CI, GitHub Actions logs, Helm, Terraform apply, Loki, AWS panes.",
    ] },
  ];
  let whatsNew = $state(false);
  let keymapOpen = $state(false);
  let mergeView = $state<string | null>(null); // #25 path under 3-pane merge
  let rebaseTarget = $state<string | null>(null); // #21 interactive rebase editor
  let quakeOpen = $state(false); // #18 drop-down terminal overlay
  function showWhatsNew() { whatsNew = true; }
  function dismissWhatsNew() {
    whatsNew = false;
    try { localStorage.setItem("anvil-seen-version", WHATS_NEW_VERSION); } catch { /* ignore */ }
  }
  $effect(() => {
    if (!onboarded) return;
    let seen = "";
    try { seen = localStorage.getItem("anvil-seen-version") || ""; } catch { /* ignore */ }
    if (seen !== WHATS_NEW_VERSION) whatsNew = true;
  });

  // Bottom terminal dock (roadmap §A #6) — a terminal panel below the active
  // view, so you can run commands under an open file (the IDE "terminal below
  // file" ask). Toggle with ⌘J; drag the top edge to resize.
  let bottomDock = $state(false);
  let dockH = $state(280);
  let dockTab = $state<"term" | "problems">("term");
  function startDockResize(e: PointerEvent) {
    e.preventDefault();
    const startY = e.clientY, startH = dockH;
    const move = (ev: PointerEvent) => {
      dockH = Math.max(120, Math.min(window.innerHeight - 160, startH - (ev.clientY - startY)));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  // Native menu bar (File/Edit/View/Window) emits a `menu` event with an action
  // string; route each to the existing handler so the menu and keybinds agree.
  function onMenu(action: string) {
    switch (action) {
      case "new-term": newTerm(); break;
      case "new-window": invoke("new_window").catch((e) => toast("Could not open new window: " + String(e).slice(0, 60), "error")); break;
      case "open-file": openFileDialog(); break;
      case "open-folder": openFolder(); break;
      case "close-tab":
        if (rail === "editor" && activeFile) closeFile(activeFile);
        else if (rail === "term") closeTerm(activeTerm);
        else if (rail === "workspace") wsClose(activeLeaf);
        else if (rail === "settings") { settingsOpen = false; rail = "term"; }
        break;
      case "settings": openSettings(); break;
      case "palette": openCommands(); break;
      case "goto-file": openFilesPalette(); break;
      case "toggle-sidebar": toggleSide(); break;
      case "zen": toggleZen(); break;
      case "zoom-in": bumpScale(1); break;
      case "zoom-out": bumpScale(-1); break;
      case "zoom-reset": resetScale(); break;
    }
  }

  // ── Dockable workspace (multipane) ──
  let paneTree = $state<PaneNode>(leaf("term", paneId("wt")));
  let paneDrag = $state<{ id: string | null }>({ id: null });
  // A top-strip tab being dragged into a workspace pane quadrant (#4/#5).
  let tabDragView = $state<{ view: ViewKind; ref?: string } | null>(null);
  function wsDropTab(targetId: string, edge: Edge) {
    if (!tabDragView) return;
    if (edge === "center") {
      paneTree = addTab(paneTree, targetId, tabDragView.view, paneRef(tabDragView.view, tabDragView.ref));
    } else {
      wsSplit(targetId, edge, tabDragView.view, tabDragView.ref);
    }
    tabDragView = null;
  }

  // Named workspace layout presets (#12), persisted in localStorage.
  let layoutPresets = $state<Record<string, PaneNode>>({});
  function saveLayoutAs() {
    const name = prompt("Save current workspace layout as:");
    if (!name) return;
    layoutPresets = { ...layoutPresets, [name]: paneTree };
    try { localStorage.setItem("anvil-layouts", JSON.stringify(layoutPresets)); } catch { /* ignore */ }
    toast(`Saved layout “${name}”`, "success");
  }
  // #66 Shareable layouts: round-trip the current pane tree through the clipboard.
  async function exportLayout() {
    try { await navigator.clipboard.writeText(JSON.stringify(paneTree)); toast("Layout copied to clipboard", "success"); }
    catch { toast("Could not copy layout", "error"); }
  }
  async function importLayout() {
    let txt = "";
    try { txt = await navigator.clipboard.readText(); } catch { /* ignore */ }
    if (!txt) { txt = prompt("Paste a workspace layout JSON:") || ""; }
    if (!txt.trim()) return;
    try { paneTree = remapTermRefs(JSON.parse(txt)); rail = "workspace"; toast("Layout imported", "success"); }
    catch { toast("Invalid layout JSON", "error"); }
  }
  function openLayoutPalette() {
    const names = Object.keys(layoutPresets);
    if (!names.length) { toast("No saved layouts yet", "info"); return; }
    palettePlaceholder = "Load workspace layout…";
    paletteItems = names.map((n) => ({
      label: n,
      run: () => { try { paneTree = remapTermRefs(layoutPresets[n]); rail = "workspace"; } catch { /* ignore */ } },
    }));
    paletteOpen = true;
  }
  let activeLeaf = $state("");
  let zoomedLeaf = $state<string | null>(null);
  // Keep activeLeaf valid as the tree changes.
  $effect(() => { if (!findLeaf(paneTree, activeLeaf)) activeLeaf = firstLeaf(paneTree).id; });
  // Drop the zoom if its pane is gone.
  $effect(() => { if (zoomedLeaf && !findLeaf(paneTree, zoomedLeaf)) zoomedLeaf = null; });
  function paneRef(v: ViewKind, srcRef?: string): string | undefined {
    if (v === "term") return paneId("wt");
    if (v === "editor") return srcRef ?? (activeFile || undefined); // carry the file
    return undefined;
  }
  function wsSplit(id: string, edge: Edge, v: ViewKind, srcRef?: string) {
    paneTree = splitLeaf(paneTree, id, edge, v, paneRef(v, srcRef)).tree;
  }
  function wsClose(id: string) { paneTree = closeLeaf(paneTree, id); }
  function wsSetView(id: string, v: ViewKind) {
    paneTree = setLeafView(paneTree, id, v, paneRef(v));
  }
  function wsResize(sid: string, i: number, d: number) { paneTree = resizeSplit(paneTree, sid, i, d); }
  function wsDock(dragId: string, targetId: string, edge: Edge) {
    // Center drop moves the dragged pane's active view in as a new TAB (#2);
    // edge drops still split.
    if (edge === "center") {
      const d = findLeaf(paneTree, dragId);
      if (d) paneTree = closeLeaf(addTab(paneTree, targetId, d.view, d.ref), dragId);
    } else {
      paneTree = dockLeaf(paneTree, dragId, targetId, edge);
    }
  }
  function wsSetActiveTab(id: string, i: number) { paneTree = setActiveTab(paneTree, id, i); }
  function wsCloseTab(id: string, i: number) { paneTree = closeTab(paneTree, id, i); }
  function wsAddTab(id: string) { paneTree = addTab(paneTree, id, "term", paneId("wt")); }

  // Editor: multiple open files, one active, dirty tracked per path.
  let openFiles = $state<string[]>([]);
  let activeFile = $state("");
  let dirtyFiles = $state<Record<string, boolean>>({});
  // #64 Pinned tabs: pinned paths sort first + survive "close others"; persisted.
  let pinnedFiles = $state<string[]>(typeof localStorage !== "undefined" ? (() => { try { return JSON.parse(localStorage.getItem("anvil-pinned-tabs") || "[]"); } catch { return []; } })() : []);
  function persistPins() { if (typeof localStorage !== "undefined") localStorage.setItem("anvil-pinned-tabs", JSON.stringify(pinnedFiles)); }
  function togglePin(f: string) { pinnedFiles = pinnedFiles.includes(f) ? pinnedFiles.filter((x) => x !== f) : [...pinnedFiles, f]; persistPins(); tabMenu = null; }
  // #64 Named tab groups: assign tabs a group; grouped tabs cluster + get a color.
  let tabGroups = $state<Record<string, string>>(typeof localStorage !== "undefined" ? (() => { try { return JSON.parse(localStorage.getItem("anvil-tab-groups") || "{}"); } catch { return {}; } })() : {});
  function persistGroups() { try { localStorage.setItem("anvil-tab-groups", JSON.stringify(tabGroups)); } catch { /* ignore */ } }
  function setGroup(f: string) { const g = prompt("Tab group name (blank to clear):", tabGroups[f] || ""); if (g === null) { tabMenu = null; return; } if (g.trim()) tabGroups = { ...tabGroups, [f]: g.trim() }; else { const { [f]: _x, ...rest } = tabGroups; tabGroups = rest; } persistGroups(); tabMenu = null; }
  const GROUP_COLORS = ["var(--accent)", "var(--green)", "var(--purple)", "var(--teal)", "var(--yellow)", "var(--blue)"];
  function groupColor(g: string): string { let h = 0; for (let i = 0; i < g.length; i++) h = (h * 31 + g.charCodeAt(i)) >>> 0; return GROUP_COLORS[h % GROUP_COLORS.length]; }
  let orderedFiles = $derived([...openFiles].sort((a, b) => {
    const pa = pinnedFiles.includes(a) ? 0 : 1, pb = pinnedFiles.includes(b) ? 0 : 1;
    if (pa !== pb) return pa - pb;
    return (tabGroups[a] || "~").localeCompare(tabGroups[b] || "~");
  }));
  let tabOverflow = $state(false);
  let recentFiles = $state<string[]>([]);
  let recentWorkspaces = $state<string[]>([]);
  let branch = $state("");

  async function openFolder() {
    const p = await invoke<string | null>("pick_folder", { start: cwd }).catch(() => null);
    if (!p) return;
    cwd = p.replace(/\/$/, "");
    recentWorkspaces = [cwd, ...recentWorkspaces.filter((w) => w !== cwd)].slice(0, 12);
    explorerOpen = true;
  }
  async function openFileDialog() {
    const p = await invoke<string | null>("pick_file", { start: cwd }).catch(() => null);
    if (p) openInEditor(p);
  }
  // #83 Go-to-anything: files + recent commits in one palette.
  async function goToAnything() {
    palettePlaceholder = "Go to anything — files & commits";
    const items: Item[] = [];
    for (const f of [...new Set([...openFiles, ...recentFiles])]) items.push({ label: baseName(f), hint: f, run: () => openInEditor(f) });
    // Symbols in the active file (LSP) — fused into the same list.
    if (activeFile && lspLang(activeFile)) {
      try {
        const syms = await fetchSymbols(lspLang(activeFile)!, activeFile);
        for (const s of syms.slice(0, 200)) items.push({ label: `◇ ${s.name}`, hint: `${baseName(activeFile)}:${s.line}`, run: () => { openInEditor(activeFile); editorGoto.set(s.line); } });
      } catch { /* ignore */ }
    }
    try {
      const log = await invoke<string>("git_log", { cwd, author: null, grep: null, path: null });
      for (const line of log.split("\n").slice(0, 25)) {
        const p = line.split("\x1f");
        const sh = p[1], an = p[2], subj = p[7];
        if (sh) items.push({ label: subj || sh, hint: `◆ ${sh} · ${an}`, run: () => { rail = "scm"; } });
      }
    } catch { /* not a git repo */ }
    paletteItems = items;
    paletteOpen = true;
  }
  function openRecentWorkspace() {
    palettePlaceholder = "Recent workspaces…";
    paletteItems = recentWorkspaces.map((w) => ({
      label: w.split("/").pop() ?? w,
      hint: w,
      run: () => { cwd = w; explorerOpen = true; },
    }));
    paletteOpen = true;
  }

  let terms = $state<{ id: string; title: string; shell?: string }[]>([{ id: "t1", title: "zsh" }]);
  let activeTerm = $state("t1");
  let splitTerm = $state<string | null>(null);
  let seq = 1;

  // Detach the active pane (or current view) into a new OS window (#17).
  function detachActivePane() {
    const lf = findLeaf(paneTree, activeLeaf);
    const view = (lf?.view ?? (rail === "editor" ? "editor" : rail)) as ViewKind;
    const ref = lf?.ref ?? (view === "editor" ? activeFile : undefined);
    const seedObj: { view: string; file?: string; cwd: string } = { view, cwd };
    if (view === "editor" && ref) seedObj.file = ref;
    const seed = encodeURIComponent(JSON.stringify(seedObj));
    invoke("new_window", { seed }).catch((e) => toast("Could not open new window: " + String(e).slice(0, 60), "error"));
  }

  function toggleSplit() {
    if (splitTerm) { splitTerm = null; return; }
    seq += 1;
    const id = `t${seq}`;
    terms = [...terms, { id, title: "zsh" }];
    splitTerm = id;
    rail = "term";
  }

  const baseName = (p: string) => p.split("/").pop() ?? p;

  // On-demand update check (#95). Degrades gracefully if no release host.
  function updateChannel(): string { try { return localStorage.getItem("anvil-update-channel") || "stable"; } catch { return "stable"; } }
  async function checkForUpdates() {
    const ch = updateChannel();
    toast(`Checking for updates (${ch})…`, "info");
    try {
      const v = await invoke<string | null>("check_update", { channel: ch });
      toast(v ? `Update available: v${v}` : "Anvil is up to date", v ? "success" : "info");
    } catch {
      toast("Update check unavailable (no release endpoint yet)", "info");
    }
  }
  function setUpdateChannel() {
    palettePlaceholder = "Update channel";
    paletteItems = ["stable", "beta"].map((c) => ({ label: c + (updateChannel() === c ? "  ✓" : ""), hint: "channel", run: () => { try { localStorage.setItem("anvil-update-channel", c); } catch { /* ignore */ } toast(`Update channel: ${c}`, "success"); } }));
    paletteOpen = true;
  }

  // Ping when an agent reply lands while you're looking elsewhere (#52).
  function notifyAgent(summary: string) {
    toast(`Agent finished: ${summary}`, "info");
    try {
      if ("Notification" in window) {
        if (Notification.permission === "granted") new Notification("Anvil · Agent", { body: summary });
        else if (Notification.permission !== "denied") Notification.requestPermission();
      }
    } catch { /* ignore */ }
  }

  function newTerm() {
    logEvent("terminal.new");
    seq += 1;
    const id = `t${seq}`;
    terms = [...terms, { id, title: "zsh" }];
    activeTerm = id;
    rail = "term";
  }
  // Terminal profile (#48): spawn a session with a specific shell.
  function newTermProfile(shell: string, title: string) {
    seq += 1;
    const id = `t${seq}`;
    terms = [...terms, { id, title, shell }];
    activeTerm = id;
    rail = "term";
  }
  function closeTerm(id: string) {
    terms = terms.filter((t) => t.id !== id);
    if (splitTerm === id) splitTerm = null;
    if (activeTerm === id) activeTerm = terms.at(-1)?.id ?? "";
    if (terms.length === 0) newTerm();
  }
  function selectTerm(id: string) {
    activeTerm = id;
    rail = "term";
  }

  // Editor navigation history (#13): back/forward through visited files (⌘⌥←/→).
  let navHistory = $state<string[]>([]);
  let navPtr = $state(-1);
  function navBack() { if (navPtr > 0) { navPtr -= 1; activeFile = navHistory[navPtr]; rail = "editor"; } }
  function navForward() { if (navPtr < navHistory.length - 1) { navPtr += 1; activeFile = navHistory[navPtr]; rail = "editor"; } }

  function openInEditor(p: string) {
    logEvent("file.open", { ext: p.split(".").pop() });
    if (!openFiles.includes(p)) openFiles = [...openFiles, p];
    activeFile = p;
    recentFiles = [p, ...recentFiles.filter((f) => f !== p)].slice(0, 30);
    if (navHistory[navPtr] !== p) {
      navHistory = [...navHistory.slice(0, navPtr + 1), p].slice(-50);
      navPtr = navHistory.length - 1;
    }
    // In the multipane workspace, route the file into the focused pane instead
    // of switching to the single-editor view.
    if (rail === "workspace" && findLeaf(paneTree, activeLeaf)) {
      paneTree = setLeafView(paneTree, activeLeaf, "editor", p);
    } else {
      rail = "editor";
    }
  }

  function openRecent() {
    palettePlaceholder = "Recent files…";
    paletteItems = recentFiles.map((f) => ({ label: baseName(f), hint: f, run: () => openInEditor(f) }));
    paletteOpen = true;
  }
  // Stack of recently closed file paths for reopen-closed-tab (⌘⇧T).
  let closedFiles = $state<string[]>([]);
  function closeFile(p: string) {
    openFiles = openFiles.filter((f) => f !== p);
    if (pinnedFiles.includes(p)) { pinnedFiles = pinnedFiles.filter((f) => f !== p); persistPins(); }
    const { [p]: _drop, ...rest } = dirtyFiles;
    dirtyFiles = rest;
    closedFiles = [p, ...closedFiles.filter((f) => f !== p)].slice(0, 20);
    if (activeFile === p) activeFile = openFiles.at(-1) ?? "";
    if (openFiles.length === 0 && rail === "editor") rail = "term";
  }
  function reopenClosed() {
    const p = closedFiles.find((f) => !openFiles.includes(f));
    if (!p) return;
    closedFiles = closedFiles.filter((f) => f !== p);
    openInEditor(p);
  }

  function showReferences(refs: { path: string; line: number; col: number }[]) {
    if (!refs.length) { toast("No references", "info"); return; }
    palettePlaceholder = `${refs.length} reference${refs.length === 1 ? "" : "s"}`;
    paletteItems = refs.map((r) => ({
      label: `${r.path.split("/").pop()}:${r.line}`,
      hint: r.path,
      run: () => { openInEditor(r.path); editorGoto.set(r.line); },
    }));
    paletteOpen = true;
  }

  function explainCode(code: string, p: string) {
    agentSeed.set(`Explain this code from ${p.split("/").pop()}:\n\n\`\`\`\n${code}\n\`\`\``);
    rail = "agent";
  }

  async function goToWorkspaceSymbol() {
    if (!activeFile || !lspLang(activeFile)) { toast("Open a file with a language server first", "info"); return; }
    const q = prompt("Search workspace symbols:");
    if (q == null) return;
    let syms: Awaited<ReturnType<typeof searchWorkspaceSymbols>> = [];
    try { syms = await searchWorkspaceSymbols(lspLang(activeFile)!, q); } catch { syms = []; }
    if (!syms.length) { toast("No symbols", "info"); return; }
    palettePlaceholder = `${syms.length} symbol${syms.length === 1 ? "" : "s"}`;
    paletteItems = syms.slice(0, 500).map((s) => ({
      label: s.name,
      hint: `${s.container ? s.container + " · " : ""}${s.path.split("/").pop()}:${s.line}`,
      run: () => { openInEditor(s.path); editorGoto.set(s.line); },
    }));
    paletteOpen = true;
  }

  async function goToSymbol() {
    if (!activeFile) { toast("Open a file first", "info"); return; }
    const lang = lspLang(activeFile);
    if (!lang) { toast("No language server for this file", "info"); return; }
    let syms: Awaited<ReturnType<typeof fetchSymbols>> = [];
    try { syms = await fetchSymbols(lang, activeFile); } catch { syms = []; }
    if (!syms.length) { toast("No symbols found", "info"); return; }
    palettePlaceholder = "Go to symbol…";
    paletteItems = syms.map((s) => ({
      label: `${"  ".repeat(s.depth)}${s.name}`,
      hint: s.detail,
      run: () => { rail = "editor"; editorGoto.set(s.line); },
    }));
    paletteOpen = true;
  }

  function openCommands() {
    palettePlaceholder = "Run a command…";
    paletteItems = [
      { label: "New Terminal", hint: "⌘T", run: newTerm },
      { label: "Split / Unsplit Terminal", hint: "⌘D", run: toggleSplit },
      { label: "New Terminal: bash", run: () => newTermProfile("/bin/bash", "bash") },
      { label: "New Terminal: zsh", run: () => newTermProfile("/bin/zsh", "zsh") },
      { label: "New Terminal: fish", run: () => newTermProfile("/opt/homebrew/bin/fish", "fish") },
      { label: "New Terminal: sh", run: () => newTermProfile("/bin/sh", "sh") },
      { label: "New Terminal: custom shell…", run: () => { const s = prompt("Shell path:", "/bin/zsh"); if (s) newTermProfile(s, s.split("/").pop() || "shell"); } },
      { label: "Toggle Bottom Terminal", hint: "⌘J", run: () => (bottomDock = !bottomDock) },
      { label: `Terminal: Broadcast Input ${get(broadcastInput) ? "(on)" : "(off)"}`, run: () => { const v = !get(broadcastInput); broadcastInput.set(v); toast(v ? "Broadcast input ON — keystrokes go to all terminals" : "Broadcast input off", v ? "info" : "success"); } },
      { label: "Terminal: Command History…", run: () => { const h = getHistory(); if (!h.length) { toast("No commands recorded yet", "info"); return; } palettePlaceholder = `${h.length} command${h.length === 1 ? "" : "s"} — Enter to rerun`; paletteItems = [...h].reverse().map((c) => ({ label: c, hint: "⏎ rerun", run: () => { rail = "term"; invoke("pty_write", { id: activeTerm, data: c + "\r" }).catch(() => toast("No active terminal", "error")); } })); paletteItems.push({ label: "Clear command history", hint: "irreversible", run: () => { clearHistory(); toast("Command history cleared", "success"); } }); paletteOpen = true; } },
      { label: `Terminal: Auto-cd to File's Folder ${get(terminalAutoCd) ? "(on)" : "(off)"}`, run: () => { toggleTerminalAutoCd(); toast(get(terminalAutoCd) ? "Active terminal follows the open file" : "Auto-cd off", "success"); } },
      { label: "Terminal: cd to Current File's Folder", run: () => { if (!activeFile) { toast("Open a file first", "info"); return; } const dir = activeFile.replace(/\/[^/]*$/, "") || "/"; invoke("pty_write", { id: activeTerm, data: `cd ${dir.includes(" ") ? `'${dir}'` : dir}\r` }).then(() => { rail = "term"; }).catch(() => toast("No active terminal", "error")); } },
      { label: "k8s: Apply Current Manifest (diff first)", run: async () => { if (!activeFile || !/\.(ya?ml)$/i.test(activeFile)) { toast("Open a YAML manifest first", "info"); return; } let diff = ""; try { diff = await invoke<string>("kube_diff", { path: activeFile }); } catch (e) { toast(String(e).slice(0, 80) || "kubectl diff failed", "error"); return; } const go = confirm(`kubectl diff for ${baseName(activeFile)}:\n\n${diff.slice(0, 1500)}\n\nApply these changes?`); if (!go) return; try { const r = await invoke<string>("kube_apply", { path: activeFile }); toast(r.trim().slice(0, 80) || "Applied", "success"); } catch (e) { toast(String(e).slice(0, 80) || "apply failed", "error"); } } },
      { label: "SSH: Connect to Host…", run: async () => { let hosts: string[] = []; try { hosts = (await invoke<string>("ssh_hosts")).split("\n").filter(Boolean); } catch { /* ignore */ } if (!hosts.length) { toast("No hosts in ~/.ssh/config", "info"); return; } palettePlaceholder = `${hosts.length} ssh host${hosts.length === 1 ? "" : "s"}`; paletteItems = hosts.map((h) => ({ label: h, hint: "ssh", run: () => { rail = "term"; invoke("pty_write", { id: activeTerm, data: `ssh ${h}\r` }).catch(() => toast("No active terminal", "error")); } })); paletteOpen = true; } },
      { label: "Close Tab", hint: "⌘W", run: () => (rail === "editor" ? closeFile(activeFile) : closeTerm(activeTerm)) },
      { label: "Find File…", hint: "⌘P", run: openFilesPalette },
      { label: "Go to Anything…", run: goToAnything },
      { label: "Recent Files…", hint: "⌘E", run: openRecent },
      { label: "Reopen Closed Tab", hint: "⌘⇧T", run: reopenClosed },
      { label: "Detach Pane to New Window", run: detachActivePane },
      { label: "Editor: Navigate Back", hint: "⌘⌥←", run: navBack },
      { label: "Editor: Navigate Forward", hint: "⌘⌥→", run: navForward },
      { label: "File History…", run: fileHistory },
      { label: `Problems… (${$problems.length})`, hint: "⇧⌘M", run: () => { bottomDock = true; dockTab = "problems"; } },
      { label: "Go to Line…", run: () => { if (!activeFile) { toast("Open a file first", "info"); return; } const n = prompt("Go to line:"); if (n && +n > 0) { rail = "editor"; editorGoto.set(Math.floor(+n)); } } },
      { label: "Ask Agent…", hint: "⌘I", run: () => { const q = prompt("Ask the agent:"); if (q && q.trim()) { agentSeed.set(q.trim()); rail = "agent"; } } },
      { label: "Agent: Enqueue Task…", run: () => { const q = prompt("Queue an agent task:"); if (q && q.trim()) { enqueueAgent(q.trim()); toast(`Queued (${get(agentQueue).length} pending)`, "success"); } } },
      { label: `Agent: Run Next Queued (${get(agentQueue).length})`, run: () => { const t = dequeueAgent(); if (!t) { toast("Queue is empty", "info"); return; } agentSeed.set(t.prompt); rail = "agent"; } },
      { label: "Agent: View Queue…", run: () => { const q = get(agentQueue); if (!q.length) { toast("Queue is empty", "info"); return; } palettePlaceholder = `${q.length} queued task${q.length === 1 ? "" : "s"}`; paletteItems = q.map((t) => ({ label: t.prompt, hint: "✕ remove", run: () => { removeQueued(t.id); toast("Removed from queue", "success"); } })); paletteItems.push({ label: "Run all (seed first, rest stay queued)", hint: "", run: () => { const n = dequeueAgent(); if (n) { agentSeed.set(n.prompt); rail = "agent"; } } }); paletteItems.push({ label: "Clear queue", hint: "irreversible", run: () => { clearQueue(); toast("Queue cleared", "success"); } }); paletteOpen = true; } },
      { label: "Agent: Run Tests & Fix", run: () => { agentSeed.set("Enable Agent mode, then: detect and run this project's test suite using your run tool (e.g. `npm test`, `cargo test`, `pytest`), read the failures, and propose minimal fixes as per-hunk diffs. Iterate until tests pass."); rail = "agent"; } },
      { label: "Agent: Set Utility Model (fast tasks)…", run: () => { const cur = (typeof localStorage !== "undefined" && localStorage.getItem("anvil-util-model")) || ""; const m = prompt("Model id for quick tasks like commit messages (blank = use default):", cur); if (m === null) return; try { if (m.trim()) localStorage.setItem("anvil-util-model", m.trim()); else localStorage.removeItem("anvil-util-model"); } catch { /* ignore */ } toast(m.trim() ? `Utility model: ${m.trim()}` : "Utility model cleared", "success"); } },
      { label: "Agent: Set Reasoning Model (agent chat)…", run: () => { const cur = (typeof localStorage !== "undefined" && localStorage.getItem("anvil-reasoning-model")) || ""; const m = prompt("Model id for agent reasoning/chat (blank = use default; reopen agent to apply):", cur); if (m === null) return; try { if (m.trim()) localStorage.setItem("anvil-reasoning-model", m.trim()); else localStorage.removeItem("anvil-reasoning-model"); } catch { /* ignore */ } toast(m.trim() ? `Reasoning model: ${m.trim()}` : "Reasoning model cleared", "success"); } },
      { label: "Markdown: Toggle Preview", hint: "⌘⇧V", run: () => { if (!isMarkdown(activeFile)) { toast("Open a Markdown file first", "info"); return; } mdPreview = !mdPreview; runbook = false; rail = "editor"; } },
      { label: "Markdown: Run as Runbook", run: () => { if (!isMarkdown(activeFile)) { toast("Open a Markdown file first", "info"); return; } runbook = !runbook; mdPreview = false; rail = "editor"; } },
      { label: `Editor: Ghost-Text Completion ${get(editorGhostText) ? "(on)" : "(off)"}`, run: () => { toggleGhostText(); toast(get(editorGhostText) ? "Ghost-text on — reopen files to apply" : "Ghost-text off", "success"); } },
      { label: `Editor: Ghost-Text Source — ${get(editorGhostSource).toUpperCase()}`, run: () => { const next = get(editorGhostSource) === "lsp" ? "llm" : "lsp"; setGhostSource(next); toast(`Ghost-text source: ${next.toUpperCase()} — reopen files to apply`, "success"); } },
      { label: "Snippets: Manage User Snippets…", run: () => { const list = getUserSnippets(); palettePlaceholder = "User snippets"; paletteItems = [{ label: "➕ Add a snippet…", hint: "ext · label · template", run: () => { const ext = prompt("File extension (e.g. ts, py, go):"); if (!ext) return; const label = prompt("Trigger label (e.g. comp):"); if (!label) return; const template = prompt("Template ( ${1:field} for tab stops, ${} exit ):"); if (!template) return; addUserSnippet({ ext: ext.replace(/^\./, "").toLowerCase(), label, template }); toast("Snippet added — reopen the file to use it", "success"); } }, ...list.map((s) => ({ label: `${s.ext}: ${s.label}`, hint: "click to remove", run: () => { removeUserSnippet(s.ext, s.label); toast("Snippet removed", "success"); } }))]; paletteOpen = true; } },
      { label: "Go to Symbol in File…", hint: "⌘⇧O", run: goToSymbol },
      { label: "Go to Symbol in Workspace…", run: goToWorkspaceSymbol },
      { label: "Explain Errors in Agent", run: () => {
        const mine = $problems.filter((p) => p.path === activeFile);
        const list = (mine.length ? mine : $problems).slice(0, 20).map((p) => `${p.path.split("/").pop()}:${p.line}  ${p.message}`).join("\n");
        if (!list) { toast("No diagnostics to explain", "info"); return; }
        agentSeed.set(`Explain these errors and suggest fixes:\n\n${list}`);
        rail = "agent";
      } },
      { label: "Save Command…", run: saveCommand },
      { label: "Run Saved Command…", run: runSavedCommand },
      { label: "Open Folder…", hint: "⌘⇧O", run: openFolder },
      { label: "Open File…", hint: "⌘O", run: openFileDialog },
      { label: "Recent Workspaces…", run: openRecentWorkspace },
      { label: "Cycle Theme", run: cycleTheme },
      { label: "Toggle Density", run: toggleDensity },
      { label: "Zoom In", hint: "⌘+", run: () => bumpScale(1) },
      { label: "Zoom Out", hint: "⌘−", run: () => bumpScale(-1) },
      { label: "Reset Zoom", hint: "⌘0", run: resetScale },
      { label: "Check for Updates", run: checkForUpdates },
      { label: "Update Channel (stable / beta)…", run: setUpdateChannel },
      { label: `Dev: FPS Overlay ${fpsOn ? "(on)" : "(off)"}`, run: toggleFps },
      { label: "Dev: Startup Timing", run: startupReport },
      { label: "Dev: PTY Flood Bench", run: ptyFloodBench },
      { label: `Telemetry: ${get(telemetryEnabled) ? "Disable" : "Enable"} (local-only)`, run: () => { toggleTelemetry(); toast(get(telemetryEnabled) ? "Local telemetry on — nothing leaves this machine" : "Telemetry off", "success"); } },
      { label: "Agent: Redaction Rules…", run: () => { const rules = get(redactionRules); palettePlaceholder = "Redaction rules (regex) — Enter to add"; paletteItems = [{ label: "➕ Add a rule…", hint: "regex", run: () => { const r = prompt("Regex to redact from agent context (e.g. \\\\bACME-[0-9]+\\\\b):"); if (r) { addRedactionRule(r); toast("Redaction rule added", "success"); } } }, ...rules.map((r) => ({ label: r, hint: "click to remove", run: () => { removeRedactionRule(r); toast("Rule removed", "success"); } }))]; paletteOpen = true; } },
      { label: "Agent: View Send Audit Log", run: () => { const log = getAuditLog(); if (!log.length) { toast("No agent sends recorded yet", "info"); return; } palettePlaceholder = `${log.length} agent send${log.length === 1 ? "" : "s"} (local)`; paletteItems = [...log].reverse().slice(0, 300).map((e) => ({ label: `${e.kind} · ${e.chars} chars`, hint: new Date(e.ts).toLocaleString() + "  " + e.preview.replace(/\n/g, " "), run: () => {} })); paletteItems.push({ label: "Clear audit log", hint: "irreversible", run: () => { clearAuditLog(); toast("Audit log cleared", "success"); } }); paletteOpen = true; } },
      { label: "Telemetry: View Local Events", run: () => { const evs = getEvents(); if (!evs.length) { toast(get(telemetryEnabled) ? "No events yet" : "Telemetry is off", "info"); return; } palettePlaceholder = `${evs.length} local event${evs.length === 1 ? "" : "s"}`; paletteItems = [...evs].reverse().slice(0, 300).map((e) => ({ label: e.name, hint: new Date(e.ts).toLocaleString() + (e.data ? "  " + JSON.stringify(e.data) : ""), run: () => {} })); paletteItems.push({ label: "Clear all local events", hint: "irreversible", run: () => { clearEvents(); toast("Local events cleared", "success"); } }); paletteOpen = true; } },
      { label: "Help: What's New", run: showWhatsNew },
      { label: "Help: Report a Problem (copy diagnostics)", run: async () => { const report = diagnosticsReport(WHATS_NEW_VERSION); try { await navigator.clipboard.writeText(report); toast("Diagnostics copied — paste into your bug report", "success"); } catch { toast("Could not copy diagnostics", "error"); } } },
      { label: `Help: View Crash Log (${getCrashes().length})`, run: () => { const cr = getCrashes(); if (!cr.length) { toast("No crashes recorded 🎉", "success"); return; } palettePlaceholder = `${cr.length} crash${cr.length === 1 ? "" : "es"} (local)`; paletteItems = [...cr].reverse().slice(0, 100).map((c) => ({ label: `${c.kind}: ${c.message}`, hint: new Date(c.ts).toLocaleString(), run: () => {} })); paletteItems.push({ label: "Clear crash log", hint: "irreversible", run: () => { clearCrashes(); toast("Crash log cleared", "success"); } }); paletteOpen = true; } },
      { label: "Help: Keyboard Shortcuts", hint: "⌘/", run: () => (keymapOpen = true) },
      { label: "Editor: Font Larger", run: () => bumpEditorFontSize(1) },
      { label: "Editor: Font Smaller", run: () => bumpEditorFontSize(-1) },
      { label: "Import Theme (JSON)…", run: importThemeJson },
      { label: "Import VS Code Theme (JSON)…", run: importVSCodeTheme },
      { label: "Import VS Code Settings", run: importVSCode },
      { label: "Keybindings: Apply Preset…", run: () => { palettePlaceholder = "Keybinding preset…"; paletteItems = Object.keys(KEY_PRESETS).map((n) => ({ label: n, hint: "preset", run: () => { applyKeymapPreset(n); toast(`Applied "${n}" keybindings`, "success"); } })); paletteOpen = true; } },
      { label: "Reload Config (~/.anvil/config.json)", run: async () => { if (!(await loadUserConfig(true))) toast("No ~/.anvil/config.json found", "info"); } },
      { label: "Export Settings to File", run: exportSettings },
      { label: "Import Settings from File", run: importSettings },
      { label: "Pin Theme + Density to Workspace", run: pinWorkspace },
      { label: "View: Terminal", run: () => (rail = "term") },
      { label: "Toggle Explorer Sidebar", hint: "⌘B", run: toggleSide },
      { label: `Workspace: Focus Dimming ${get(focusDimming) ? "(on)" : "(off)"}`, run: () => { toggleFocusDimming(); toast(get(focusDimming) ? "Inactive panes dimmed" : "Focus dimming off", "success"); } },
      { label: "View: Source Control", run: () => (rail = "scm") },
      { label: "Git: Pull (fast-forward)", run: async () => { try { await invoke("git_pull", { cwd }); toast("Pulled", "success"); } catch (e) { toast(String(e).slice(0, 80) || "Pull failed", "error"); } } },
      { label: "Git: Push", run: async () => { try { await invoke("git_push", { cwd }); toast("Pushed", "success"); } catch (e) { toast(String(e).slice(0, 80) || "Push failed", "error"); } } },
      { label: "Git: Fetch All", run: async () => { try { await invoke("git_fetch", { cwd }); toast("Fetched", "success"); } catch { toast("Fetch failed", "error"); } } },
      { label: "Git: 3-Pane Merge (current file)", run: () => { if (!activeFile) { toast("Open the conflicted file first", "info"); return; } mergeView = activeFile; } },
      { label: "Git: Generate PR Body (agent → clipboard)", run: generatePrBody },
      { label: "Git: Interactive Rebase onto…", run: () => { const target = prompt("Rebase onto (branch / ref / commit):", "origin/main"); if (target) rebaseTarget = target; } },
      { label: "Git: Interactive Rebase in Terminal…", run: () => { const target = prompt("Rebase onto (branch / ref / commit):", "origin/main"); if (!target) return; rail = "term"; invoke("pty_write", { id: activeTerm, data: `git rebase -i ${target}\r` }).catch(() => toast("No active terminal", "error")); } },
      { label: "Git: Worktrees…", run: async () => { let raw = ""; try { raw = await invoke<string>("git_worktrees", { cwd }); } catch { toast("Not a git repository", "error"); return; } const rows = raw.split("\n").filter(Boolean).map((l) => { const [p, b] = l.split("\t"); return { p, b }; }); palettePlaceholder = `${rows.length} worktree${rows.length === 1 ? "" : "s"}`; paletteItems = rows.map((w) => ({ label: w.b, hint: w.p + (w.p === cwd ? "  (current)" : ""), run: () => { cwd = w.p; explorerOpen = true; toast(`Switched to ${w.b}`, "success"); } })); paletteItems.push({ label: "➕ Add worktree…", hint: "branch → sibling dir", run: async () => { const br = prompt("Branch for the new worktree:"); if (!br) return; const path = prompt("Path for the worktree:", `${cwd}-${br.replace(/[^\w.-]+/g, "-")}`); if (!path) return; try { await invoke("git_worktree_add", { cwd, path, branch: br }); toast("Worktree added", "success"); } catch (e) { toast(String(e).slice(0, 80) || "add failed", "error"); } } }); paletteOpen = true; } },
      { label: "Git: Amend Last Commit (staged)", run: async () => { try { await invoke("git_amend", { cwd }); toast("Amended last commit", "success"); } catch (e) { toast(String(e).slice(0, 80) || "Amend failed", "error"); } } },
      { label: "GitHub: Create PR (gh pr create --fill)", run: async () => { try { const r = await invoke<string>("gh_pr_create", { cwd }); toast(r.split("\n").find(Boolean)?.slice(0, 80) || "PR created", "success"); } catch (e) { toast(String(e).slice(0, 90) || "PR create failed", "error"); } } },
      { label: "GitHub: View PR in Browser", run: async () => { try { await invoke("gh_pr_web", { cwd }); } catch (e) { toast(String(e).slice(0, 80) || "No PR for this branch", "error"); } } },
      { label: "AWS: SSO Login", run: () => { invoke("pty_write", { id: activeTerm, data: "aws sso login\n" }); rail = "term"; } },
      { label: "AWS: EC2 Instances", run: () => { invoke("pty_write", { id: activeTerm, data: "aws ec2 describe-instances --query 'Reservations[].Instances[].{ID:InstanceId,Type:InstanceType,State:State.Name,Name:Tags[?Key==`Name`]|[0].Value}' --output table\n" }); rail = "term"; } },
      { label: "AWS: S3 Buckets", run: () => { invoke("pty_write", { id: activeTerm, data: "aws s3 ls\n" }); rail = "term"; } },
      { label: "AWS: Lambda Functions", run: () => { invoke("pty_write", { id: activeTerm, data: "aws lambda list-functions --query 'Functions[].{Name:FunctionName,Runtime:Runtime,Mem:MemorySize}' --output table\n" }); rail = "term"; } },
      { label: "AWS: RDS Instances", run: () => { invoke("pty_write", { id: activeTerm, data: "aws rds describe-db-instances --query 'DBInstances[].{ID:DBInstanceIdentifier,Engine:Engine,Class:DBInstanceClass,Status:DBInstanceStatus}' --output table\n" }); rail = "term"; } },
      { label: "Secrets: SSM Get Parameter…", run: () => { const k = prompt("SSM parameter name (e.g. /app/db/password):"); if (k) { invoke("pty_write", { id: activeTerm, data: `aws ssm get-parameter --name '${k}' --with-decryption --query Parameter.Value --output text\n` }); rail = "term"; } } },
      { label: "Secrets: Vault Read…", run: () => { const k = prompt("Vault path (e.g. secret/data/app):"); if (k) { invoke("pty_write", { id: activeTerm, data: `vault kv get '${k}'\n` }); rail = "term"; } } },
      { label: "Secrets: Keychain Find…", run: () => { const k = prompt("Keychain service name:"); if (k) { invoke("pty_write", { id: activeTerm, data: `security find-generic-password -s '${k}' -w\n` }); rail = "term"; } } },
      { label: "AWS: Switch Profile…", run: async () => {
        let profiles: string[] = [];
        try { profiles = (await invoke<string>("aws_profiles")).split("\n").filter(Boolean); } catch { /* ignore */ }
        if (!profiles.length) { toast("No AWS profiles in ~/.aws/config", "info"); return; }
        palettePlaceholder = "AWS profile…";
        paletteItems = profiles.map((p) => ({ label: p, run: async () => {
          await invoke("set_aws_profile", { profile: p }).catch((e) => toast("Failed to set AWS profile: " + String(e).slice(0, 60), "error"));
          invoke("pty_write", { id: activeTerm, data: `export AWS_PROFILE=${p}\n` });
          toast(`AWS_PROFILE=${p}`, "success");
        } }));
        paletteOpen = true;
      } },
      { label: "GitLab: CI Pipelines (glab ci list)", run: () => { invoke("pty_write", { id: activeTerm, data: "glab ci list\n" }); rail = "term"; } },
      { label: "GitLab: Pipeline Logs (glab ci trace)", run: () => { invoke("pty_write", { id: activeTerm, data: "glab ci trace\n" }); rail = "term"; } },
      { label: "GitLab: Retry Pipeline (glab ci retry)", run: () => { invoke("pty_write", { id: activeTerm, data: "glab ci retry\n" }); rail = "term"; } },
      { label: "View: Search", run: () => (rail = "search") },
      { label: "View: AI Agent", run: () => (rail = "agent") },
      { label: "View: Kubernetes", run: () => (rail = "k8s") },
      { label: "View: CI / Pipelines", run: () => (rail = "ci") },
      { label: "View: Terraform / Terragrunt", run: () => (rail = "terraform") },
      { label: "View: Helm", run: () => (rail = "helm") },
      { label: "View: Observability (Metrics / Logs)", run: () => (rail = "obs") },
      { label: "View: DevOps (Terraform / Helm / Observability)", run: () => (rail = "devops") },
      { label: "View: Workspace (multipane)", run: () => (rail = "workspace") },
      { label: "Workspace: Balance Panes", run: () => { paneTree = balanceTree(paneTree); rail = "workspace"; } },
      { label: "Workspace: Close Other Panes", run: () => { paneTree = closeOtherPanes(paneTree, activeLeaf); rail = "workspace"; } },
      { label: "Workspace: Save Layout As…", run: saveLayoutAs },
      { label: "Workspace: Load Layout…", run: openLayoutPalette },
      { label: "Workspace: Export Layout (copy)", run: exportLayout },
      { label: "Workspace: Import Layout (paste)", run: importLayout },
      { label: "View: Settings", run: openSettings },
      { label: "Toggle Zen / Terminal Mode", hint: "⌘.", run: () => toggleZen() },
    ];
    paletteOpen = true;
  }

  // Saved commands / prompt library (#59), persisted in localStorage.
  let savedCommands = $state<string[]>([]);
  function saveCommand() {
    const c = prompt("Save a command to run later:");
    if (!c) return;
    savedCommands = [c, ...savedCommands.filter((x) => x !== c)].slice(0, 50);
    try { localStorage.setItem("anvil-saved-cmds", JSON.stringify(savedCommands)); } catch { /* ignore */ }
    toast("Command saved", "success");
  }
  function runSavedCommand() {
    if (!savedCommands.length) { toast("No saved commands yet", "info"); return; }
    palettePlaceholder = "Run saved command…";
    paletteItems = savedCommands.map((c) => ({
      label: c,
      run: () => { invoke("pty_write", { id: activeTerm, data: c + "\n" }); rail = "term"; },
    }));
    paletteOpen = true;
  }

  async function fileHistory() {
    if (!activeFile) { toast("Open a file first", "info"); return; }
    let raw = "";
    try { raw = await invoke<string>("git_file_log", { cwd, path: activeFile }); } catch { toast("No git history for this file", "error"); return; }
    const rows = raw.split("\n").filter(Boolean).map((l) => l.split("\x1f"));
    if (!rows.length) { toast("No commits touch this file", "info"); return; }
    palettePlaceholder = `History — ${baseName(activeFile)}`;
    paletteItems = rows.map(([rev, short, author, , subject]) => ({
      label: subject || short,
      hint: `${short} · ${author}`,
      run: () => { diffTarget = { rev }; rail = "diff"; },
    }));
    paletteOpen = true;
  }

  async function openFilesPalette() {
    palettePlaceholder = "Go to file…";
    const root = cwd.replace(/\/$/, "");
    const files = await invoke<string[]>("walk_dir", { root });
    paletteItems = files.map((f) => ({ label: f, run: () => openInEditor(`${root}/${f}`) }));
    paletteOpen = true;
  }

  // Maps remappable action ids → their handlers (mirrors KEY_ACTIONS in keymap.ts).
  const KEY_FNS: Record<string, () => void> = {
    "new-terminal": newTerm,
    "reopen-tab": reopenClosed,
    "command-palette": openCommands,
    "go-to-file": openFilesPalette,
    "recent-files": openRecent,
    "open-file": openFileDialog,
    "open-folder": openFolder,
    "new-window": () => { invoke("new_window").catch((e) => toast("Could not open new window: " + String(e).slice(0, 60), "error")); },
    "split-terminal": toggleSplit,
    "bottom-dock": () => { bottomDock = !bottomDock; },
    "explorer": toggleSide,
    "zen": () => { toggleZen(); },
    "search": () => { rail = "search"; },
  };
  function onCustomKey(e: KeyboardEvent) {
    const ov = get(keyOverrides);
    if (!Object.keys(ov).length) return;
    const combo = comboOf(e);
    for (const [id, c] of Object.entries(ov)) {
      if (c === combo && KEY_FNS[id]) { e.preventDefault(); KEY_FNS[id](); return; }
    }
  }

  function onKey(e: KeyboardEvent) {
    // #18 Quake terminal: Ctrl+` toggles a drop-down terminal overlay.
    if (e.ctrlKey && !e.metaKey && e.key === "`") { e.preventDefault(); quakeOpen = !quakeOpen; return; }
    if (quakeOpen && e.key === "Escape") { quakeOpen = false; return; }
    if (!e.metaKey) return;
    if (e.shiftKey && (e.key === "t" || e.key === "T")) { e.preventDefault(); reopenClosed(); }
    else if (e.key === "t") { e.preventDefault(); newTerm(); }
    else if (e.key === "w") {
      e.preventDefault();
      if (rail === "editor" && activeFile) closeFile(activeFile);
      else if (rail === "term") closeTerm(activeTerm);
      else if (rail === "workspace") wsClose(activeLeaf);
    }
    else if (e.key === "k") { e.preventDefault(); openCommands(); }
    else if (e.key === "p") { e.preventDefault(); openFilesPalette(); }
    else if (e.key === "e") { e.preventDefault(); openRecent(); }
    else if (e.key === "n") { e.preventDefault(); invoke("new_window").catch((e) => toast("Could not open new window: " + String(e).slice(0, 60), "error")); }
    else if (e.key === "o") { e.preventDefault(); openFolder(); }
    else if (e.key === "d") { e.preventDefault(); toggleSplit(); }
    else if (e.key === "j") { e.preventDefault(); bottomDock = !bottomDock; }
    else if (e.shiftKey && (e.key === "f" || e.key === "F")) { e.preventDefault(); rail = "search"; }
    else if (e.shiftKey && (e.key === "b" || e.key === "B")) { e.preventDefault(); toggleRail(); }
    else if (e.shiftKey && (e.key === "o" || e.key === "O")) { e.preventDefault(); goToSymbol(); }
    else if (e.shiftKey && (e.key === "m" || e.key === "M")) { e.preventDefault(); bottomDock = true; dockTab = "problems"; }
    else if (e.shiftKey && (e.key === "v" || e.key === "V")) { if (isMarkdown(activeFile)) { e.preventDefault(); mdPreview = !mdPreview; rail = "editor"; } }
    else if (e.key === "i") { e.preventDefault(); rail = "agent"; }
    else if (e.key === "/") { e.preventDefault(); keymapOpen = true; }
    else if (rail === "editor" && e.altKey && e.key === "ArrowLeft") { e.preventDefault(); navBack(); }
    else if (rail === "editor" && e.altKey && e.key === "ArrowRight") { e.preventDefault(); navForward(); }
    else if (rail === "workspace" && e.shiftKey && e.key === "Enter") {
      e.preventDefault();
      zoomedLeaf = zoomedLeaf ? null : activeLeaf;
    }
    else if (rail === "workspace" && e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      e.preventDefault();
      // ⌥⌘←/→ cycles focus between panes (#9).
      const ids = leafIds(paneTree);
      if (ids.length > 1) {
        const i = Math.max(0, ids.indexOf(activeLeaf));
        const n = ids.length;
        activeLeaf = ids[e.key === "ArrowRight" ? (i + 1) % n : (i - 1 + n) % n];
      }
    }
    else if (rail === "workspace" && e.key === "\\") {
      e.preventDefault();
      const lf = findLeaf(paneTree, activeLeaf);
      if (lf) wsSplit(lf.id, e.shiftKey ? "bottom" : "right", lf.view, lf.ref);
    }
    else if (e.key === "=" || e.key === "+") { e.preventDefault(); bumpScale(1); }
    else if (e.key === "-" || e.key === "_") { e.preventDefault(); bumpScale(-1); }
    else if (e.key === "0") { e.preventDefault(); resetScale(); }
    else if (e.key === ".") {
      // In the editor ⌘. is the LSP code-action / quick-fix; only outside it does
      // ⌘. mean "jump to a zen terminal".
      if (document.activeElement?.closest(".cm-editor")) return;
      e.preventDefault(); toggleZen();
    }
    else if (e.key >= "1" && e.key <= "9") {
      e.preventDefault();
      // ⌘1–9 jumps to the Nth tab across the strip (terminals, then files).
      const tabList = [
        ...terms.map((t) => () => selectTerm(t.id)),
        ...openFiles.map((f) => () => { activeFile = f; rail = "editor"; }),
      ];
      tabList[Number(e.key) - 1]?.();
    }
  }

  // #19 Auto-cd: when enabled, the active terminal follows the open file's dir.
  let lastAutoCdDir = "";
  $effect(() => {
    if (!$terminalAutoCd || !activeFile || !activeTerm) return;
    const dir = activeFile.replace(/\/[^/]*$/, "") || "/";
    if (dir === lastAutoCdDir) return;
    lastAutoCdDir = dir;
    invoke("pty_write", { id: activeTerm, data: `cd ${dir.includes(" ") ? `'${dir}'` : dir}\r` }).catch((e) => console.warn("pty_write cd failed", e));
  });

  // Apply a workspace's pinned theme/density when its folder becomes active.
  $effect(() => {
    const ws = wsSettings[cwd];
    if (ws?.theme) applyTheme(ws.theme);
    if (ws?.density) applyDensity(ws.density);
  });

  // Shared config schema (#90/#91): validate + apply known keys, return issues.
  function applyConfig(cfg: Record<string, unknown>): string[] {
    const issues: string[] = [];
    if ("theme" in cfg) { if (typeof cfg.theme === "string") applyTheme(cfg.theme); else issues.push("theme must be a string"); }
    if ("density" in cfg) { if (cfg.density === "compact" || cfg.density === "regular") applyDensity(cfg.density); else issues.push("density must be 'compact' or 'regular'"); }
    if ("formatOnSave" in cfg) { if (typeof cfg.formatOnSave === "boolean") editorFormatOnSave.set(cfg.formatOnSave); else issues.push("formatOnSave must be a boolean"); }
    if ("tabSize" in cfg) { if (typeof cfg.tabSize === "number" && cfg.tabSize >= 1 && cfg.tabSize <= 8) editorTabSize.set(Math.floor(cfg.tabSize)); else issues.push("tabSize must be a number 1–8"); }
    if ("wordWrap" in cfg) { if (typeof cfg.wordWrap === "boolean") editorWordWrap.set(cfg.wordWrap); else issues.push("wordWrap must be a boolean"); }
    return issues;
  }

  // #90 User config file `~/.anvil/config.json` — loaded at startup and on demand
  // ("Reload Config"); same schema as the per-project file, lower precedence.
  async function loadUserConfig(announce = false): Promise<boolean> {
    let home = "";
    try { home = await invoke<string>("home_dir"); } catch { return false; }
    let raw = "";
    try { raw = await invoke<string>("read_file", { path: `${home}/.anvil/config.json` }); } catch { return false; }
    let cfg: Record<string, unknown>;
    try { cfg = JSON.parse(raw); } catch { toast("~/.anvil/config.json is invalid JSON", "error"); return false; }
    const issues = applyConfig(cfg);
    if (issues.length) toast(`Config: ${issues[0]}`, "error");
    else if (announce) toast("Reloaded ~/.anvil/config.json", "success");
    return true;
  }

  // Per-project settings (#91): a checked-in `.anvil/settings.json` overrides
  // theme / density / editor prefs for everyone who opens the folder.
  let lastProjectCwd = "";
  $effect(() => {
    const dir = cwd;
    if (!dir || dir === lastProjectCwd) return;
    lastProjectCwd = dir;
    (async () => {
      let raw = "";
      try { raw = await invoke<string>("read_file", { path: `${dir}/.anvil/settings.json` }); } catch { return; }
      let cfg: Record<string, unknown>;
      try { cfg = JSON.parse(raw); } catch { toast(".anvil/settings.json is invalid JSON", "error"); return; }
      const issues = applyConfig(cfg);
      if (issues.length) toast(`.anvil/settings.json: ${issues[0]}`, "error");
      else toast(`Applied .anvil/settings.json`, "info");
    })();
  });

  // Track the current git branch + upstream ahead/behind for the status bar.
  let aheadBehind = $state<{ a: number; b: number } | null>(null);
  $effect(() => {
    const c = cwd;
    if (!c) { branch = ""; aheadBehind = null; return; }
    invoke<string>("git_current_branch", { cwd: c })
      .then((b) => (branch = b.trim()))
      .catch(() => (branch = ""));
    invoke<string>("git_ahead_behind", { cwd: c })
      .then((s) => { const [a, b] = s.trim().split(/\s+/).map(Number); aheadBehind = (a || b) ? { a, b } : null; })
      .catch(() => (aheadBehind = null));
  });

  async function saveState() {
    try {
      await invoke("write_state", {
        contents: JSON.stringify({ cwd, rail, explorerOpen, openFiles, activeFile, terms, activeTerm, seq, recentFiles, recentWorkspaces, wsSettings, paneTree, activeLeaf }),
      });
    } catch { /* ignore */ }
  }

  let restored = false;
  let lastErrToast = 0;
  onMount(async () => {
    // Surface uncaught errors/rejections (throttled) so a failure is visible,
    // not a silent white-screen. crash.ts also records them to the ring buffer.
    installCrashHandlers((_kind, message) => {
      const t = performance.now();
      if (t - lastErrToast < 4000) return;
      lastErrToast = t;
      toast("Unexpected error: " + message.slice(0, 120), "error");
    });
    bootMs = Math.round(performance.now() - bootStart);
    requestAnimationFrame(() => { firstPaintMs = Math.round(performance.now() - bootStart); });
    initTheme();
    initDensity();
    initScale();
    initOpacity();
    loadUserConfig();
    // Live reload (#90): re-read the user config when the window regains focus.
    window.addEventListener("focus", () => loadUserConfig());
    // #20 Open file paths clicked in a terminal.
    terminalOpenPath.subscribe((req) => { if (req) { openInEditor(req.path); if (req.line) editorGoto.set(req.line); terminalOpenPath.set(null); } });
    // #72 Report flood-bench drain time when the command finishes (OSC 133 D).
    lastExit.subscribe(() => { if (floodArmed) { floodArmed = false; toast(`PTY flood drained in ${Math.round(performance.now() - floodT0)}ms`, "success"); } });
    initFonts();
    // Warm every lazy view chunk once the app is idle, so the first switch to a
    // page is an instant mount instead of a chunk download + parse on click.
    // Pure module preload — nothing renders or fetches here.
    const prefetchViews = () => {
      for (const f of [SourceControl, Editor, DiffView, SearchPanel, AgentPanel, Settings, DevOps, Kube, CI, Terraform, Helm, Observability, FileView]) {
        f().catch(() => {});
      }
    };
    if (typeof requestIdleCallback === "function") requestIdleCallback(prefetchViews, { timeout: 4000 });
    else setTimeout(prefetchViews, 2500);
    // Theme (incl. custom overrides + system light/dark follow) handled in initTheme.
    if (isDetached && detachSeed) {
      cwd = detachSeed.cwd || (await invoke<string>("home_dir").catch(() => ""));
      const v = detachSeed.view ?? "term";
      paneTree = leaf(v as ViewKind, detachSeed.file, paneId("wt"));
      if (v === "editor" && detachSeed.file) {
        openFiles = [detachSeed.file];
        activeFile = detachSeed.file;
        rail = "editor";
      } else {
        rail = v === "term" ? "term" : v;
      }
      restored = true;
      window.addEventListener("keydown", onCustomKey, true);
      await listen<string>("menu", (e) => onMenu(e.payload));
      return;
    }
    let st: any = {};
    try { st = JSON.parse(await invoke<string>("read_state")); } catch { st = {}; }
    cwd = st.cwd || (await invoke<string>("home_dir"));
    if (Array.isArray(st.terms) && st.terms.length) {
      terms = st.terms;
      activeTerm = st.activeTerm || terms[0].id;
      seq = st.seq || terms.length;
    }
    if (Array.isArray(st.openFiles)) { openFiles = st.openFiles; activeFile = st.activeFile || openFiles.at(-1) || ""; }
    if (Array.isArray(st.recentFiles)) recentFiles = st.recentFiles;
    if (Array.isArray(st.recentWorkspaces)) recentWorkspaces = st.recentWorkspaces;
    if (st.wsSettings && typeof st.wsSettings === "object") wsSettings = st.wsSettings;
    if (st.paneTree && typeof st.paneTree === "object") { try { paneTree = remapTermRefs(st.paneTree); } catch { /* ignore */ } }
    if (typeof st.activeLeaf === "string" && findLeaf(paneTree, st.activeLeaf)) activeLeaf = st.activeLeaf;
    if (st.rail && st.rail !== "diff") rail = st.rail;
    if (typeof st.explorerOpen === "boolean") explorerOpen = st.explorerOpen;
    // Migrate old single-rail "files" mode → persistent explorer + editor view.
    if (rail === "files") { explorerOpen = true; rail = activeFile ? "editor" : "term"; }
    restored = true;
    // Editable keymap (#82): custom shortcuts run via a capture-phase listener,
    // layered over the default onKey handler.
    window.addEventListener("keydown", onCustomKey, true);
    try { layoutPresets = JSON.parse(localStorage.getItem("anvil-layouts") || "{}"); } catch { layoutPresets = {}; }
    try { savedCommands = JSON.parse(localStorage.getItem("anvil-saved-cmds") || "[]"); } catch { savedCommands = []; }
    await listen<string>("menu", (e) => onMenu(e.payload));
  });

  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void [cwd, rail, explorerOpen, openFiles, activeFile, terms, activeTerm, recentFiles, recentWorkspaces, wsSettings, paneTree, activeLeaf];
    if (!restored || isDetached) return;
    // Debounce: pane drags / rapid layout changes shouldn't write state on every
    // frame (#94 — drop needless reactivity/IO).
    clearTimeout(saveTimer);
    saveTimer = setTimeout(saveState, 400);
  });

  // ── Tab drag-reorder + context menu (roadmap §A #15 / #16) ──
  type TabKind = "term" | "file";
  let dragTab = $state<{ kind: TabKind; id: string } | null>(null);
  let tabMenu = $state<{ x: number; y: number; kind: TabKind; id: string } | null>(null);

  function reorder<T>(arr: T[], from: T, to: T): T[] {
    if (from === to) return arr;
    const a = [...arr];
    const fi = a.indexOf(from);
    if (fi < 0) return arr;
    a.splice(fi, 1);
    const ti = a.indexOf(to);
    a.splice(ti < 0 ? a.length : ti, 0, from);
    return a;
  }
  function tabDrop(kind: TabKind, id: string) {
    if (!dragTab || dragTab.kind !== kind) { dragTab = null; return; }
    if (kind === "term") {
      const from = terms.find((t) => t.id === dragTab!.id);
      const to = terms.find((t) => t.id === id);
      if (from && to) terms = reorder(terms, from, to);
    } else {
      openFiles = reorder(openFiles, dragTab.id, id);
    }
    dragTab = null;
  }
  function tabCtx(e: MouseEvent, kind: TabKind, id: string) {
    e.preventDefault();
    tabMenu = { x: e.clientX, y: e.clientY, kind, id };
  }
  function closeOthers(kind: TabKind, id: string) {
    if (kind === "term") { for (const t of [...terms]) if (t.id !== id) closeTerm(t.id); }
    else { for (const f of [...openFiles]) if (f !== id && !pinnedFiles.includes(f)) closeFile(f); }
    tabMenu = null;
  }
  function closeRight(kind: TabKind, id: string) {
    if (kind === "term") {
      const i = terms.findIndex((t) => t.id === id);
      for (const t of terms.slice(i + 1)) closeTerm(t.id);
    } else {
      const i = openFiles.indexOf(id);
      for (const f of openFiles.slice(i + 1)) closeFile(f);
    }
    tabMenu = null;
  }
  function copyTabPath(kind: TabKind, id: string) {
    const txt = kind === "file" ? id : (terms.find((t) => t.id === id)?.title ?? id);
    navigator.clipboard.writeText(txt).catch((e) => console.warn("clipboard write failed", e));
    tabMenu = null;
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="app" class:zen class:rail-auto={$autoHideRail}>
  {#if zen}<div class="zen-bar" data-tauri-drag-region></div>{/if}
  <div class="tabs" data-tauri-drag-region>
    {#each terms as t (t.id)}
      <div class="tab {rail === 'term' && activeTerm === t.id ? 'on' : ''}" onclick={() => selectTerm(t.id)} title={t.title}
        draggable="true" class:drag={dragTab?.kind === 'term' && dragTab.id === t.id}
        ondragstart={() => { dragTab = { kind: 'term', id: t.id }; tabDragView = { view: 'term', ref: t.id }; }}
        ondragend={() => { dragTab = null; tabDragView = null; }} ondragover={(e) => e.preventDefault()}
        ondrop={() => tabDrop('term', t.id)} oncontextmenu={(e) => tabCtx(e, 'term', t.id)}>
        <span class="tt">{t.title}</span>
        <span class="x" onclick={(e) => { e.stopPropagation(); closeTerm(t.id); }}>×</span>
      </div>
    {/each}
    {#each orderedFiles as f (f)}
      <div class="tab {rail === 'editor' && activeFile === f ? 'on' : ''}" class:pinned-tab={pinnedFiles.includes(f)} onclick={() => { activeFile = f; rail = 'editor'; }} title={tabGroups[f] ? `${f}  ·  group: ${tabGroups[f]}` : f}
        style={tabGroups[f] ? `box-shadow: inset 0 -2px 0 ${groupColor(tabGroups[f])}` : ''}
        draggable="true" class:drag={dragTab?.kind === 'file' && dragTab.id === f}
        ondragstart={() => { dragTab = { kind: 'file', id: f }; tabDragView = { view: 'editor', ref: f }; }}
        ondragend={() => { dragTab = null; tabDragView = null; }} ondragover={(e) => e.preventDefault()}
        ondrop={() => tabDrop('file', f)} oncontextmenu={(e) => tabCtx(e, 'file', f)}>
        {#if pinnedFiles.includes(f)}<span class="pin" onclick={(e) => { e.stopPropagation(); togglePin(f); }} title="Unpin"><Icon name="pin" size={9} /></span>{/if}
        <span class="tt">{baseName(f)}</span>{#if dirtyFiles[f]}<span class="dirty"></span>{/if}
        <span class="x" onclick={(e) => { e.stopPropagation(); closeFile(f); }}>×</span>
      </div>
    {/each}
    {#if openFiles.length > 1}
      <div class="newtab" title="All open tabs" onclick={() => (tabOverflow = !tabOverflow)}>⌄</div>
    {/if}
    {#if settingsOpen}
      <div class="tab {rail === 'settings' ? 'on' : ''}" onclick={() => (rail = 'settings')}>
        <span class="tab-ic"><Icon name="settings" size={12} /></span><span class="tt">Settings</span>
        <span class="x" onclick={(e) => { e.stopPropagation(); settingsOpen = false; if (rail === 'settings') rail = 'term'; }}>×</span>
      </div>
    {/if}
    <div class="newtab" title="New terminal (⌘T)" onclick={newTerm}><Icon name="plus" size={15} /></div>
    <div class="spacer" data-tauri-drag-region></div>
  </div>

  {#if tabMenu}
    <div class="ctxscrim" onclick={() => (tabMenu = null)} oncontextmenu={(e) => { e.preventDefault(); tabMenu = null; }} role="presentation"></div>
    <div class="tabctx" style:left="{tabMenu.x}px" style:top="{tabMenu.y}px">
      <button onclick={() => { if (tabMenu!.kind === 'term') closeTerm(tabMenu!.id); else closeFile(tabMenu!.id); tabMenu = null; }}>Close</button>
      <button onclick={() => closeOthers(tabMenu!.kind, tabMenu!.id)}>Close Others</button>
      <button onclick={() => closeRight(tabMenu!.kind, tabMenu!.id)}>Close to the Right</button>
      {#if tabMenu.kind === 'file'}<button onclick={() => togglePin(tabMenu!.id)}>{pinnedFiles.includes(tabMenu.id) ? 'Unpin Tab' : 'Pin Tab'}</button>{/if}
      {#if tabMenu.kind === 'file'}<button onclick={() => setGroup(tabMenu!.id)}>{tabGroups[tabMenu.id] ? `Group: ${tabGroups[tabMenu.id]}…` : 'Add to Group…'}</button>{/if}
      <button onclick={() => copyTabPath(tabMenu!.kind, tabMenu!.id)}>{tabMenu.kind === 'file' ? 'Copy Path' : 'Copy Title'}</button>
    </div>
  {/if}

  {#if tabOverflow}
    <div class="ctxscrim" onclick={() => (tabOverflow = false)} role="presentation"></div>
    <div class="taboverflow">
      {#each orderedFiles as f (f)}
        <button class:on={activeFile === f && rail === 'editor'} onclick={() => { activeFile = f; rail = 'editor'; tabOverflow = false; }} title={f}>
          {#if pinnedFiles.includes(f)}<span class="ofpin"><Icon name="pin" size={9} /></span>{/if}<span class="oftt">{baseName(f)}</span>{#if dirtyFiles[f]}<span class="dirty"></span>{/if}
        </button>
      {/each}
    </div>
  {/if}

  <div class="main">
    {#if $autoHideRail}<div class="rail-hot"></div>{/if}
    {#if !railHidden}
    <nav class="rail">
      <div class="i {rail === 'term' ? 'on' : ''}" title="Terminal" onclick={() => (rail = 'term')}><Icon name="terminal" /></div>
      <div class="i panel {explorerOpen ? 'pinned' : ''}" title="Explorer (⌘B)" onclick={toggleSide}><Icon name="folder" /></div>
      <div class="i {rail === 'scm' ? 'on' : ''}" title="Source Control" onclick={() => (rail = 'scm')}><Icon name="branch" /></div>
      <div class="i {rail === 'search' ? 'on' : ''}" title="Search (⌘⇧F)" onclick={() => (rail = 'search')}><Icon name="search" /></div>
      <div class="i {rail === 'agent' ? 'on' : ''}" title="AI Agent" onclick={() => (rail = 'agent')}><Icon name="agent" /></div>
      {#if railEnabled('devops', $extEnabled)}<div class="i {rail === 'k8s' ? 'on' : ''}" title="Kubernetes" onclick={() => (rail = 'k8s')}><Icon name="kube" /></div>{/if}
      {#if railEnabled('devops', $extEnabled)}<div class="i {rail === 'ci' ? 'on' : ''}" title="CI / Pipelines" onclick={() => (rail = 'ci')}><Icon name="ci" /></div>{/if}
      {#if railEnabled('devops', $extEnabled)}<div class="i {rail === 'terraform' ? 'on' : ''}" title="Terraform / Terragrunt" onclick={() => (rail = 'terraform')}><Icon name="terraform" /></div>{/if}
      {#if railEnabled('devops', $extEnabled)}<div class="i {rail === 'helm' ? 'on' : ''}" title="Helm" onclick={() => (rail = 'helm')}><Icon name="helm" /></div>{/if}
      {#if railEnabled('devops', $extEnabled)}<div class="i {rail === 'obs' ? 'on' : ''}" title="Observability (Metrics / Logs)" onclick={() => (rail = 'obs')}><Icon name="chart" /></div>{/if}
      {#if railEnabled('devops', $extEnabled)}<div class="i {rail === 'devops' ? 'on' : ''}" title="DevOps (PRs / GitLab / AWS / Incidents)" onclick={() => (rail = 'devops')}><Icon name="devops" /></div>{/if}
      <div class="i {rail === 'workspace' ? 'on' : ''}" title="Workspace (multipane)" onclick={() => (rail = 'workspace')}><Icon name="workspace" /></div>
      <div class="i grow {rail === 'settings' ? 'on' : ''}" title="Settings" onclick={openSettings}><Icon name="settings" /></div>
    </nav>
    {/if}

    {#if explorerOpen}
    <aside class="side">
      <div class="sect">Sessions <span class="n">{terms.length}</span></div>
      {#each terms as t (t.id)}
        <div class="row {activeTerm === t.id ? 'cur' : ''}" onclick={() => selectTerm(t.id)}>
          <span class="ic">›_</span>{t.title}
        </div>
      {/each}
      <div class="sect">Explorer <button class="sect-x" title="Hide explorer (⌘B)" onclick={() => (explorerOpen = false)}><Icon name="close" size={11} /></button></div>
      {#if cwd}<FileBrowser bind:path={cwd} onOpenFile={openInEditor} />{/if}
    </aside>
    {/if}

    <svelte:boundary onerror={(e) => { console.error("view crashed", e); toast("This view hit an error — use Reload view", "error"); }}>
    <section class="content">
      <div class="pane-head">
        {#if rail === "scm"}<span class="ph-ic accent"><Icon name="branch" /></span> Source Control — {baseName(cwd)}
        {:else if rail === "diff"}<span class="accent">±</span> Diff — {diffTarget?.rev ?? diffTarget?.path}
        {:else if rail === "search"}<span class="ph-ic accent"><Icon name="search" /></span> Search
        {:else if rail === "agent"}<span class="ph-ic accent"><Icon name="agent" /></span> Agent
        {:else if rail === "k8s"}<span class="ph-ic accent"><Icon name="kube" /></span> Kubernetes
        {:else if rail === "ci"}<span class="ph-ic accent"><Icon name="ci" /></span> CI / Pipelines
        {:else if rail === "terraform"}<span class="ph-ic accent"><Icon name="terraform" /></span> Terraform
        {:else if rail === "helm"}<span class="ph-ic accent"><Icon name="helm" /></span> Helm
        {:else if rail === "obs"}<span class="ph-ic accent"><Icon name="chart" /></span> Observability
        {:else if rail === "devops"}<span class="ph-ic accent"><Icon name="devops" /></span> DevOps
        {:else if rail === "workspace"}<span class="ph-ic accent"><Icon name="workspace" /></span> Workspace — {baseName(cwd)}
        {:else if rail === "settings"}<span class="ph-ic accent"><Icon name="settings" /></span> Settings
        {:else if rail === "files"}<span class="ph-ic accent"><Icon name="folder" /></span> Explorer
        {:else if rail === "editor"}<span class="accent"></span> {activeFile || "Welcome"}
        {:else}<span class="ph-ic accent"><Icon name="terminal" /></span> {terms.find((t) => t.id === activeTerm)?.title ?? "zsh"}{/if}
      </div>

      <div class="term-row" style:display={rail === "term" ? "flex" : "none"}>
        {#each terms as t (t.id)}
          {@const shown = t.id === activeTerm || t.id === splitTerm}
          <div
            class="term-wrap"
            style:display={shown ? "block" : "none"}
            style:flex={shown ? "1" : "0"}
            style:border-left={splitTerm && t.id === splitTerm ? "1px solid var(--border)" : "none"}
            onclickcapture={() => (activeTerm = t.id)}
            role="presentation"
          >
            <Terminal id={t.id} {cwd} shell={t.shell ?? ""} active={rail === "term" && shown} />
          </div>
        {/each}
      </div>

      {#if rail === "diff" && diffTarget}
        <div class="view">
          <div class="difftop"><button class="back" onclick={() => (rail = "scm")}>← Source Control</button></div>
          {#key JSON.stringify(diffTarget)}
            {#if diffTarget.rev && diffTarget.path}{#await DiffView() then M}<M.default {cwd} rev={diffTarget.rev} path={diffTarget.path} />{/await}
            {:else if diffTarget.rev}<CommitDetail {cwd} rev={diffTarget.rev} />
            {:else}{#await DiffView() then M}<M.default {cwd} path={diffTarget.path} staged={diffTarget.staged} />{/await}{/if}
          {/key}
        </div>
      {:else if rail === "editor" && activeFile}
        <div class="view">
          {#if runbook && isMarkdown(activeFile)}
            {#key activeFile}{#await RunbookView() then M}<M.default path={activeFile} onRun={(c) => { invoke("pty_write", { id: activeTerm, data: c + "\n" }); }} />{/await}{/key}
          {:else if mdPreview && isMarkdown(activeFile)}
            {#key activeFile}{#await MarkdownPreview() then M}<M.default path={activeFile} />{/await}{/key}
          {:else if isNonText(activeFile)}
            {#key activeFile}{#await FileView() then M}<M.default path={activeFile} />{/await}{/key}
          {:else}
            {#await Editor() then M}
              <M.default path={activeFile} onDirty={(d) => (dirtyFiles = { ...dirtyFiles, [activeFile]: d })} onOpen={(np, ln) => { openInEditor(np); if (ln) editorGoto.set(ln); }} onReferences={showReferences} onExplain={explainCode} />
            {/await}
          {/if}
        </div>
      {:else if rail === "workspace"}
        <div class="view ws">
          {#snippet paneView(lf: Leaf)}
            {#key lf.tabs[lf.active]?.id}
            {#if lf.view === "term"}
              <Terminal id={lf.ref ?? lf.id} {cwd} active={rail === "workspace"} />
            {:else if lf.view === "files"}
              {#key cwd}<FileBrowser bind:path={cwd} onOpenFile={openInEditor} />{/key}
            {:else if lf.view === "scm"}
              {#key cwd}{#await SourceControl() then M}<M.default {cwd} onOpenDiff={(t) => { diffTarget = t; }} />{/await}{/key}
            {:else if lf.view === "search"}
              {#key cwd}{#await SearchPanel() then M}<M.default root={cwd} onOpen={(p) => openInEditor(p)} />{/await}{/key}
            {:else if lf.view === "agent"}
              {#await AgentPanel() then M}<M.default {cwd} attachPath={activeFile}
                listFiles={() => invoke<string[]>("walk_dir", { root: cwd.replace(/\/$/, "") })}
                onReadFile={(p) => invoke<string>("read_file", { path: p })}
                onApplyFile={(path, content) => { invoke("write_file", { path, contents: content }); }}
                getTerminalText={() => readTerminal(activeTerm)}
                onRunCommand={(c) => invoke("pty_write", { id: activeTerm, data: c + "\n" })} />{/await}
            {:else if lf.view === "devops"}
              {#key cwd}{#await DevOps() then M}<M.default {cwd} onRunCommand={(c) => invoke("pty_write", { id: activeTerm, data: c + "\n" })} />{/await}{/key}
            {:else if lf.view === "editor" && (lf.ref || activeFile)}
              {@const p = lf.ref || activeFile}
              {#if isNonText(p)}
                {#key p}{#await FileView() then M}<M.default path={p} />{/await}{/key}
              {:else}
                {#await Editor() then M}<M.default path={p} onDirty={(d) => (dirtyFiles = { ...dirtyFiles, [p]: d })} onOpen={(np, ln) => { openInEditor(np); if (ln) editorGoto.set(ln); }} onReferences={showReferences} onExplain={explainCode} />{/await}
              {/if}
            {:else}
              <div class="ws-empty">Pick a view ↑ or open a file</div>
            {/if}
            {/key}
          {/snippet}
          <PaneGrid node={paneTree} view={paneView} drag={paneDrag} activeId={activeLeaf}
            onSplit={wsSplit} onClose={wsClose} onSetView={wsSetView} onResize={wsResize} onDock={wsDock}
            onSetActiveTab={wsSetActiveTab} onCloseTab={wsCloseTab} onAddTab={wsAddTab}
            extDrag={tabDragView} onDropExternal={wsDropTab} zoomId={zoomedLeaf} dim={$focusDimming}
            onFocusLeaf={(id) => (activeLeaf = id)}
            onDragStart={(id) => (paneDrag = { id })} onDragEnd={() => (paneDrag = { id: null })} />
        </div>
      {:else if rail === "settings"}
        <div class="view">{#await Settings() then M}<M.default />{/await}</div>
      {:else if rail === "editor" || rail === "files"}
        <div class="view"><Welcome recent={recentFiles} onOpenRecent={openInEditor} onNewTerminal={newTerm} onCommandPalette={openCommands} /></div>
      {/if}

      <!-- Heavy rail views: mounted on first visit, then shown/hidden (no re-fetch on switch). -->
      {#if mountedRails.scm}
        <div class="view" style:display={rail === "scm" ? "block" : "none"}>{#key cwd}{#await SourceControl() then M}<M.default {cwd} onOpenDiff={(t) => { diffTarget = t; rail = "diff"; }} />{/await}{/key}</div>
      {/if}
      {#if mountedRails.search}
        <div class="view" style:display={rail === "search" ? "block" : "none"}>{#key cwd}{#await SearchPanel() then M}<M.default root={cwd} onOpen={(p) => openInEditor(p)} />{/await}{/key}</div>
      {/if}
      {#if mountedRails.k8s}
        <div class="view" style:display={rail === "k8s" ? "block" : "none"}>{#key cwd}{#await Kube() then M}<M.default {cwd} onRunCommand={sendToTerm} />{/await}{/key}</div>
      {/if}
      {#if mountedRails.ci}
        <div class="view" style:display={rail === "ci" ? "block" : "none"}>{#key cwd}{#await CI() then M}<M.default {cwd} active={rail === "ci"} onRunCommand={sendToTerm} />{/await}{/key}</div>
      {/if}
      {#if mountedRails.terraform}
        <div class="view" style:display={rail === "terraform" ? "block" : "none"}>{#key cwd}{#await Terraform() then M}<M.default {cwd} onRunCommand={sendToTerm} />{/await}{/key}</div>
      {/if}
      {#if mountedRails.helm}
        <div class="view" style:display={rail === "helm" ? "block" : "none"}>{#await Helm() then M}<M.default />{/await}</div>
      {/if}
      {#if mountedRails.obs}
        <div class="view" style:display={rail === "obs" ? "block" : "none"}>{#await Observability() then M}<M.default />{/await}</div>
      {/if}
      {#if mountedRails.devops}
        <div class="view" style:display={rail === "devops" ? "block" : "none"}>{#key cwd}{#await DevOps() then M}<M.default {cwd} onRunCommand={sendToTerm} />{/await}{/key}</div>
      {/if}

      <!-- Agent stays mounted so a request keeps running after you switch views. -->
      <div class="view" style:display={rail === "agent" ? "block" : "none"}>
        {#await AgentPanel() then M}<M.default
          {cwd}
          attachPath={activeFile}
          listFiles={() => invoke<string[]>("walk_dir", { root: cwd.replace(/\/$/, "") })}
          onReadFile={(p) => invoke<string>("read_file", { path: p })}
          onApplyFile={(path, content) => { invoke("write_file", { path, contents: content }); toast(`Applied edit to ${path.split("/").pop()}`, "success"); }}
          getTerminalText={() => readTerminal(activeTerm)}
          onRunCommand={(cmd) => { invoke("pty_write", { id: activeTerm, data: cmd + "\n" }); rail = "term"; toast("Command sent to terminal", "success"); }}
          onReply={(summary) => { if (rail !== "agent" || document.hidden) notifyAgent(summary); }}
        />{/await}
      </div>

      {#if bottomDock}
        <div class="bdock" style:height="{dockH}px">
          <div class="bdock-resize" onpointerdown={startDockResize} role="separator" tabindex="-1" aria-label="Resize terminal"></div>
          <div class="bdock-head">
            <button class="bdock-tab" class:on={dockTab === "term"} onclick={() => (dockTab = "term")}>
              <Icon name="terminal" size={12} /> Terminal
            </button>
            <button class="bdock-tab" class:on={dockTab === "problems"} onclick={() => (dockTab = "problems")}>
              <Icon name="alert" size={12} /> Problems{#if $problems.length} <span class="bdock-badge">{$problems.length}</span>{/if}
            </button>
            <span class="x" onclick={() => (bottomDock = false)} title="Close (⌘J)">×</span>
          </div>
          <div class="bdock-term" class:hidden={dockTab !== "term"}><Terminal id="dock" {cwd} active={bottomDock && dockTab === "term"} /></div>
          {#if dockTab === "problems"}
            <div class="bdock-term"><Problems onOpen={(p, ln) => { openInEditor(p); editorGoto.set(ln); }} /></div>
          {/if}
        </div>
      {/if}
    </section>
    {#snippet failed(error, reset)}
      <section class="content">
        <div class="crash-fallback">
          <div class="cf-title">This view hit an error.</div>
          <div class="cf-msg">{String(error)}</div>
          <button class="cf-btn" onclick={reset}>Reload view</button>
        </div>
      </section>
    {/snippet}
    </svelte:boundary>
  </div>

  <div class="status">
    <span class="si"><Icon name="branch" size={12} /> {branch || "—"}{#if aheadBehind} <span class="ab">↑{aheadBehind.a} ↓{aheadBehind.b}</span>{/if}</span>
    <span title={cwd}>{baseName(cwd) || "~"}</span>
    <div class="r">
      <span class="si" onclick={toggleDensity} title="Toggle density" style="cursor:default">{$density}</span>
      <span class="si" onclick={cycleTheme} title="Cycle theme" style="cursor:default">{themeLabel($activeTheme)}</span>
      <span class="ok" title="Ready">●</span>
      <span>UTF-8</span>
    </div>
  </div>

  {#if zen}<div class="zen-exit" onclick={() => toggleZen()} role="button" tabindex="-1" title="Exit zen mode (⌘.)">⌘. exit zen</div>{/if}
  <Palette bind:open={paletteOpen} items={paletteItems} placeholder={palettePlaceholder} />
  <Dialog />
  <Toasts />

  {#if !onboarded}
    <div class="onboard-scrim" role="presentation">
      <div class="onboard">
        <h2 class="ob-h">{TOUR[obStep].title}</h2>
        <p class="ob-tag">{TOUR[obStep].body}</p>
        <ul class="ob-tips">
          {#each TOUR[obStep].tips as t}<li>{@html t}</li>{/each}
        </ul>
        <div class="ob-step">Step {obStep + 1} of {TOUR.length}</div>
        <div class="ob-nav">
          <button class="ob-skip" onclick={dismissOnboard}>Skip</button>
          <span style="flex:1"></span>
          {#if obStep > 0}<button class="ob-back" onclick={tourBack}>Back</button>{/if}
          <button class="ob-go" onclick={tourNext}>{obStep === TOUR.length - 1 ? "Get started" : "Next"}</button>
        </div>
      </div>
    </div>
  {/if}

  {#if whatsNew && onboarded}
    <WhatsNew version={WHATS_NEW_VERSION} notes={WHATS_NEW_NOTES} onClose={dismissWhatsNew} />
  {/if}

  {#if keymapOpen}
    <Keymap onClose={() => (keymapOpen = false)} />
  {/if}

  {#if mergeView}
    <MergeView3 {cwd} path={mergeView} onClose={() => (mergeView = null)} onResolved={() => toast("Conflict resolved + staged", "success")} />
  {/if}

  {#if rebaseTarget}
    <RebasePlan {cwd} target={rebaseTarget} onClose={() => (rebaseTarget = null)} onDone={() => toast("Rebase complete", "success")} />
  {/if}

  {#if quakeOpen}
    <div class="quake-scrim" role="presentation" onclick={() => (quakeOpen = false)}></div>
    <div class="quake">
      <div class="quake-head"><span>Quake terminal</span><span class="quake-hint">Ctrl+` to toggle · Esc to close</span></div>
      <div class="quake-term"><Terminal id="quake" {cwd} active={quakeOpen} /></div>
    </div>
  {/if}

  {#if fpsOn}
    <div class="fps-overlay">{fps} fps · {frameMs}ms</div>
  {/if}

  {#if $agentQueue.length}
    <button class="agentq-chip" title="Queued agent tasks — click to run the next" onclick={() => { const t = dequeueAgent(); if (t) { agentSeed.set(t.prompt); rail = "agent"; } }}>⚙ {$agentQueue.length} queued</button>
  {/if}

  {#if !$online}
    <div class="offline-chip" title="No network — agent, k8s, and observability calls will fail until you reconnect">⚠ offline — network features paused</div>
  {/if}

  {#if $broadcastInput}
    <button class="broadcast-chip" title="Broadcast input is ON — every keystroke goes to all terminals" onclick={() => broadcastInput.set(false)}>
      ⊚ broadcast · click to stop
    </button>
  {/if}
</div>

<style>
  .pane-head .ph-ic { display: inline-flex; align-items: center; vertical-align: -2px; margin-right: 3px; }
  .status .si { display: inline-flex; align-items: center; gap: 4px; }
  .view { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  /* Bottom terminal dock (⌘J) — sits under the active view. */
  .bdock { flex: 0 0 auto; display: flex; flex-direction: column; min-height: 0;
    border-top: 1px solid var(--border); background: var(--bg); position: relative; }
  .bdock-resize { position: absolute; top: -3px; left: 0; right: 0; height: 6px; cursor: row-resize; z-index: 5; }
  .bdock-head { display: flex; align-items: center; gap: 2px;
    padding: 2px 8px; border-bottom: 1px solid var(--border); flex: 0 0 auto; }
  .bdock-tab { display: inline-flex; align-items: center; gap: 6px; font-size: 10px; border: 0;
    background: transparent; color: var(--text3); text-transform: uppercase; letter-spacing: 0.07em;
    font-weight: 600; cursor: default; padding: 4px 10px; border-radius: 5px; }
  .bdock-tab:hover { color: var(--text2); }
  .bdock-tab.on { color: var(--text); background: var(--sel); }
  .bdock-badge { font-family: var(--font-mono); font-size: 9px; padding: 0 4px; border-radius: 7px;
    background: color-mix(in srgb, var(--text) 12%, transparent); letter-spacing: 0; }
  .bdock-head .x { margin-left: auto; cursor: default; color: var(--text3); font-size: 15px; line-height: 1; padding: 0 4px; }
  .bdock-head .x:hover { color: var(--text); }
  .bdock-term { flex: 1; min-height: 0; padding: 4px 8px; }
  .bdock-term.hidden { display: none; }
  .view.ws { padding: 6px; gap: 0; }
  .ws-empty { display: flex; align-items: center; justify-content: center; height: 100%; color: var(--text3); font-size: 12.5px; }
  .term-row { flex: 1; min-height: 0; }
  .hint { padding: 24px; color: var(--text3); }
  .difftop { padding: 6px 12px; border-bottom: 1px solid var(--border); flex: 0 0 auto; }
  .back { border: 0; background: transparent; color: var(--accent); font-size: 12px; cursor: default; }
  .tab .x { margin-left: 8px; color: var(--text3); font-size: 13px; }
  .tab .x:hover { color: var(--text); }
  .dirty { display: inline-block; width: 7px; height: 7px; margin-left: 7px; border-radius: 50%;
    background: var(--accent); vertical-align: middle; }
  .newtab { display: flex; align-items: center; padding: 0 12px; color: var(--text3); font-size: 16px;
    -webkit-app-region: no-drag; cursor: default; }
  .newtab:hover { color: var(--text); }
</style>
