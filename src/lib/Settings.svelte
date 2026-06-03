<script lang="ts">
  import { themes, activeTheme, applyTheme, systemMode, systemLight, systemDark, setSystemMode, setSystemPair, LIGHT_THEMES, DARK_THEMES, themeLabel } from "$lib/themes";
  import { density, applyDensity } from "$lib/density";
  import { autoHideRail, setAutoHideRail } from "$lib/layout-settings";
  import { EXTENSIONS, extEnabled, isExtEnabled, toggleExt } from "$lib/extensions";
  import { KEY_ACTIONS, keyOverrides, comboFor, comboOf, setKeyOverride, clearKeyOverride } from "$lib/keymap";

  // Editable keymap recording (#82): capture the next combo for an action.
  let recording = $state<string | null>(null);
  function startRecord(id: string) {
    recording = id;
    const h = (e: KeyboardEvent) => {
      e.preventDefault(); e.stopPropagation();
      if (e.key === "Escape") { recording = null; window.removeEventListener("keydown", h, true); return; }
      if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) return; // wait for the real key
      setKeyOverride(id, comboOf(e));
      recording = null;
      window.removeEventListener("keydown", h, true);
    };
    window.addEventListener("keydown", h, true);
  }
  import {
    editorFontSize, editorTabSize, editorWordWrap, editorMinimap, editorLigatures, editorLineHeight, editorLetterSpacing, editorStickyScroll, editorInlayHints, editorFormatOnSave, editorBlameAlways,
    bumpEditorFontSize, setEditorTabSize, toggleWordWrap, toggleMinimap, toggleLigatures, bumpEditorLineHeight, bumpEditorLetterSpacing, toggleStickyScroll, toggleInlayHints, toggleFormatOnSave, toggleBlameAlways,
  } from "$lib/editor-settings";
  import { termFontSize, termCursorBlink, termCursorStyle, CURSOR_STYLES, termLineHeight, termLetterSpacing, termScrollback, bumpTermFontSize, toggleTermBlink, setTermCursorStyle, bumpTermLineHeight, bumpTermLetterSpacing, setTermScrollback } from "$lib/terminal-settings";
  import { uiScale, bumpScale, resetScale } from "$lib/scale";
  import { windowOpacity, applyOpacity } from "$lib/window-opacity";
  import { uiFont, monoFont, editorBold, termBold, UI_FONTS, MONO_FONTS, setUiFont, setMonoFont, toggleEditorBold, toggleTermBold } from "$lib/fonts";
  import Icon from "$lib/Icon.svelte";
  import { ACCOUNTS, getValue, setValue, clearValue, hasValue, type AccountField } from "$lib/accounts";
  import { onMount } from "svelte";

  const themeNames = Object.keys(themes);
  const swatchKeys = ["bg", "panel", "accent", "text"] as const;

  // Live custom-color overrides (persisted as anvil-custom-theme, applied over
  // whatever base theme is active).
  const COLOR_KEYS = ["bg", "panel", "panel2", "border", "text", "text2", "text3", "sel",
    "accent", "accent2", "green", "red", "blue", "purple", "teal", "yellow"];
  const COLOR_LABEL: Record<string, string> = {
    bg: "Background", panel: "Panel", panel2: "Panel 2", border: "Border",
    text: "Text", text2: "Text 2", text3: "Text 3", sel: "Selection",
    accent: "Accent", accent2: "Accent 2", green: "Green", red: "Red",
    blue: "Blue", purple: "Purple", teal: "Teal", yellow: "Yellow",
  };
  function loadOverrides(): Record<string, string> {
    if (typeof localStorage === "undefined") return {};
    try { return JSON.parse(localStorage.getItem("anvil-custom-theme") || "{}"); } catch { return {}; }
  }
  let overrides = $state<Record<string, string>>(loadOverrides());
  const colorVal = (k: string) => overrides[k] ?? themes[$activeTheme]?.ui[k] ?? "#000000";
  function setColor(k: string, v: string) {
    overrides = { ...overrides, [k]: v };
    document.documentElement.style.setProperty(`--${k}`, v);
    if (typeof localStorage !== "undefined") localStorage.setItem("anvil-custom-theme", JSON.stringify(overrides));
  }
  function clearColors() {
    overrides = {};
    if (typeof localStorage !== "undefined") localStorage.removeItem("anvil-custom-theme");
    applyTheme($activeTheme);
  }

  type Section = "appearance" | "editor" | "terminal" | "extensions" | "accounts" | "keymap" | "about";
  let section = $state<Section>("appearance");
  const NAV: { id: Section; label: string; icon: string; kw: string }[] = [
    { id: "appearance", label: "Appearance", icon: "theme", kw: "theme color font density scale zoom system light dark auto-hide custom palette" },
    { id: "editor", label: "Editor", icon: "pencil", kw: "font size tab wrap minimap ligatures line height letter spacing sticky scroll inlay hints format save blame" },
    { id: "terminal", label: "Terminal", icon: "terminal", kw: "font size cursor style blink bold line height letter spacing shell" },
    { id: "extensions", label: "Extensions", icon: "devops", kw: "extension kubernetes github actions grafana terraform aws plugin store enable disable" },
    { id: "accounts", label: "Accounts", icon: "key", kw: "llm api key github token aws grafana keychain secret" },
    { id: "keymap", label: "Keymap", icon: "command", kw: "keyboard shortcut binding hotkey" },
    { id: "about", label: "About", icon: "info", kw: "version anvil" },
  ];
  // Settings search (#89): filter the nav by label or keywords.
  let navFilter = $state("");
  const navShown = $derived(
    navFilter.trim()
      ? NAV.filter((n) => (n.label + " " + n.kw).toLowerCase().includes(navFilter.trim().toLowerCase()))
      : NAV,
  );

  // Accounts: live values + "saved" status. Secrets shown masked once stored.
  let acctVals = $state<Record<string, string>>({});
  let acctSaved = $state<Record<string, boolean>>({});
  async function loadAccounts() {
    for (const f of ACCOUNTS) {
      acctSaved[f.key] = await hasValue(f);
      acctVals[f.key] = f.secret ? "" : await getValue(f);
    }
  }
  async function saveAccount(f: AccountField) {
    const v = acctVals[f.key] ?? "";
    if (!v) return;
    await setValue(f, v);
    acctSaved[f.key] = true;
    if (f.secret) acctVals[f.key] = "";
  }
  async function clearAccount(f: AccountField) {
    await clearValue(f);
    acctSaved[f.key] = false;
    acctVals[f.key] = "";
  }
  onMount(loadAccounts);

  const KEYS: { k: string; d: string }[] = [
    { k: "⌘T", d: "New terminal" }, { k: "⌘W", d: "Close tab" },
    { k: "⌘N", d: "New window" }, { k: "⌘⇧T", d: "Reopen closed tab" },
    { k: "⌘1–9", d: "Switch to tab N" }, { k: "⌘J", d: "Toggle bottom terminal" },
    { k: "⌘K", d: "Command palette" }, { k: "⌘P", d: "Go to file" },
    { k: "⌘E", d: "Recent files" }, { k: "⌘O", d: "Open file" },
    { k: "⌘⇧O", d: "Open folder" }, { k: "⌘D", d: "Split / unsplit terminal" },
    { k: "⌘\\", d: "Split workspace pane (⇧ = downward)" },
    { k: "⌥⌘←/→", d: "Focus previous / next pane" },
    { k: "⌘⇧⏎", d: "Zoom / unzoom focused pane" },
    { k: "⌘⌥←/→", d: "Editor: navigate back / forward" },
    { k: "⌘⇧F", d: "Search workspace" }, { k: "⌘F", d: "Find (terminal / editor)" },
    { k: "⌘S", d: "Save file (format on save)" }, { k: "⌥B", d: "Toggle git blame" },
    { k: "⌘B", d: "Toggle sidebar" }, { k: "⌘.", d: "Zen / terminal mode" },
    { k: "⌘ + / − / 0", d: "Zoom in / out / reset" },
  ];
