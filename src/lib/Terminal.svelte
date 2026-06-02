<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { SearchAddon } from "@xterm/addon-search";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { ImageAddon } from "@xterm/addon-image";
  import "@xterm/xterm/css/xterm.css";
  import { invoke, Channel } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { activeTheme, themes } from "$lib/themes";
  import { windowOpacity } from "$lib/window-opacity";
  import { termFontSize, termCursorBlink, termCursorStyle, termLineHeight, termLetterSpacing, termScrollback } from "$lib/terminal-settings";
  import { monoFont, termBold } from "$lib/fonts";
  import { registerTerminal, unregisterTerminal, broadcastInput, liveTerminals } from "$lib/term-registry";
  import { feedInput } from "$lib/command-history";
  import { terminalOpenPath } from "$lib/terminal-open";
  import { recordPrompt, recordExit, clearBlocks } from "$lib/command-blocks";

  const monoStack = (f: string) => `"${f}", "Symbols Nerd Font Mono", "SF Mono", Menlo, ui-monospace, monospace`;
  import { get } from "svelte/store";

  let { id, cwd = "", active = true, shell = "" }: { id: string; cwd?: string; active?: boolean; shell?: string } = $props();

  let host: HTMLDivElement;
  let term: Terminal;
  let fit: FitAddon;
  let search: SearchAddon;
  let unlisten: UnlistenFn[] = [];
  let ro: ResizeObserver;
  let resizeRaf = 0;
  let lastCols = 0, lastRows = 0;
  let unsubTheme: () => void;
  let unsubOpacity: () => void;

  // Build an xterm theme whose background is mixed toward transparent by the
  // window-opacity value (a). At a=1 the background is the theme's solid color;
  // below 1 it becomes translucent so the vibrancy backdrop shows through.
  function themeFor(name: string, a: number) {
    const base = themes[name].xterm;
    if (a >= 1) return base;
    const hex = (base.background ?? "#000000").replace("#", "");
    const r = parseInt(hex.slice(0, 2), 16), g = parseInt(hex.slice(2, 4), 16), b = parseInt(hex.slice(4, 6), 16);
    return { ...base, background: `rgba(${r}, ${g}, ${b}, ${a})` };
  }

  let searchOpen = $state(false);
  let searchQuery = $state("");
  let searchInput = $state<HTMLInputElement>();
  let matchInfo = $state<{ idx: number; count: number }>({ idx: -1, count: 0 });

  function find(forward = true) {
    const opts = { incremental: true };
    if (forward) search?.findNext(searchQuery, opts);
    else search?.findPrevious(searchQuery, opts);
  }
  function searchKey(e: KeyboardEvent) {
    if (e.key === "Escape") { searchOpen = false; term?.focus(); }
    else if (e.key === "Enter") find(!e.shiftKey);
  }

  onMount(async () => {
    // Ensure the mono font is loaded BEFORE xterm measures cell width, otherwise
    // it sizes cells to the fallback font and the glyphs render with big gaps.
    const fam = get(monoFont);
    try {
      await document.fonts.load(`13px "${fam}"`);
      await document.fonts.load(`bold 13px "${fam}"`);
      await document.fonts.load('13px "Symbols Nerd Font Mono"');
      await document.fonts.ready;
    } catch { /* fonts API unavailable */ }

    term = new Terminal({
      fontFamily: monoStack(fam),
      fontWeight: get(termBold) ? "bold" : "normal",
      fontWeightBold: "bold",
      fontSize: get(termFontSize),
      lineHeight: get(termLineHeight),
      letterSpacing: get(termLetterSpacing),
      theme: themeFor($activeTheme, get(windowOpacity)),
      cursorBlink: get(termCursorBlink),
      cursorStyle: get(termCursorStyle),
      allowProposedApi: true,
      allowTransparency: true,
      scrollback: get(termScrollback),
    });

    unsubTheme = activeTheme.subscribe((name) => {
      if (term) term.options.theme = themeFor(name, get(windowOpacity));
    });
    // Re-tint the terminal background when window opacity changes so the macOS
    // vibrancy material shows through at <1.
    unsubOpacity = windowOpacity.subscribe((a) => {
      if (term) term.options.theme = themeFor(get(activeTheme), a);
    });
    fit = new FitAddon();
    term.loadAddon(fit);
    search = new SearchAddon();
    term.loadAddon(search);
    // Inline images (#14): sixel + iTerm inline-image protocol.
    try { term.loadAddon(new ImageAddon({ sixelSupport: true })); } catch { /* webgl needed; ignore if unavailable */ }
    // Command blocks (#12): capture OSC 133 A (prompt) / D;<code> (exit) marks.
    term.parser.registerOscHandler(133, (data: string) => {
      const buf = term.buffer.active;
      const line = buf.baseY + buf.cursorY;
      const parts = data.split(";");
      if (parts[0] === "A") recordPrompt(id, line);
      else if (parts[0] === "D") recordExit(id, Number(parts[1] ?? "0") || 0);
      return true;
    });
    // Expose recent parsed buffer text to the agent (#32).
    registerTerminal(id, () => {
      const buf = term.buffer.active;
      const out: string[] = [];
      const start = Math.max(0, buf.length - 200);
      for (let i = start; i < buf.length; i++) out.push(buf.getLine(i)?.translateToString(true) ?? "");
      return out.join("\n").replace(/\n{3,}/g, "\n\n").trimEnd();
    });
    search.onDidChangeResults((r) => {
      if (r) matchInfo = { idx: r.resultIndex, count: r.resultCount };
      else matchInfo = { idx: -1, count: 0 };
    });
    term.loadAddon(new WebLinksAddon((_e, uri) => { openUrl(uri).catch((e) => console.warn("openUrl failed", e)); }));
    // File/path smart links (#20): click a file path → open it in the editor.
    const PATH_RE = /(?:[~.]{0,2}\/)?[\w.\-/]+\.[A-Za-z][\w]*(?::\d+)?/g;
    term.loadAddon(new WebLinksAddon((_e, uri) => {
      const m = /^(.*?)(?::(\d+))?$/.exec(uri);
      let p = m?.[1] ?? uri;
      const line = m?.[2] ? Number(m[2]) : undefined;
      if (!p.startsWith("/") && !p.startsWith("~")) p = `${cwd.replace(/\/$/, "")}/${p}`;
      terminalOpenPath.set({ path: p, line });
    }, { urlRegex: PATH_RE }));
    // Copy-on-select (#20): mirror the X11/iTerm convention — selecting copies.
    term.onSelectionChange(() => { const sel = term.getSelection(); if (sel) navigator.clipboard.writeText(sel).catch((e) => console.warn("clipboard write failed", e)); });
    term.open(host);
    fit.fit();
    // GPU renderer for crisp, fast text; fall back silently if WebGL is lost.
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      term.loadAddon(webgl);
    } catch { /* no WebGL — canvas renderer stays */ }

    // ⌘F opens the terminal search box (intercept before the key reaches the PTY).
    term.attachCustomKeyEventHandler((e) => {
      if (e.metaKey && (e.key === "f" || e.key === "F")) {
        if (e.type === "keydown") {
          searchOpen = true;
          queueMicrotask(() => searchInput?.focus());
        }
        return false;
      }
      return true;
    });

    unlisten.push(
      await listen<{ id: string }>("pty://exit", (e) => {
        if (e.payload.id === id) term.writeln("\r\n\x1b[2m[process exited]\x1b[0m");
      }),
    );

    // Raw PTY bytes stream over a per-terminal binary channel (no base64).
    // Accept whatever concrete shape the IPC layer hands us (ArrayBuffer for
    // raw bodies, typed/number arrays, or a string) so a transport change can't
    // silently blank the terminal.
    const onData = new Channel<ArrayBuffer | ArrayBufferView | number[] | string>();
    onData.onmessage = (msg) => {
      if (typeof msg === "string") term.write(msg);
      else if (msg instanceof ArrayBuffer) term.write(new Uint8Array(msg));
      else if (ArrayBuffer.isView(msg)) term.write(new Uint8Array(msg.buffer, msg.byteOffset, msg.byteLength));
      else term.write(new Uint8Array(msg));
    };

    await invoke("pty_spawn", { id, cols: term.cols, rows: term.rows, cwd, shell, onData });
    term.onData((d) => {
      feedInput(id, d);
      if (get(broadcastInput)) { for (const tid of liveTerminals()) invoke("pty_write", { id: tid, data: d }); }
      else invoke("pty_write", { id, data: d });
    });

    ro = new ResizeObserver(() => {
      // Coalesce resize bursts (window drag fires many callbacks/sec) into one
      // fit per frame, and only round-trip pty_resize when dims actually change.
      if (resizeRaf) return;
      resizeRaf = requestAnimationFrame(() => {
        resizeRaf = 0;
        // Skip while hidden (display:none) — fitting a 0-size box glitches reflow.
        if (!host || host.clientWidth < 2 || host.clientHeight < 2) return;
        fit.fit();
        if (term.cols !== lastCols || term.rows !== lastRows) {
          lastCols = term.cols; lastRows = term.rows;
          invoke("pty_resize", { id, cols: term.cols, rows: term.rows });
        }
      });
    });
    ro.observe(host);
    term.focus();
  });

  // #78 Tell the backend to throttle this PTY's coalescer while off-screen.
  $effect(() => { invoke("pty_set_active", { id, active }).catch((e) => console.warn("pty_set_active failed", e)); });

  // Re-fit + restore on becoming visible (rail switch). Two RAFs so layout has
  // flushed before measuring.
  $effect(() => {
    if (!active || !term || !host) return;
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (!host || host.clientWidth < 2) return;
      fit.fit();
      invoke("pty_resize", { id, cols: term.cols, rows: term.rows });
      term.scrollToBottom();
      term.focus();
    }));
  });

  // Live-apply terminal settings from the Settings page.
  $effect(() => {
    const fs = $termFontSize;
    if (term) {
      term.options.fontSize = fs;
      fit?.fit();
      invoke("pty_resize", { id, cols: term.cols, rows: term.rows });
    }
  });
  $effect(() => {
    const blink = $termCursorBlink;
    if (term) term.options.cursorBlink = blink;
  });
  $effect(() => {
    const style = $termCursorStyle;
    if (term) term.options.cursorStyle = style;
  });
  $effect(() => {
    const sb = $termScrollback;
    if (term) term.options.scrollback = sb; // live-resize existing buffers (#76)
  });
  $effect(() => {
    const lh = $termLineHeight, ls = $termLetterSpacing;
    if (!term) return;
    term.options.lineHeight = lh;
    term.options.letterSpacing = ls;
    fit?.fit();
    invoke("pty_resize", { id, cols: term.cols, rows: term.rows });
  });
  $effect(() => {
    const fam = $monoFont, bold = $termBold;
    if (!term) return;
    Promise.all([
      document.fonts.load(`13px "${fam}"`),
      document.fonts.load(`bold 13px "${fam}"`),
    ]).then(() => {
      term.options.fontFamily = monoStack(fam);
      // Base weight for normal cells. With the WebGL renderer, glyphs are cached
      // in a texture atlas keyed by their draw attributes — changing the base
      // weight won't re-render already-cached cells until the atlas is cleared.
      term.options.fontWeight = bold ? "bold" : "normal";
      term.options.fontWeightBold = "bold";
      term.clearTextureAtlas();
      fit?.fit();
      invoke("pty_resize", { id, cols: term.cols, rows: term.rows });
      term.refresh(0, term.rows - 1);
    });
  });

  onDestroy(() => {
    unsubTheme?.();
    unsubOpacity?.();
    if (resizeRaf) cancelAnimationFrame(resizeRaf);
    ro?.disconnect();
    unlisten.forEach((u) => u());
    unregisterTerminal(id);
    clearBlocks(id);
    invoke("pty_kill", { id });
    term?.dispose();
  });

  // Right-click context menu.
  let menu = $state<{ x: number; y: number } | null>(null);
  function ctx(e: MouseEvent) { e.preventDefault(); menu = { x: e.clientX, y: e.clientY }; }
  async function copySel() { const s = term?.getSelection(); if (s) await navigator.clipboard.writeText(s).catch((e) => console.warn("clipboard write failed", e)); menu = null; }
  async function pasteClip() { try { const t = await navigator.clipboard.readText(); if (t) invoke("pty_write", { id, data: t }); } catch (e) { console.warn("clipboard read failed", e); } menu = null; term?.focus(); }
  function selectAllTerm() { term?.selectAll(); menu = null; }
  function clearTerm() { term?.clear(); menu = null; term?.focus(); }
  function runSel() {
    const s = term?.getSelection();
    if (s) invoke("pty_write", { id, data: s.replace(/\n+$/, "") + "\r" });
    menu = null; term?.focus();
  }
