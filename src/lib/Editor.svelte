<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { EditorState, Compartment, StateEffect, StateField, type Extension } from "@codemirror/state";
  import { EditorView, keymap, Decoration, type DecorationSet, WidgetType } from "@codemirror/view";
  import { indentUnit, forceParsing, foldedRanges, foldEffect } from "@codemirror/language";
  import { basicSetup } from "codemirror";
  import { indentWithTab } from "@codemirror/commands";
  import { indentationMarkers } from "@replit/codemirror-indentation-markers";
  import { showMinimap } from "@replit/codemirror-minimap";
  import { colorSwatches } from "$lib/cm-color";
  import { cmTheme } from "$lib/cm-theme";
  import { cmLang } from "$lib/cm-lang";
  import { cmSnippets } from "$lib/cm-snippets";
  import { activeTheme } from "$lib/themes";
  import { editorFontSize, editorTabSize, editorWordWrap, editorLigatures, editorLineHeight, editorLetterSpacing, editorBlameAlways, editorFormatOnSave, editorMinimap, editorInlayHints, editorGhostText, editorGhostSource, editorGoto } from "$lib/editor-settings";
  import { cmGhost } from "$lib/cm-ghost";
  import { vim } from "@replit/codemirror-vim";
  import { editorVimMode } from "$lib/editor-settings";
  import { monoFont, editorBold } from "$lib/fonts";
  import { parseConflicts, resolvedLines, resolveAll, type MergeChoice } from "$lib/merge";
  import { lspLang, ensureLsp, didOpen, didChange } from "$lib/lsp";
  import { cmLsp, formatDoc, cmInlayHints, fetchSymbols, enclosingSymbols, type RefLoc, type OutlineSym } from "$lib/cm-lsp";
  import { editorStickyScroll } from "$lib/editor-settings";
  import { editorLive } from "$lib/editor-live";
  import { toast } from "$lib/toast";

  let { path, onDirty, onOpen, onReferences, onExplain }: {
    path: string;
    onDirty?: (d: boolean) => void;
    onOpen?: (path: string, line?: number, col?: number) => void;
    onReferences?: (refs: RefLoc[]) => void;
    onExplain?: (code: string, path: string) => void;
  } = $props();

  const monoStack = (f: string) => `"${f}", "Symbols Nerd Font Mono", "SF Mono", Menlo, ui-monospace, monospace`;

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  let loadedPath = "";
  let unsub: () => void;

  const states = new Map<string, EditorState>();
  const dirtyPaths = new Set<string>();
  const mtimes = new Map<string, number>();
  const versions = new Map<string, number>();
  let watchTimer: ReturnType<typeof setInterval> | undefined;
  let changeTimer: ReturnType<typeof setTimeout> | undefined;
  let gutterTimer: ReturnType<typeof setTimeout> | undefined;

  // Reconfigurable slots.
  const themeComp = new Compartment();
  const langComp = new Compartment();
  const wrapComp = new Compartment();
  const tabComp = new Compartment();
  const fontComp = new Compartment();
  const minimapComp = new Compartment();
  const inlayComp = new Compartment();

  function inlayExt(p: string) {
    const lang = lspLang(p);
    return $editorInlayHints && lang ? cmInlayHints(lang, p) : [];
  }

  function minimapExt() {
    return $editorMinimap
      ? showMinimap.compute([], () => ({
          create: () => ({ dom: document.createElement("div") }),
          displayText: "blocks",
          showOverlay: "always",
        }))
      : [];
  }

  // ── Breadcrumb + sticky scroll (from LSP document symbols) ──
  const symbolsCache = new Map<string, OutlineSym[]>();
  let crumbs = $state<{ name: string; line: number }[]>([]);
  let crumbFile = $state("");
  let stickyRows = $state<{ text: string; line: number }[]>([]);

  async function loadSymbols(p: string) {
    const lang = lspLang(p);
    if (!lang) { symbolsCache.set(p, []); recomputeScope(); return; }
    try { symbolsCache.set(p, await fetchSymbols(lang, p)); } catch { symbolsCache.set(p, []); }
    if (loadedPath === p) recomputeScope();
  }
  function recomputeScope() {
    if (!view) return;
    const syms = symbolsCache.get(loadedPath) ?? [];
    const line = view.state.doc.lineAt(view.state.selection.main.head).number;
    crumbs = enclosingSymbols(syms, line).map((s) => ({ name: s.name, line: s.line }));
    crumbFile = loadedPath.split("/").pop() ?? "";
    if ($editorStickyScroll && syms.length) {
      const top = view.lineBlockAtHeight(view.scrollDOM.scrollTop).from;
      const topLine = view.state.doc.lineAt(top).number;
      stickyRows = enclosingSymbols(syms, topLine)
        .filter((s) => s.line < topLine)
        .map((s) => ({ text: view!.state.doc.line(Math.min(s.line, view!.state.doc.lines)).text, line: s.line }));
    } else stickyRows = [];
  }
  function jumpLine(line: number) {
    if (!view) return;
    const l = view.state.doc.line(Math.min(line, view.state.doc.lines));
    view.dispatch({ selection: { anchor: l.from }, effects: EditorView.scrollIntoView(l.from, { y: "start", yMargin: 36 }) });
    view.focus();
  }

  function fontTheme(): Extension {
    return EditorView.theme({
      "&": { fontSize: `${$editorFontSize}px` },
      ".cm-scroller": {
        fontFamily: monoStack($monoFont),
        lineHeight: String($editorLineHeight || 1.55),
        letterSpacing: `${$editorLetterSpacing ?? 0}px`,
        fontVariantLigatures: $editorLigatures ? "normal" : "none",
        fontWeight: $editorBold ? "600" : "400",
      },
    });
  }

  // ── git gutter + blame: line decorations driven by StateEffects ──
  const setGutter = StateEffect.define<DecorationSet>();
  const gutterField = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(v, tr) {
      v = v.map(tr.changes);
      for (const e of tr.effects) if (e.is(setGutter)) v = e.value;
      return v;
    },
    provide: (f) => EditorView.decorations.from(f),
  });
  const setBlame = StateEffect.define<DecorationSet>();
  const blameField = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(v, tr) {
      v = v.map(tr.changes);
      for (const e of tr.effects) if (e.is(setBlame)) v = e.value;
      return v;
    },
    provide: (f) => EditorView.decorations.from(f),
  });
  const setMarks = StateEffect.define<DecorationSet>();
  const marksField = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(v, tr) {
      v = v.map(tr.changes);
      for (const e of tr.effects) if (e.is(setMarks)) v = e.value;
      return v;
    },
    provide: (f) => EditorView.decorations.from(f),
  });

  // ── Bookmarks (#85): per-file marked lines, persisted ──
  const bookmarks = new Map<string, Set<number>>();
  (function loadMarks() {
    if (typeof localStorage === "undefined") return;
    try {
      const o = JSON.parse(localStorage.getItem("anvil-bookmarks") || "{}");
      for (const [p, arr] of Object.entries<number[]>(o)) bookmarks.set(p, new Set(arr));
    } catch { /* ignore */ }
  })();
  function saveMarks() {
    if (typeof localStorage === "undefined") return;
    const o: Record<string, number[]> = {};
    for (const [p, s] of bookmarks) if (s.size) o[p] = [...s].sort((a, b) => a - b);
    try { localStorage.setItem("anvil-bookmarks", JSON.stringify(o)); } catch { /* ignore */ }
  }
  function refreshMarks() {
    if (!view) return;
    const s = bookmarks.get(loadedPath);
    const doc = view.state.doc;
    const decos = [...(s ?? [])]
      .filter((ln) => ln >= 1 && ln <= doc.lines)
      .sort((a, b) => a - b)
      .map((ln) => Decoration.line({ class: "anvil-mark" }).range(doc.line(ln).from));
    view.dispatch({ effects: setMarks.of(Decoration.set(decos, true)) });
  }
  function toggleBookmark() {
    if (!view) return;
    const ln = view.state.doc.lineAt(view.state.selection.main.head).number;
    let s = bookmarks.get(loadedPath);
    if (!s) { s = new Set(); bookmarks.set(loadedPath, s); }
    s.has(ln) ? s.delete(ln) : s.add(ln);
    saveMarks();
    refreshMarks();
  }
  function jumpBookmark(dir: 1 | -1) {
    if (!view) return;
    const s = bookmarks.get(loadedPath);
    if (!s || !s.size) return;
    const cur = view.state.doc.lineAt(view.state.selection.main.head).number;
    const sorted = [...s].sort((a, b) => a - b);
    const next = dir === 1 ? sorted.find((l) => l > cur) ?? sorted[0] : [...sorted].reverse().find((l) => l < cur) ?? sorted[sorted.length - 1];
    jumpLine(next);
  }

  class BlameWidget extends WidgetType {
    text: string; color: string;
    constructor(text: string, color: string) { super(); this.text = text; this.color = color; }
    eq(o: BlameWidget) { return o.text === this.text && o.color === this.color; }
    toDOM() { const s = document.createElement("span"); s.className = "cm-blame"; s.style.color = this.color; s.textContent = `    ${this.text}`; return s; }
  }

  // Large-file mode (#10): past ~2 MB skip syntax highlighting, LSP, color
  // swatches, indent markers, minimap — keep the editor responsive.
  const BIG_FILE = 2_000_000;
  function baseExtensions(p: string, big = false): Extension[] {
    return [
      ...($editorVimMode ? [vim()] : []), // #89 vim keybindings — must precede keymaps
      basicSetup,
      themeComp.of(cmTheme($activeTheme)),
      langComp.of(big ? [] : cmLang(p)),
      ...(big ? [] : [cmSnippets(p)]),
      wrapComp.of($editorWordWrap ? EditorView.lineWrapping : []),
      tabComp.of([EditorState.tabSize.of($editorTabSize), indentUnit.of(" ".repeat($editorTabSize))]),
      fontComp.of(fontTheme()),
      ...(big ? [] : [colorSwatches()]),
      ...(big ? [] : [indentationMarkers({
        highlightActiveBlock: true,
        hideFirstIndent: true,
        thickness: 1,
        colors: {
          light: "color-mix(in srgb, var(--text3) 38%, transparent)",
          dark: "color-mix(in srgb, var(--text3) 38%, transparent)",
          activeLight: "color-mix(in srgb, var(--text2) 85%, transparent)",
          activeDark: "color-mix(in srgb, var(--text2) 85%, transparent)",
        },
      })]),
      minimapComp.of(big ? [] : minimapExt()),
      inlayComp.of(big ? [] : inlayExt(p)),
      ...(!big && lspLang(p) ? [cmLsp(lspLang(p)!, p, { onOpen: (np, ln, col) => onOpen?.(np, ln, col), onReferences: (refs) => onReferences?.(refs) })] : []),
      ...(!big && lspLang(p) && $editorGhostText ? [cmGhost(lspLang(p)!, p, $editorGhostSource)] : []),
      gutterField,
      blameField,
      marksField,
      keymap.of([
        indentWithTab,
        { key: "Mod-s", preventDefault: true, run: () => { save(); return true; } },
        { key: "Alt-b", preventDefault: true, run: () => { toggleBlame(); return true; } },
        { key: "Mod-Alt-e", preventDefault: true, run: (v) => { const sel = v.state.sliceDoc(v.state.selection.main.from, v.state.selection.main.to); if (sel.trim()) onExplain?.(sel, loadedPath); return true; } },
        { key: "Mod-Alt-k", preventDefault: true, run: () => { toggleBookmark(); return true; } },
        { key: "Mod-Alt-j", preventDefault: true, run: () => { jumpBookmark(1); return true; } },
        { key: "Mod-Alt-Shift-j", preventDefault: true, run: () => { jumpBookmark(-1); return true; } },
      ]),
      EditorView.updateListener.of((u) => {
        if (u.selectionSet || u.docChanged) recomputeScope();
        if (!u.docChanged) return;
        const p2 = loadedPath;
        dirtyPaths.add(p2);
        onDirty?.(true);
        scanConflicts();
        scheduleGutter();
        pushChange(p2);
        if (/\.(md|markdown)$/i.test(p2)) editorLive.set({ path: p2, text: u.state.doc.toString() }); // #6
      }),
      EditorView.domEventHandlers({ scroll() { recomputeScope(); return false; } }),
    ];
  }

  function pushChange(p: string) {
    const lang = lspLang(p);
    if (!lang || !view) return;
    clearTimeout(changeTimer);
    const text = view.state.doc.toString();
    changeTimer = setTimeout(() => {
      const v = (versions.get(p) ?? 1) + 1;
      versions.set(p, v);
      didChange(lang, p, text, v);
    }, 250);
  }

  // #8 Code-fold persistence across restarts: save fold line-ranges per path.
  function saveFolds(p: string, state: EditorState) {
    if (!p) return;
    const ranges: { from: number; to: number }[] = [];
    foldedRanges(state).between(0, state.doc.length, (from, to) => {
      ranges.push({ from: state.doc.lineAt(from).number, to: state.doc.lineAt(to).number });
    });
    try {
      if (ranges.length) localStorage.setItem(`anvil-folds:${p}`, JSON.stringify(ranges));
      else localStorage.removeItem(`anvil-folds:${p}`);
    } catch { /* ignore */ }
  }
  function restoreFolds(p: string) {
    let ranges: { from: number; to: number }[] = [];
    try { ranges = JSON.parse(localStorage.getItem(`anvil-folds:${p}`) || "[]"); } catch { return; }
    if (!ranges.length || !view) return;
    const lines = view.state.doc.lines;
    const effects = [];
    for (const r of ranges) {
      if (r.from < 1 || r.to > lines || r.from >= r.to) continue;
      const from = view.state.doc.line(r.from).to;
      const to = view.state.doc.line(r.to).to;
      if (from < to) effects.push(foldEffect.of({ from, to }));
    }
    if (effects.length) view.dispatch({ effects });
  }

  // #68 Cursor + scroll restore across restarts (per path).
  function saveViewPos(p: string) {
    if (!p || !view) return;
    try { localStorage.setItem(`anvil-viewpos:${p}`, JSON.stringify({ head: view.state.selection.main.head, top: view.scrollDOM.scrollTop })); } catch { /* ignore */ }
  }
  function restoreViewPos(p: string) {
    let pos: { head: number; top: number } | null = null;
    try { pos = JSON.parse(localStorage.getItem(`anvil-viewpos:${p}`) || "null"); } catch { return; }
    if (!pos || !view) return;
    const head = Math.min(Math.max(0, pos.head), view.state.doc.length);
    view.dispatch({ selection: { anchor: head } });
    requestAnimationFrame(() => { if (view && loadedPath === p) view.scrollDOM.scrollTop = pos!.top; });
  }

  async function load(p: string) {
    if (!view || !p || p === loadedPath) return;
    // Stash the outgoing doc so unsaved edits survive a tab switch.
    if (loadedPath) { saveFolds(loadedPath, view.state); saveViewPos(loadedPath); states.set(loadedPath, view.state); }
    loadedPath = p;
    let state = states.get(p);
    let fresh = false;
    if (!state) {
      fresh = true;
      let text = "";
      try { text = await invoke<string>("read_file", { path: p }); } catch (e) { text = ""; toast("Could not open file: " + String(e).slice(0, 80), "error"); }
      if (loadedPath !== p) return; // a newer load() superseded this one mid-await
      state = EditorState.create({ doc: text, extensions: baseExtensions(p, text.length > BIG_FILE) });
      states.set(p, state);
      try { mtimes.set(p, await invoke<number>("file_mtime", { path: p })); } catch { /* ignore */ }
      const lang = lspLang(p);
      if (lang) {
        const dir = p.slice(0, p.lastIndexOf("/")) || "/";
        ensureLsp(lang, dir).then((ok) => {
          if (!ok) return;
          versions.set(p, 1);
          didOpen(lang, p, text, 1);
        });
      }
    }
    view.setState(state);
    // A fresh state is built with current theme/lang/settings already; only a
    // restored (cached) state may carry stale config and needs reconfiguring.
    if (!fresh) {
      view.dispatch({
        effects: [
          themeComp.reconfigure(cmTheme($activeTheme)),
          langComp.reconfigure(cmLang(p)),
          wrapComp.reconfigure($editorWordWrap ? EditorView.lineWrapping : []),
          tabComp.reconfigure([EditorState.tabSize.of($editorTabSize), indentUnit.of(" ".repeat($editorTabSize))]),
          fontComp.reconfigure(fontTheme()),
        ],
      });
    }
    // Parse the visible viewport synchronously so syntax is colored on the first
    // paint instead of "popping in" a frame later (Lezer parses async by default).
    forceParsing(view, view.viewport.to, 80);
    if (fresh) { restoreFolds(p); restoreViewPos(p); }
    scanConflicts();
    refreshGutter();
    if ($editorBlameAlways) renderBlame(); else clearBlame();
    crumbs = []; stickyRows = [];
    refreshMarks();
    if (symbolsCache.has(p)) recomputeScope(); else loadSymbols(p);
    onDirty?.(dirtyPaths.has(p));
  }

  async function save() {
    if (!view) return;
    const p = loadedPath;
    const lang = lspLang(p);
    if ($editorFormatOnSave && lang) {
      try { await formatDoc(lang, p, view, $editorTabSize); } catch { /* server can't format — save as-is */ }
    }
    try {
      await invoke("write_file", { path: p, contents: view.state.doc.toString() });
      dirtyPaths.delete(p);
      try { mtimes.set(p, await invoke<number>("file_mtime", { path: p })); } catch { /* ignore */ }
      onDirty?.(false);
      refreshGutter();
      loadSymbols(p);
    } catch (e) { toast("Save failed: " + String(e).slice(0, 80), "error"); }
  }

  // ── Merge-conflict resolver (#34) ──
  let conflictCount = $state(0);
  let hasBase = $state(false);
  const hasConflicts = $derived(conflictCount > 0);
  function docLines(): string[] { return view ? view.state.doc.toString().split("\n") : []; }
  function scanConflicts() {
    const conflicts = parseConflicts(docLines());
    conflictCount = conflicts.length;
    hasBase = conflicts.some((c) => c.base.length > 0);
  }
  function lineRange(startLine: number, endLine: number): { from: number; to: number } {
    const doc = view!.state.doc;
    const from = doc.line(startLine + 1).from;
    const to = doc.line(Math.min(doc.lines, endLine + 1)).to;
    return { from, to };
  }
  function resolveFirst(choice: MergeChoice) {
    if (!view) return;
    const [c] = parseConflicts(docLines());
    if (!c) return;
    const { from, to } = lineRange(c.start, c.end);
    view.dispatch({ changes: { from, to, insert: resolvedLines(c, choice).join("\n") } });
    scanConflicts();
  }
  function resolveEvery(choice: MergeChoice) {
    if (!view) return;
    const merged = resolveAll(docLines(), choice).join("\n");
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: merged } });
    scanConflicts();
  }

  // ── Git gutter (#32) ──
  function gutterDecos(diff: string): DecorationSet {
    if (!view) return Decoration.none;
    const doc = view.state.doc;
    const marks: { line: number; cls: string }[] = [];
    let newLine = 0;
    let pendingDel = false;
    for (const ln of diff.split("\n")) {
      const m = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@/.exec(ln);
      if (m) { newLine = parseInt(m[1], 10); pendingDel = false; continue; }
      if (newLine === 0) continue;
      const c = ln[0];
      if (c === "+") { marks.push({ line: newLine, cls: pendingDel ? "anvil-g-mod" : "anvil-g-add" }); newLine++; }
      else if (c === "-") { pendingDel = true; marks.push({ line: Math.max(1, newLine), cls: "anvil-g-del" }); }
      else { pendingDel = false; if (c === " ") newLine++; }
    }
    const decos = marks
      .filter((mk) => mk.line >= 1 && mk.line <= doc.lines)
      .map((mk) => Decoration.line({ class: mk.cls }).range(doc.line(mk.line).from));
    decos.sort((a, b) => a.from - b.from);
    return Decoration.set(decos, true);
  }
  async function refreshGutter() {
    if (!view || !loadedPath) return;
    const dir = loadedPath.slice(0, loadedPath.lastIndexOf("/")) || "/";
    let raw = "";
    try { raw = await invoke<string>("git_diff", { cwd: dir, path: loadedPath, staged: false }); }
    catch { view.dispatch({ effects: setGutter.of(Decoration.none) }); return; }
    view.dispatch({ effects: setGutter.of(gutterDecos(raw)) });
  }
  function scheduleGutter() { clearTimeout(gutterTimer); gutterTimer = setTimeout(refreshGutter, 400); }

  // ── Inline git blame (Alt+B) ──
  function parseBlame(raw: string): { author: string; date: string; time: number }[] {
    const out: { author: string; date: string; time: number }[] = [];
    let cur: { author?: string; time?: number } = {};
    for (const l of raw.split("\n")) {
      if (/^[0-9a-f]{40} \d+ \d+/.test(l)) cur = {};
      else if (l.startsWith("author ")) cur.author = l.slice(7);
      else if (l.startsWith("author-time ")) cur.time = parseInt(l.slice(12), 10);
      else if (l.startsWith("\t")) {
        const d = cur.time ? new Date(cur.time * 1000).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "2-digit" }) : "";
        out.push({ author: cur.author ?? "?", date: d, time: cur.time ?? 0 });
      }
    }
    return out;
  }
  let blameOn = false;
  function clearBlame() { blameOn = false; view?.dispatch({ effects: setBlame.of(Decoration.none) }); }
  // Age heatmap (#26): recent lines tint toward accent, old toward text3.
  function blameColor(t: number, min: number, max: number): string {
    if (!t || max <= min) return "var(--text3)";
    const r = Math.max(0, Math.min(1, (t - min) / (max - min))); // 0 old → 1 new
    return `color-mix(in srgb, var(--accent) ${Math.round(r * 70)}%, var(--text3))`;
  }
  async function renderBlame() {
    if (!view) return;
    const dir = loadedPath.slice(0, loadedPath.lastIndexOf("/")) || "/";
    let raw = "";
    try { raw = await invoke<string>("git_blame", { cwd: dir, path: loadedPath }); } catch { return; }
    const b = parseBlame(raw);
    const times = b.map((e) => e.time).filter(Boolean);
    const min = Math.min(...times), max = Math.max(...times);
    const doc = view.state.doc;
    const decos = b
      .filter((_, i) => i + 1 <= doc.lines)
      .map((e, i) => Decoration.widget({ widget: new BlameWidget(`${e.author} • ${e.date}`, blameColor(e.time, min, max)), side: 1 }).range(doc.line(i + 1).to));
    blameOn = true;
    view.dispatch({ effects: setBlame.of(Decoration.set(decos, true)) });
  }
  function toggleBlame() { if (blameOn) clearBlame(); else renderBlame(); }

  // ── External-change reload ──
  async function pollExternalChanges() {
    const p = loadedPath;
    if (!p || dirtyPaths.has(p) || !view || document.hidden) return;
    let mt = 0;
    try { mt = await invoke<number>("file_mtime", { path: p }); } catch { return; }
    if (mt !== 0 && mt !== mtimes.get(p)) {
      mtimes.set(p, mt);
      let text = "";
      try { text = await invoke<string>("read_file", { path: p }); } catch { return; }
      if (view.state.doc.toString() !== text) {
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } });
        dirtyPaths.delete(p);
        onDirty?.(false);
      }
    }
  }

  onMount(() => {
    view = new EditorView({ parent: host, state: EditorState.create({ doc: "", extensions: baseExtensions(path) }) });
    unsub = activeTheme.subscribe((n) => view?.dispatch({ effects: themeComp.reconfigure(cmTheme(n)) }));
    watchTimer = setInterval(pollExternalChanges, 2000);
    load(path);
  });

  $effect(() => { load(path); });
  $effect(() => {
    const n = $editorGoto;
    if (n && view) {
      const line = view.state.doc.line(Math.min(n, view.state.doc.lines));
      view.dispatch({ selection: { anchor: line.from }, effects: EditorView.scrollIntoView(line.from, { y: "center" }) });
      view.focus();
      editorGoto.set(null);
    }
  });
  // Live settings → reconfigure the active editor.
  $effect(() => {
    void [$editorFontSize, $editorLineHeight, $editorLetterSpacing, $editorLigatures, $monoFont, $editorBold];
    view?.dispatch({ effects: fontComp.reconfigure(fontTheme()) });
  });
  $effect(() => { view?.dispatch({ effects: wrapComp.reconfigure($editorWordWrap ? EditorView.lineWrapping : []) }); });
  $effect(() => { view?.dispatch({ effects: tabComp.reconfigure([EditorState.tabSize.of($editorTabSize), indentUnit.of(" ".repeat($editorTabSize))]) }); });
  $effect(() => { void $editorMinimap; view?.dispatch({ effects: minimapComp.reconfigure(minimapExt()) }); });
  $effect(() => { void $editorInlayHints; if (view && loadedPath) view.dispatch({ effects: inlayComp.reconfigure(inlayExt(loadedPath)) }); });

  onDestroy(() => {
    if (view && loadedPath) { saveFolds(loadedPath, view.state); saveViewPos(loadedPath); }
    unsub?.();
    if (watchTimer) clearInterval(watchTimer);
    if (changeTimer) clearTimeout(changeTimer);
    if (gutterTimer) clearTimeout(gutterTimer);
    view?.destroy();
  });
