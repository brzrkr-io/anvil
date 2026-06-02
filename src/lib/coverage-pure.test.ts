import { describe, it, expect, beforeEach } from "vitest";
import { findLeaf, firstLeaf, leafCount, remapTermRefs } from "./panes.js";
import { containerState, parseContainers } from "./docker-ps.js";
import { runState, runRank, parseRuns, failingRuns } from "./actions-runs.js";
import { getSnippets, addSnippet, removeSnippet } from "./snippets.js";
import { isExtEnabled, toggleExt, railEnabled, extEnabled } from "./extensions.js";
import { get } from "svelte/store";

/* eslint-disable @typescript-eslint/no-explicit-any */
const A: any = { kind: "leaf", id: "a", view: "term", ref: "t1" };
const B: any = { kind: "leaf", id: "b", view: "editor" };
const SPLIT: any = { kind: "split", dir: "row", children: [A, B], sizes: [0.5, 0.5] };

describe("panes tree helpers", () => {
  it("finds leaves depth-first and returns null when absent", () => {
    expect(findLeaf(SPLIT, "b")).toBe(B);
    expect(findLeaf(SPLIT, "zzz")).toBeNull();
  });
  it("firstLeaf descends to the first leaf and leafCount counts them", () => {
    expect(firstLeaf(SPLIT)).toBe(A);
    expect(leafCount(SPLIT)).toBe(2);
    expect(leafCount(A)).toBe(1);
  });
  it("remapTermRefs migrates a tab-less leaf and re-issues terminal refs", () => {
    const out: any = remapTermRefs(A);
    expect(out.tabs).toHaveLength(1);
    expect(out.tabs[0].ref).not.toBe("t1"); // fresh pty id
  });
});

describe("docker-ps containerState", () => {
  it("uses the State field when present", () => {
    expect(containerState({ State: "running" })).toBe("running");
    expect(containerState({ State: "exited" })).toBe("exited");
    expect(containerState({ State: "paused" })).toBe("paused");
  });
  it("falls back to parsing the Status string for older CLIs", () => {
    expect(containerState({ Status: "Up 3 minutes" })).toBe("running");
    expect(containerState({ Status: "Up (Paused)" })).toBe("paused");
    expect(containerState({ Status: "Exited (0) 2h ago" })).toBe("exited");
    expect(containerState({})).toBe("other");
  });
  it("parses newline-delimited json rows", () => {
    const out = parseContainers(JSON.stringify({ ID: "abc", Names: "api", Image: "img", State: "running", Status: "Up", Ports: "" }));
    expect(out[0].state).toBe("running");
  });
});

describe("actions-runs", () => {
  it("maps status/conclusion to a run state", () => {
    expect(runState({ status: "in_progress" })).toBe("running");
    expect(runState({ status: "completed", conclusion: "failure" })).toBe("fail");
    expect(runState({ status: "completed", conclusion: "success" })).toBe("pass");
    expect(runState({ status: "completed", conclusion: "skipped" })).toBe("neutral");
    expect(runRank("fail")).toBeLessThan(runRank("pass"));
  });
  it("parses runs failing-first and ignores non-arrays", () => {
    expect(parseRuns("nope")).toEqual([]);
    expect(parseRuns('{"a":1}')).toEqual([]);
    const rows = parseRuns(JSON.stringify([
      { databaseId: 1, status: "completed", conclusion: "success" },
      { databaseId: 2, status: "completed", conclusion: "failure" },
    ]));
    expect(rows[0].id).toBe("2");
    expect(failingRuns(rows)).toBe(1);
  });
});

describe("snippets", () => {
  beforeEach(() => localStorage.clear());
  it("returns the built-in defaults when nothing is stored or storage is corrupt", () => {
    expect(getSnippets().length).toBeGreaterThan(0);
    localStorage.setItem("anvil-snippets", "{bad");
    expect(getSnippets().length).toBeGreaterThan(0);
  });
  it("assigns the first id when no snippets exist yet", () => {
    localStorage.setItem("anvil-snippets", "[]");
    addSnippet("first", "echo");
    expect(getSnippets().some((s) => s.label === "first")).toBe(true);
  });
  it("adds and removes a custom snippet", () => {
    const before = getSnippets().length;
    addSnippet("mine", "echo hi");
    expect(getSnippets().length).toBe(before + 1);
    const added = getSnippets().find((s) => s.label === "mine")!;
    removeSnippet(added.id);
    expect(getSnippets().find((s) => s.label === "mine")).toBeUndefined();
  });
});

describe("extensions enable logic", () => {
  it("applies the built-in default when a value is unset, else the explicit value", () => {
    expect(isExtEnabled("caldera", {})).toBe(true);   // builtin
    expect(isExtEnabled("grafana", {})).toBe(false);  // not builtin
    expect(isExtEnabled("grafana", { grafana: true })).toBe(true);
  });
  it("toggleExt flips and railEnabled reflects it", () => {
    const start = isExtEnabled("grafana", get(extEnabled));
    toggleExt("grafana");
    expect(isExtEnabled("grafana", get(extEnabled))).toBe(!start);
    expect(typeof railEnabled("grafana", get(extEnabled))).toBe("boolean");
  });
});