</script>

<div class="settings">
  <nav class="snav">
    <div class="stitle">Settings</div>
    <input class="sfilter" bind:value={navFilter} placeholder="Search settings…" spellcheck="false" />
    {#each navShown as n (n.id)}
      <button class="snav-item {section === n.id ? 'on' : ''}" onclick={() => (section = n.id)}>
        <span class="sic"><Icon name={n.icon} size={15} /></span>{n.label}
      </button>
    {/each}
    {#if navShown.length === 0}<div class="snofilter">No matches</div>{/if}
  </nav>

  <div class="spane">
    {#if section === "appearance"}
      <h2>Appearance</h2>
      <section>
        <div class="section-header">Theme</div>
        <div class="theme-grid">
          {#each themeNames as name (name)}
            {@const ui = themes[name].ui}
            <button class="theme-card {$activeTheme === name ? 'active' : ''}" onclick={() => applyTheme(name)} title={name}>
              <div class="swatches">
                {#each swatchKeys as key (key)}<span class="swatch" style="background: {ui[key] ?? 'transparent'}"></span>{/each}
              </div>
              <span class="theme-name">{themeLabel(name)}</span>
            </button>
          {/each}
        </div>
      </section>
      <section>
        <div class="section-header">System (follow OS light/dark)</div>
        <div class="opt">
          <span class="opt-lbl">Auto switch by OS appearance</span>
          <button class="btn {$systemMode ? 'active' : ''}" onclick={() => setSystemMode(!$systemMode)}>{$systemMode ? "On" : "Off"}</button>
        </div>
        {#if $systemMode}
          <div class="opt"><span class="opt-lbl">Light variant</span>
            <select class="pick" value={$systemLight} onchange={(e) => setSystemPair((e.currentTarget as HTMLSelectElement).value, $systemDark)}>
              {#each LIGHT_THEMES as t (t)}<option value={t}>{themeLabel(t)}</option>{/each}
            </select>
          </div>
          <div class="opt"><span class="opt-lbl">Dark variant</span>
            <select class="pick" value={$systemDark} onchange={(e) => setSystemPair($systemLight, (e.currentTarget as HTMLSelectElement).value)}>
              {#each DARK_THEMES as t (t)}<option value={t}>{themeLabel(t)}</option>{/each}
            </select>
          </div>
        {/if}
      </section>
      <section>
        <div class="section-header">Layout</div>
        <div class="opt">
          <span class="opt-lbl">Density</span>
          <div class="row2">
            <button class="btn {$density === 'compact' ? 'active' : ''}" onclick={() => applyDensity("compact")}>Compact</button>
            <button class="btn {$density === 'regular' ? 'active' : ''}" onclick={() => applyDensity("regular")}>Regular</button>
          </div>
        </div>
        <div class="opt">
          <span class="opt-lbl">Auto-hide activity bar <span class="kbd-hint">slides in from left edge</span></span>
          <button class="btn {$autoHideRail ? 'active' : ''}" onclick={() => setAutoHideRail(!$autoHideRail)}>{$autoHideRail ? "On" : "Off"}</button>
        </div>
      </section>
      <section>
        <div class="section-header">Fonts</div>
        <div class="opt"><span class="opt-lbl">UI font</span>
          <select class="pick" style="font-family:'{$uiFont}'" value={$uiFont} onchange={(e) => setUiFont((e.currentTarget as HTMLSelectElement).value as any)}>
            {#each UI_FONTS as f (f)}<option value={f} style="font-family:'{f}'">{f}</option>{/each}
          </select>
        </div>
        <div class="opt"><span class="opt-lbl">Code / terminal font</span>
          <select class="pick" style="font-family:'{$monoFont}'" value={$monoFont} onchange={(e) => setMonoFont((e.currentTarget as HTMLSelectElement).value as any)}>
            {#each MONO_FONTS as f (f)}<option value={f} style="font-family:'{f}'">{f}</option>{/each}
          </select>
        </div>
      </section>
      <section>
        <div class="section-header">UI Scale</div>
        <div class="opt">
          <span class="opt-lbl">Zoom whole app <span class="kbd-hint">⌘ + / − / 0</span></span>
          <div class="stepper">
            <button onclick={() => bumpScale(-1)}>−</button>
            <span class="val">{Math.round($uiScale * 100)}%</span>
            <button onclick={() => bumpScale(1)}>+</button>
            <button class="reset" onclick={resetScale}>Reset</button>
          </div>
        </div>
      </section>
      <section>
        <div class="section-header">Window Transparency</div>
        <div class="opt">
          <span class="opt-lbl">Translucency <span class="kbd-hint">macOS vibrancy</span></span>
          <div class="stepper">
            <input type="range" min="0.5" max="1" step="0.02" value={$windowOpacity}
              oninput={(e) => applyOpacity(+(e.currentTarget as HTMLInputElement).value)} style="width:140px" />
            <span class="val">{Math.round($windowOpacity * 100)}%</span>
            <button class="reset" onclick={() => applyOpacity(1)}>Off</button>
          </div>
        </div>
      </section>
      <section>
        <div class="section-header">Custom Colors
          {#if Object.keys(overrides).length}<button class="link" onclick={clearColors}>Reset all</button>{/if}
        </div>
        <div class="color-grid">
          {#each COLOR_KEYS as k (k)}
            <label class="swatch-row">
              <input type="color" value={colorVal(k)} oninput={(e) => setColor(k, (e.currentTarget as HTMLInputElement).value)} />
              <span class="sw-lbl">{COLOR_LABEL[k]}{#if overrides[k]}<span class="sw-mod">•</span>{/if}</span>
            </label>
          {/each}
        </div>
      </section>

    {:else if section === "editor"}
      <h2>Editor</h2>
      <section>
        <div class="opt"><span class="opt-lbl">Font size</span>
          <div class="stepper"><button onclick={() => bumpEditorFontSize(-1)}>−</button><span class="val">{$editorFontSize}px</span><button onclick={() => bumpEditorFontSize(1)}>+</button></div>
        </div>
        <div class="opt"><span class="opt-lbl">Line height</span>
          <div class="stepper"><button onclick={() => bumpEditorLineHeight(-0.05)}>−</button><span class="val">{$editorLineHeight.toFixed(2)}</span><button onclick={() => bumpEditorLineHeight(0.05)}>+</button></div>
        </div>
        <div class="opt"><span class="opt-lbl">Letter spacing</span>
          <div class="stepper"><button onclick={() => bumpEditorLetterSpacing(-0.5)}>−</button><span class="val">{$editorLetterSpacing}px</span><button onclick={() => bumpEditorLetterSpacing(0.5)}>+</button></div>
        </div>
        <div class="opt"><span class="opt-lbl">Tab size</span>
          <div class="row2">{#each [2, 4, 8] as n (n)}<button class="btn {$editorTabSize === n ? 'active' : ''}" onclick={() => setEditorTabSize(n)}>{n}</button>{/each}</div>
        </div>
        <div class="opt"><span class="opt-lbl">Word wrap</span>
          <button class="btn {$editorWordWrap ? 'active' : ''}" onclick={toggleWordWrap}>{$editorWordWrap ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Font ligatures <span class="kbd-hint">Fira / JetBrains / Cascadia</span></span>
          <button class="btn {$editorLigatures ? 'active' : ''}" onclick={toggleLigatures}>{$editorLigatures ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Format on save <span class="kbd-hint">via language server</span></span>
          <button class="btn {$editorFormatOnSave ? 'active' : ''}" onclick={toggleFormatOnSave}>{$editorFormatOnSave ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Minimap</span>
          <button class="btn {$editorMinimap ? 'active' : ''}" onclick={toggleMinimap}>{$editorMinimap ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Inlay hints <span class="kbd-hint">types / params (LSP)</span></span>
          <button class="btn {$editorInlayHints ? 'active' : ''}" onclick={toggleInlayHints}>{$editorInlayHints ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Sticky scroll <span class="kbd-hint">pin enclosing scope</span></span>
          <button class="btn {$editorStickyScroll ? 'active' : ''}" onclick={toggleStickyScroll}>{$editorStickyScroll ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Inline blame <span class="kbd-hint">always show (⌥B toggles)</span></span>
          <button class="btn {$editorBlameAlways ? 'active' : ''}" onclick={toggleBlameAlways}>{$editorBlameAlways ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Bold text</span>
          <button class="btn {$editorBold ? 'active' : ''}" onclick={toggleEditorBold}>{$editorBold ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Font family</span><span class="font-note">{$monoFont} — change in Appearance</span></div>
      </section>

    {:else if section === "terminal"}
      <h2>Terminal</h2>
      <section>
        <div class="opt"><span class="opt-lbl">Font size</span>
          <div class="stepper"><button onclick={() => bumpTermFontSize(-1)}>−</button><span class="val">{$termFontSize}px</span><button onclick={() => bumpTermFontSize(1)}>+</button></div>
        </div>
        <div class="opt"><span class="opt-lbl">Line height</span>
          <div class="stepper"><button onclick={() => bumpTermLineHeight(-0.05)}>−</button><span class="val">{$termLineHeight.toFixed(2)}</span><button onclick={() => bumpTermLineHeight(0.05)}>+</button></div>
        </div>
        <div class="opt"><span class="opt-lbl">Letter spacing</span>
          <div class="stepper"><button onclick={() => bumpTermLetterSpacing(-0.5)}>−</button><span class="val">{$termLetterSpacing}px</span><button onclick={() => bumpTermLetterSpacing(0.5)}>+</button></div>
        </div>
        <div class="opt"><span class="opt-lbl">Scrollback lines</span>
          <div class="stepper"><button onclick={() => setTermScrollback($termScrollback - 1000)}>−</button><span class="val">{$termScrollback === 0 ? "off" : $termScrollback.toLocaleString()}</span><button onclick={() => setTermScrollback($termScrollback + 1000)}>+</button></div>
        </div>
        <div class="opt"><span class="opt-lbl">Cursor style</span>
          <div class="row2">{#each CURSOR_STYLES as s (s)}<button class="btn {$termCursorStyle === s ? 'active' : ''}" onclick={() => setTermCursorStyle(s)}>{s[0].toUpperCase() + s.slice(1)}</button>{/each}</div>
        </div>
        <div class="opt"><span class="opt-lbl">Cursor blink</span>
          <button class="btn {$termCursorBlink ? 'active' : ''}" onclick={toggleTermBlink}>{$termCursorBlink ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Bold text</span>
          <button class="btn {$termBold ? 'active' : ''}" onclick={toggleTermBold}>{$termBold ? "On" : "Off"}</button>
        </div>
        <div class="opt"><span class="opt-lbl">Font family</span><span class="font-note">{$monoFont} — change in Appearance</span></div>
      </section>

    {:else if section === "extensions"}
      <h2>Extensions</h2>
      <p class="acct-note">DevOps integrations are modelled as extensions. Disabling one hides its surface (rail icon + commands).</p>
      <section>
        {#each EXTENSIONS as ext (ext.id)}
          <div class="opt">
            <span class="opt-lbl">{ext.name}
              {#if !ext.builtin}<span class="acct-badge" style="color:var(--text3);border-color:var(--border)">available</span>{/if}
              <span class="kbd-hint">{ext.description}{#if ext.permissions?.length} · {ext.permissions.join(", ")}{/if}</span>
            </span>
            {#if ext.builtin}
              <button class="btn {isExtEnabled(ext.id, $extEnabled) ? 'active' : ''}" onclick={() => toggleExt(ext.id)}>{isExtEnabled(ext.id, $extEnabled) ? "Enabled" : "Disabled"}</button>
            {:else}
              <button class="btn" disabled>Install…</button>
            {/if}
          </div>
        {/each}
      </section>

    {:else if section === "accounts"}
      <h2>Accounts</h2>
      <p class="acct-note">Secrets are stored in your <strong>macOS Keychain</strong> — never written to disk in plaintext by Anvil.</p>
      <section>
        {#each ACCOUNTS as f (f.key)}
          <div class="acct">
            <div class="acct-top">
              <span class="acct-lbl">{f.label}</span>
              {#if acctSaved[f.key]}<span class="acct-badge">{f.secret ? "stored" : "set"}</span>{/if}
            </div>
            <div class="acct-row">
              <input
                class="acct-in"
                type={f.secret ? "password" : "text"}
                bind:value={acctVals[f.key]}
                placeholder={f.secret && acctSaved[f.key] ? "•••••••• (stored — type to replace)" : f.placeholder}
                spellcheck="false"
                autocomplete="off"
                onkeydown={(e) => e.key === "Enter" && saveAccount(f)}
              />
              <button class="btn" disabled={!acctVals[f.key]} onclick={() => saveAccount(f)}>Save</button>
              {#if acctSaved[f.key]}<button class="btn danger" onclick={() => clearAccount(f)}>Clear</button>{/if}
            </div>
            {#if f.hint}<span class="acct-hint">{f.hint}</span>{/if}
          </div>
        {/each}
      </section>

    {:else if section === "keymap"}
      <h2>Keymap</h2>
      <section>
        <div class="section-header">Customizable shortcuts</div>
        <div class="keys">
          {#each KEY_ACTIONS as a (a.id)}
            <div class="keyrow">
              <button class="kbd rec {recording === a.id ? 'on' : ''}" onclick={() => startRecord(a.id)}>{recording === a.id ? "press keys…" : comboFor(a.id, $keyOverrides)}</button>
              <span class="kd">{a.label}</span>
              {#if $keyOverrides[a.id]}<button class="link" onclick={() => clearKeyOverride(a.id)}>reset</button>{/if}
            </div>
          {/each}
        </div>
      </section>
      <section>
        <div class="section-header">Reference</div>
        <div class="keys">
          {#each KEYS as row (row.k)}
            <div class="keyrow"><kbd>{row.k}</kbd><span class="kd">{row.d}</span></div>
          {/each}
        </div>
      </section>

    {:else}
      <h2>About</h2>
      <section>
        <div class="about">
          <div class="wm">Anvil<span class="dot">.</span></div>
          <div class="ver">Version 0.1.0</div>
          <p class="tag">The AI-native console for 100% of your work.</p>
          <div class="meta">Tauri · SvelteKit · Rust · Maple Mono</div>
        </div>
      </section>
    {/if}
  </div>
</div>

<style>
  /* Dense, flat settings surface — hairline groups, no cards, quiet accents. */
  .settings { display: flex; height: 100%; min-height: 0; background: var(--bg); color: var(--text);
    font-family: var(--font-ui); }

  /* ── sidebar nav ── */
  .snav { flex: 0 0 168px; border-right: 1px solid var(--border); padding: 10px 8px; display: flex;
    flex-direction: column; gap: 1px; background: color-mix(in srgb, var(--panel) 40%, var(--bg)); }
  .sfilter { margin: 0 6px 8px; padding: 5px 8px; background: var(--bg); border: 1px solid var(--border);
    border-radius: 6px; color: var(--text); font-family: var(--font-ui); font-size: 11.5px; outline: 0; }
  .sfilter:focus { border-color: var(--accent); }
  .snofilter { padding: 6px 12px; font-size: 11px; color: var(--text3); }
  .stitle { font-size: 9px; font-weight: 700; letter-spacing: 0.13em; text-transform: uppercase;
    color: var(--text3); padding: 4px 10px 9px; }
  .snav-item { display: flex; align-items: center; gap: 9px; border: 0; background: transparent;
    color: var(--text2); font-family: var(--font-ui); font-size: 12px; font-weight: 500; padding: 5px 10px;
    border-radius: 6px; text-align: left; cursor: default; transition: background 0.1s, color 0.1s; }
  .snav-item:hover { background: var(--panel2); color: var(--text); }
  .snav-item.on { background: var(--panel2); color: var(--text); font-weight: 600; }
  .sic { display: inline-flex; align-items: center; width: 15px; color: currentColor; opacity: 0.75; }

  /* ── content pane ── */
  .spane { flex: 1; min-width: 0; overflow-y: auto; padding: 14px 16px 18px; display: flex; flex-direction: column;
    gap: 14px; }
  h2 { margin: 0; font-size: 12px; font-weight: 600; letter-spacing: 0.01em; color: var(--text); }

  /* a group: label header + thin rule, rows below — no card chrome */
  section { display: flex; flex-direction: column; max-width: 620px; }
  .section-header { display: flex; align-items: center; font-size: 11px; font-weight: 600;
    color: var(--text3); padding: 0 0 5px; margin-top: 2px;
    border-bottom: 1px solid var(--border); margin-bottom: 6px; }
  section > .theme-grid, section > .color-grid { margin-top: 2px; }

  /* theme cards — flat, small */
  .theme-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(124px, 1fr)); gap: 6px; }
  .theme-card { display: flex; align-items: center; gap: 8px; padding: 6px 8px; background: transparent;
    border: 1px solid var(--border); border-radius: 6px; cursor: default; text-align: left; transition: border-color 0.1s, background 0.1s; }
  .theme-card:hover { border-color: var(--text3); background: var(--panel2); }
  .theme-card.active { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 9%, transparent); }
  .swatches { display: flex; gap: 3px; flex-shrink: 0; }
  .swatch { width: 11px; height: 11px; border-radius: 3px; flex-shrink: 0; }
  .theme-name { font-size: 11px; color: var(--text2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .theme-card.active .theme-name { color: var(--text); }

  /* segmented control + buttons — quiet, monochrome-forward */
  .row2 { display: inline-flex; background: var(--bg); border: 1px solid var(--border); border-radius: 6px; padding: 2px; gap: 1px; }
  .row2 .btn { border: 0; background: transparent; border-radius: 4px; padding: 3px 12px; }
  .row2 .btn.active { background: var(--panel2); color: var(--text); box-shadow: inset 0 0 0 1px var(--border); }
  .btn { padding: 4px 12px; background: var(--bg); border: 1px solid var(--border); border-radius: 6px;
    color: var(--text2); font-size: 12px; font-weight: 500; font-family: var(--font-ui); cursor: default;
    transition: background 0.1s, color 0.1s, border-color 0.1s; }
  .btn:hover { color: var(--text); border-color: var(--text3); }
  .btn.active { border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 13%, transparent); color: var(--accent); }
  .btn.danger { color: var(--red); background: transparent; border-color: transparent; }
  .btn:disabled { opacity: 0.4; }

  /* settings rows — hairline-separated list */
  .opt { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 30px; padding: 3px 0; }
  .opt + .opt { border-top: 1px solid color-mix(in srgb, var(--border) 55%, transparent); }
  .opt-lbl { font-size: 12px; color: var(--text); }
  .stepper { display: inline-flex; align-items: center; gap: 5px; }
  .stepper button { width: 23px; height: 23px; display: inline-flex; align-items: center; justify-content: center;
    background: var(--bg); border: 1px solid var(--border); border-radius: 6px; color: var(--text2); font-size: 14px; cursor: default; }
  .stepper button:hover { border-color: var(--text3); color: var(--text); }
  .stepper .val { font-family: var(--font-mono); font-size: 11.5px; color: var(--text); min-width: 38px; text-align: center; }
  .stepper .reset { width: auto; padding: 0 9px; height: 23px; font-size: 11px; font-family: var(--font-ui); }
  .font-note { font-size: 12px; font-family: var(--font-mono); color: var(--text3); }
  .kbd-hint { font-family: var(--font-mono); font-size: 10px; color: var(--text3); margin-left: 6px; }
  .link { margin-left: auto; border: 0; background: transparent; color: var(--accent); font-size: 10.5px; cursor: default; text-transform: none; font-weight: 600; }

  .color-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: 5px 10px; }
  .swatch-row { display: flex; align-items: center; gap: 7px; cursor: default; }
  .swatch-row input[type="color"] { width: 19px; height: 19px; padding: 0; border: 1px solid var(--border);
    border-radius: 5px; background: transparent; cursor: default; }
  .sw-lbl { font-size: 11px; color: var(--text2); }
  .sw-mod { color: var(--accent); margin-left: 3px; }
  .pick { background: var(--bg); border: 1px solid var(--border); border-radius: 6px; color: var(--text);
    font-family: var(--font-ui); font-size: 12px; padding: 4px 9px; outline: 0; cursor: default; }

  .keys { display: flex; flex-direction: column; }
  .keyrow { display: flex; align-items: center; gap: 12px; padding: 4px 2px; }
  .keyrow + .keyrow { border-top: 1px solid color-mix(in srgb, var(--border) 50%, transparent); }
  kbd { flex: 0 0 60px; font-family: var(--font-mono); font-size: 11px; color: var(--text2);
    background: var(--bg); border: 1px solid var(--border); border-radius: 5px; padding: 1px 6px; text-align: center; }
  .kd { font-size: 12px; color: var(--text2); }
  .rec { flex: 0 0 64px; font-family: var(--font-mono); font-size: 11px; color: var(--text2);
    background: var(--bg); border: 1px solid var(--border); border-radius: 5px; padding: 1px 6px;
    text-align: center; cursor: default; }
  .rec:hover { border-color: var(--text3); color: var(--text); }
  .rec.on { border-color: var(--accent); color: var(--accent); }

  .acct-note { font-size: 11.5px; color: var(--text3); margin: -6px 0 4px; max-width: 620px; }
  .acct { display: flex; flex-direction: column; gap: 5px; padding: 9px 0; }
  .acct + .acct { border-top: 1px solid color-mix(in srgb, var(--border) 55%, transparent); }
  .acct-top { display: flex; align-items: center; gap: 8px; }
  .acct-lbl { font-size: 12px; color: var(--text); font-weight: 600; }
  .acct-badge { font-size: 9px; color: var(--green); border: 1px solid color-mix(in srgb, var(--green) 55%, var(--border)); border-radius: 4px; padding: 0 5px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
  .acct-row { display: flex; gap: 6px; }
  .acct-in { flex: 1; background: var(--bg); border: 1px solid var(--border); border-radius: 6px;
    color: var(--text); font-size: 12px; font-family: var(--font-mono); padding: 5px 9px; outline: 0; transition: border-color 0.1s; }
  .acct-in:focus { border-color: var(--accent); }
  .acct-hint { font-size: 10.5px; color: var(--text3); }

  .about { display: flex; flex-direction: column; gap: 4px; align-items: flex-start; }
  .wm { font-size: 26px; font-weight: 700; letter-spacing: -0.02em; }
  .wm .dot { color: var(--accent); }
  .ver { font-family: var(--font-mono); font-size: 11px; color: var(--text3); }
  .tag { margin: 6px 0 0; font-size: 12.5px; color: var(--text2); max-width: 340px; }
  .meta { font-size: 11px; color: var(--text3); margin-top: 4px; }
</style>
