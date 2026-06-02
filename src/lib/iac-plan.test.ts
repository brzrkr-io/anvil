import { describe, it, expect } from "vitest";
import { parsePlanSummary, planBadge, lineClass } from "./iac-plan.js";

describe("parsePlanSummary", () => {
  it("parses a changes plan line", () => {
    const out = "...\nPlan: 3 to add, 1 to change, 2 to destroy.\n";
    expect(parsePlanSummary(out)).toEqual({ add: 3, change: 1, destroy: 2, none: false });
  });

  it("reports no changes when the plan is clean", () => {
    expect(parsePlanSummary("No changes. Your infrastructure matches the configuration.")).toEqual({
      add: 0, change: 0, destroy: 0, none: true,
    });
  });

  it("does not report no-changes for non-plan output", () => {
    expect(parsePlanSummary("No changes detected in validate", false)).toBeNull();
  });

  it("returns null when there's no plan result", () => {
    expect(parsePlanSummary("Initializing the backend...")).toBeNull();
  });
});

describe("planBadge", () => {
  it("is drift when a plan has changes", () => {
    expect(planBadge({ add: 1, change: 0, destroy: 0, none: false })).toBe("drift");
  });
  it("is clean for a no-changes plan", () => {
    expect(planBadge({ add: 0, change: 0, destroy: 0, none: true })).toBe("clean");
  });
  it("is empty with no result", () => {
    expect(planBadge(null)).toBe("");
    expect(planBadge(undefined)).toBe("");
  });
});

describe("lineClass", () => {
  it("classifies add / destroy / change / replace lines", () => {
    expect(lineClass("  + resource")).toBe("add");
    expect(lineClass("  - resource")).toBe("del");
    expect(lineClass("  ~ resource")).toBe("chg");
    expect(lineClass("-/+ resource")).toBe("rec");
  });
  it("classifies error and success lines", () => {
    expect(lineClass("Error: invalid")).toBe("err");
    expect(lineClass("Apply complete!")).toBe("ok");
  });
  it("returns empty for ordinary lines", () => {
    expect(lineClass("  some text")).toBe("");
  });
});
