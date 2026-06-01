import { describe, it, expect } from "vitest";
import { parseConflicts, resolvedLines, resolveAll } from "./merge.js";

const plain = [
  "top",
  "<<<<<<< HEAD",
  "ours-1",
  "ours-2",
  "=======",
  "theirs-1",
  ">>>>>>> feature",
  "bottom",
];

const diff3 = [
  "<<<<<<< HEAD",
  "ours",
  "||||||| merged common ancestors",
  "base",
  "=======",
  "theirs",
  ">>>>>>> branch",
];

describe("parseConflicts", () => {
  it("parses a 2-way conflict with no base section", () => {
    const [c] = parseConflicts(plain);
    expect(c.start).toBe(1);
    expect(c.end).toBe(6);
    expect(c.ours).toEqual(["ours-1", "ours-2"]);
    expect(c.theirs).toEqual(["theirs-1"]);
    expect(c.base).toEqual([]);
    expect(c.oursLabel).toBe("HEAD");
    expect(c.theirsLabel).toBe("feature");
  });

  it("parses a diff3 conflict, keeping base separate from ours", () => {
    const [c] = parseConflicts(diff3);
    expect(c.ours).toEqual(["ours"]);
    expect(c.base).toEqual(["base"]);
    expect(c.theirs).toEqual(["theirs"]);
  });

  it("finds multiple conflicts in one file", () => {
    const lines = [...plain, ...plain];
    expect(parseConflicts(lines)).toHaveLength(2);
  });

  it("ignores an unterminated marker", () => {
    expect(parseConflicts(["<<<<<<< HEAD", "ours", "no end"])).toHaveLength(0);
  });

  it("returns nothing for a clean file", () => {
    expect(parseConflicts(["a", "b", "c"])).toHaveLength(0);
  });
});

describe("resolvedLines", () => {
  const [c] = parseConflicts(diff3);
  it("ours / theirs / base each return their own section", () => {
    expect(resolvedLines(c, "ours")).toEqual(["ours"]);
    expect(resolvedLines(c, "theirs")).toEqual(["theirs"]);
    expect(resolvedLines(c, "base")).toEqual(["base"]);
  });
  it("both concatenates ours then theirs", () => {
    expect(resolvedLines(c, "both")).toEqual(["ours", "theirs"]);
  });
});

describe("resolveAll", () => {
  it("replaces the conflict span and preserves surrounding lines", () => {
    expect(resolveAll(plain, "ours")).toEqual(["top", "ours-1", "ours-2", "bottom"]);
    expect(resolveAll(plain, "theirs")).toEqual(["top", "theirs-1", "bottom"]);
  });

  it("is a no-op on a clean file", () => {
    expect(resolveAll(["a", "b"], "ours")).toEqual(["a", "b"]);
  });
});
