<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { SearchAddon } from "@xterm/addon-search";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { WebglAddon } from "@xterm/addon-webgl";
  import { CanvasAddon } from "@xterm/addon-canvas";
  import { ImageAddon } from "@xterm/addon-image";
  import "@xterm/xterm/css/xterm.css";
  import { invoke, Channel } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { activeTheme, themes } from "$lib/themes";
  import { windowOpacity } from "$lib/window-opacity";
  import { termFontSize, termCursorBlink, termCursorStyle, termLineHeight, termLetterSpacing, termScrollback, termCmdSep } from "$lib/terminal-settings";
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
  let atBottom = $state(true); // false → show the "jump to bottom" pill
  let cmdDecos: { dispose: () => void }[] = [];
  // Draw a faint separator above the current prompt line (OSC 133 A). Markers
  // auto-dispose when their line scrolls out of the buffer; cap the list anyway.
  function addCmdSeparator() {
    try {
      const marker = term.registerMarker(0);
      if (!marker) return;
      const deco = term.registerDecoration({ marker, width: term.cols });
      if (!deco) return;
      deco.onRender((el: HTMLElement) => el.classList.add("cmd-sep"));
      cmdDecos.push(deco);
      if (cmdDecos.length > 600) cmdDecos.shift()?.dispose();
    } catch { /* decorations unavailable */ }
  }
  let ro: ResizeObserver;
  let resizeRaf = 0;
  let lastCols = 0, lastRows = 0;
  let unsubTheme: () => void;
  let unsubOpacity: () => void;
  let selTimer: ReturnType<typeof setTimeout> | undefined;

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
      // Only pay the alpha-blend cost when the window is actually translucent;
      // at full opacity (the common case) keep the WebGL renderer opaque/fast.
      allowTransparency: get(windowOpacity) < 1,
      scrollback: get(termScrollback),
      // Snappy + crisp: instant scroll, faster wheel with a modifier, rescale
      // overlapping wide glyphs, and a lighter outline cursor when unfocused.
      smoothScrollDuration: 0,
      rescaleOverlappingGlyphs: true,
      cursorInactiveStyle: "outline",
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
    // Also draw a faint separator line above each prompt so commands are visually
    // grouped (only when shell integration emits OSC 133 + the setting is on).
    term.parser.registerOscHandler(133, (data: string) => {
      const buf = term.buffer.active;
      const line = buf.baseY + buf.cursorY;
      const parts = data.split(";");
      if (parts[0] === "A") { recordPrompt(id, line); if (get(termCmdSep)) addCmdSeparator(); }
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
    // No `g` flag: WebLinksAddon builds `new RegExp(src, (flags||"")+"g")`, so a
    // `g` here yields "gg" → "Invalid flags supplied to RegExp constructor".
    const PATH_RE = /(?:[~.]{0,2}\/)?[\w.\-/]+\.[A-Za-z][\w]*(?::\d+)?/;
    term.loadAddon(new WebLinksAddon((_e, uri) => {
      const m = /^(.*?)(?::(\d+))?$/.exec(uri);
      let p = m?.[1] ?? uri;
      const line = m?.[2] ? Number(m[2]) : undefined;
      if (!p.startsWith("/") && !p.startsWith("~")) p = `${cwd.replace(/\/$/, "")}/${p}`;
      terminalOpenPath.set({ path: p, line });
    }, { urlRegex: PATH_RE }));
    // Copy-on-select (#20): mirror the X11/iTerm convention — selecting copies.
    // Debounced so a drag-select doesn't fire a clipboard write on every change.
    term.onSelectionChange(() => {
      clearTimeout(selTimer);
      selTimer = setTimeout(() => {
        const sel = term.getSelection();
        if (sel) navigator.clipboard.writeText(sel).catch((e) => console.warn("clipboard write failed", e));
      }, 150);
    });
    term.open(host);
    fit.fit();
    // GPU renderer for crisp, fast text. If WebGL is unavailable/lost, fall back
    // to the CANVAS renderer (still far faster than xterm's default DOM renderer).
    let webglOk = false;
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => { webgl.dispose(); try { term.loadAddon(new CanvasAddon()); } catch { /* DOM */ } });
      term.loadAddon(webgl);
      webglOk = true;
    } catch { /* no WebGL */ }
    if (!webglOk) {
      try { term.loadAddon(new CanvasAddon()); } catch { /* DOM renderer — slowest, last resort */ }
    }

    // ⌘F opens the terminal search box (intercept before the key reaches the PTY).
    term.attachCustomKeyEventHandler((e) => {
      if (e.metaKey && (e.key === "f" || e.key === "F")) {
        if (e.type === "keydown") {
          searchOpen = true;
          queueMicrotask(() => searchInput?.focus());
        }
        return false;
      }
      // Let app-level ⌘ shortcuts (⌘W close tab, ⌘T new tab, ⌘K palette, …)
      // bubble to the window handler instead of being written to the PTY.
      // ⌘C/⌘V stay with the terminal for copy/paste.
      if (e.metaKey && !["c", "C", "v", "V"].includes(e.key)) return false;
      return true;
    });

    // Raw PTY bytes stream over a per-terminal binary channel (no base64).
    const onData = new Channel<ArrayBuffer | ArrayBufferView | number[] | string>();
    onData.onmessage = (msg) => {
      if (typeof msg === "string") term.write(msg);
      else if (msg instanceof ArrayBuffer) term.write(new Uint8Array(msg));
      else if (ArrayBuffer.isView(msg)) term.write(new Uint8Array(msg.buffer, msg.byteOffset, msg.byteLength));
      else term.write(new Uint8Array(msg));
    };

    const sendInput = (d: string) => {
      feedInput(id, d);
      if (get(broadcastInput)) { for (const tid of liveTerminals()) invoke("pty_write", { id: tid, data: d }); }
      else invoke("pty_write", { id, data: d });
    };
    term.onData(sendInput);

    // Mouse-wheel scrolling — explicit so it works regardless of zoom/renderer.
    // Normal screen → scroll the scrollback buffer; alternate screen (Claude
    // Code, hermes, vim, less) → arrow keys (iTerm/kitty "alternate scroll"),
    // unless the app grabbed the mouse (Shift overrides to force-scroll).
    // IMPORTANT: attached BEFORE the pty_spawn / listen awaits below — if one of
    // those rejected, onMount aborted here and the terminal was left with no
    // wheel handler, which read as "scroll-back doesn't work".
    let wheelAccum = 0;
    const onWheel = (e: WheelEvent) => {
      try {
        if (!host || !(e.target instanceof Node) || !host.contains(e.target)) return;
        const modes = (term as unknown as { modes?: { mouseTrackingMode?: string; applicationCursorKeysMode?: boolean } }).modes;
        const onAlt = term.buffer.active.type === "alternate";
        const mouseOn = !!(modes?.mouseTrackingMode && modes.mouseTrackingMode !== "none");
        e.preventDefault();
        e.stopPropagation();
        const PX_PER_LINE = e.deltaMode === 1 ? 1 : e.deltaMode === 2 ? Math.max(1, term.rows) : 24;
        wheelAccum += e.deltaY / PX_PER_LINE;
        const dir = wheelAccum >= 0 ? 1 : -1;
        let n = Math.floor(Math.abs(wheelAccum));
        if (!n) return;
        wheelAccum -= dir * n;
        if (e.altKey) n *= 5;
        const cap = Math.min(n, 8);
        if (mouseOn && !e.shiftKey) {
          // App grabbed the mouse (claude code, hermes, less, vim with `mouse`):
          // forward the wheel as SGR mouse events so the app scrolls its OWN
          // viewport. 64 = wheel up, 65 = wheel down. Previously we returned here,
          // which is why scrolling those apps did nothing.
          const rect = host.getBoundingClientRect();
          const col = Math.min(term.cols, Math.max(1, Math.floor((e.clientX - rect.left) / (rect.width / term.cols)) + 1));
          const row = Math.min(term.rows, Math.max(1, Math.floor((e.clientY - rect.top) / (rect.height / term.rows)) + 1));
          const btn = dir > 0 ? 65 : 64;
          sendInput(`\x1b[<${btn};${col};${row}M`.repeat(cap));
        } else if (onAlt) {
          // Alternate screen, no mouse tracking: legacy "alternate scroll" → arrows.
          const appCursor = !!modes?.applicationCursorKeysMode;
          const key = dir > 0 ? (appCursor ? "\x1bOB" : "\x1b[B") : appCursor ? "\x1bOA" : "\x1b[A";
          sendInput(key.repeat(cap));
        } else {
          term.scrollLines(dir * n);
          term.refresh(0, term.rows - 1);
        }
      } catch { /* ignore */ }
    };
    window.addEventListener("wheel", onWheel, { passive: false, capture: true });
    unlisten.push(() => window.removeEventListener("wheel", onWheel, true));

    // Track scroll position so we can offer a "jump to bottom" pill.
    term.onScroll(() => {
      const b = term.buffer.active;
      atBottom = b.viewportY >= b.baseY;
    });

    unlisten.push(
      await listen<{ id: string }>("pty://exit", (e) => {
        if (e.payload.id === id) term.writeln("\r\n\x1b[2m[process exited]\x1b[0m");
      }),
    );
    await invoke("pty_spawn", { id, cols: term.cols, rows: term.rows, cwd, shell, onData });

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
    clearTimeout(selTimer);
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
  // Clear existing separators when the setting is turned off (new ones simply
  // stop being added). New prompts re-add them when it's on.
  $effect(() => { if (!$termCmdSep) { cmdDecos.forEach((d) => d.dispose()); cmdDecos = []; } });
  function jumpToBottom() { term?.scrollToBottom(); atBottom = true; term?.focus(); }
  function runSel() {
    const s = term?.getSelection();
    if (s) invoke("pty_write", { id, data: s.replace(/\n+$/, "") + "\r" });
    menu = null; term?.focus();
  }
</script>

<div class="term-host" oncontextmenu={ctx} role="presentation">
  {#if !atBottom}
    <button class="jump-bottom" onclick={jumpToBottom} title="Jump to bottom">↓</button>
  {/if}
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
  /* Faint separator line above each shell prompt (OSC 133). The decoration
     element spans the prompt row; a top border reads as a divider between the
     previous command's output and the next prompt. */
  :global(.xterm .cmd-sep) {
    border-top: 1px solid var(--border);
    opacity: 0.35;
    pointer-events: none;
  }
  /* Jump-to-bottom pill, shown when scrolled up into history. */
  .jump-bottom {
    position: absolute; right: 12px; bottom: 10px; z-index: 12;
    width: 26px; height: 26px; display: inline-flex; align-items: center; justify-content: center;
    border: 1px solid var(--border); border-radius: 50%; background: var(--bg1);
    color: var(--text2); cursor: default; box-shadow: 0 3px 10px rgba(0,0,0,0.32);
    font-size: 13px; line-height: 1;
  }
  .jump-bottom:hover { background: var(--sel); color: var(--text); }
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
