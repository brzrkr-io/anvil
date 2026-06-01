import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { uiScale, applyScale, bumpScale, resetScale } from "./scale";

beforeEach(() => {
  localStorage.clear();
  // reset document state between tests
  document.documentElement.style.removeProperty("--scale");
  delete document.documentElement.dataset.scaled;
});

describe("applyScale — clamp + persist + CSS var", () => {
  it("persists a valid scale value", () => {
    applyScale(1.2);
    expect(localStorage.getItem("anvil-ui-scale")).toBe("1.2");
    expect(get(uiScale)).toBe(1.2);
  });

  it("clamps below MIN (0.7) to 0.7", () => {
    applyScale(0.1);
    expect(get(uiScale)).toBe(0.7);
    expect(localStorage.getItem("anvil-ui-scale")).toBe("0.7");
  });

  it("clamps above MAX (1.8) to 1.8", () => {
    applyScale(5.0);
    expect(get(uiScale)).toBe(1.8);
    expect(localStorage.getItem("anvil-ui-scale")).toBe("1.8");
  });

  it("sets --scale CSS property on documentElement", () => {
    applyScale(1.3);
    expect(document.documentElement.style.getPropertyValue("--scale")).toBe("1.3");
  });

  it("sets data-scaled attribute when not 1", () => {
    applyScale(1.2);
    expect(document.documentElement.dataset.scaled).toBe("1");
  });

  it("removes data-scaled when exactly 1", () => {
    applyScale(1.2);
    applyScale(1);
    expect(document.documentElement.dataset.scaled).toBeUndefined();
  });
});

describe("bumpScale", () => {
  it("increments by STEP (0.1)", () => {
    applyScale(1.0);
    bumpScale(1);
    expect(get(uiScale)).toBeCloseTo(1.1, 5);
  });

  it("does not exceed MAX", () => {
    applyScale(1.8);
    bumpScale(5);
    expect(get(uiScale)).toBe(1.8);
  });

  it("does not go below MIN", () => {
    applyScale(0.7);
    bumpScale(-5);
    expect(get(uiScale)).toBe(0.7);
  });
});

describe("resetScale", () => {
  it("resets to 1.0 and persists", () => {
    applyScale(1.5);
    resetScale();
    expect(get(uiScale)).toBe(1);
    expect(localStorage.getItem("anvil-ui-scale")).toBe("1");
  });
});
