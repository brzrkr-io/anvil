import { describe, it, expect } from "vitest";
import { EXTENSIONS, isExtEnabled, railEnabled } from "./extensions";

describe("extensions model", () => {
  it("built-ins default enabled, others default disabled", () => {
    expect(isExtEnabled("kubernetes", {})).toBe(true);
    expect(isExtEnabled("grafana", {})).toBe(false);
  });
  it("explicit map overrides the default", () => {
    expect(isExtEnabled("kubernetes", { kubernetes: false })).toBe(false);
    expect(isExtEnabled("grafana", { grafana: true })).toBe(true);
  });
  it("a rail shows when ≥1 of its extensions is enabled", () => {
    expect(railEnabled("devops", {})).toBe(true); // k8s + actions default on
    expect(railEnabled("devops", { kubernetes: false, "github-actions": false })).toBe(false);
  });
  it("a rail with no gating extensions is always shown", () => {
    expect(railEnabled("term", {})).toBe(true);
  });
  it("every extension has a stable id + name", () => {
    for (const e of EXTENSIONS) { expect(e.id).toBeTruthy(); expect(e.name).toBeTruthy(); }
  });
});
