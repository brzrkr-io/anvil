import { describe, it, expect } from "vitest";
import { runState, runRank, parseRuns, failingRuns } from "./actions-runs.js";

describe("runState", () => {
  it("maps completed conclusions", () => {
    expect(runState({ status: "completed", conclusion: "success" })).toBe("pass");
    expect(runState({ status: "completed", conclusion: "failure" })).toBe("fail");
    expect(runState({ status: "completed", conclusion: "timed_out" })).toBe("fail");
    expect(runState({ status: "completed", conclusion: "skipped" })).toBe("neutral");
  });
  it("maps in-flight runs to running", () => {
    expect(runState({ status: "in_progress" })).toBe("running");
    expect(runState({ status: "queued" })).toBe("running");
  });
});

describe("runRank", () => {
  it("orders fail < running < pass < neutral", () => {
    expect(runRank("fail")).toBeLessThan(runRank("running"));
    expect(runRank("running")).toBeLessThan(runRank("pass"));
    expect(runRank("pass")).toBeLessThan(runRank("neutral"));
  });
});

describe("parseRuns", () => {
  const raw = JSON.stringify([
    { databaseId: 1, status: "completed", conclusion: "success", displayTitle: "ok", workflowName: "CI", headBranch: "main", event: "push" },
    { databaseId: 2, status: "completed", conclusion: "failure", displayTitle: "broke", workflowName: "CI", headBranch: "feat", event: "pull_request" },
    { databaseId: 3, status: "in_progress", displayTitle: "running", workflowName: "Deploy", headBranch: "main", event: "push" },
  ]);

  it("sorts failing first, then running, then passing", () => {
    expect(parseRuns(raw).map((r) => r.id)).toEqual(["2", "3", "1"]);
  });
  it("carries workflow, title, branch and state", () => {
    const fail = parseRuns(raw).find((r) => r.id === "2")!;
    expect(fail).toMatchObject({ workflow: "CI", title: "broke", branch: "feat", state: "fail" });
  });
  it("drops rows without an id and survives non-JSON", () => {
    expect(parseRuns('[{"status":"completed","conclusion":"success"}]')).toEqual([]);
    expect(parseRuns("gh: not authenticated")).toEqual([]);
    expect(parseRuns("[]")).toEqual([]);
  });
});

describe("failingRuns", () => {
  it("counts only failed runs", () => {
    expect(failingRuns([{ state: "fail" }, { state: "fail" }, { state: "running" }, { state: "pass" }])).toBe(2);
  });
});
