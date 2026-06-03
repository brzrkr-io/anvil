import { describe, it, expect } from "vitest";
import { parseMrRows } from "./gl-mrs";

describe("parseMrRows (#46)", () => {
  const raw = JSON.stringify([
    { iid: 2, title: "Add cache", source_branch: "feat/cache", target_branch: "main", web_url: "https://gl/mr/2", draft: false },
    { iid: 1, title: "Draft: wip thing", source_branch: "wip", target_branch: "main", web_url: "https://gl/mr/1", work_in_progress: true },
  ]);

  it("maps fields and sorts by iid ascending", () => {
    const rows = parseMrRows(raw);
    expect(rows.map((r) => r.iid)).toEqual(["1", "2"]);
    expect(rows[1]).toMatchObject({ title: "Add cache", source: "feat/cache", target: "main", url: "https://gl/mr/2" });
  });

  it("flags drafts from draft, work_in_progress, or a Draft:/WIP: title", () => {
    const rows = parseMrRows(raw);
    expect(rows.find((r) => r.iid === "1")!.draft).toBe(true);
    expect(rows.find((r) => r.iid === "2")!.draft).toBe(false);
    expect(parseMrRows(JSON.stringify([{ iid: 3, title: "WIP: x" }]))[0].draft).toBe(true);
  });

  it("returns [] for non-JSON or a glab error string", () => {
    expect(parseMrRows("glab: not authenticated")).toEqual([]);
    expect(parseMrRows("{}")).toEqual([]);
  });
});
