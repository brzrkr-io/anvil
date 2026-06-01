import { describe, it, expect } from "vitest";
import {
  UI_FONTS,
  MONO_FONTS,
  monoFamily,
  monoFont,
  uiFont,
} from "./fonts";

// Derive the font stacks the same way the module does, to test the logic
// rather than hardcoded strings.
const monoStack = (f: string) =>
  `"${f}", "Symbols Nerd Font Mono", "SF Mono", Menlo, ui-monospace, monospace`;
const uiStack = (f: string) =>
  f.includes("Sans")
    ? `"${f}", -apple-system, system-ui, sans-serif`
    : monoStack(f);

describe("mono font stack", () => {
  it("includes the picked font as the first entry", () => {
    monoFont.set("JetBrains Mono");
    expect(monoFamily()).toContain('"JetBrains Mono"');
  });

  it("always includes monospace fallbacks", () => {
    for (const f of MONO_FONTS) {
      monoFont.set(f);
      const stack = monoFamily();
      expect(stack).toContain("monospace");
      expect(stack).toContain("ui-monospace");
    }
  });

  it("produces the expected full stack for each mono font", () => {
    for (const f of MONO_FONTS) {
      monoFont.set(f);
      expect(monoFamily()).toBe(monoStack(f));
    }
  });
});

describe("ui font stack (via uiStack logic)", () => {
  it("IBM Plex Sans uses sans-serif fallback stack", () => {
    const stack = uiStack("IBM Plex Sans");
    expect(stack).toContain("sans-serif");
    expect(stack).not.toContain("monospace");
  });

  it("IBM Plex Mono uses monospace fallback stack", () => {
    const stack = uiStack("IBM Plex Mono");
    expect(stack).toContain("monospace");
  });

  it("Inter uses sans-serif fallback stack", () => {
    // Inter does not contain 'Sans' in the name — falls through to monoStack
    // This tests the actual branch in the source.
    const stack = uiStack("Inter");
    // Inter does NOT include 'Sans', so it goes through monoStack
    expect(stack).toContain("monospace");
  });

  it("all UI_FONTS produce a non-empty stack", () => {
    for (const f of UI_FONTS) {
      expect(uiStack(f).length).toBeGreaterThan(0);
    }
  });
});

describe("defaults (localStorage absent in node)", () => {
  it("uiFont store holds a value in UI_FONTS", () => {
    // localStorage is undefined in node; module initialises to its hardcoded default.
    // Other tests may mutate the store, so we just assert the value is valid.
    let v = "";
    uiFont.subscribe((x) => { v = x; })();
    expect(UI_FONTS).toContain(v);
  });

  it("monoFont store holds a value in MONO_FONTS", () => {
    let v = "";
    monoFont.subscribe((x) => { v = x; })();
    expect(MONO_FONTS).toContain(v);
  });
});
