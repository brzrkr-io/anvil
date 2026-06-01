import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  autoHideRail, setAutoHideRail, toggleAutoHideRail,
  focusDimming, setFocusDimming, toggleFocusDimming,
  terminalAutoCd, toggleTerminalAutoCd,
} from "./layout-settings";

beforeEach(() => localStorage.clear());

describe("autoHideRail — set + persist + toggle", () => {
  it("setAutoHideRail(true) persists '1'", () => {
    setAutoHideRail(true);
    expect(get(autoHideRail)).toBe(true);
    expect(localStorage.getItem("anvil-autohide-rail")).toBe("1");
  });

  it("setAutoHideRail(false) persists '0'", () => {
    setAutoHideRail(false);
    expect(get(autoHideRail)).toBe(false);
    expect(localStorage.getItem("anvil-autohide-rail")).toBe("0");
  });

  it("toggleAutoHideRail flips false → true + persists", () => {
    autoHideRail.set(false);
    toggleAutoHideRail();
    expect(get(autoHideRail)).toBe(true);
    expect(localStorage.getItem("anvil-autohide-rail")).toBe("1");
  });

  it("toggleAutoHideRail flips true → false + persists", () => {
    autoHideRail.set(true);
    toggleAutoHideRail();
    expect(get(autoHideRail)).toBe(false);
    expect(localStorage.getItem("anvil-autohide-rail")).toBe("0");
  });
});

describe("focusDimming — set + persist + toggle", () => {
  it("setFocusDimming(true) persists '1'", () => {
    setFocusDimming(true);
    expect(get(focusDimming)).toBe(true);
    expect(localStorage.getItem("anvil-focus-dim")).toBe("1");
  });

  it("setFocusDimming(false) persists '0'", () => {
    setFocusDimming(false);
    expect(get(focusDimming)).toBe(false);
    expect(localStorage.getItem("anvil-focus-dim")).toBe("0");
  });

  it("toggleFocusDimming flips and persists", () => {
    focusDimming.set(false);
    toggleFocusDimming();
    expect(get(focusDimming)).toBe(true);
    expect(localStorage.getItem("anvil-focus-dim")).toBe("1");
  });
});

describe("terminalAutoCd — toggle + persist", () => {
  it("flips false → true and persists '1'", () => {
    terminalAutoCd.set(false);
    toggleTerminalAutoCd();
    expect(get(terminalAutoCd)).toBe(true);
    expect(localStorage.getItem("anvil-term-autocd")).toBe("1");
  });

  it("flips true → false and persists '0'", () => {
    terminalAutoCd.set(true);
    toggleTerminalAutoCd();
    expect(get(terminalAutoCd)).toBe(false);
    expect(localStorage.getItem("anvil-term-autocd")).toBe("0");
  });
});
