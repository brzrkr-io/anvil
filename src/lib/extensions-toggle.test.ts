import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { toggleExt, extEnabled, isExtEnabled } from "./extensions";

beforeEach(() => {
  localStorage.clear();
  // Reset the store by clearing localStorage and re-setting to empty
  extEnabled.set({});
});

describe("toggleExt", () => {
  it("disables a built-in extension that is currently enabled by default", () => {
    // kubernetes is built-in so isExtEnabled returns true from the default
    toggleExt("kubernetes");
    const map = get(extEnabled);
    expect(map["kubernetes"]).toBe(false);
  });

  it("enables a non-built-in extension that is currently disabled by default", () => {
    toggleExt("grafana");
    const map = get(extEnabled);
    expect(map["grafana"]).toBe(true);
  });

  it("persists the toggled state to localStorage", () => {
    toggleExt("kubernetes");
    const stored = JSON.parse(localStorage.getItem("anvil-ext")!);
    expect(stored["kubernetes"]).toBe(false);
  });

  it("double-toggling a built-in returns it to enabled", () => {
    toggleExt("kubernetes");
    toggleExt("kubernetes");
    const map = get(extEnabled);
    expect(isExtEnabled("kubernetes", map)).toBe(true);
  });
});
