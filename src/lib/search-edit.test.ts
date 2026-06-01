import { describe, it, expect } from "vitest";
import { applyLineEdits, groupByFile, type PathLineEdit } from "./search-edit.js";

describe("applyLineEdits", () => {
  const content = "a\nb\nc\nd";

  it("replaces a single line by 1-based number", () => {
    expect(applyLineEdits(content, [{ line: 2, text: "B" }])).toBe("a\nB\nc\nd");
  });

  it("applies multiple edits in one pass", () => {
    expect(applyLineEdits(content, [{ line: 1, text: "A" }, { line: 4, text: "D" }])).toBe("A\nb\nc\nD");
  });

  it("ignores out-of-range line numbers", () => {
    expect(applyLineEdits(content, [{ line: 0, text: "x" }, { line: 99, text: "y" }])).toBe(content);
  });

  it("preserves the trailing newline structure", () => {
    expect(applyLineEdits("x\n", [{ line: 1, text: "y" }])).toBe("y\n");
  });
});

describe("groupByFile", () => {
  it("buckets edits by path, dropping the path from each entry", () => {
    const edits: PathLineEdit[] = [
      { path: "a.ts", line: 1, text: "x" },
      { path: "b.ts", line: 3, text: "y" },
      { path: "a.ts", line: 5, text: "z" },
    ];
    const g = groupByFile(edits);
    expect(g.get("a.ts")).toEqual([{ line: 1, text: "x" }, { line: 5, text: "z" }]);
    expect(g.get("b.ts")).toEqual([{ line: 3, text: "y" }]);
    expect([...g.keys()]).toEqual(["a.ts", "b.ts"]);
  });
});
