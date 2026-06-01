// Tests for the DOM + localStorage paths in themes.ts.
// The existing themes.test.ts covers pure helpers (themeLabel, isLight, etc.).
import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  activeTheme, systemMode,
  applyTheme, cycleTheme, setSystemMode, setSystemPair, initTheme,
  themes, THEME_KEYS,
} from "./themes";

beforeEach(() => {
  localStorage.clear();
  // reset documentElement inline styles
  document.documentElement.removeAttribute("style");
  document.documentElement.style.colorScheme = "";
  activeTheme.set("anvil-dark");
  systemMode.set(false);
});

describe("applyTheme — CSS vars + persist + colorScheme", () => {
  it("sets --bg on documentElement matching the theme", () => {
    applyTheme("anvil-dark");
    const bg = document.documentElement.style.getPropertyValue("--bg");
    expect(bg).toBe(themes["anvil-dark"].ui.bg);
  });

  it("sets --accent on documentElement matching the theme", () => {
    applyTheme("anvil-dark");
    const accent = document.documentElement.style.getPropertyValue("--accent");
    expect(accent).toBe(themes["anvil-dark"].ui.accent);
  });

  it("persists the theme key to localStorage", () => {
    applyTheme("tokyo-night");
    expect(localStorage.getItem("anvil-theme")).toBe("tokyo-night");
    expect(get(activeTheme)).toBe("tokyo-night");
  });

  it("sets colorScheme=light for a light theme", () => {
    applyTheme("anvil-light");
    expect(document.documentElement.style.colorScheme).toBe("light");
  });

  it("sets colorScheme=dark for a dark theme", () => {
    applyTheme("anvil-dark");
    expect(document.documentElement.style.colorScheme).toBe("dark");
  });

  it("turns off system mode when called while systemMode is on", () => {
    systemMode.set(true);
    localStorage.setItem("anvil-system-mode", "1");
    applyTheme("dracula");
    expect(get(systemMode)).toBe(false);
    expect(localStorage.getItem("anvil-system-mode")).toBe("0");
  });
});

describe("applyTheme — custom-override layering", () => {
  it("custom-color overrides win over base theme bg", () => {
    localStorage.setItem("anvil-custom-theme", JSON.stringify({ bg: "#ff0000" }));
    applyTheme("anvil-dark");
    expect(document.documentElement.style.getPropertyValue("--bg")).toBe("#ff0000");
  });

  it("custom overrides do not affect other theme vars", () => {
    localStorage.setItem("anvil-custom-theme", JSON.stringify({ bg: "#ff0000" }));
    applyTheme("anvil-dark");
    const panel = document.documentElement.style.getPropertyValue("--panel");
    expect(panel).toBe(themes["anvil-dark"].ui.panel);
  });

  it("malformed custom JSON is ignored gracefully", () => {
    localStorage.setItem("anvil-custom-theme", "not valid json");
    expect(() => applyTheme("anvil-dark")).not.toThrow();
    expect(document.documentElement.style.getPropertyValue("--bg")).toBe(themes["anvil-dark"].ui.bg);
  });
});

describe("cycleTheme", () => {
  it("advances to the next theme in THEME_KEYS", () => {
    activeTheme.set(THEME_KEYS[0]);
    cycleTheme();
    expect(get(activeTheme)).toBe(THEME_KEYS[1]);
  });

  it("wraps around from the last theme to the first", () => {
    const last = THEME_KEYS[THEME_KEYS.length - 1];
    activeTheme.set(last);
    cycleTheme();
    expect(get(activeTheme)).toBe(THEME_KEYS[0]);
  });

  it("persists the new theme", () => {
    activeTheme.set(THEME_KEYS[0]);
    cycleTheme();
    expect(localStorage.getItem("anvil-theme")).toBe(THEME_KEYS[1]);
  });
});

describe("setSystemMode", () => {
  it("persists '1' when turned on", () => {
    setSystemMode(true);
    expect(get(systemMode)).toBe(true);
    expect(localStorage.getItem("anvil-system-mode")).toBe("1");
  });

  it("persists '0' when turned off", () => {
    setSystemMode(false);
    expect(get(systemMode)).toBe(false);
    expect(localStorage.getItem("anvil-system-mode")).toBe("0");
  });
});

describe("setSystemPair", () => {
  it("persists both light and dark keys", () => {
    setSystemPair("anvil-light", "gruvbox-dark");
    expect(localStorage.getItem("anvil-system-light")).toBe("anvil-light");
    expect(localStorage.getItem("anvil-system-dark")).toBe("gruvbox-dark");
  });
});

describe("initTheme — restores from localStorage", () => {
  it("applies the stored theme key", () => {
    localStorage.setItem("anvil-theme", "nord");
    initTheme();
    expect(get(activeTheme)).toBe("nord");
  });

  it("defaults to anvil-dark when nothing is stored", () => {
    initTheme();
    expect(get(activeTheme)).toBe("anvil-dark");
  });
});
