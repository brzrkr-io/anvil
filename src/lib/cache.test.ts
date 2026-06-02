import { describe, it, expect, beforeEach } from "vitest";
import { readCache, writeCache } from "./cache.js";

describe("cache", () => {
  beforeEach(() => localStorage.clear());

  it("round-trips a value so a page can repaint last-known data instantly", () => {
    writeCache("pods", [{ name: "api" }]);
    expect(readCache<{ name: string }[]>("pods")).toEqual([{ name: "api" }]);
  });

  it("returns null for a key that was never written", () => {
    expect(readCache("missing")).toBeNull();
  });

  it("returns null (not throw) when the stored JSON is corrupt", () => {
    localStorage.setItem("anvil-cache:bad", "{not json");
    expect(readCache("bad")).toBeNull();
  });

  it("namespaces keys so two caches can't collide", () => {
    writeCache("a", 1);
    writeCache("b", 2);
    expect(readCache("a")).toBe(1);
    expect(readCache("b")).toBe(2);
  });
});
