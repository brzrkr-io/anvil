// Tests for the localStorage + DOM paths in fonts.ts that were blocked in the
// node environment. The existing fonts.test.ts covers stack-builder pure logic.
import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  uiFont, monoFont, editorBold, termBold,
  setUiFont, setMonoFont, toggleEditorBold, toggleTermBold, initFonts,
  UI_FONTS, MONO_FONTS,
} from "./fonts";

beforeEach(() => {
  localStorage.clear();
  document.documentElement.style.removeProperty("--font-ui");
  document.documentElement.style.removeProperty("--font-mono");
});

describe("setUiFont — persist + CSS var", () => {
  it("persists the chosen font to localStorage", () => {
    setUiFont("Inter");
    expect(localStorage.getItem("anvil-ui-font")).toBe("Inter");
    expect(get(uiFont)).toBe("Inter");
  });

  it("sets --font-ui on documentElement", () => {
    setUiFont("IBM Plex Sans");
    const v = document.documentElement.style.getPropertyValue("--font-ui");
    expect(v).toContain("IBM Plex Sans");
  });

  it("rounds through all UI_FONTS without error", () => {
    for (const f of UI_FONTS) {
      setUiFont(f);
      expect(get(uiFont)).toBe(f);
    }
  });
});

describe("setMonoFont — persist + CSS var", () => {
  it("persists the chosen font to localStorage", () => {
    setMonoFont("Fira Code");
    expect(localStorage.getItem("anvil-mono-font")).toBe("Fira Code");
    expect(get(monoFont)).toBe("Fira Code");
  });

  it("sets --font-mono on documentElement", () => {
    setMonoFont("JetBrains Mono");
    const v = document.documentElement.style.getPropertyValue("--font-mono");
    expect(v).toContain("JetBrains Mono");
  });

  it("rounds through all MONO_FONTS without error", () => {
    for (const f of MONO_FONTS) {
      setMonoFont(f);
      expect(get(monoFont)).toBe(f);
    }
  });
});

describe("toggleEditorBold — flip + persist", () => {
  it("flips false → true and persists '1'", () => {
    editorBold.set(false);
    toggleEditorBold();
    expect(get(editorBold)).toBe(true);
    expect(localStorage.getItem("anvil-editor-bold")).toBe("1");
  });

  it("flips true → false and persists '0'", () => {
    editorBold.set(true);
    toggleEditorBold();
    expect(get(editorBold)).toBe(false);
    expect(localStorage.getItem("anvil-editor-bold")).toBe("0");
  });
});

describe("toggleTermBold — flip + persist", () => {
  it("flips false → true and persists '1'", () => {
    termBold.set(false);
    toggleTermBold();
    expect(get(termBold)).toBe(true);
    expect(localStorage.getItem("anvil-term-bold")).toBe("1");
  });

  it("flips true → false and persists '0'", () => {
    termBold.set(true);
    toggleTermBold();
    expect(get(termBold)).toBe(false);
    expect(localStorage.getItem("anvil-term-bold")).toBe("0");
  });
});

describe("initFonts — applies stored choices to CSS", () => {
  it("applies the current uiFont and monoFont stores to CSS vars", () => {
    setUiFont("IBM Plex Sans");
    setMonoFont("IBM Plex Mono");
    document.documentElement.style.removeProperty("--font-ui");
    document.documentElement.style.removeProperty("--font-mono");
    initFonts();
    expect(document.documentElement.style.getPropertyValue("--font-ui")).toContain("IBM Plex Sans");
    expect(document.documentElement.style.getPropertyValue("--font-mono")).toContain("IBM Plex Mono");
  });
});
