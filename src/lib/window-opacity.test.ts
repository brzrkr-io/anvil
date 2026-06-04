import { describe, it, expect, beforeEach } from "vitest";
import { applyOpacity, windowOpacity } from "./window-opacity.js";
import { get } from "svelte/store";

describe("window-opacity", () => {
  beforeEach(() => { localStorage.clear(); document.documentElement.className = ""; });

  it("clamps below the minimum so the window never becomes invisible", () => {
    applyOpacity(0.05);
    expect(get(windowOpacity)).toBe(0.15);
  });

  it("clamps above 1 (fully opaque is the ceiling)", () => {
    applyOpacity(2);
    expect(get(windowOpacity)).toBe(1);
  });

  it("drives --win-alpha and only marks translucent below 1", () => {
    applyOpacity(0.8);
    expect(document.documentElement.style.getPropertyValue("--win-alpha")).toBe("0.8");
    expect(document.documentElement.classList.contains("translucent")).toBe(true);
    applyOpacity(1);
    expect(document.documentElement.classList.contains("translucent")).toBe(false);
  });

  it("persists the chosen value", () => {
    applyOpacity(0.7);
    expect(localStorage.getItem("anvil-win-opacity")).toBe("0.7");
  });
});
