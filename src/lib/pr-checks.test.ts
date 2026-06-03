import { describe, it, expect } from "vitest";
import { checkState, rollupChecks, prRank, parsePrRows, orderStacks, type PrRow } from "./pr-checks.js";

describe("checkState", () => {
  it("maps CheckRun conclusions", () => {
    expect(checkState({ status: "COMPLETED", conclusion: "SUCCESS" })).toBe("pass");
    expect(checkState({ status: "COMPLETED", conclusion: "FAILURE" })).toBe("fail");
    expect(checkState({ status: "COMPLETED", conclusion: "TIMED_OUT" })).toBe("fail");
    expect(checkState({ status: "COMPLETED", conclusion: "SKIPPED" })).toBe("none");
  });
  it("maps in-flight CheckRuns to pending", () => {
    expect(checkState({ status: "IN_PROGRESS" })).toBe("pending");
    expect(checkState({ status: "QUEUED" })).toBe("pending");
  });
  it("maps legacy StatusContext state", () => {
    expect(checkState({ state: "SUCCESS" })).toBe("pass");
    expect(checkState({ state: "FAILURE" })).toBe("fail");
    expect(checkState({ state: "ERROR" })).toBe("fail");
    expect(checkState({ state: "PENDING" })).toBe("pending");
  });
});

describe("rollupChecks", () => {
  it("is none with no checks", () => {
    expect(rollupChecks([])).toBe("none");
    expect(rollupChecks(null)).toBe("none");
  });
  it("fails if any check fails (worst-wins)", () => {
    expect(rollupChecks([
      { status: "COMPLETED", conclusion: "SUCCESS" },
      { status: "IN_PROGRESS" },
      { status: "COMPLETED", conclusion: "FAILURE" },
    ])).toBe("fail");
  });
  it("is pending when a check is still running and none failed", () => {
    expect(rollupChecks([
      { status: "COMPLETED", conclusion: "SUCCESS" },
      { status: "IN_PROGRESS" },
    ])).toBe("pending");
  });
  it("passes when all complete successfully", () => {
    expect(rollupChecks([
      { status: "COMPLETED", conclusion: "SUCCESS" },
      { state: "SUCCESS" },
    ])).toBe("pass");
  });
  it("is none when only skipped/neutral checks exist", () => {
    expect(rollupChecks([{ status: "COMPLETED", conclusion: "SKIPPED" }])).toBe("none");
  });
});

describe("prRank", () => {
  it("orders fail < pending < pass < none", () => {
    expect(prRank("fail")).toBeLessThan(prRank("pending"));
    expect(prRank("pending")).toBeLessThan(prRank("pass"));
    expect(prRank("pass")).toBeLessThan(prRank("none"));
  });
});

describe("parsePrRows", () => {
  const raw = JSON.stringify([
    { number: 1, title: "green", headRefName: "a", isDraft: false, statusCheckRollup: [{ status: "COMPLETED", conclusion: "SUCCESS" }] },
    { number: 2, title: "broken", headRefName: "b", isDraft: false, statusCheckRollup: [{ status: "COMPLETED", conclusion: "FAILURE" }] },
    { number: 3, title: "running", headRefName: "c", isDraft: true, statusCheckRollup: [{ status: "IN_PROGRESS" }] },
  ]);

  it("sorts failing first, then pending, then passing", () => {
    expect(parsePrRows(raw).map((r) => r.num)).toEqual(["2", "3", "1"]);
  });
  it("carries title, branch, draft and rolled-up check state", () => {
    const fail = parsePrRows(raw).find((r) => r.num === "2")!;
    expect(fail).toMatchObject({ title: "broken", branch: "b", draft: false, checks: "fail" });
    expect(parsePrRows(raw).find((r) => r.num === "3")!.draft).toBe(true);
  });
  it("returns [] for a gh error string (not JSON)", () => {
    expect(parsePrRows("gh: not authenticated")).toEqual([]);
  });
  it("returns [] for an empty PR list", () => {
    expect(parsePrRows("[]")).toEqual([]);
  });
});

describe("orderStacks (#27)", () => {
  const pr = (num: string, branch: string, base: string): PrRow => ({
    num, title: num, branch, base, draft: false, checks: "none",
  });

  it("nests a child PR under its parent with increasing depth", () => {
    // main ← feat-a ← feat-b (each PR's base is the branch below it)
    const rows = [pr("1", "feat-a", "main"), pr("2", "feat-b", "feat-a")];
    const out = orderStacks(rows);
    expect(out.map((r) => [r.num, r.depth])).toEqual([["1", 0], ["2", 1]]);
  });

  it("treats PRs targeting the default branch as roots at depth 0", () => {
    const rows = [pr("1", "feat-a", "main"), pr("2", "feat-x", "main")];
    expect(orderStacks(rows).every((r) => r.depth === 0)).toBe(true);
  });

  it("orders a 3-deep stack parent→child→grandchild", () => {
    const rows = [
      pr("3", "c", "b"),
      pr("1", "a", "main"),
      pr("2", "b", "a"),
    ];
    expect(orderStacks(rows).map((r) => `${r.num}@${r.depth}`)).toEqual(["1@0", "2@1", "3@2"]);
  });

  it("never drops a PR even if its base forms a cycle", () => {
    const rows = [pr("1", "a", "b"), pr("2", "b", "a")];
    expect(orderStacks(rows).map((r) => r.num).sort()).toEqual(["1", "2"]);
  });
});
