import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { setFileProblems, problems } from "./diagnostics";

// The module-level byFile Map persists across tests. Clear all known paths before each test.
// We clear every path that any test in this file might set.
const TEST_PATHS = ["app.ts", "a.ts", "b.ts", "file-a.ts", "file-b.ts"];
beforeEach(() => {
  for (const p of TEST_PATHS) setFileProblems(p, []);
});

describe("setFileProblems", () => {
  it("adds problems for a file and they appear in the store", () => {
    setFileProblems("app.ts", [
      { path: "app.ts", line: 5, message: "Type error", severity: 1 },
    ]);
    const ps = get(problems);
    expect(ps).toHaveLength(1);
    expect(ps[0].message).toBe("Type error");
  });

  it("removing problems for a file (empty array) removes them from the store", () => {
    setFileProblems("app.ts", [
      { path: "app.ts", line: 5, message: "Type error", severity: 1 },
    ]);
    setFileProblems("app.ts", []);
    expect(get(problems)).toHaveLength(0);
  });

  it("problems from multiple files are combined and sorted by severity", () => {
    setFileProblems("a.ts", [
      { path: "a.ts", line: 1, message: "Warning", severity: 2 },
    ]);
    setFileProblems("b.ts", [
      { path: "b.ts", line: 1, message: "Error", severity: 1 },
    ]);
    const ps = get(problems);
    expect(ps).toHaveLength(2);
    // severity 1 (error) sorts before severity 2 (warning)
    expect(ps[0].message).toBe("Error");
    expect(ps[1].message).toBe("Warning");
  });

  it("updating problems for a file replaces the previous set", () => {
    setFileProblems("app.ts", [
      { path: "app.ts", line: 1, message: "Old error", severity: 1 },
    ]);
    setFileProblems("app.ts", [
      { path: "app.ts", line: 2, message: "New error", severity: 1 },
    ]);
    const ps = get(problems);
    expect(ps).toHaveLength(1);
    expect(ps[0].message).toBe("New error");
  });

  it("store starts as an empty array", () => {
    // After the beforeEach cleanup, the store should be empty
    expect(get(problems)).toHaveLength(0);
  });
});
