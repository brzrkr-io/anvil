import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { density, applyDensity, initDensity, toggleDensity } from "./density";

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.density;
  density.set("regular");
});

describe("applyDensity — persist + document attribute", () => {
  it("sets compact: persists and applies data-density='compact'", () => {
    applyDensity("compact");
    expect(get(density)).toBe("compact");
    expect(localStorage.getItem("anvil-density")).toBe("compact");
    expect(document.documentElement.dataset.density).toBe("compact");
  });

  it("sets regular: persists and applies data-density='regular'", () => {
    applyDensity("regular");
    expect(get(density)).toBe("regular");
    expect(localStorage.getItem("anvil-density")).toBe("regular");
    expect(document.documentElement.dataset.density).toBe("regular");
  });
});

describe("initDensity — load from localStorage", () => {
  it("restores 'compact' from storage", () => {
    localStorage.setItem("anvil-density", "compact");
    initDensity();
    expect(get(density)).toBe("compact");
    expect(document.documentElement.dataset.density).toBe("compact");
  });

  it("defaults to 'regular' when nothing is stored", () => {
    initDensity();
    expect(get(density)).toBe("regular");
  });

  it("defaults to 'regular' for an unrecognised stored value", () => {
    localStorage.setItem("anvil-density", "unknown-value");
    initDensity();
    expect(get(density)).toBe("regular");
  });
});

describe("toggleDensity", () => {
  it("compact → regular", () => {
    applyDensity("compact");
    toggleDensity();
    expect(get(density)).toBe("regular");
    expect(localStorage.getItem("anvil-density")).toBe("regular");
  });

  it("regular → compact", () => {
    applyDensity("regular");
    toggleDensity();
    expect(get(density)).toBe("compact");
    expect(localStorage.getItem("anvil-density")).toBe("compact");
  });
});
