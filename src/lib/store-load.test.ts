import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { get } from "svelte/store";

// These modules read persisted values at import time. To cover the load branches
// (valid-stored vs out-of-range-default vs no-storage) we seed localStorage then
// re-import a fresh module instance.
beforeEach(() => { localStorage.clear(); vi.resetModules(); });
afterEach(() => vi.unstubAllGlobals());

describe("store load branches", () => {
  it("editor-settings restores in-range persisted values", async () => {
    localStorage.setItem("anvil-editor-fs", "20");
    localStorage.setItem("anvil-editor-tab", "4");
    localStorage.setItem("anvil-editor-wrap", "1");
    localStorage.setItem("anvil-editor-minimap", "0");
    localStorage.setItem("anvil-editor-ghost-src", "llm");
    const m = await import("./editor-settings.js");
    expect(get(m.editorFontSize)).toBe(20);
    expect(get(m.editorTabSize)).toBe(4);
    expect(get(m.editorWordWrap)).toBe(true);
    expect(get(m.editorMinimap)).toBe(false);
    expect(get(m.editorGhostSource)).toBe("llm");
  });

  it("editor-settings falls back to defaults for out-of-range values", async () => {
    localStorage.setItem("anvil-editor-fs", "999");
    localStorage.setItem("anvil-editor-tab", "0");
    const m = await import("./editor-settings.js");
    expect(get(m.editorFontSize)).toBe(13);
    expect(get(m.editorTabSize)).toBe(2);
  });

  it("terminal-settings restores persisted values incl. a valid cursor style", async () => {
    localStorage.setItem("anvil-term-fs", "16");
    localStorage.setItem("anvil-term-cursor", "bar");
    localStorage.setItem("anvil-term-blink", "0");
    const m = await import("./terminal-settings.js");
    expect(get(m.termFontSize)).toBe(16);
    expect(get(m.termCursorStyle)).toBe("bar");
    expect(get(m.termCursorBlink)).toBe(false);
  });

  it("terminal-settings rejects an invalid persisted cursor style", async () => {
    localStorage.setItem("anvil-term-cursor", "diamond");
    const m = await import("./terminal-settings.js");
    expect(get(m.termCursorStyle)).toBe("block");
  });

  it("fonts restore persisted families and reject unknown ones", async () => {
    localStorage.setItem("anvil-ui-font", "Inter");
    localStorage.setItem("anvil-mono-font", "not-a-font");
    const m = await import("./fonts.js");
    expect(get(m.uiFont)).toBe("Inter");
    expect(get(m.monoFont)).toBe("IBM Plex Mono"); // default
  });

  it("scale and window-opacity restore in-range persisted values", async () => {
    localStorage.setItem("anvil-ui-scale", "1.3");
    localStorage.setItem("anvil-win-opacity", "0.6");
    const s = await import("./scale.js");
    const o = await import("./window-opacity.js");
    expect(get(s.uiScale)).toBe(1.3);
    expect(get(o.windowOpacity)).toBe(0.6);
  });

  it("telemetry respects a persisted opt-in flag", async () => {
    localStorage.setItem("anvil-telemetry-on", "1");
    const m = await import("./telemetry.js");
    expect(get(m.telemetryEnabled)).toBe(true);
  });

  it("every store module defaults sanely when storage is unavailable (SSR)", async () => {
    vi.stubGlobal("localStorage", undefined);
    const ed = await import("./editor-settings.js");
    const tm = await import("./terminal-settings.js");
    const fo = await import("./fonts.js");
    const sc = await import("./scale.js");
    const op = await import("./window-opacity.js");
    const te = await import("./telemetry.js");
    expect(get(ed.editorFontSize)).toBe(13);
    expect(get(tm.termFontSize)).toBe(13);
    expect(get(fo.uiFont)).toBe("IBM Plex Sans");
    expect(get(sc.uiScale)).toBe(1);
    expect(get(op.windowOpacity)).toBe(1);
    expect(get(te.telemetryEnabled)).toBe(false);
  });
});

