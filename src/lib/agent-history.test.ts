import { describe, it, expect, beforeEach } from "vitest";
import { archiveRun, listRuns, loadRun, deleteRun, runTitle } from "./agent-history.js";

const chat = [{ role: "user", text: "Why is the atlas reconcile failing?" }, { role: "assistant", text: "Checking…" }];

describe("agent-history", () => {
  beforeEach(() => localStorage.clear());

  it("titles a run from the first user message", () => {
    expect(runTitle(chat)).toBe("Why is the atlas reconcile failing?");
    expect(runTitle([{ role: "assistant", text: "hi" }])).toBe("Agent session");
  });

  it("archives a conversation and reloads it by id", () => {
    const meta = archiveRun(chat, "r1", 1000)!;
    expect(meta.title).toContain("atlas reconcile");
    expect(listRuns().map((r) => r.id)).toEqual(["r1"]);
    expect(loadRun("r1")).toEqual(chat);
  });

  it("ignores an empty chat", () => {
    expect(archiveRun([], "r2", 1)).toBeNull();
    expect(listRuns()).toEqual([]);
  });

  it("newest run is first and deleting removes it", () => {
    archiveRun(chat, "a", 1);
    archiveRun(chat, "b", 2);
    expect(listRuns().map((r) => r.id)).toEqual(["b", "a"]);
    deleteRun("b");
    expect(listRuns().map((r) => r.id)).toEqual(["a"]);
    expect(loadRun("b")).toEqual([]);
  });

  it("tolerates corrupt index", () => {
    localStorage.setItem("anvil-agent-runs", "{bad");
    expect(listRuns()).toEqual([]);
  });
});
