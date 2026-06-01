import { describe, it, expect } from "vitest";
import { get } from "svelte/store";
import { online } from "./offline";

// happy-dom initialises navigator.onLine=true by default.
// The module registers window online/offline event listeners at import time.

describe("online store — initial value", () => {
  it("reflects navigator.onLine at module load (true in happy-dom)", () => {
    // happy-dom sets navigator.onLine = true, so the store initialises true.
    expect(get(online)).toBe(true);
  });
});

describe("online store — tracks window events", () => {
  it("goes false when the 'offline' event fires", () => {
    window.dispatchEvent(new Event("offline"));
    expect(get(online)).toBe(false);
  });

  it("goes true when the 'online' event fires after going offline", () => {
    window.dispatchEvent(new Event("offline"));
    window.dispatchEvent(new Event("online"));
    expect(get(online)).toBe(true);
  });
});
