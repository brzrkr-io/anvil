import { describe, it, expect } from "vitest";
import { healthRank, byHealth, failingCount, oneLine, shortRev, type FluxLike } from "./flux-health.js";

function mk(p: Partial<FluxLike>): FluxLike {
  return { name: "x", ns: "default", ready: "ok", suspended: false, message: "", revision: "", ...p };
}

describe("healthRank", () => {
  it("ranks failing first, then suspended, then unknown, then ok", () => {
    expect(healthRank({ ready: "fail", suspended: false })).toBe(0);
    expect(healthRank({ ready: "ok", suspended: true })).toBe(1);
    expect(healthRank({ ready: "unknown", suspended: false })).toBe(2);
    expect(healthRank({ ready: "ok", suspended: false })).toBe(3);
  });

  it("treats a failing item as failing even if also suspended", () => {
    expect(healthRank({ ready: "fail", suspended: true })).toBe(0);
  });
});

describe("byHealth", () => {
  it("floats broken items to the top, alpha within a rank", () => {
    const items = [
      mk({ name: "z-ok", ready: "ok" }),
      mk({ name: "a-ok", ready: "ok" }),
      mk({ name: "broken", ready: "fail" }),
      mk({ name: "paused", suspended: true }),
    ];
    const order = [...items].sort(byHealth).map((i) => i.name);
    expect(order).toEqual(["broken", "paused", "a-ok", "z-ok"]);
  });

  it("orders by namespace+name within the same rank", () => {
    const items = [
      mk({ ns: "prod", name: "b", ready: "fail" }),
      mk({ ns: "prod", name: "a", ready: "fail" }),
      mk({ ns: "dev", name: "z", ready: "fail" }),
    ];
    const order = [...items].sort(byHealth).map((i) => i.ns + "/" + i.name);
    expect(order).toEqual(["dev/z", "prod/a", "prod/b"]);
  });
});

describe("failingCount", () => {
  it("counts only outright failures, not suspended", () => {
    const items = [
      mk({ ready: "fail" }),
      mk({ ready: "fail" }),
      mk({ ready: "ok", suspended: true }),
      mk({ ready: "ok" }),
      mk({ ready: "unknown" }),
    ];
    expect(failingCount(items)).toBe(2);
  });

  it("is zero for an all-healthy list", () => {
    expect(failingCount([mk({}), mk({})])).toBe(0);
  });
});

describe("oneLine", () => {
  it("collapses whitespace and newlines to a single line", () => {
    expect(oneLine("install retries\n  exhausted:  timed out")).toBe("install retries exhausted: timed out");
  });
});

describe("shortRev", () => {
  it("keeps a branch ref prefix plus 7 hex chars", () => {
    expect(shortRev("main@sha1:abcd1234ef5678")).toBe("main@sha1:abcd123");
  });
  it("shortens a bare digest", () => {
    expect(shortRev("sha256:deadbeefcafe1234")).toBe("sha256:deadbee");
  });
  it("truncates a non-hex revision", () => {
    expect(shortRev("v1.2.3-some-very-long-tag-name-here")).toBe("v1.2.3-some-very-long-ta…");
  });
});
