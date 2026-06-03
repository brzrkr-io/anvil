import { describe, it, expect } from "vitest";
import { edgeFromRect, passedThreshold, dropAction, DRAG_THRESHOLD, type TabDrag } from "./tabdrag";

const RECT = { left: 0, top: 0, width: 100, height: 100 };

describe("edgeFromRect — which side a pointer maps to", () => {
  it("near the left edge → left (drop splits horizontally)", () => {
    expect(edgeFromRect(RECT, 5, 50)).toBe("left");
  });
  it("near the right edge → right", () => {
    expect(edgeFromRect(RECT, 95, 50)).toBe("right");
  });
  it("near the top edge → top", () => {
    expect(edgeFromRect(RECT, 50, 5)).toBe("top");
  });
  it("near the bottom edge → bottom", () => {
    expect(edgeFromRect(RECT, 50, 95)).toBe("bottom");
  });
  it("the middle → center (drop adds a tab, no split)", () => {
    expect(edgeFromRect(RECT, 50, 50)).toBe("center");
  });
  it("left/right win over top/bottom in a corner", () => {
    // top-left corner is within both the left and top margins; horizontal wins.
    expect(edgeFromRect(RECT, 2, 2)).toBe("left");
  });
  it("is offset-aware (rect not at origin)", () => {
    expect(edgeFromRect({ left: 200, top: 100, width: 100, height: 100 }, 205, 150)).toBe("left");
    expect(edgeFromRect({ left: 200, top: 100, width: 100, height: 100 }, 250, 150)).toBe("center");
  });
});

describe("passedThreshold — a click must not become a drag", () => {
  it("a tiny jitter stays a click", () => {
    expect(passedThreshold(10, 10, 10 + DRAG_THRESHOLD, 10)).toBe(false);
  });
  it("moving past the threshold on either axis starts the drag", () => {
    expect(passedThreshold(10, 10, 10 + DRAG_THRESHOLD + 1, 10)).toBe(true);
    expect(passedThreshold(10, 10, 10, 10 - DRAG_THRESHOLD - 1)).toBe(true);
  });
});

describe("dropAction — release resolves to a tree op by source + edge", () => {
  const topFile: TabDrag = { view: "editor", ref: "/a.ts", label: "a.ts" };
  const paneTab: TabDrag = { view: "term", ref: "wt1", label: "Terminal", from: { leafId: "L1", index: 2 } };

  it("no target hint → no-op (caller treats as a click)", () => {
    expect(dropAction(topFile, null)).toEqual({ kind: "none" });
  });
  it("top-strip tab, center → add as a tab on the target pane", () => {
    expect(dropAction(topFile, { leafId: "L2", edge: "center" })).toEqual({ kind: "addTab", leafId: "L2" });
  });
  it("top-strip tab, edge → split the target pane", () => {
    expect(dropAction(topFile, { leafId: "L2", edge: "right" })).toEqual({ kind: "split", leafId: "L2", edge: "right" });
  });
  it("pane tab, center on a DIFFERENT pane → move the tab", () => {
    expect(dropAction(paneTab, { leafId: "L2", edge: "center" })).toEqual({ kind: "moveTab", from: { leafId: "L1", index: 2 }, to: "L2" });
  });
  it("pane tab, edge on a DIFFERENT pane → split target with the tab", () => {
    expect(dropAction(paneTab, { leafId: "L2", edge: "bottom" })).toEqual({ kind: "splitFrom", from: { leafId: "L1", index: 2 }, leafId: "L2", edge: "bottom" });
  });
  it("pane tab dropped back on its own pane → no-op", () => {
    expect(dropAction(paneTab, { leafId: "L1", edge: "center" })).toEqual({ kind: "none" });
    expect(dropAction(paneTab, { leafId: "L1", edge: "left" })).toEqual({ kind: "none" });
  });
});
