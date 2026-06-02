import { describe, it, expect, beforeEach } from "vitest";
import { bumpUsage, rankItems, withTracking } from "./palette-rank.js";

beforeEach(() => localStorage.clear());

describe("palette-rank", () => {
  const items = [{ label: "A" }, { label: "B" }, { label: "C" }];

  it("keeps original order when nothing is used (stable)", () => {
    expect(rankItems(items).map((i) => i.label)).toEqual(["A", "B", "C"]);
  });

  it("floats the most-used to the top", () => {
    bumpUsage("C");
    bumpUsage("C");
    bumpUsage("B");
    expect(rankItems(items).map((i) => i.label)).toEqual(["C", "B", "A"]);
  });

  it("breaks ties by original index", () => {
    bumpUsage("B");
    bumpUsage("C");
    expect(rankItems(items).map((i) => i.label)).toEqual(["B", "C", "A"]);
  });

  it("withTracking records a use when run() is called", () => {
    let ran = false;
    const wrapped = withTracking({ label: "X", run: () => { ran = true; } });
    wrapped.run();
    expect(ran).toBe(true);
    expect(rankItems([{ label: "Y" }, { label: "X" }]).map((i) => i.label)).toEqual(["X", "Y"]);
  });

  it("survives corrupt storage", () => {
    localStorage.setItem("anvil-cmd-usage", "{bad");
    expect(rankItems(items).map((i) => i.label)).toEqual(["A", "B", "C"]);
  });
});
