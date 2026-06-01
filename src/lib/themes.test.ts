import { describe, it, expect } from "vitest";
import {
  themeLabel,
  isLight,
  LIGHT_THEMES,
  DARK_THEMES,
  THEME_KEYS,
  THEME_LABELS,
  themes,
} from "./themes";

describe("themeLabel", () => {
  it("returns the human label for a known key", () => {
    expect(themeLabel("anvil-dark")).toBe("Anvil Dark");
    expect(themeLabel("rose-pine-dawn")).toBe("Rosé Pine Dawn");
    expect(themeLabel("catppuccin-mocha")).toBe("Catppuccin Mocha");
  });

  it("title-cases from hyphens for an unknown key", () => {
    expect(themeLabel("my-custom-theme")).toBe("My Custom Theme");
    expect(themeLabel("single")).toBe("Single");
  });

  it("every key in themes has an entry in THEME_LABELS", () => {
    for (const key of Object.keys(themes)) {
      expect(THEME_LABELS).toHaveProperty(key);
    }
  });
});

describe("isLight", () => {
  it("classifies *-light themes as light", () => {
    expect(isLight("anvil-light")).toBe(true);
    expect(isLight("solarized-light")).toBe(true);
    expect(isLight("gruvbox-light")).toBe(true);
    expect(isLight("github-light")).toBe(true);
    expect(isLight("catppuccin-latte")).toBe(false); // no "light" or "dawn"
  });

  it("classifies *-dawn themes as light", () => {
    expect(isLight("rose-pine-dawn")).toBe(true);
  });

  it("classifies standard dark themes as dark", () => {
    expect(isLight("anvil-dark")).toBe(false);
    expect(isLight("tokyo-night")).toBe(false);
    expect(isLight("dracula")).toBe(false);
  });
});

describe("LIGHT_THEMES / DARK_THEMES", () => {
  it("LIGHT_THEMES contains only themes classified as light", () => {
    for (const name of LIGHT_THEMES) {
      expect(isLight(name)).toBe(true);
    }
  });

  it("DARK_THEMES contains only themes classified as dark", () => {
    for (const name of DARK_THEMES) {
      expect(isLight(name)).toBe(false);
    }
  });

  it("LIGHT_THEMES + DARK_THEMES covers all THEME_KEYS", () => {
    const all = new Set([...LIGHT_THEMES, ...DARK_THEMES]);
    for (const key of THEME_KEYS) {
      expect(all.has(key)).toBe(true);
    }
  });

  it("LIGHT_THEMES and DARK_THEMES are disjoint", () => {
    const lightSet = new Set(LIGHT_THEMES);
    for (const name of DARK_THEMES) {
      expect(lightSet.has(name)).toBe(false);
    }
  });

  it("known light themes appear in LIGHT_THEMES", () => {
    expect(LIGHT_THEMES).toContain("anvil-light");
    expect(LIGHT_THEMES).toContain("rose-pine-dawn");
  });
});

describe("THEME_LABELS completeness", () => {
  it("has a label for every key in themes", () => {
    for (const key of THEME_KEYS) {
      expect(typeof THEME_LABELS[key]).toBe("string");
      expect(THEME_LABELS[key].length).toBeGreaterThan(0);
    }
  });

  it("THEME_LABELS and themes have the same number of keys", () => {
    expect(Object.keys(THEME_LABELS)).toHaveLength(Object.keys(themes).length);
  });
});