describe("setters tolerate missing storage (SSR write path)", () => {
  it("editor-settings setters no-op storage but still update stores", async () => {
    vi.stubGlobal("localStorage", undefined);
    const m = await import("./editor-settings.js");
    m.setEditorFontSize(18); m.bumpEditorFontSize(1);
    m.setEditorTabSize(4);
    m.toggleWordWrap(); m.toggleMinimap(); m.toggleLigatures();
    m.bumpEditorLineHeight(0.1); m.bumpEditorLetterSpacing(0.5);
    m.toggleStickyScroll(); m.toggleInlayHints(); m.toggleFormatOnSave();
    m.toggleBlameAlways(); m.toggleVimMode(); m.toggleGhostText();
    m.setGhostSource("llm");
    const { get } = await import("svelte/store");
    expect(get(m.editorGhostSource)).toBe("llm");
  });

  it("terminal-settings setters no-op storage but still update stores", async () => {
    vi.stubGlobal("localStorage", undefined);
    const m = await import("./terminal-settings.js");
    m.setTermScrollback(5000); m.setTermFontSize(15); m.bumpTermFontSize(1);
    m.toggleTermBlink(); m.setTermCursorStyle("underline");
    m.bumpTermLineHeight(0.1); m.bumpTermLetterSpacing(0.5);
    const { get } = await import("svelte/store");
    expect(get(m.termCursorStyle)).toBe("underline");
  });

  it("fonts setters no-op storage but still update stores", async () => {
    vi.stubGlobal("localStorage", undefined);
    const m = await import("./fonts.js");
    m.setUiFont("Inter"); m.setMonoFont("Fira Code");
    m.toggleEditorBold(); m.toggleTermBold();
    m.initFonts();
    expect(m.monoFamily()).toContain("Fira Code");
  });

  it("scale + opacity + telemetry tolerate missing storage", async () => {
    vi.stubGlobal("localStorage", undefined);
    const s = await import("./scale.js");
    s.applyScale(1.2); s.bumpScale(1); s.resetScale(); s.initScale();
    const o = await import("./window-opacity.js");
    o.applyOpacity(0.7); o.initOpacity();
    const t = await import("./telemetry.js");
    t.setTelemetry(true); t.logEvent("x"); t.toggleTelemetry(); t.clearEvents();
    expect(t.getEvents()).toEqual([]);
  });
});

describe("storage-touching modules default + write safely without storage", () => {
  beforeEach(() => vi.resetModules());
  it("read/load + write paths no-op when localStorage is unavailable", async () => {
    vi.stubGlobal("localStorage", undefined);
    const cache = await import("./cache.js");
    expect(cache.readCache("x")).toBeNull();
    cache.writeCache("x", 1);

    const pr = await import("./palette-rank.js");
    pr.bumpUsage("a");
    expect(pr.rankItems([{ label: "a" }])).toHaveLength(1);

    const rd = await import("./redaction.js");
    rd.setRedactionRules(["x"]);
    rd.auditAgentSend("k", "t");
    expect(rd.getAuditLog()).toEqual([]);
    expect(rd.applyRedaction("hello")).toBeTypeOf("string");

    const ch = await import("./command-history.js");
    expect(ch.getHistory()).toEqual([]);
    ch.clearHistory();

    const cr = await import("./crash.js");
    expect(Array.isArray(cr.getCrashes())).toBe(true);
    cr.clearCrashes();

    const cs = await import("./cm-snippets.js");
    expect(Array.isArray(cs.getUserSnippets())).toBe(true);

    const sn = await import("./snippets.js");
    expect(Array.isArray(sn.getSnippets())).toBe(true);


    const lay = await import("./layout-settings.js");
    lay.setAutoHideRail(true);
    lay.toggleFocusDimming();
    lay.toggleTerminalAutoCd();

    const km = await import("./keymap.js");
    km.clearKeyOverride("anything");

    const acc = await import("./accounts.js");
    const f = acc.ACCOUNTS.find((a) => !a.secret);
    if (f) { await acc.getValue(f); await acc.setValue(f, "x"); await acc.clearValue(f); }
  });
});

describe("offline store SSR", () => {
  beforeEach(() => vi.resetModules());
  it("defaults to online when navigator is unavailable", async () => {
    vi.stubGlobal("navigator", undefined);
    const m = await import("./offline.js");
    const { get } = await import("svelte/store");
    expect(get(m.online)).toBe(true);
  });
});
