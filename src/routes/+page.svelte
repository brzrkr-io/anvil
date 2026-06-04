<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Terminal from "$lib/Terminal.svelte";
  import Problems from "$lib/Problems.svelte";
  const SourceControl = () => import("$lib/SourceControl.svelte");
  import FileBrowser from "$lib/FileBrowser.svelte";
  import Resizer from "$lib/Resizer.svelte";
  // Editor + DiffView pull in Monaco (~4 MB); load them lazily on first use
  // so app startup stays fast (#90).
  const Editor = () => import("$lib/Editor.svelte");
  const FileView = () => import("$lib/FileView.svelte");
  const MarkdownPreview = () => import("$lib/MarkdownPreview.svelte");
  const WebPreview = () => import("$lib/WebPreview.svelte");
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
    focusTerm();
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
  import { installCrashHandlers, getCrashes, clearCrashes, diagnosticsReport, originFrame } from "$lib/crash";
  import { redactionRules, addRedactionRule, removeRedactionRule, getAuditLog, clearAuditLog } from "$lib/redaction";
  import { getUserSnippets, addUserSnippet, removeUserSnippet } from "$lib/user-snippets";
  // Settings is a large surface and never the startup view — load on demand (#74).
  const Settings = () => import("$lib/Settings.svelte");
  // DevOps (kubectl/CI) panes aren't the startup view; load them
  // lazily so initial render stays cheap (#92).
  const DevOps = () => import("$lib/DevOps.svelte");
  const Kube = () => import("$lib/Kube.svelte");
  const CI = () => import("$lib/CI.svelte");
  const Terraform = () => import("$lib/Terraform.svelte");
  const Observability = () => import("$lib/Observability.svelte");

  // Unified tabs: any path that shows a rail view (rail icon, palette, shortcut,
  // diff-back) registers it as a closable tab. Single source of truth so we don't
  // touch every call site. VIEW_META is defined below; referenced lazily here.
  $effect(() => { if (VIEW_META[rail] && !viewTabs.includes(rail)) viewTabs = [...viewTabs, rail]; });
  function sendToTerm(cmd: string) {
    invoke("pty_write", { id: activeTerm, data: cmd + "\n" });
    focusTerm();
    toast("Sent to terminal", "info");
  }
  // Command snippets (roadmap E41): save / manage reusable terminal commands.
  function saveSnippetFlow() {
    const label = prompt("Snippet name:"); if (!label) return;
    const command = prompt("Command:"); if (!command) return;
    addSnippet(label, command);
    toast(`Saved snippet "${label}"`, "success");
  }
  function manageSnippetsFlow() {
    palettePlaceholder = "Remove a snippet";
    paletteItems = getSnippets().map((s) => ({ label: s.label, hint: "✕ remove", run: () => { removeSnippet(s.id); toast("Removed", "success"); } }));
    paletteOpen = true;
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
  import { leaf, splitLeaf, closeLeaf, resizeSplit, setView as setLeafView, paneId, remapTermRefs, seedPaneSeq, firstLeaf, findLeaf, balanceTree, closeOthers as closeOtherPanes, leafIds, addTab, setActiveTab, closeTab, terminalRefs, type PaneNode, type Leaf, type ViewKind, type Edge } from "$lib/panes";
  import { edgeFromRect, passedThreshold, dropAction, type TabDrag } from "$lib/tabdrag";
  import Palette, { type Item } from "$lib/Palette.svelte";
  import Toasts from "$lib/Toasts.svelte";
  import NotificationCenter from "$lib/NotificationCenter.svelte";
  import Dialog from "$lib/Dialog.svelte";
  import Welcome from "$lib/Welcome.svelte";
  import Skeleton from "$lib/Skeleton.svelte";
  import { askText } from "$lib/dialog";
  import { getProfiles, saveProfile, setActiveProfile, profileSummary, type EnvProfile } from "$lib/profiles";
  import WhatsNew from "$lib/WhatsNew.svelte";
  import Keymap from "$lib/Keymap.svelte";
  import Doctor from "$lib/Doctor.svelte";
  import Attention from "$lib/Attention.svelte";
  import MergeView3 from "$lib/MergeView3.svelte";
  import RebasePlan from "$lib/RebasePlan.svelte";
  import { toast, notifications } from "$lib/toast";
  import { get } from "svelte/store";
  import { activeTheme, initTheme, cycleTheme, applyTheme, themeLabel } from "$lib/themes";
  import { density, initDensity, toggleDensity, applyDensity, type Density } from "$lib/density";
  import { initScale } from "$lib/scale";
  import { bumpTermFontSize, setTermFontSize } from "$lib/terminal-settings";
  // ⌘+/−/0 = content text size: bump the editor + terminal font together (like
  // VS Code/Zed). It does NOT scale chrome or the Settings/Explorer sidebars.
  const CONTENT_FS = 13;
  function zoomContent(dir: number) { bumpEditorFontSize(dir); bumpTermFontSize(dir); }
  function zoomContentReset() { setEditorFontSize(CONTENT_FS); setTermFontSize(CONTENT_FS); }
  import { initOpacity } from "$lib/window-opacity";
  import { initFonts } from "$lib/fonts";
  import { autoHideRail, focusDimming, toggleFocusDimming, terminalAutoCd, toggleTerminalAutoCd } from "$lib/layout-settings";
  import Icon from "$lib/Icon.svelte";
  import { bumpEditorFontSize, setEditorFontSize, editorGoto, editorFormatOnSave, editorTabSize, editorWordWrap, editorGhostText, toggleGhostText, editorGhostSource, setGhostSource } from "$lib/editor-settings";
  import { lspLang, ensureLsp, lspStatus, restartLsp } from "$lib/lsp";
  // Server label per language for the status-bar LSP indicator.
  const LSP_LABEL: Record<string, string> = {
    rust: "rust-analyzer", go: "gopls", typescript: "tsserver", python: "pyright",
    cpp: "clangd", terraform: "terraform-ls", yaml: "yaml-ls", json: "json-ls",
    shellscript: "bash-ls", lua: "lua-ls", dockerfile: "docker-ls",
  };
  // cm-lsp statically pulls the whole CodeMirror vendor chunk (~1 MB). Load it
  // lazily so it stays out of the cold-start graph — Editor is already lazy via
  // {#await}, and only the symbol-search/breadcrumb handlers below need it.
  const loadCmLsp = () => import("$lib/cm-lsp");
  import { problems } from "$lib/diagnostics";
  import { railEnabled, extEnabled } from "$lib/extensions";
  import { agentSeed, agentInvestigate } from "$lib/agent-seed";
  import { getSnippets, addSnippet, removeSnippet } from "$lib/snippets";
  import { integrationFor, type IntegrationShell } from "$lib/shell-integration";
  import { rankItems, withTracking } from "$lib/palette-rank";
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

  // Whole-app-as-workspace (Zed model): the multipane grid is the permanent
  // content surface. `rail` starts on — and overwhelmingly stays — "workspace";
  // only modal-ish surfaces (settings/diff/panel) ever set it elsewhere.
  let rail = $state("workspace");
  // Flux failing-object count, surfaced on the k8s rail icon so trouble is
  // visible without opening the panel. Reflects the Flux panel's active tab.
  let kubeFails = $state(0);
  // Agent-driven ops: a failing resource seeds a live, gated investigation.
  function investigate(prompt: string) { agentInvestigate.set(prompt); openView("agent"); }
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

  // Per-window session keys so multiple windows (⌘N) don't clobber each other's
  // layout in the shared (per-origin) localStorage or the state file. Primary
  // window keeps the legacy key for back-compat.
  const winLabel = (() => {
    try { return (window as any).__TAURI_INTERNALS__ ? getCurrentWindow().label : "main"; } catch { return "main"; }
  })();
  const sessionKey = winLabel === "main" ? "anvil-session" : `anvil-session:${winLabel}`;

  let settingsOpen = $state(false);
  let zen = $state(false);
  // Zen is a distraction-free terminal: entering focuses a terminal pane in the
  // grid (creating one if none exists); exiting just drops the chrome-hiding flag.
  function toggleZen() {
    if (zen) {
      zen = false;
    } else {
      focusTerm();
      zen = true;
    }
  }
  function openSettings() { settingsOpen = true; rail = "settings"; explorerOpen = false; }
  // Explorer is a persistent left panel, independent of the main view (#74): it
  // stays open while you're in the editor/terminal instead of being a `rail`
  // mode that other views replace.
  let explorerOpen = $state(false);
  // Resizable explorer/sessions sidebar width (persisted).
  let sideW = $state((() => { try { return Number(localStorage.getItem("anvil-side-w")) || 230; } catch { return 230; } })());
  function toggleSide() { explorerOpen = !explorerOpen; }
  // The grid is always the IDE surface now, so the Explorer is just a toggleable
  // left dock alongside it.
  function openExplorer() {
    explorerOpen = !explorerOpen;
    // Opening the tree lands the focused grid pane on the editor — the last open
    // file, or the anvil splash when none. Route straight to the grid (not
    // openView, whose non-workspace path would push "editor" into viewTabs and
    // crash the tab render, since "editor" has no VIEW_META entry).
    if (explorerOpen) {
      settingsOpen = false;
      rail = "workspace";
      if (findLeaf(paneTree, activeLeaf)) wsSetView(activeLeaf, "editor");
    }
  }
  async function newRootFile() {
    const name = await askText({ title: "New file", placeholder: "name.ext" });
    if (!name) return;
    const path = `${cwd.replace(/\/$/, "")}/${name}`;
    await invoke("create_path", { path, isDir: false }).catch(() => {});
    explorerOpen = true;
    openInEditor(path);
  }
  async function newRootFolder() {
    const name = await askText({ title: "New folder" });
    if (!name) return;
    await invoke("create_path", { path: `${cwd.replace(/\/$/, "")}/${name}`, isDir: true }).catch(() => {});
    explorerOpen = true;
  }

  // #4 Per-environment profiles: switch kube context + namespace + AWS profile in
  // one move so every surface follows the same environment.
  async function applyProfile(p: EnvProfile) {
    try {
      if (p.kubeContext) await invoke("kube_use_context", { name: p.kubeContext });
      if (p.namespace) await invoke("kube_set_namespace", { namespace: p.namespace });
      if (p.awsProfile) await invoke("set_aws_profile", { profile: p.awsProfile });
      void refreshKubeCtx();
      setActiveProfile(p.name);
      toast(`Environment → ${p.name}${profileSummary(p) ? ` (${profileSummary(p)})` : ""}`, "success");
    } catch (e) {
      toast("Profile switch failed: " + String(e).slice(0, 80), "error");
    }
  }
  function switchProfilePalette() {
    const profs = getProfiles();
    if (!profs.length) { toast("No environment profiles yet — save one first", "info"); return; }
    palettePlaceholder = "Switch environment…";
    paletteItems = profs.map((p) => ({ label: p.name, hint: profileSummary(p), run: () => applyProfile(p) }));
    paletteOpen = true;
  }
  async function saveCurrentProfile() {
    const name = await askText({ title: "Save environment profile", placeholder: "prod / stage / dev" });
    if (!name) return;
    const [ctx, ns] = await Promise.allSettled([
      invoke<string>("kube_current_context"),
      invoke<string>("kube_current_namespace"),
    ]);
    const aws = typeof localStorage !== "undefined" ? localStorage.getItem("anvil-acct-aws-profile") || undefined : undefined;
    saveProfile({
      name,
      kubeContext: ctx.status === "fulfilled" ? ctx.value.trim() || undefined : undefined,
      namespace: ns.status === "fulfilled" ? ns.value.trim() || undefined : undefined,
      awsProfile: aws,
    });
    setActiveProfile(name);
    toast(`Saved environment profile “${name}”`, "success");
  }

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
    { title: "Welcome to anvil", body: "The AI-native console for 100% of your work — terminal, editor, git, and DevOps in one native surface.", tips: ["Press <kbd>⌘K</kbd> any time — every action lives in the command palette."] },
    { title: "Code & terminal together", body: "Open files alongside live shells.", tips: ["<kbd>⌘O</kbd> open file · <kbd>⌘⇧O</kbd> open folder", "<kbd>⌘T</kbd> new terminal · <kbd>⌘J</kbd> terminal under your editor", "<kbd>⌘\\</kbd> split workspace panes · drag a tab onto a pane edge"] },
    { title: "Git & DevOps built in", body: "Source Control has a Terax-style commit panel, swimlane history, and per-hunk staging. The k8s, Terraform, and CI surfaces sort failing-first — broken Flux reconciles, drifted stacks, and red checks float to the top so you see what needs attention without scanning.", tips: ["The <kbd>gen</kbd> button writes your commit message from the staged diff.", "Watch for the red count badge on the Kubernetes rail icon."] },
    { title: "Your AI agent drives ops", body: "On any failing resource — a Flux reconcile, a Terraform plan, a PR with red CI — hit Investigate. The agent runs the right diagnostic, reads it, and proposes a fix you approve. It never mutates on its own; every command is approval-gated.", tips: ["<kbd>⌘I</kbd> ask the agent · <kbd>⌘,</kbd> Settings for themes, fonts, keymap.", "Tool results are treated as untrusted — the agent won't act on instructions hidden in command output."] },
  ];
  // Finishing the tour drops straight into the Connections doctor so a new user's
  // first action is checking their tools/auth are wired for their environment.
  function tourNext() { if (obStep < TOUR.length - 1) obStep += 1; else { dismissOnboard(); doctorOpen = true; } }
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
  let doctorOpen = $state(false);
  let attentionOpen = $state(false);
  let notifOpen = $state(false);
  const unreadCount = $derived($notifications.filter((n) => !n.read).length);
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
        closeActiveTab();
        break;
      case "close-window": getCurrentWindow().close(); break;
      case "settings": openSettings(); break;
      case "palette": openCommands(); break;
      case "goto-file": openFilesPalette(); break;
      case "toggle-sidebar": toggleSide(); break;
      case "zen": toggleZen(); break;
      case "zoom-in": zoomContent(1); break;
      case "zoom-out": zoomContent(-1); break;
      case "zoom-reset": zoomContentReset(); break;
    }
  }

  // ── Dockable workspace (multipane) ──
  let paneTree = $state<PaneNode>(leaf("term", paneId("wt")));

  // ── Pointer-based tab drag-to-split (IDE-grade; #4/#5) ─────────────────────
  // HTML5 drag-and-drop silently no-ops in the app's WebView, so dragging a tab
  // onto a pane uses pointerdown/move/up + elementFromPoint hit-testing instead.
  // Pure math + the drop decision live in tabdrag.ts; this is the DOM glue.
  let tabDrag = $state<TabDrag | null>(null);       // the live drag (past threshold)
  let dragXY = $state<{ x: number; y: number }>({ x: 0, y: 0 }); // ghost position
  let dropHint = $state<{ leafId: string; edge: Edge } | null>(null); // pane+edge under cursor
  let reorderTo = $state<string | null>(null); // file-tab strip reorder target (no pane hit)

  function leafRectAt(x: number, y: number): { leafId: string; rect: DOMRect } | null {
    const el = document.elementFromPoint(x, y)?.closest("[data-leaf-id]") as HTMLElement | null;
    const id = el?.dataset.leafId;
    return el && id ? { leafId: id, rect: el.getBoundingClientRect() } : null;
  }

  // Start a drag from any tab (top-strip file/view OR a pane's own tab). `from`
  // marks a pane tab so a drop can MOVE it; absent → it came from the top strip.
  function startTabDrag(e: PointerEvent, payload: TabDrag) {
    if (e.button !== 0) return;
    const startX = e.clientX, startY = e.clientY;
    let active = false;
    // Coalesce the per-move hit-test (elementFromPoint + getBoundingClientRect)
    // to one update per animation frame, matching the splitter-resize pattern in
    // PaneGrid — a fast drag fires dozens of pointermoves per frame, but the
    // ghost/dropzone only repaint once per frame anyway.
    let curX = startX, curY = startY;
    let raf = 0;
    const flush = () => {
      raf = 0;
      dragXY = { x: curX, y: curY };
      const hit = leafRectAt(curX, curY);
      if (hit) {
        dropHint = { leafId: hit.leafId, edge: edgeFromRect(hit.rect, curX, curY) };
        reorderTo = null;
      } else {
        dropHint = null;
        // Not over a pane → maybe over another file tab (strip reorder).
        const tabEl = document.elementFromPoint(curX, curY)?.closest("[data-file-tab]") as HTMLElement | null;
        reorderTo = !payload.from ? (tabEl?.dataset.fileTab ?? null) : null;
      }
    };
    const move = (ev: PointerEvent) => {
      if (!active) {
        if (!passedThreshold(startX, startY, ev.clientX, ev.clientY)) return;
        active = true;
        tabDrag = payload; // commit the drag only past the click/drag threshold
      }
      ev.preventDefault(); // suppress text selection once dragging
      curX = ev.clientX; curY = ev.clientY;
      if (!raf) raf = requestAnimationFrame(flush);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      if (raf) { cancelAnimationFrame(raf); raf = 0; }
      if (active) {
        flush(); // settle on the final pointer position before resolving the drop
        if (dropHint) applyTabDrop(payload, dropHint);
        else if (reorderTo && payload.ref) openFiles = reorder(openFiles, payload.ref, reorderTo);
      }
      tabDrag = null; dropHint = null; reorderTo = null;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function applyTabDrop(drag: TabDrag, hint: { leafId: string; edge: Edge }) {
    const act = dropAction(drag, hint);
    rail = "workspace";
    switch (act.kind) {
      case "addTab":
        paneTree = addTab(paneTree, act.leafId, drag.view, paneRef(drag.view, drag.ref));
        activeLeaf = act.leafId;
        break;
      case "moveTab": {
        paneTree = addTab(paneTree, act.to, drag.view, drag.ref);
        paneTree = closeTab(paneTree, act.from.leafId, act.from.index);
        activeLeaf = act.to;
        break;
      }
      case "split": {
        const r = splitLeaf(paneTree, act.leafId, act.edge, drag.view, paneRef(drag.view, drag.ref));
        paneTree = r.tree;
        activeLeaf = r.newLeafId;
        break;
      }
      case "splitFrom": {
        const r = splitLeaf(paneTree, act.leafId, act.edge, drag.view, drag.ref);
        paneTree = closeTab(r.tree, act.from.leafId, act.from.index);
        activeLeaf = r.newLeafId;
        break;
      }
    }
  }

  // A pane's own tab begins a drag (drag-to-split/move into another pane).
  function paneTabPointerDown(e: PointerEvent, leafId: string, index: number) {
    const lf = findLeaf(paneTree, leafId);
    const t = lf?.tabs[index];
    if (!t) return;
    const label = t.view === "editor" && t.ref ? (t.ref.split("/").pop() ?? "Editor") : (VIEW_LABEL[t.view] ?? t.view);
    startTabDrag(e, { view: t.view, ref: t.ref, label, from: { leafId, index } });
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
    try { paneTree = remapTermRefs(JSON.parse(txt)); seedPaneSeq(paneTree); rail = "workspace"; toast("Layout imported", "success"); }
    catch { toast("Invalid layout JSON", "error"); }
  }
  function openLayoutPalette() {
    const names = Object.keys(layoutPresets);
    if (!names.length) { toast("No saved layouts yet", "info"); return; }
    palettePlaceholder = "Load workspace layout…";
    paletteItems = names.map((n) => ({
      label: n,
      run: () => { try { paneTree = remapTermRefs(layoutPresets[n]); seedPaneSeq(paneTree); rail = "workspace"; } catch { /* ignore */ } },
    }));
    paletteOpen = true;
  }
  let activeLeaf = $state("");
  let zoomedLeaf = $state<string | null>(null);
  // Keep activeLeaf valid as the tree changes.
  $effect(() => { if (!findLeaf(paneTree, activeLeaf)) activeLeaf = firstLeaf(paneTree).id; });
  // Drop the zoom if its pane is gone.
  $effect(() => { if (zoomedLeaf && !findLeaf(paneTree, zoomedLeaf)) zoomedLeaf = null; });
  // #99 PTY lifecycle reconciler: the workspace owns when a shell dies, not the
  // <Terminal> component. On every tree change, kill the PTYs whose terminal id
  // has LEFT the tree (an explicit tab/pane close). Ids that merely moved or got
  // backgrounded by a view-switch stay in the tree, so their shell survives and
  // re-attaches when the term tab is shown again.
  let liveTermRefs = new Set<string>();
  $effect(() => {
    const now = new Set(terminalRefs(paneTree));
    for (const ref of liveTermRefs) if (!now.has(ref)) invoke("pty_kill", { id: ref }).catch(() => {});
    liveTermRefs = now;
  });
  // Free this window's shells on close — the reconciler can't fire on teardown.
  onDestroy(() => { clearInterval(kctxTimer); for (const ref of liveTermRefs) invoke("pty_kill", { id: ref }).catch(() => {}); });
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
  function wsSetActiveTab(id: string, i: number) { paneTree = setActiveTab(paneTree, id, i); }
  function wsCloseTab(id: string, i: number) { paneTree = closeTab(paneTree, id, i); }
  function wsAddTab(id: string) { paneTree = addTab(paneTree, id, "term", paneId("wt")); }

  // Editor: multiple open files, one active, dirty tracked per path.
  let openFiles = $state<string[]>([]);
  let activeFile = $state("");
  // Active file's language-server status for the bottom status-bar indicator.
  const lspCurLang = $derived(activeFile ? lspLang(activeFile) : null);
  const lspCurState = $derived(lspCurLang ? ($lspStatus[lspCurLang] ?? "down") : null);
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
  let plusMenu = $state<{ x: number; y: number } | null>(null);

  // Terax-style openable panels: web preview / markdown / git history live as
  // real tabs in the tab bar (kept alive, draggable) alongside terminals & files.
  type PanelKind = "preview" | "markdown" | "githistory";
  let panels = $state<{ id: string; kind: PanelKind; title: string; url?: string; file?: string }[]>([]);
  let activePanel = $state("");
  const panelIcon = (k: PanelKind) => (k === "preview" ? "globe" : k === "markdown" ? "pencil" : "history");
  function openPanel(kind: PanelKind, opts: { title: string; url?: string; file?: string }) {
    seq += 1;
    const id = `p${seq}`;
    panels = [...panels, { id, kind, ...opts }];
    activePanel = id;
    rail = "panel";
  }
  function selectPanel(id: string) {
    activePanel = id;
    rail = "panel";
  }
  function closePanel(id: string) {
    panels = panels.filter((p) => p.id !== id);
    if (activePanel === id) {
      activePanel = panels.at(-1)?.id ?? "";
      rail = activePanel ? "panel" : "workspace";
    }
  }

  // Unified tab model (Terax-style): rail views (SCM/Search/Agent/DevOps/…) open as
  // closable tabs in the tab bar. `rail` stays the source of truth for what shows;
  // viewTabs tracks which rail views are pinned as tabs. Additive over the existing
  // keep-alive view mounts — nothing about those views changes. (Helm/Flux/Workloads
  // live inside the Kubernetes view, so there's no separate helm tab here.)
  const VIEW_META: Record<string, { title: string; icon: string }> = {
    scm: { title: "Source Control", icon: "branch" },
    search: { title: "Search", icon: "search" },
    agent: { title: "AI Agent", icon: "agent" },
    k8s: { title: "Kubernetes", icon: "kube" },
    ci: { title: "CI / Pipelines", icon: "ci" },
    terraform: { title: "Terraform", icon: "terraform" },
    obs: { title: "Observability", icon: "chart" },
    devops: { title: "DevOps", icon: "devops" },
  };
  // Display label for every ViewKind (drag-ghost / pane-tab text). VIEW_META only
  // covers rail views; this adds the file/terminal/explorer leaves.
  const VIEW_LABEL: Record<ViewKind, string> = {
    term: "Terminal", editor: "Editor", files: "Explorer", scm: "Source Control",
    search: "Search", agent: "AI Agent", devops: "DevOps", settings: "Settings",
    welcome: "Welcome", k8s: "Kubernetes", ci: "CI / Pipelines", terraform: "Terraform", obs: "Observability",
  };
  let viewTabs = $state<string[]>([]);
  // Views that can live inside a workspace pane (paneView renders these).
  const PANE_VIEWS = new Set(["term", "editor", "files", "scm", "search", "agent", "devops", "k8s", "ci", "terraform", "obs"]);
  function openView(kind: string) {
    if (kind === "k8s") void refreshKubeCtx();
    // Whole-app-as-workspace: when the grid is up, the rail drives the ACTIVE
    // pane instead of switching to a separate full-screen mode (no PTY churn).
    if (rail === "workspace" && PANE_VIEWS.has(kind) && findLeaf(paneTree, activeLeaf)) {
      const lf = findLeaf(paneTree, activeLeaf)!;
      const cur = lf.tabs[lf.active];
      // Switching a TERMINAL pane to another view keeps its shell alive: open the
      // view as a tab (or re-activate an existing one) instead of replacing the
      // terminal, so returning to the term tab re-attaches the same session (#99).
      if (cur?.view === "term" && kind !== "term") {
        const existing = lf.tabs.findIndex((t) => t.view === kind);
        paneTree = existing >= 0
          ? setActiveTab(paneTree, lf.id, existing)
          : addTab(paneTree, lf.id, kind as ViewKind, paneRef(kind as ViewKind));
      } else {
        wsSetView(activeLeaf, kind as ViewKind);
      }
      explorerOpen = false;
      return;
    }
    if (!viewTabs.includes(kind)) viewTabs = [...viewTabs, kind];
    rail = kind;
    // App views (SCM/Search/k8s/CI/…) take the full pane — the Explorer sidebar
    // is for file work (editor/terminal), so collapse it like every other view.
    explorerOpen = false;
  }
  function closeView(kind: string) {
    viewTabs = viewTabs.filter((k) => k !== kind);
    if (rail === kind) {
      const next = viewTabs.at(-1);
      if (next) rail = next;
      else if (panels.length) { activePanel = panels.at(-1)!.id; rail = "panel"; }
      else rail = "workspace";
    }
  }
  // Close whichever tab is currently active. Single source of truth for both the
  // ⌘W shortcut and the "close-tab" menu command so they can never drift.
  function closeActiveTab() {
    if (rail === "panel") { closePanel(activePanel); return; }
    if (rail === "diff") { rail = "workspace"; return; }
    if (rail === "settings") { settingsOpen = false; rail = "workspace"; return; }
    if (viewTabs.includes(rail)) { closeView(rail); return; }
    // Workspace grid: ⌘W closes the active file tab (mirrors clicking its ×).
    // Opening a file forces rail="workspace", so this — not a dead `editor`
    // branch — is what ⌘W actually hits when a file is open. After closing, show
    // the next open file in the focused pane, or drop the pane if none remain.
    if (activeFile) {
      const closing = activeFile;
      const lf = findLeaf(paneTree, activeLeaf);
      closeFile(closing);
      if (lf?.view === "editor" && lf.ref === closing) {
        // Show the next open file, or fall back to the anvil welcome splash
        // (recent files + shortcuts) when this was the last tab — never leave the
        // just-closed file rendering, and don't kill the pane.
        paneTree = setLeafView(paneTree, lf.id, "editor", activeFile || undefined);
      }
      return;
    }
    // No file tab open → close the focused grid pane (e.g. a terminal).
    wsClose(activeLeaf);
  }
  let recentFiles = $state<string[]>([]);
  let recentWorkspaces = $state<string[]>([]);
  let branch = $state("");
  // Active kubeconfig context, surfaced in the status bar so mutating the WRONG
  // cluster is unmissable (#9/#28). Refreshed on mount, on env-profile switch, on
  // opening the k8s view, and on a slow poll (catches an external `use-context`).
  let kubeCtx = $state("");
  let kctxTimer: ReturnType<typeof setInterval> | undefined;
  async function refreshKubeCtx() {
    try { kubeCtx = ((await invoke<string>("kube_current_context")) || "").trim(); }
    catch { kubeCtx = ""; }
  }

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
        const { fetchSymbols } = await loadCmLsp();
        const syms = await fetchSymbols(lspLang(activeFile)!, activeFile);
        for (const s of syms.slice(0, 200)) items.push({ label: `◇ ${s.name}`, hint: `${baseName(activeFile)}:${s.line}`, run: () => { openInEditor(activeFile); editorGoto.set(s.line); } });
      } catch { /* ignore */ }
    }
    try {
      const log = await invoke<string>("git_log", { cwd, author: null, grep: null, path: null });
      for (const line of log.split("\n").slice(0, 25)) {
        const p = line.split("\x1f");
        const sh = p[1], an = p[2], subj = p[7];
        if (sh) items.push({ label: subj || sh, hint: `◆ ${sh} · ${an}`, run: () => { openView("scm"); } });
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

  // Legacy terminal list — kept only so older persisted sessions round-trip
  // through saveState/read_state unchanged. In the always-grid model terminals
  // live as pane leaves whose PTY id is the leaf `ref`; nothing renders this
  // list anymore. `activeTerm` is DERIVED from the grid so every
  // `pty_write({ id: activeTerm })` targets the focused (or most-recent) grid
  // terminal.
  let terms = $state<{ id: string; title: string; shell?: string }[]>([{ id: "t1", title: "zsh" }]);
  // Shell override per terminal PTY ref, for "New Terminal: bash/fish/…" profiles
  // (#48) — the grid <Terminal> reads it to spawn the chosen shell.
  let termShells = $state<Record<string, string>>({});
  // The most-recently focused grid terminal's PTY id (= its leaf ref), so
  // `activeTerm` keeps pointing at "the terminal you were just in" even after you
  // focus a non-terminal pane.
  let lastTermRef = $state("");
  // All terminal leaves in document order.
  function termLeaves(node: PaneNode): Leaf[] {
    if (node.kind === "leaf") return node.view === "term" && node.ref ? [node] : [];
    return node.children.flatMap(termLeaves);
  }
  // Active terminal = the focused leaf's ref when it's a terminal, else the most
  // recent grid terminal, else the first terminal leaf, else "" (none).
  const activeTerm = $derived.by(() => {
    const lf = findLeaf(paneTree, activeLeaf);
    if (lf?.view === "term" && lf.ref) return lf.ref;
    const terms = termLeaves(paneTree);
    if (lastTermRef && terms.some((t) => t.ref === lastTermRef)) return lastTermRef;
    return terms[0]?.ref ?? "";
  });
  // Remember the last terminal you focused, so activeTerm survives a jump to a
  // non-terminal pane.
  $effect(() => {
    const lf = findLeaf(paneTree, activeLeaf);
    if (lf?.view === "term" && lf.ref) lastTermRef = lf.ref;
  });
  // Make a terminal the visible/active pane: focus the current leaf if it's
  // already a terminal, else focus the most-recent/first terminal leaf, else
  // spawn one in the active pane. Replaces the old `rail = "term"` flip now that
  // the grid is the permanent surface.
  function focusTerm() {
    const lf = findLeaf(paneTree, activeLeaf);
    if (lf?.view === "term") return;
    const terms = termLeaves(paneTree);
    const target = terms.find((t) => t.ref === lastTermRef) ?? terms[0];
    if (target) { activeLeaf = target.id; return; }
    wsAddTab(activeLeaf); // no terminal yet — open one in the focused pane
  }
  // The focused pane's view — drives the activity-rail active-state highlight now
  // that `rail` is effectively pinned to "workspace".
  const activeView = $derived(findLeaf(paneTree, activeLeaf)?.view ?? "");
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

  // Split the active pane to the right with a fresh terminal (⌘D).
  function toggleSplit() {
    const lf = findLeaf(paneTree, activeLeaf);
    if (lf) wsSplit(lf.id, "right", "term");
  }

  const baseName = (p: string) => p.split("/").pop() ?? p;

  // On-demand update check (#95). Degrades gracefully if no release host.
  function updateChannel(): string { try { return localStorage.getItem("anvil-update-channel") || "stable"; } catch { return "stable"; } }
  let installingUpdate = false;
  async function installUpdate(v: string) {
    if (installingUpdate) return;
    if (!confirm(`Install Anvil v${v} and restart now?\n\nThe download is verified against Anvil's signing key before it's applied.`)) return;
    installingUpdate = true;
    toast(`Downloading v${v}…`, "info");
    try {
      await invoke("install_update", { channel: updateChannel() });
      // On success the app restarts and never returns here.
    } catch (e) {
      installingUpdate = false;
      toast("Update failed: " + String(e).slice(0, 100), "error");
    }
  }
  async function checkForUpdates() {
    const ch = updateChannel();
    toast(`Checking for updates (${ch})…`, "info");
    try {
      const v = await invoke<string | null>("check_update", { channel: ch });
      if (v) await installUpdate(v);
      else toast("Anvil is up to date", "info");
    } catch {
      toast("Update check unavailable (no release endpoint yet)", "info");
    }
  }
  // Quiet auto-check shortly after launch; only surfaces if an update exists.
  async function autoCheckUpdate() {
    try {
      const v = await invoke<string | null>("check_update", { channel: updateChannel() });
      if (v) await installUpdate(v);
    } catch { /* no endpoint / offline — stay silent */ }
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

  // Open a new terminal as a tab in the focused grid pane.
  function newTerm() {
    logEvent("terminal.new");
    wsAddTab(activeLeaf);
  }
  // Terminal profile (#48): open a terminal tab running a specific shell. The
  // shell is keyed by the new terminal's PTY ref so the grid <Terminal> spawns it.
  function newTermProfile(shell: string, _title: string) {
    const ref = paneId("wt");
    termShells = { ...termShells, [ref]: shell };
    paneTree = addTab(paneTree, activeLeaf, "term", ref);
  }

  // Editor navigation history (#13): back/forward through visited files (⌘⌥←/→).
  let navHistory = $state<string[]>([]);
  let navPtr = $state(-1);
  // Show a file in the focused grid pane without touching nav history (used by
  // back/forward and "go to line/symbol" which target the already-open file).
  function showInGrid(p: string) {
    activeFile = p;
    if (rail !== "workspace") rail = "workspace";
    if (findLeaf(paneTree, activeLeaf)) paneTree = setLeafView(paneTree, activeLeaf, "editor", p);
  }
  function navBack() { if (navPtr > 0) { navPtr -= 1; showInGrid(navHistory[navPtr]); } }
  function navForward() { if (navPtr < navHistory.length - 1) { navPtr += 1; showInGrid(navHistory[navPtr]); } }

  function openInEditor(p: string) {
    logEvent("file.open", { ext: p.split(".").pop() });
    if (!openFiles.includes(p)) openFiles = [...openFiles, p];
    activeFile = p;
    recentFiles = [p, ...recentFiles.filter((f) => f !== p)].slice(0, 30);
    if (navHistory[navPtr] !== p) {
      navHistory = [...navHistory.slice(0, navPtr + 1), p].slice(-50);
      navPtr = navHistory.length - 1;
    }
    // The grid is the home: open the file in the focused pane. If a modal-ish
    // surface (settings/diff/panel) is up, drop back to the grid first.
    if (rail !== "workspace") rail = "workspace";
    if (findLeaf(paneTree, activeLeaf)) paneTree = setLeafView(paneTree, activeLeaf, "editor", p);
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
    if (openFiles.length === 0 && rail === "editor") rail = "workspace";
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
    openView("agent");
  }

  async function goToWorkspaceSymbol() {
    if (!activeFile || !lspLang(activeFile)) { toast("Open a file with a language server first", "info"); return; }
    const q = prompt("Search workspace symbols:");
    if (q == null) return;
    const { searchWorkspaceSymbols } = await loadCmLsp();
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
    const { fetchSymbols } = await loadCmLsp();
    let syms: Awaited<ReturnType<typeof fetchSymbols>> = [];
    try { syms = await fetchSymbols(lang, activeFile); } catch { syms = []; }
    if (!syms.length) { toast("No symbols found", "info"); return; }
    palettePlaceholder = "Go to symbol…";
    paletteItems = syms.map((s) => ({
      label: `${"  ".repeat(s.depth)}${s.name}`,
      hint: s.detail,
      run: () => { showInGrid(activeFile); editorGoto.set(s.line); },
    }));
    paletteOpen = true;
  }

  // #15 GitOps: route the active manifest edit through a branch + PR instead of
  // `kubectl apply`. The change lands in git and reconciles from there — never
  // touches the cluster directly. Requires the file saved to disk first.
  async function proposeManifestPr() {
    if (!activeFile) { toast("Open a manifest in the editor first", "info"); return; }
    if (dirtyFiles[activeFile]) { toast("Save the file first (⌘S), then propose the PR", "info"); return; }
    const rel = activeFile.startsWith(cwd + "/") ? activeFile.slice(cwd.length + 1) : activeFile;
    const base = baseName(activeFile).replace(/\.[^.]+$/, "");
    const branch = await askText({ title: "Propose change as PR", placeholder: "branch name", value: `manifest/${base}` });
    if (!branch) return;
    const message = await askText({ title: "Commit message", value: `Update ${baseName(activeFile)}` });
    if (!message) return;
    toast("Pushing branch + opening PR…", "info");
    try {
      await invoke("git_branch_commit_push", { cwd, branch, paths: [rel], message });
      const out = await invoke<string>("gh_pr_create", { cwd });
      const url = String(out).trim().split("\n").find((l) => l.startsWith("http")) || "PR opened";
      toast(`PR opened: ${url}`, "success");
    } catch (e) {
      toast(String(e).slice(0, 160) || "PR failed", "error");
    }
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
      { label: "Terminal: Enable Command Separators (shell setup)…", hint: "copies OSC 133 snippet", run: () => {
        palettePlaceholder = "Pick your shell — the setup is copied to the clipboard";
        paletteItems = (["zsh", "bash", "fish"] as IntegrationShell[]).map((sh) => ({
          label: sh, hint: integrationFor(sh).rc,
          run: async () => { const { snippet, rc } = integrationFor(sh); try { await navigator.clipboard.writeText(snippet + "\n"); toast(`Copied — paste into ${rc}, then restart the shell`, "success"); } catch { toast("Clipboard unavailable", "error"); } },
        }));
        paletteOpen = true;
      } },
      { label: "GitOps: Propose Manifest Change as PR…", hint: "branch + commit + push + gh pr", run: proposeManifestPr },
      { label: `Terminal: Broadcast Input ${get(broadcastInput) ? "(on)" : "(off)"}`, run: () => { const v = !get(broadcastInput); broadcastInput.set(v); toast(v ? "Broadcast input ON — keystrokes go to all terminals" : "Broadcast input off", v ? "info" : "success"); } },
      { label: "Terminal: Command History…", run: () => { const h = getHistory(); if (!h.length) { toast("No commands recorded yet", "info"); return; } palettePlaceholder = `${h.length} command${h.length === 1 ? "" : "s"} — Enter to rerun`; paletteItems = [...h].reverse().map((c) => ({ label: c, hint: "⏎ rerun", run: () => { focusTerm(); invoke("pty_write", { id: activeTerm, data: c + "\r" }).catch(() => toast("No active terminal", "error")); } })); paletteItems.push({ label: "Clear command history", hint: "irreversible", run: () => { clearHistory(); toast("Command history cleared", "success"); } }); paletteOpen = true; } },
      { label: `Terminal: Auto-cd to File's Folder ${get(terminalAutoCd) ? "(on)" : "(off)"}`, run: () => { toggleTerminalAutoCd(); toast(get(terminalAutoCd) ? "Active terminal follows the open file" : "Auto-cd off", "success"); } },
      { label: "Terminal: cd to Current File's Folder", run: () => { if (!activeFile) { toast("Open a file first", "info"); return; } const dir = activeFile.replace(/\/[^/]*$/, "") || "/"; invoke("pty_write", { id: activeTerm, data: `cd ${dir.includes(" ") ? `'${dir}'` : dir}\r` }).then(() => { focusTerm(); }).catch(() => toast("No active terminal", "error")); } },
      { label: "k8s: Diff Current Manifest vs Cluster", hint: "read-only · GitOps", run: async () => { if (!activeFile || !/\.(ya?ml)$/i.test(activeFile)) { toast("Open a YAML manifest first", "info"); return; } let diff = ""; try { diff = await invoke<string>("kube_diff", { path: activeFile }); } catch (e) { toast(String(e).slice(0, 80) || "kubectl diff failed", "error"); return; } if (!diff.trim()) { toast("No drift — manifest matches the cluster", "success"); return; } agentSeed.set(`Live cluster differs from ${baseName(activeFile)}. This is GitOps — do NOT kubectl apply; the change must land via a git commit that Flux reconciles. Here's the drift:\n\n\`\`\`diff\n${diff.slice(0, 4000)}\n\`\`\``); openView("agent"); } },
      { label: "SSH: Connect to Host…", run: async () => { let hosts: string[] = []; try { hosts = (await invoke<string>("ssh_hosts")).split("\n").filter(Boolean); } catch { /* ignore */ } if (!hosts.length) { toast("No hosts in ~/.ssh/config", "info"); return; } palettePlaceholder = `${hosts.length} ssh host${hosts.length === 1 ? "" : "s"}`; paletteItems = hosts.map((h) => ({ label: h, hint: "ssh", run: () => { focusTerm(); invoke("pty_write", { id: activeTerm, data: `ssh ${h}\r` }).catch(() => toast("No active terminal", "error")); } })); paletteOpen = true; } },
      { label: "Close Tab", hint: "⌘W", run: closeActiveTab },
      { label: "Find File…", hint: "⌘P", run: openFilesPalette },
      { label: "Go to Anything…", run: goToAnything },
      { label: "Recent Files…", hint: "⌘E", run: openRecent },
      { label: "Reopen Closed Tab", hint: "⌘⇧T", run: reopenClosed },
      { label: "Detach Pane to New Window", run: detachActivePane },
      { label: "Editor: Navigate Back", hint: "⌘⌥←", run: navBack },
      { label: "Editor: Navigate Forward", hint: "⌘⌥→", run: navForward },
      { label: "File History…", run: fileHistory },
      { label: "Git: Reflog…", hint: "recover lost commits", run: gitReflog },
      { label: "Git: Compare to Branch…", hint: "ahead/behind + files", run: gitBranchCompare },
      { label: `Problems… (${$problems.length})`, hint: "⇧⌘M", run: () => { bottomDock = true; dockTab = "problems"; } },
      { label: "Go to Line…", run: () => { if (!activeFile) { toast("Open a file first", "info"); return; } const n = prompt("Go to line:"); if (n && +n > 0) { showInGrid(activeFile); editorGoto.set(Math.floor(+n)); } } },
      { label: "Ask Agent…", hint: "⌘I", run: () => { const q = prompt("Ask the agent:"); if (q && q.trim()) { agentSeed.set(q.trim()); openView("agent"); } } },
      { label: "Agent: Enqueue Task…", run: () => { const q = prompt("Queue an agent task:"); if (q && q.trim()) { enqueueAgent(q.trim()); toast(`Queued (${get(agentQueue).length} pending)`, "success"); } } },
      { label: `Agent: Run Next Queued (${get(agentQueue).length})`, run: () => { const t = dequeueAgent(); if (!t) { toast("Queue is empty", "info"); return; } agentSeed.set(t.prompt); openView("agent"); } },
      { label: "Agent: View Queue…", run: () => { const q = get(agentQueue); if (!q.length) { toast("Queue is empty", "info"); return; } palettePlaceholder = `${q.length} queued task${q.length === 1 ? "" : "s"}`; paletteItems = q.map((t) => ({ label: t.prompt, hint: "✕ remove", run: () => { removeQueued(t.id); toast("Removed from queue", "success"); } })); paletteItems.push({ label: "Run all (seed first, rest stay queued)", hint: "", run: () => { const n = dequeueAgent(); if (n) { agentSeed.set(n.prompt); openView("agent"); } } }); paletteItems.push({ label: "Clear queue", hint: "irreversible", run: () => { clearQueue(); toast("Queue cleared", "success"); } }); paletteOpen = true; } },
      { label: "Agent: Run Tests & Fix", run: () => { agentSeed.set("Enable Agent mode, then: detect and run this project's test suite using your run tool (e.g. `npm test`, `cargo test`, `pytest`), read the failures, and propose minimal fixes as per-hunk diffs. Iterate until tests pass."); openView("agent"); } },
      { label: "Agent: Diagnose Terminal Output", hint: "last failure", run: () => { const t = readTerminal(activeTerm).slice(-4000).trim(); if (!t) { toast("Active terminal is empty", "info"); return; } agentSeed.set(`Diagnose the problem in this terminal output and propose a fix. Use your run tool to investigate (logs, status, describe, plan) before concluding — don't guess.\n\n\`\`\`\n${t}\n\`\`\``); openView("agent"); } },
      { label: "Agent: Set Utility Model (fast tasks)…", run: () => { const cur = (typeof localStorage !== "undefined" && localStorage.getItem("anvil-util-model")) || ""; const m = prompt("Model id for quick tasks like commit messages (blank = use default):", cur); if (m === null) return; try { if (m.trim()) localStorage.setItem("anvil-util-model", m.trim()); else localStorage.removeItem("anvil-util-model"); } catch { /* ignore */ } toast(m.trim() ? `Utility model: ${m.trim()}` : "Utility model cleared", "success"); } },
      { label: "Agent: Set Reasoning Model (agent chat)…", run: () => { const cur = (typeof localStorage !== "undefined" && localStorage.getItem("anvil-reasoning-model")) || ""; const m = prompt("Model id for agent reasoning/chat (blank = use default; reopen agent to apply):", cur); if (m === null) return; try { if (m.trim()) localStorage.setItem("anvil-reasoning-model", m.trim()); else localStorage.removeItem("anvil-reasoning-model"); } catch { /* ignore */ } toast(m.trim() ? `Reasoning model: ${m.trim()}` : "Reasoning model cleared", "success"); } },
      { label: "Terminal: Run Snippet…", hint: "quick commands", run: () => { palettePlaceholder = "Run snippet in terminal"; paletteItems = getSnippets().map((s) => ({ label: s.label, hint: s.command, run: () => sendToTerm(s.command) })); paletteItems.push({ label: "➕ Save a snippet…", hint: "", run: saveSnippetFlow }); paletteItems.push({ label: "🗑 Manage snippets…", hint: "", run: manageSnippetsFlow }); paletteOpen = true; } },
      { label: "Terminal: Save Snippet…", hint: "reusable command", run: saveSnippetFlow },
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
        openView("agent");
      } },
      { label: "Save Command…", run: saveCommand },
      { label: "Run Saved Command…", run: runSavedCommand },
      { label: "Open Folder…", hint: "⌘⇧O", run: openFolder },
      { label: "Open File…", hint: "⌘O", run: openFileDialog },
      { label: "Recent Workspaces…", run: openRecentWorkspace },
      { label: "Cycle Theme", run: cycleTheme },
      { label: "Toggle Density", run: toggleDensity },
      { label: "Zoom In (text)", hint: "⌘+", run: () => zoomContent(1) },
      { label: "Zoom Out (text)", hint: "⌘−", run: () => zoomContent(-1) },
      { label: "Reset Zoom (text)", hint: "⌘0", run: zoomContentReset },
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
      { label: "Help: Take the Tour", hint: "guided intro", run: () => { obStep = 0; onboarded = false; } },
      { label: "Help: Report a Problem (copy diagnostics)", run: async () => { const report = diagnosticsReport(WHATS_NEW_VERSION); try { await navigator.clipboard.writeText(report); toast("Diagnostics copied — paste into your bug report", "success"); } catch { toast("Could not copy diagnostics", "error"); } } },
      { label: `Help: View Crash Log (${getCrashes().length})`, run: () => { const cr = getCrashes(); if (!cr.length) { toast("No crashes recorded 🎉", "success"); return; } palettePlaceholder = `${cr.length} crash${cr.length === 1 ? "" : "es"} (local)`; paletteItems = [...cr].reverse().slice(0, 100).map((c) => ({ label: `${c.kind}: ${c.message}`, hint: new Date(c.ts).toLocaleString(), run: () => {} })); paletteItems.push({ label: "Clear crash log", hint: "irreversible", run: () => { clearCrashes(); toast("Crash log cleared", "success"); } }); paletteOpen = true; } },
      { label: "Help: Keyboard Shortcuts", hint: "⌘/", run: () => (keymapOpen = true) },
      { label: "What needs attention", hint: "⌘⇧A · failing GitOps / pods / CI", run: () => (attentionOpen = true) },
      { label: "Connections: Check tools & auth", hint: "k8s · aws · gh · glab · docker", run: () => (doctorOpen = true) },
      { label: "Environment: Switch Profile…", hint: "ctx + ns + aws in one move", run: switchProfilePalette },
      { label: "Environment: Save Current as Profile…", hint: "snapshot ctx/ns/aws", run: saveCurrentProfile },
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
      { label: "View: Terminal", run: focusTerm },
      { label: "Toggle Explorer Sidebar", hint: "⌘B", run: toggleSide },
      { label: `Workspace: Focus Dimming ${get(focusDimming) ? "(on)" : "(off)"}`, run: () => { toggleFocusDimming(); toast(get(focusDimming) ? "Inactive panes dimmed" : "Focus dimming off", "success"); } },
      { label: "View: Source Control", run: () => openView("scm") },
      { label: "Git: Pull (fast-forward)", run: async () => { try { await invoke("git_pull", { cwd }); toast("Pulled", "success"); } catch (e) { toast(String(e).slice(0, 80) || "Pull failed", "error"); } } },
      { label: "Git: Push", run: async () => { try { await invoke("git_push", { cwd }); toast("Pushed", "success"); } catch (e) { toast(String(e).slice(0, 80) || "Push failed", "error"); } } },
      { label: "Git: Fetch All", run: async () => { try { await invoke("git_fetch", { cwd }); toast("Fetched", "success"); } catch { toast("Fetch failed", "error"); } } },
      { label: "Git: 3-Pane Merge (current file)", run: () => { if (!activeFile) { toast("Open the conflicted file first", "info"); return; } mergeView = activeFile; } },
      { label: "Git: Generate PR Body (agent → clipboard)", run: generatePrBody },
      { label: "Git: Interactive Rebase onto…", run: () => { const target = prompt("Rebase onto (branch / ref / commit):", "origin/main"); if (target) rebaseTarget = target; } },
      { label: "Git: Interactive Rebase in Terminal…", run: () => { const target = prompt("Rebase onto (branch / ref / commit):", "origin/main"); if (!target) return; focusTerm(); invoke("pty_write", { id: activeTerm, data: `git rebase -i ${target}\r` }).catch(() => toast("No active terminal", "error")); } },
      { label: "Git: Worktrees…", run: async () => { let raw = ""; try { raw = await invoke<string>("git_worktrees", { cwd }); } catch { toast("Not a git repository", "error"); return; } const rows = raw.split("\n").filter(Boolean).map((l) => { const [p, b] = l.split("\t"); return { p, b }; }); palettePlaceholder = `${rows.length} worktree${rows.length === 1 ? "" : "s"}`; paletteItems = rows.map((w) => ({ label: w.b, hint: w.p + (w.p === cwd ? "  (current)" : ""), run: () => { cwd = w.p; explorerOpen = true; toast(`Switched to ${w.b}`, "success"); } })); paletteItems.push({ label: "➕ Add worktree…", hint: "branch → sibling dir", run: async () => { const br = prompt("Branch for the new worktree:"); if (!br) return; const path = prompt("Path for the worktree:", `${cwd}-${br.replace(/[^\w.-]+/g, "-")}`); if (!path) return; try { await invoke("git_worktree_add", { cwd, path, branch: br }); toast("Worktree added", "success"); } catch (e) { toast(String(e).slice(0, 80) || "add failed", "error"); } } }); paletteOpen = true; } },
      { label: "Git: Amend Last Commit (staged)", run: async () => { try { await invoke("git_amend", { cwd }); toast("Amended last commit", "success"); } catch (e) { toast(String(e).slice(0, 80) || "Amend failed", "error"); } } },
      { label: "GitHub: Create PR (gh pr create --fill)", run: async () => { try { const r = await invoke<string>("gh_pr_create", { cwd }); toast(r.split("\n").find(Boolean)?.slice(0, 80) || "PR created", "success"); } catch (e) { toast(String(e).slice(0, 90) || "PR create failed", "error"); } } },
      { label: "GitHub: View PR in Browser", run: async () => { try { await invoke("gh_pr_web", { cwd }); } catch (e) { toast(String(e).slice(0, 80) || "No PR for this branch", "error"); } } },
      { label: "AWS: SSO Login", run: () => { invoke("pty_write", { id: activeTerm, data: "aws sso login\n" }); focusTerm(); } },
      { label: "AWS: EC2 Instances", run: () => { invoke("pty_write", { id: activeTerm, data: "aws ec2 describe-instances --query 'Reservations[].Instances[].{ID:InstanceId,Type:InstanceType,State:State.Name,Name:Tags[?Key==`Name`]|[0].Value}' --output table\n" }); focusTerm(); } },
      { label: "AWS: S3 Buckets", run: () => { invoke("pty_write", { id: activeTerm, data: "aws s3 ls\n" }); focusTerm(); } },
      { label: "AWS: Lambda Functions", run: () => { invoke("pty_write", { id: activeTerm, data: "aws lambda list-functions --query 'Functions[].{Name:FunctionName,Runtime:Runtime,Mem:MemorySize}' --output table\n" }); focusTerm(); } },
      { label: "Secrets: Read → Copy…", hint: "ssm · vault · 1password · keychain", run: () => { palettePlaceholder = "Secret source — value is copied, never shown or stored"; paletteItems = [
        { label: "AWS SSM Parameter", hint: "ssm", run: () => readSecret("ssm", "SSM parameter name (e.g. /prod/db/url):") },
        { label: "Vault", hint: "vault", run: () => readSecret("vault", "Vault path (e.g. secret/data/app):") },
        { label: "1Password", hint: "op", run: () => readSecret("op", "Secret reference (op://vault/item/field):") },
        { label: "macOS Keychain", hint: "keychain", run: () => readSecret("keychain", "Keychain service name:") },
      ]; paletteOpen = true; } },
      { label: "GitHub: Run Workflow…", hint: "workflow_dispatch", run: runWorkflow },
      { label: "Sentry: Recent Issues…", hint: "unresolved · 14d", run: sentryIssues },
      { label: "Slack: Post Message…", hint: "incoming webhook", run: slackPost },
      { label: "AWS: RDS Instances", run: () => { invoke("pty_write", { id: activeTerm, data: "aws rds describe-db-instances --query 'DBInstances[].{ID:DBInstanceIdentifier,Engine:Engine,Class:DBInstanceClass,Status:DBInstanceStatus}' --output table\n" }); focusTerm(); } },
      { label: "Secrets: SSM Get Parameter…", run: () => { const k = prompt("SSM parameter name (e.g. /app/db/password):"); if (k) { invoke("pty_write", { id: activeTerm, data: `aws ssm get-parameter --name '${k}' --with-decryption --query Parameter.Value --output text\n` }); focusTerm(); } } },
      { label: "Secrets: Vault Read…", run: () => { const k = prompt("Vault path (e.g. secret/data/app):"); if (k) { invoke("pty_write", { id: activeTerm, data: `vault kv get '${k}'\n` }); focusTerm(); } } },
      { label: "Secrets: Keychain Find…", run: () => { const k = prompt("Keychain service name:"); if (k) { invoke("pty_write", { id: activeTerm, data: `security find-generic-password -s '${k}' -w\n` }); focusTerm(); } } },
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
      { label: "GitLab: CI Pipelines (glab ci list)", run: () => { invoke("pty_write", { id: activeTerm, data: "glab ci list\n" }); focusTerm(); } },
      { label: "GitLab: Pipeline Logs (glab ci trace)", run: () => { invoke("pty_write", { id: activeTerm, data: "glab ci trace\n" }); focusTerm(); } },
      { label: "GitLab: Retry Pipeline (glab ci retry)", run: () => { invoke("pty_write", { id: activeTerm, data: "glab ci retry\n" }); focusTerm(); } },
      { label: "View: Search", run: () => openView("search") },
      { label: "View: AI Agent", run: () => openView("agent") },
      { label: "View: Kubernetes", run: () => openView("k8s") },
      { label: "View: CI / Pipelines", run: () => openView("ci") },
      { label: "View: Terraform / Terragrunt", run: () => openView("terraform") },
      { label: "View: Helm", hint: "in Kubernetes", run: () => openView("k8s") },
      { label: "View: Observability (Metrics / Logs)", run: () => openView("obs") },
      { label: "View: DevOps (Terraform / Helm / Observability)", run: () => openView("devops") },
      { label: "Workspace: Balance Panes", run: () => { paneTree = balanceTree(paneTree); rail = "workspace"; } },
      { label: "Workspace: Close Other Panes", run: () => { paneTree = closeOtherPanes(paneTree, activeLeaf); rail = "workspace"; } },
      { label: "Workspace: Save Layout As…", run: saveLayoutAs },
      { label: "Workspace: Load Layout…", run: openLayoutPalette },
      { label: "Workspace: Export Layout (copy)", run: exportLayout },
      { label: "Workspace: Import Layout (paste)", run: importLayout },
      { label: "View: Settings", run: openSettings },
      { label: "Toggle Zen / Terminal Mode", hint: "⌘.", run: () => toggleZen() },
    ];
    // H74: float most-used commands up, and record each run.
    paletteItems = rankItems(paletteItems.map(withTracking));
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
      run: () => { invoke("pty_write", { id: activeTerm, data: c + "\n" }); focusTerm(); },
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

  function cfg(k: string): string { try { return localStorage.getItem(k) || ""; } catch { return ""; } }
  function setCfg(k: string, v: string): void { try { localStorage.setItem(k, v); } catch { /* ignore */ } }

  // C24 workflow dispatch — pick a workflow, trigger it on the current branch.
  async function runWorkflow() {
    let raw = "";
    try { raw = await invoke<string>("gh_workflow_list", { cwd }); } catch { toast("gh workflow list failed", "error"); return; }
    const wfs = raw.split("\n").filter(Boolean).map((l) => l.split("\t")[0]).filter(Boolean);
    if (!wfs.length) { toast("No workflows found", "info"); return; }
    let gitRef = "HEAD";
    try { const b = await invoke<string>("git_branches", { cwd }); const head = b.split("\n").find((l) => l.startsWith("*")); if (head) gitRef = head.split("\t")[1] || "HEAD"; } catch { /* ignore */ }
    palettePlaceholder = `Run workflow on ${gitRef}`;
    paletteItems = wfs.map((w) => ({ label: w, hint: "dispatch", run: async () => { try { await invoke("gh_workflow_run", { cwd, workflow: w, gitRef }); toast(`Triggered "${w}" on ${gitRef}`, "success"); } catch (e) { toast(String(e).slice(0, 120) || "dispatch failed", "error"); } } }));
    paletteOpen = true;
  }

  // I85 Sentry — list recent unresolved issues; pick to copy the permalink.
  async function sentryIssues() {
    let org = cfg("anvil-sentry-org"), proj = cfg("anvil-sentry-project"), token = cfg("anvil-sentry-token");
    const baseUrl = cfg("anvil-sentry-base");
    if (!org) { org = prompt("Sentry org slug:") || ""; if (!org) return; setCfg("anvil-sentry-org", org); }
    if (!proj) { proj = prompt("Sentry project slug:") || ""; if (!proj) return; setCfg("anvil-sentry-project", proj); }
    if (!token) { token = prompt("Sentry auth token:") || ""; if (!token) return; setCfg("anvil-sentry-token", token); }
    try {
      const raw = await invoke<string>("sentry_issues", { base: baseUrl, org, project: proj, token });
      const issues = JSON.parse(raw);
      if (!Array.isArray(issues) || !issues.length) { toast("No unresolved Sentry issues 🎉", "success"); return; }
      palettePlaceholder = `${issues.length} unresolved Sentry issue${issues.length === 1 ? "" : "s"}`;
      paletteItems = issues.slice(0, 100).map((i: { title?: string; metadata?: { value?: string }; count?: string; culprit?: string; permalink?: string }) => ({ label: i.title || i.metadata?.value || "issue", hint: `${i.count ?? "?"}× · ${i.culprit ?? ""}`.slice(0, 60), run: () => { if (i.permalink) { navigator.clipboard?.writeText(i.permalink).catch(() => {}); toast("Issue link copied", "success"); } } }));
      paletteOpen = true;
    } catch (e) { toast("Sentry: " + String(e).slice(0, 100), "error"); }
  }

  // I88 Slack — post a message to a stored incoming-webhook.
  async function slackPost() {
    let wh = cfg("anvil-slack-webhook");
    if (!wh) { wh = prompt("Slack incoming-webhook URL:") || ""; if (!wh) return; setCfg("anvil-slack-webhook", wh); }
    const text = prompt("Message to post:"); if (!text) return;
    try { await invoke("slack_post", { webhook: wh, text }); toast("Posted to Slack", "success"); } catch (e) { toast("Slack: " + String(e).slice(0, 100), "error"); }
  }

  // I83 read-only secret fetch — copy to clipboard, never display or persist.
  async function readSecret(source: string, label: string) {
    const key = prompt(label);
    if (!key) return;
    try {
      const v = await invoke<string>("secret_read", { source, key });
      await navigator.clipboard?.writeText(v).catch(() => {});
      toast("Secret fetched → copied to clipboard (not stored)", "success");
    } catch (e) {
      toast("Read failed: " + String(e).slice(0, 100), "error");
    }
  }

  // G67 reflog browser — list recent HEAD moves; pick one to copy its hash.
  async function gitReflog() {
    let raw = "";
    try { raw = await invoke<string>("git_reflog", { cwd }); } catch { toast("Not a git repository", "error"); return; }
    const rows = raw.split("\n").filter(Boolean).map((l) => l.split("\t"));
    if (!rows.length) { toast("Empty reflog", "info"); return; }
    palettePlaceholder = "Reflog — pick to copy hash";
    paletteItems = rows.map(([hash, sel, msg]) => ({
      label: msg || sel,
      hint: `${hash} · ${sel}`,
      run: () => { navigator.clipboard?.writeText(hash).catch(() => {}); toast(`Copied ${hash}`, "success"); },
    }));
    paletteOpen = true;
  }
  // G63 branch compare — pick a base, show ahead/behind + changed files.
  async function gitBranchCompare() {
    let braw = "";
    try { braw = await invoke<string>("git_branches", { cwd }); } catch { toast("Not a git repository", "error"); return; }
    const branches = braw.split("\n").filter(Boolean).map((l) => l.split("\t")[1]).filter(Boolean);
    palettePlaceholder = "Compare current branch to…";
    paletteItems = branches.map((b) => ({
      label: b,
      hint: "base",
      run: async () => {
        try {
          const out = await invoke<string>("git_branch_compare", { cwd, base: b });
          const [summary, ...files] = out.split("\n");
          toast(summary, "info");
          const fileRows = files.filter(Boolean);
          palettePlaceholder = summary;
          paletteItems = fileRows.length
            ? fileRows.map((f) => { const [st, ...rest] = f.split("\t"); const path = rest.join("\t"); return { label: path, hint: st, run: () => openInEditor(`${cwd.replace(/\/$/, "")}/${path}`) }; })
            : [{ label: "No file differences", hint: "", run: () => {} }];
          paletteOpen = true;
        } catch (e) { toast(String(e).slice(0, 120) || "compare failed", "error"); }
      },
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
    "search": () => { openView("search"); },
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
      closeActiveTab();
    }
    else if (e.key === "k") { e.preventDefault(); openCommands(); }
    else if (e.key === "p") { e.preventDefault(); openFilesPalette(); }
    else if (e.key === "e") { e.preventDefault(); openRecent(); }
    else if (e.key === "n") { e.preventDefault(); invoke("new_window").catch((e) => toast("Could not open new window: " + String(e).slice(0, 60), "error")); }
    else if (e.key === "o") { e.preventDefault(); openFolder(); }
    else if (e.key === "d") { e.preventDefault(); toggleSplit(); }
    else if (e.key === "j") { e.preventDefault(); bottomDock = !bottomDock; }
    else if (e.shiftKey && (e.key === "a" || e.key === "A")) { e.preventDefault(); attentionOpen = true; }
    else if (e.shiftKey && (e.key === "f" || e.key === "F")) { e.preventDefault(); openView("search"); }
    else if (e.shiftKey && (e.key === "b" || e.key === "B")) { e.preventDefault(); toggleRail(); }
    else if (e.shiftKey && (e.key === "o" || e.key === "O")) { e.preventDefault(); goToSymbol(); }
    else if (e.shiftKey && (e.key === "m" || e.key === "M")) { e.preventDefault(); bottomDock = true; dockTab = "problems"; }
    else if (e.shiftKey && (e.key === "v" || e.key === "V")) { if (isMarkdown(activeFile)) { e.preventDefault(); mdPreview = !mdPreview; rail = "editor"; } }
    else if (e.key === "i") { e.preventDefault(); openView("agent"); }
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
    else if (e.key === "=" || e.key === "+") { e.preventDefault(); zoomContent(1); }
    else if (e.key === "-" || e.key === "_") { e.preventDefault(); zoomContent(-1); }
    else if (e.key === "0") { e.preventDefault(); zoomContentReset(); }
    else if (e.key === ".") {
      // In the editor ⌘. is the LSP code-action / quick-fix; only outside it does
      // ⌘. mean "jump to a zen terminal".
      if (document.activeElement?.closest(".cm-editor")) return;
      e.preventDefault(); toggleZen();
    }
    else if (e.key >= "1" && e.key <= "9") {
      e.preventDefault();
      // ⌘1–9 focuses the Nth pane in the grid.
      const ids = leafIds(paneTree);
      const id = ids[Number(e.key) - 1];
      if (id) activeLeaf = id;
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
  // Skip the initial restore: on relaunch initTheme() has already applied the
  // user's last manual pick, so re-applying a stale pinned theme here is the
  // "theme won't stick" bug. The pin only takes over on a real mid-session
  // switch to a different folder.
  let lastWsCwd = "";
  $effect(() => {
    const dir = cwd;
    const first = lastWsCwd === "";
    lastWsCwd = dir;
    if (first || !dir) return;
    const ws = wsSettings[dir];
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
    const json = JSON.stringify({ cwd, rail, explorerOpen, openFiles, activeFile, terms, activeTerm, seq, recentFiles, recentWorkspaces, wsSettings, paneTree, activeLeaf });
    // Mirror to localStorage *synchronously* first: it survives an abrupt quit or
    // crash even if the async file write below gets cut off mid-exit.
    try { localStorage.setItem(sessionKey, json); } catch { /* ignore */ }
    try { await invoke("write_state", { contents: json, label: winLabel }); } catch { /* ignore */ }
  }

  let restored = false;
  let lastErrToast = 0;
  onMount(async () => {
    // Surface uncaught errors/rejections (throttled) so a failure is visible,
    // not a silent white-screen. crash.ts also records them to the ring buffer.
    installCrashHandlers((_kind, message, origin) => {
      const t = performance.now();
      if (t - lastErrToast < 4000) return;
      lastErrToast = t;
      const where = origin ? `  (${origin})` : "";
      toast("Unexpected error: " + message.slice(0, 100) + where, "error");
    });
    void refreshKubeCtx();
    kctxTimer = setInterval(refreshKubeCtx, 30000);
    bootMs = Math.round(performance.now() - bootStart);
    requestAnimationFrame(() => { firstPaintMs = Math.round(performance.now() - bootStart); });
    initTheme();
    initDensity();
    initScale();
    initOpacity();
    loadUserConfig();
    // Quiet update check once the app has settled (no-op without a release host).
    setTimeout(autoCheckUpdate, 8000);
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
      for (const f of [SourceControl, Editor, DiffView, SearchPanel, AgentPanel, Settings, DevOps, Kube, CI, Terraform, Observability, FileView]) {
        f().catch(() => {});
      }
    };
    if (typeof requestIdleCallback === "function") requestIdleCallback(prefetchViews, { timeout: 4000 });
    else setTimeout(prefetchViews, 2500);
    // Theme (incl. custom overrides + system light/dark follow) handled in initTheme.
    if (isDetached && detachSeed) {
      cwd = detachSeed.cwd || (await invoke<string>("home_dir").catch(() => ""));
      const v = detachSeed.view ?? "term";
      // The grid is the surface even in a detached window: seed a single leaf
      // showing the requested view and stay on "workspace".
      paneTree = leaf(v as ViewKind, v === "term" ? paneId("wt") : detachSeed.file, paneId("wt"));
      activeLeaf = firstLeaf(paneTree).id;
      if (v === "editor" && detachSeed.file) {
        openFiles = [detachSeed.file];
        activeFile = detachSeed.file;
      }
      rail = "workspace";
      restored = true;
      window.addEventListener("keydown", onCustomKey, true);
      await listen<string>("menu", (e) => onMenu(e.payload));
      return;
    }
    let st: any = {};
    // Prefer the synchronous localStorage mirror (freshest, crash-safe); fall back
    // to the on-disk state file (older sessions / before this mirror existed).
    try { st = JSON.parse(localStorage.getItem(sessionKey) || ""); } catch { st = {}; }
    if (!st || typeof st !== "object" || !st.cwd) {
      try { st = JSON.parse(await invoke<string>("read_state", { label: winLabel })); } catch { st = {}; }
    }
    cwd = st.cwd || (await invoke<string>("home_dir"));
    // terms[] is vestigial now (grid terminals are leaves), but restore it so an
    // older session round-trips; seq keeps minting unique legacy ids.
    if (Array.isArray(st.terms) && st.terms.length) {
      terms = st.terms;
      seq = st.seq || terms.length;
    }
    if (Array.isArray(st.openFiles)) { openFiles = st.openFiles; activeFile = st.activeFile || openFiles.at(-1) || ""; }
    if (Array.isArray(st.recentFiles)) recentFiles = st.recentFiles;
    if (Array.isArray(st.recentWorkspaces)) recentWorkspaces = st.recentWorkspaces;
    if (st.wsSettings && typeof st.wsSettings === "object") wsSettings = st.wsSettings;
    if (st.paneTree && typeof st.paneTree === "object") { try { paneTree = remapTermRefs(st.paneTree); seedPaneSeq(paneTree); } catch { /* ignore */ } }
    if (typeof st.activeLeaf === "string" && findLeaf(paneTree, st.activeLeaf)) activeLeaf = st.activeLeaf;
    if (typeof st.explorerOpen === "boolean") explorerOpen = st.explorerOpen;
    // Always-grid migration: the grid is the only content surface. Any persisted
    // single-view rail (term/editor/files/panel or a PANE_VIEW like scm/k8s/…)
    // coerces to "workspace", seeding the active leaf with that view so the
    // restored session lands on the same content inside the grid. Only "settings"
    // (a modal overlay) survives as a non-grid rail.
    if (st.rail === "settings") { rail = "settings"; settingsOpen = true; }
    else {
      rail = "workspace";
      const lf = findLeaf(paneTree, activeLeaf);
      const seed = st.rail === "files" ? "files" : st.rail === "panel" ? null : st.rail;
      if (st.rail === "files") explorerOpen = true;
      // Don't clobber a restored multi-pane layout; only seed when the tree is a
      // lone leaf (the default), so a saved grid is preserved as-is.
      if (lf && paneTree.kind === "leaf" && seed && PANE_VIEWS.has(seed)) {
        paneTree = setLeafView(paneTree, lf.id, seed as ViewKind, seed === "editor" ? (activeFile || undefined) : seed === "term" ? paneId("wt") : undefined);
      }
    }
    restored = true;
    // Crash/quit safety: flush the session immediately (cancel the debounce) when
    // the window is hidden or about to close — saveState writes localStorage
    // synchronously, so the last layout survives even an abrupt exit.
    const flushState = () => { if (!restored || isDetached) return; clearTimeout(saveTimer); void saveState(); };
    window.addEventListener("pagehide", flushState);
    document.addEventListener("visibilitychange", () => { if (document.hidden) flushState(); });
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

  // ── File-tab reorder + context menu (roadmap §A #15 / #16) ──
  // Only file tabs live in the top strip now (terminals are grid panes), so the
  // `kind` discriminant collapses to "file". Reorder is driven by the pointer
  // drag controller (startTabDrag → reorder) — no HTML5 drag here.
  type TabKind = "file";
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
  function tabCtx(e: MouseEvent, kind: TabKind, id: string) {
    e.preventDefault();
    tabMenu = { x: e.clientX, y: e.clientY, kind, id };
  }
  function closeOthers(_kind: TabKind, id: string) {
    for (const f of [...openFiles]) if (f !== id && !pinnedFiles.includes(f)) closeFile(f);
    tabMenu = null;
  }
  function closeRight(_kind: TabKind, id: string) {
    const i = openFiles.indexOf(id);
    for (const f of openFiles.slice(i + 1)) closeFile(f);
    tabMenu = null;
  }
  function copyTabPath(_kind: TabKind, id: string) {
    navigator.clipboard.writeText(id).catch((e) => console.warn("clipboard write failed", e));
    tabMenu = null;
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="app" class:zen class:rail-auto={$autoHideRail}>
  {#if zen}<div class="zen-bar" data-tauri-drag-region></div>{/if}
  <!-- No data-tauri-drag-region here: it would hijack mousedown on the draggable
       tab pills into an OS window-drag, breaking tab→pane drag-and-drop. The
       empty .spacer below carries the drag region for moving the window. -->
  <div class="tabs">
    {#each orderedFiles as f, i (f + '#' + i)}
      <div class="tab {activeFile === f ? 'on' : ''}" class:pinned-tab={pinnedFiles.includes(f)} role="button" tabindex="0" onclick={() => openInEditor(f)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openInEditor(f))} title={tabGroups[f] ? `${f}  ·  group: ${tabGroups[f]}` : f}
        style={tabGroups[f] ? `box-shadow: inset 0 -2px 0 ${groupColor(tabGroups[f])}` : ''}
        data-file-tab={f} class:drag={tabDrag?.ref === f && !tabDrag?.from}
        onpointerdown={(e) => startTabDrag(e, { view: 'editor', ref: f, label: baseName(f) })} oncontextmenu={(e) => tabCtx(e, 'file', f)}>
        {#if pinnedFiles.includes(f)}<span class="pin" role="button" tabindex="0" onclick={(e) => { e.stopPropagation(); togglePin(f); }} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), e.stopPropagation(), togglePin(f))} title="Unpin"><Icon name="pin" size={9} /></span>{/if}
        <span class="tt">{baseName(f)}</span>{#if dirtyFiles[f]}<span class="dirty"></span>{/if}
        <span class="x" role="button" tabindex="0" onclick={(e) => { e.stopPropagation(); closeFile(f); }} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), e.stopPropagation(), closeFile(f))}>×</span>
      </div>
    {/each}
    {#each panels as p (p.id)}
      <div class="tab {rail === 'panel' && activePanel === p.id ? 'on' : ''}" role="button" tabindex="0" onclick={() => selectPanel(p.id)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), selectPanel(p.id))} title={p.title}>
        <span class="tab-ic"><Icon name={panelIcon(p.kind)} size={12} /></span>
        <span class="tt">{p.title}</span>
        <span class="x" role="button" tabindex="0" onclick={(e) => { e.stopPropagation(); closePanel(p.id); }} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), e.stopPropagation(), closePanel(p.id))}>×</span>
      </div>
    {/each}
    {#each viewTabs.filter((vk) => VIEW_META[vk]) as vk (vk)}
      <div class="tab {rail === vk ? 'on' : ''}" class:drag={tabDrag?.view === vk && !tabDrag?.ref && !tabDrag?.from} role="button" tabindex="0" title={VIEW_META[vk].title}
        onpointerdown={(e) => startTabDrag(e, { view: vk as ViewKind, label: VIEW_META[vk].title })}
        onclick={() => openView(vk)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView(vk))}>
        <span class="tab-ic"><Icon name={VIEW_META[vk].icon} size={12} /></span>
        <span class="tt">{VIEW_META[vk].title}</span>
        <span class="x" role="button" tabindex="0" onclick={(e) => { e.stopPropagation(); closeView(vk); }}
          onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), e.stopPropagation(), closeView(vk))}>×</span>
      </div>
    {/each}
    {#if openFiles.length > 1}
      <div class="newtab" role="button" tabindex="0" title="All open tabs" onclick={() => (tabOverflow = !tabOverflow)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), (tabOverflow = !tabOverflow))}>⌄</div>
    {/if}
    {#if settingsOpen}
      <div class="tab {rail === 'settings' ? 'on' : ''}" role="button" tabindex="0" onclick={() => (rail = 'settings')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), (rail = 'settings'))}>
        <span class="tab-ic"><Icon name="settings" size={12} /></span><span class="tt">Settings</span>
        <span class="x" role="button" tabindex="0" onclick={(e) => { e.stopPropagation(); settingsOpen = false; if (rail === 'settings') rail = 'workspace'; }} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), e.stopPropagation(), (settingsOpen = false, rail === 'settings' && (rail = 'workspace')))}>×</span>
      </div>
    {/if}
    <div class="newtab" role="button" tabindex="0" title="New…" onclick={(e) => { if (plusMenu) { plusMenu = null; return; } const r = e.currentTarget.getBoundingClientRect(); plusMenu = { x: r.left, y: r.bottom + 4 }; }} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); if (plusMenu) { plusMenu = null; } else { const r = (e.currentTarget as HTMLElement).getBoundingClientRect(); plusMenu = { x: r.left, y: r.bottom + 4 }; } } }}><Icon name="plus" size={15} /></div>
    <div class="spacer" data-tauri-drag-region></div>
  </div>

  {#if plusMenu}
    <div class="ctxscrim" onclick={() => (plusMenu = null)} role="presentation"></div>
    <div class="plusmenu" style:left="{plusMenu.x}px" style:top="{plusMenu.y}px">
      <button onclick={() => { newTerm(); plusMenu = null; }}><Icon name="terminal" size={13} /><span>New Terminal</span><kbd>⌘T</kbd></button>
      <button onclick={() => { openFileDialog(); plusMenu = null; }}><Icon name="pencil" size={13} /><span>Open File…</span><kbd>⌘E</kbd></button>
      <div class="plus-sep"></div>
      <button onclick={() => { openPanel('preview', { title: 'Web Preview', url: 'http://localhost:5173' }); plusMenu = null; }}><Icon name="globe" size={13} /><span>Web Preview</span></button>
      <button onclick={() => { const f = isMarkdown(activeFile) ? activeFile : `${cwd.replace(/\/$/, '')}/README.md`; openPanel('markdown', { title: `Preview: ${baseName(f)}`, file: f }); plusMenu = null; }}><Icon name="pencil" size={13} /><span>Markdown Preview</span></button>
      <button onclick={() => { openPanel('githistory', { title: 'Git History' }); plusMenu = null; }}><Icon name="history" size={13} /><span>Git History</span></button>
      <div class="plus-sep"></div>
      <button onclick={() => { openView('scm'); plusMenu = null; }}><Icon name="branch" size={13} /><span>Source Control</span></button>
      <button onclick={() => { openView('search'); plusMenu = null; }}><Icon name="search" size={13} /><span>Search</span><kbd>⌘⇧F</kbd></button>
      <button onclick={() => { openView('agent'); plusMenu = null; }}><Icon name="agent" size={13} /><span>AI Agent</span></button>
    </div>
  {/if}

  {#if tabMenu}
    <div class="ctxscrim" onclick={() => (tabMenu = null)} oncontextmenu={(e) => { e.preventDefault(); tabMenu = null; }} role="presentation"></div>
    <div class="tabctx" style:left="{tabMenu.x}px" style:top="{tabMenu.y}px">
      <button onclick={() => { closeFile(tabMenu!.id); tabMenu = null; }}>Close</button>
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
      {#each orderedFiles as f, i (f + '#' + i)}
        <button class:on={activeFile === f} onclick={() => { openInEditor(f); tabOverflow = false; }} title={f}>
          {#if pinnedFiles.includes(f)}<span class="ofpin"><Icon name="pin" size={9} /></span>{/if}<span class="oftt">{baseName(f)}</span>{#if dirtyFiles[f]}<span class="dirty"></span>{/if}
        </button>
      {/each}
    </div>
  {/if}

  <div class="main">
    {#if $autoHideRail}<div class="rail-hot"></div>{/if}
    {#if !railHidden}
    <nav class="rail" aria-label="Activity bar">
      <div class="brandmark" role="button" tabindex="0" title="Anvil — Command Palette (⌘K)" onclick={openCommands} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openCommands())}><Icon name="basin" size={20} /></div>
      <div class="i {activeView === 'term' ? 'on' : ''}" role="button" tabindex="0" title="Terminal" onclick={focusTerm} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), focusTerm())}><Icon name="terminal" /></div>
      <div class="i panel {explorerOpen ? 'pinned' : ''}" role="button" tabindex="0" title="Explorer (⌘B)" onclick={openExplorer} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openExplorer())}><Icon name="folder" /></div>
      <div class="i {activeView === 'scm' ? 'on' : ''}" role="button" tabindex="0" title="Source Control" onclick={() => openView('scm')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('scm'))}><Icon name="branch" /></div>
      <div class="i {activeView === 'search' ? 'on' : ''}" role="button" tabindex="0" title="Search (⌘⇧F)" onclick={() => openView('search')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('search'))}><Icon name="search" /></div>
      <div class="i agent {activeView === 'agent' ? 'on' : ''}" role="button" tabindex="0" title="AI Agent" onclick={() => openView('agent')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('agent'))}><Icon name="agent" /></div>
      {#if railEnabled('devops', $extEnabled)}<div class="i {activeView === 'k8s' ? 'on' : ''}" role="button" tabindex="0" title={kubeFails ? `Kubernetes — ${kubeFails} failing` : "Kubernetes"} onclick={() => openView('k8s')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('k8s'))}><Icon name="kube" />{#if kubeFails}<span class="rail-badge">{kubeFails}</span>{/if}</div>{/if}
      {#if railEnabled('devops', $extEnabled)}<div class="i {activeView === 'ci' ? 'on' : ''}" role="button" tabindex="0" title="CI / Pipelines" onclick={() => openView('ci')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('ci'))}><Icon name="ci" /></div>{/if}
      {#if railEnabled('devops', $extEnabled)}<div class="i {activeView === 'terraform' ? 'on' : ''}" role="button" tabindex="0" title="Terraform / Terragrunt" onclick={() => openView('terraform')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('terraform'))}><Icon name="terraform" /></div>{/if}
      {#if railEnabled('devops', $extEnabled)}<div class="i {activeView === 'obs' ? 'on' : ''}" role="button" tabindex="0" title="Observability (Metrics / Logs)" onclick={() => openView('obs')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('obs'))}><Icon name="chart" /></div>{/if}
      {#if railEnabled('devops', $extEnabled)}<div class="i {activeView === 'devops' ? 'on' : ''}" role="button" tabindex="0" title="DevOps (PRs / GitLab / AWS / Incidents)" onclick={() => openView('devops')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('devops'))}><Icon name="devops" /></div>{/if}
      <div class="i grow {rail === 'settings' ? 'on' : ''}" role="button" tabindex="0" title="Settings" onclick={openSettings} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openSettings())}><Icon name="settings" /></div>
    </nav>
    {/if}

    {#if explorerOpen}
    <aside class="side" style="width:{sideW}px">
      <div class="sect">Explorer <button class="sect-x" title="Hide explorer (⌘B)" onclick={() => (explorerOpen = false)}><Icon name="close" size={11} /></button></div>
      {#if cwd}<FileBrowser bind:path={cwd} onOpenFile={openInEditor} />{/if}
    </aside>
    <Resizer bind:size={sideW} min={160} max={480} def={230} storeKey="anvil-side-w" />
    {/if}

    <svelte:boundary onerror={(e) => { console.error("view crashed", e); toast("This view hit an error — use Reload view", "error"); }}>
    <section class="content">
      {#if rail !== "workspace"}
      <div class="pane-head">
        {#if rail === "diff"}<span class="accent">±</span> Diff — {diffTarget?.rev ?? diffTarget?.path}
        {:else if rail === "settings"}<span class="ph-ic accent"><Icon name="settings" /></span> Settings
        {:else if rail === "editor"}<span class="accent"></span> {activeFile || "Welcome"}
        {:else if rail === "panel"}{@const ap = panels.find((p) => p.id === activePanel)}<span class="ph-ic accent"><Icon name={ap ? panelIcon(ap.kind) : "globe"} /></span> {ap?.title ?? ""}
        {/if}
      </div>
      {/if}

      <!-- Terax-style openable panels (preview / markdown / git history), kept alive like terminals. -->
      <div class="panel-row" style:display={rail === "panel" ? "block" : "none"}>
        {#each panels as p (p.id)}
          {@const on = rail === "panel" && p.id === activePanel}
          <div class="view" style:display={on ? "block" : "none"}>
            {#if p.kind === "preview"}
              {#await WebPreview()}<Skeleton rows={5} />{:then M}<M.default bind:url={p.url} />{/await}
            {:else if p.kind === "markdown"}
              {#key p.file}{#await MarkdownPreview()}<Skeleton rows={8} />{:then M}<M.default path={p.file ?? ""} />{/await}{/key}
            {:else if p.kind === "githistory"}
              {#key cwd}{#await SourceControl()}<Skeleton rows={10} />{:then M}<M.default {cwd} onOpenDiff={(t) => { diffTarget = t; rail = "diff"; }} />{/await}{/key}
            {/if}
          </div>
        {/each}
      </div>

      {#if rail === "diff" && diffTarget}
        <div class="view">
          <div class="difftop"><button class="back" onclick={() => openView("scm")}>← Source Control</button></div>
          {#key JSON.stringify(diffTarget)}
            {#if diffTarget.rev && diffTarget.path}{#await DiffView()}<Skeleton rows={14} />{:then M}<M.default {cwd} rev={diffTarget.rev} path={diffTarget.path} />{/await}
            {:else if diffTarget.rev}<CommitDetail {cwd} rev={diffTarget.rev} />
            {:else}{#await DiffView()}<Skeleton rows={14} />{:then M}<M.default {cwd} path={diffTarget.path} staged={diffTarget.staged} />{/await}{/if}
          {/key}
        </div>
      {:else if rail === "editor" && activeFile}
        <div class="view">
          {#if runbook && isMarkdown(activeFile)}
            {#key activeFile}{#await RunbookView()}<Skeleton rows={10} />{:then M}<M.default path={activeFile} onRun={(c) => { invoke("pty_write", { id: activeTerm, data: c + "\n" }); }} />{/await}{/key}
          {:else if mdPreview && isMarkdown(activeFile)}
            {#key activeFile}{#await MarkdownPreview()}<Skeleton rows={10} />{:then M}<M.default path={activeFile} />{/await}{/key}
          {:else if isNonText(activeFile)}
            {#key activeFile}{#await FileView()}<Skeleton rows={6} />{:then M}<M.default path={activeFile} />{/await}{/key}
          {:else}
            {#await Editor()}
              <Skeleton rows={16} />
            {:then M}
              <M.default path={activeFile} onDirty={(d) => (dirtyFiles = { ...dirtyFiles, [activeFile]: d })} onOpen={(np, ln) => { openInEditor(np); if (ln) editorGoto.set(ln); }} onReferences={showReferences} onExplain={explainCode} />
            {/await}
          {/if}
        </div>
      {:else if rail === "settings"}
        <div class="view">{#await Settings()}<Skeleton rows={12} />{:then M}<M.default />{/await}</div>
      {/if}

      <!-- The grid is the permanent content surface — keep it MOUNTED even when a
           modal-ish overlay (diff/settings/markdown-preview/panel) is showing, so
           terminal panes keep their PTYs instead of respawning on every flip. -->
      <div class="view ws" style:display={rail === "workspace" ? "flex" : "none"}>
          {#snippet paneView(lf: Leaf)}
            <div class="pane-view">
            {#key lf.tabs[lf.active]?.id}
            {#if lf.view === "term"}
              <Terminal id={lf.ref ?? lf.id} {cwd} shell={termShells[lf.ref ?? ""] ?? ""} active={rail === "workspace" && lf.id === activeLeaf} />
            {:else if lf.view === "files"}
              {#key cwd}<FileBrowser bind:path={cwd} onOpenFile={openInEditor} />{/key}
            {:else if lf.view === "scm"}
              {#key cwd}{#await SourceControl()}<Skeleton rows={10} />{:then M}<M.default {cwd} onOpenDiff={(t) => { diffTarget = t; }} />{/await}{/key}
            {:else if lf.view === "search"}
              {#key cwd}{#await SearchPanel()}<Skeleton rows={8} />{:then M}<M.default root={cwd} onOpen={(p) => openInEditor(p)} />{/await}{/key}
            {:else if lf.view === "agent"}
              {#await AgentPanel()}<Skeleton rows={8} />{:then M}<M.default {cwd} attachPath={activeFile}
                listFiles={() => invoke<string[]>("walk_dir", { root: cwd.replace(/\/$/, "") })}
                onReadFile={(p) => invoke<string>("read_file", { path: p })}
                onApplyFile={(path, content) => { invoke("write_file", { path, contents: content }); toast(`Applied edit to ${path.split("/").pop()}`, "success"); }}
                getTerminalText={() => readTerminal(activeTerm)}
                onRunCommand={(c) => { invoke("pty_write", { id: activeTerm, data: c + "\n" }); focusTerm(); }}
                onReply={(summary) => { if (document.hidden) notifyAgent(summary); }} />{/await}
            {:else if lf.view === "devops"}
              {#key cwd}{#await DevOps()}<Skeleton rows={9} />{:then M}<M.default {cwd} onRunCommand={(c) => invoke("pty_write", { id: activeTerm, data: c + "\n" })} onInvestigate={investigate} />{/await}{/key}
            {:else if lf.view === "k8s"}
              {#key cwd}{#await Kube()}<Skeleton rows={10} />{:then M}<M.default {cwd} active={true} onRunCommand={sendToTerm} onHealth={(n) => (kubeFails = n)} onInvestigate={investigate} onCheckConnections={() => (doctorOpen = true)} />{:catch e}<div class="view-err">Kubernetes view failed to load: {e}</div>{/await}{/key}
            {:else if lf.view === "ci"}
              {#key cwd}{#await CI()}<Skeleton rows={12} />{:then M}<M.default {cwd} active={true} onRunCommand={sendToTerm} onInvestigate={investigate} />{/await}{/key}
            {:else if lf.view === "terraform"}
              {#key cwd}{#await Terraform()}<Skeleton rows={7} />{:then M}<M.default {cwd} onRunCommand={sendToTerm} onInvestigate={investigate} />{/await}{/key}
            {:else if lf.view === "obs"}
              {#await Observability()}<Skeleton rows={6} />{:then M}<M.default />{/await}
            {:else if lf.view === "editor" && openFiles.includes(lf.ref || activeFile)}
              {@const p = lf.ref || activeFile}
              {#if isNonText(p)}
                {#key p}{#await FileView()}<Skeleton rows={6} />{:then M}<M.default path={p} />{/await}{/key}
              {:else}
                {#await Editor()}<Skeleton rows={16} />{:then M}<M.default path={p} onDirty={(d) => (dirtyFiles = { ...dirtyFiles, [p]: d })} onOpen={(np, ln) => { openInEditor(np); if (ln) editorGoto.set(ln); }} onReferences={showReferences} onExplain={explainCode} />{/await}
              {/if}
            {:else}
              <Welcome recent={recentFiles} onOpenRecent={openInEditor} onNewTerminal={newTerm} onCommandPalette={openCommands} onNewFile={newRootFile} onNewFolder={newRootFolder} onOpenFile={openFileDialog} onOpenFolder={openFolder} />
            {/if}
            {/key}
            </div>
          {/snippet}
          <div class="grid-fill">
          <PaneGrid node={paneTree} view={paneView} activeId={activeLeaf} solo={paneTree.kind === "leaf"}
            onSplit={wsSplit} onClose={wsClose} onSetView={wsSetView} onResize={wsResize}
            onSetActiveTab={wsSetActiveTab} onCloseTab={wsCloseTab} onAddTab={wsAddTab}
            {dropHint} onTabPointerDown={paneTabPointerDown} zoomId={zoomedLeaf} dim={$focusDimming}
            onFocusLeaf={(id) => (activeLeaf = id)} />
          </div>
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
            <span class="x" role="button" tabindex="0" onclick={() => (bottomDock = false)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), (bottomDock = false))} title="Close (⌘J)">×</span>
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
          {#if originFrame((error as Error)?.stack)}
            <div class="cf-where">at {originFrame((error as Error)?.stack)}</div>
          {/if}
          <div class="cf-actions">
            <button class="cf-btn" onclick={reset}>Reload view</button>
            <button class="cf-btn" onclick={() => navigator.clipboard.writeText(`${String(error)}\n${(error as Error)?.stack ?? ""}`).then(() => toast("Error copied", "success"))}>Copy error</button>
          </div>
        </div>
      </section>
    {/snippet}
    </svelte:boundary>
  </div>

  <div class="status">
    <span class="si" role="button" tabindex="0" style="cursor:default" title="Open Source Control"
      onclick={() => openView('scm')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('scm'))}><Icon name="branch" size={12} /> {branch || "—"}{#if aheadBehind} <span class="ab">↑{aheadBehind.a} ↓{aheadBehind.b}</span>{/if}</span>
    <span class="si" role="button" tabindex="0" style="cursor:default" title={`${cwd} — toggle Explorer`}
      onclick={toggleSide} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), toggleSide())}><Icon name="folder" size={11} /> {baseName(cwd) || "~"}</span>
    {#if kubeCtx}
      <span class="si kctx" class:prod={/prod|prd|production/i.test(kubeCtx)} role="button" tabindex="0"
        title={`Active kube context: ${kubeCtx} — open Kubernetes`} style="cursor:default"
        onclick={() => openView('k8s')} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openView('k8s'))}><Icon name="kube" size={11} /> {kubeCtx}</span>
    {/if}
    <div class="r">
      <span class="si" role="button" tabindex="0" onclick={toggleDensity} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), toggleDensity())} title="Toggle density" style="cursor:default">{$density}</span>
      <span class="si" role="button" tabindex="0" onclick={cycleTheme} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), cycleTheme())} title="Cycle theme" style="cursor:default">{themeLabel($activeTheme)}</span>
      <span class="si bell" role="button" tabindex="0" onclick={() => (notifOpen = !notifOpen)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), notifOpen = !notifOpen)} title="Notifications" style="cursor:default"><Icon name="bell" size={12} />{#if unreadCount}<span class="bell-badge">{unreadCount > 9 ? "9+" : unreadCount}</span>{/if}</span>
      {#if lspCurLang}
        <span class="si lsp {lspCurState}" role="button" tabindex="0" style="cursor:default"
          title={lspCurState === "up" ? `${LSP_LABEL[lspCurLang] ?? lspCurLang} connected — click to restart` : lspCurState === "starting" ? `${LSP_LABEL[lspCurLang] ?? lspCurLang} starting…` : `${LSP_LABEL[lspCurLang] ?? lspCurLang} not running — click to start`}
          onclick={() => restartLsp(lspCurLang!, cwd)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), restartLsp(lspCurLang!, cwd))}>
          <span class="lsp-dot"></span>{LSP_LABEL[lspCurLang] ?? lspCurLang}
        </span>
      {/if}
      <span class="ok" title="Ready">●</span>
      <span>UTF-8</span>
    </div>
  </div>

  {#if zen}<div class="zen-exit" role="button" tabindex="0" onclick={() => toggleZen()} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), toggleZen())} title="Exit zen mode (⌘.)">⌘. exit zen</div>{/if}
  <Palette bind:open={paletteOpen} items={paletteItems} placeholder={palettePlaceholder} />
  <Dialog />
  <Toasts />
  <NotificationCenter bind:open={notifOpen} />

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
          <button class="ob-go" onclick={tourNext}>{obStep === TOUR.length - 1 ? "Check connections" : "Next"}</button>
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
  {#if doctorOpen}
    <Doctor onClose={() => (doctorOpen = false)} onRunCommand={sendToTerm} />
  {/if}
  {#if attentionOpen}
    <Attention {cwd} onClose={() => (attentionOpen = false)} onOpenView={openView} onInvestigate={investigate} />
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
    <button class="agentq-chip" title="Queued agent tasks — click to run the next" onclick={() => { const t = dequeueAgent(); if (t) { agentSeed.set(t.prompt); openView("agent"); } }}>⚙ {$agentQueue.length} queued</button>
  {/if}

  {#if !$online}
    <div class="offline-chip" title="No network — agent, k8s, and observability calls will fail until you reconnect">⚠ offline — network features paused</div>
  {/if}

  {#if $broadcastInput}
    <button class="broadcast-chip" title="Broadcast input is ON — every keystroke goes to all terminals" onclick={() => broadcastInput.set(false)}>
      ⊚ broadcast · click to stop
    </button>
  {/if}

  {#if tabDrag}
    <!-- Floating drag ghost: follows the cursor while a tab is being dragged
         onto a pane (the .dropzone highlight is rendered inside the target leaf). -->
    <div class="dragghost" style:transform="translate3d({dragXY.x + 12}px, {dragXY.y + 12}px, 0)" aria-hidden="true">{tabDrag.label}</div>
  {/if}
</div>

<style>
  .pane-head .ph-ic { display: inline-flex; align-items: center; vertical-align: -2px; margin-right: 3px; }
  .status .si { display: inline-flex; align-items: center; gap: 4px; }
  .status .si.lsp .lsp-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--text3); flex: 0 0 auto; }
  .status .si.lsp.up .lsp-dot { background: var(--status-verified, var(--green, #3fb950)); }
  .status .si.lsp.starting .lsp-dot { background: var(--status-attention, var(--yellow, #d8a657)); animation: lsp-pulse 1s ease-in-out infinite; }
  .status .si.lsp.down .lsp-dot { background: var(--status-risk, var(--red, #e5484d)); }
  @keyframes lsp-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }
  .status .si.bell { position: relative; }
  .status .si.bell .bell-badge { position: absolute; top: -4px; right: -6px; min-width: 13px; height: 13px;
    padding: 0 3px; border-radius: 7px; background: var(--red); color: #fff; font-size: 9px; line-height: 13px;
    text-align: center; font-variant-numeric: tabular-nums; }
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
  .view.ws { padding: 0; gap: 0; position: relative; }
  /* Pin the pane grid to a definite-size box. WebKit won't resolve height:100%
     against a flex-stretched (indefinite) parent, so nested split panes floated
     inset with open space. absolute inset:0 gives the grid a definite frame; the
     flex .cell/.leaf chain then fills it edge-to-edge. */
  .grid-fill { position: absolute; inset: 0; display: flex; }
  .grid-fill > :global(.split), .grid-fill > :global(.leaf) { flex: 1 1 auto; min-width: 0; min-height: 0; }
  /* Every pane's content fills the pane (was rendering content-sized → a tiny
     floating card for non-editor views like DevOps/Kube). Force the rendered
     view component to flex-fill. */
  .pane-view { height: 100%; width: 100%; display: flex; flex-direction: column; min-width: 0; min-height: 0; overflow: hidden; }
  .pane-view > :global(*) { flex: 1 1 auto; min-width: 0; min-height: 0; }
  .difftop { padding: 6px 12px; border-bottom: 1px solid var(--border); flex: 0 0 auto; }
  .back { border: 0; background: transparent; color: var(--accent); font-size: 12px; cursor: default; }
  .tab .x { margin-left: 8px; color: var(--text3); font-size: 13px; }
  .tab .x:hover { color: var(--text); }
  .dirty { display: inline-block; width: 7px; height: 7px; margin-left: 7px; border-radius: 50%;
    background: var(--accent); vertical-align: middle; }
  /* Floating drag ghost (pointer-based tab drag). Positioned via a GPU transform
     (set inline, cursor + 12px offset) so it tracks the pointer on the compositor
     without per-move layout, and stays off elementFromPoint hit-testing. */
  .dragghost { position: fixed; left: 0; top: 0; z-index: 100; pointer-events: none; will-change: transform;
    background: var(--panel); color: var(--accent); border: 1px solid var(--accent); border-radius: 6px;
    padding: 3px 9px; font-family: var(--font-ui); font-size: 11.5px; font-weight: 500; white-space: nowrap;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.35); max-width: 220px; overflow: hidden; text-overflow: ellipsis; }
  .newtab { display: flex; align-items: center; padding: 0 12px; color: var(--text3); font-size: 16px;
    -webkit-app-region: no-drag; cursor: default; }
  .newtab:hover { color: var(--text); }
</style>