</script>

<div class="term-host" oncontextmenu={ctx} role="presentation">
  {#if searchOpen}
    <div class="search">
      <input
        bind:this={searchInput}
        bind:value={searchQuery}
        oninput={() => find(true)}
        onkeydown={searchKey}
        placeholder="Find in terminal"
        spellcheck="false"
      />
      <span class="count">{#if searchQuery && matchInfo.count >= 0}{matchInfo.count === 0 ? "0/0" : `${matchInfo.idx + 1}/${matchInfo.count}`}{/if}</span>
      <button onclick={() => find(false)} title="Previous (⇧⏎)">↑</button>
      <button onclick={() => find(true)} title="Next (⏎)">↓</button>
      <button onclick={() => { searchOpen = false; term?.focus(); }} title="Close (Esc)">×</button>
    </div>
  {/if}
  <div class="xterm-host" bind:this={host}></div>
  {#if menu}
    <div class="ctxscrim" onclick={() => (menu = null)} oncontextmenu={(e) => { e.preventDefault(); menu = null; }} role="presentation"></div>
    <div class="ctxmenu" style="left:{menu.x}px;top:{menu.y}px">
      <button onclick={copySel}>Copy</button>
      <button onclick={pasteClip}>Paste</button>
      <button onclick={runSel}>Run Selection</button>
      <button onclick={selectAllTerm}>Select All</button>
      <button onclick={clearTerm}>Clear</button>
    </div>
  {/if}
</div>

<style>
  .term-host { position: relative; width: 100%; height: 100%; }
  .xterm-host { width: 100%; height: 100%; }
  .ctxscrim { position: fixed; inset: 0; z-index: 40; }
  .ctxmenu {
    position: fixed; z-index: 41; min-width: 140px; padding: 4px;
    background: var(--glass-2); backdrop-filter: blur(var(--blur)) saturate(1.3); -webkit-backdrop-filter: blur(var(--blur)) saturate(1.3);
    border: 1px solid var(--border); border-radius: var(--radius);
    box-shadow: var(--elev-2), inset 0 1px 0 var(--hairline); display: flex; flex-direction: column;
  }
  .ctxmenu button {
    text-align: left; border: 0; background: transparent; color: var(--text);
    font-family: var(--font-ui); font-size: 12.5px; padding: 6px 10px; border-radius: 6px; cursor: default;
  }
  .ctxmenu button:hover { background: var(--sel); }
  .search {
    position: absolute; top: 8px; right: 14px; z-index: 5; display: flex; align-items: center; gap: 4px;
    background: var(--glass-2); backdrop-filter: blur(var(--blur)) saturate(1.3); -webkit-backdrop-filter: blur(var(--blur)) saturate(1.3);
    border: 1px solid var(--border); border-radius: var(--radius); padding: 4px 6px;
    box-shadow: var(--elev-2), inset 0 1px 0 var(--hairline);
  }
  .search input {
    width: 180px; border: 0; outline: 0; background: transparent; color: var(--text);
    font-size: 12.5px; font-family: var(--font-ui);
  }
  .search .count { font-family: var(--font-mono); font-size: 11px; color: var(--text3); min-width: 30px; text-align: right; }
  .search button {
    border: 0; background: transparent; color: var(--text3); cursor: default; font-size: 13px;
    padding: 2px 5px; border-radius: 5px;
  }
  .search button:hover { color: var(--text); background: var(--sel); }
</style>