</script>

<div class="ed-wrap">
  {#if hasConflicts}
    <div class="conflictbar">
      <span class="cw">⚠ {conflictCount} conflict{conflictCount === 1 ? "" : "s"}</span>
      <button onclick={() => resolveFirst("ours")}>Ours</button>
      <button onclick={() => resolveFirst("theirs")}>Theirs</button>
      {#if hasBase}<button onclick={() => resolveFirst("base")}>Base</button>{/if}
      <button onclick={() => resolveFirst("both")}>Both</button>
      <span class="csep">·</span>
      <button onclick={() => resolveEvery("ours")} title="Resolve every conflict as ours">All ours</button>
      <button onclick={() => resolveEvery("theirs")} title="Resolve every conflict as theirs">All theirs</button>
    </div>
  {/if}
  {#if crumbFile}
    <div class="breadcrumb">
      <span class="crumb file">{crumbFile}</span>
      {#each crumbs as c (c.line + c.name)}
        <span class="sep">›</span>
        <button class="crumb sym" onclick={() => jumpLine(c.line)}>{c.name}</button>
      {/each}
    </div>
  {/if}
  <div class="ed-host">
    {#if stickyRows.length}
      <div class="sticky">
        {#each stickyRows as s (s.line)}
          <button class="stickyrow mono" onclick={() => jumpLine(s.line)}>{s.text}</button>
        {/each}
      </div>
    {/if}
    <div class="ed" bind:this={host}></div>
  </div>
</div>

<style>
  .ed-wrap { display: flex; flex-direction: column; width: 100%; height: 100%; min-height: 0; }
  .ed-host { flex: 1; min-height: 0; width: 100%; position: relative; }
  .ed { height: 100%; width: 100%; overflow: hidden; }
  .breadcrumb {
    flex: 0 0 auto; display: flex; align-items: center; gap: 5px; height: 26px; padding: 0 12px;
    border-bottom: 1px solid var(--border); font-size: 11.5px; color: var(--text3); overflow: hidden; white-space: nowrap;
  }
  .breadcrumb .sep { color: var(--text3); opacity: 0.6; }
  .breadcrumb .crumb { border: 0; background: transparent; font-family: var(--font-ui); font-size: 11.5px;
    color: var(--text2); cursor: default; padding: 1px 3px; border-radius: 4px; }
  .breadcrumb .crumb.file { color: var(--text2); font-weight: 500; }
  .breadcrumb .crumb.sym:hover { background: var(--sel); color: var(--text); }
  .sticky {
    position: absolute; top: 0; left: 0; right: 0; z-index: 4; pointer-events: auto;
    background: var(--panel); border-bottom: 1px solid var(--border); box-shadow: 0 4px 8px rgba(0,0,0,0.18);
  }
  .stickyrow {
    display: block; width: 100%; text-align: left; border: 0; background: transparent; color: var(--text2);
    font-size: 12.5px; line-height: 1.6; padding: 0 12px; white-space: pre; overflow: hidden;
    text-overflow: ellipsis; cursor: default;
  }
  .stickyrow:hover { background: var(--panel2); }
  .ed :global(.cm-editor) { height: 100%; }
  .ed :global(.cm-editor.cm-focused) { outline: none; }
  /* git gutter: a colored bar at the left edge of changed lines */
  .ed :global(.cm-line.anvil-g-add), .ed :global(.cm-line.anvil-g-mod), .ed :global(.cm-line.anvil-g-del) { position: relative; }
  .ed :global(.cm-line.anvil-g-add::before), .ed :global(.cm-line.anvil-g-mod::before), .ed :global(.cm-line.anvil-g-del::before) {
    content: ""; position: absolute; left: -8px; top: 0; bottom: 0; width: 2px;
  }
  .ed :global(.cm-line.anvil-g-add::before) { background: var(--green); }
  .ed :global(.cm-line.anvil-g-mod::before) { background: var(--blue); }
  .ed :global(.cm-line.anvil-g-del::before) { background: var(--red); }
  .ed :global(.cm-blame) { color: var(--text3); font-style: italic; opacity: 0.75; }
  /* Bookmarks (#85): accent bar + faint tint on marked lines */
  .ed :global(.cm-line.anvil-mark) { position: relative; background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .ed :global(.cm-line.anvil-mark::before) {
    content: ""; position: absolute; left: -8px; top: 0; bottom: 0; width: 2px; background: var(--accent);
  }
  /* Inline color swatch (#5) */
  .ed :global(.cm-color-swatch) {
    display: inline-block; width: 10px; height: 10px; border-radius: 3px; margin: 0 4px -1px 0;
    border: 1px solid var(--border); position: relative; cursor: pointer; vertical-align: baseline;
  }
  .ed :global(.cm-color-input) {
    position: absolute; inset: 0; opacity: 0; width: 100%; height: 100%; border: 0; padding: 0; cursor: pointer;
  }
  .conflictbar {
    flex: 0 0 auto; display: flex; align-items: center; gap: 8px; padding: 5px 12px;
    background: var(--panel2); border-bottom: 1px solid var(--border); font-size: 12px;
  }
  .conflictbar .cw { color: var(--red); margin-right: auto; }
  .conflictbar .csep { color: var(--text3); }
  .conflictbar button {
    border: 1px solid var(--border); background: var(--bg); color: var(--accent);
    font-family: var(--font-ui); font-size: 11.5px; padding: 3px 9px; border-radius: 6px; cursor: default;
  }
  .conflictbar button:hover { background: var(--sel); color: var(--text); }
</style>
