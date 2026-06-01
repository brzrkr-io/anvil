import { describe, it, expect } from "vitest";
import { diffLines, applyHunks } from "./linediff.js";

describe("diffLines", () => {
  it("returns no hunks for identical text", () => {
    expect(diffLines("a\nb\nc", "a\nb\nc")).toEqual([]);
  });

  it("captures a single replaced line as one hunk", () => {
    const h = diffLines("a\nb\nc", "a\nB\nc");
    expect(h).toHaveLength(1);
    expect(h[0]).toMatchObject({ oldStart: 1, oldLines: ["b"], newLines: ["B"] });
  });

  it("captures a pure insertion (no old lines)", () => {
    const h = diffLines("a\nc", "a\nb\nc");
    expect(h).toHaveLength(1);
    expect(h[0].oldLines).toEqual([]);
    expect(h[0].newLines).toEqual(["b"]);
  });

  it("captures a pure deletion (no new lines)", () => {
    const h = diffLines("a\nb\nc", "a\nc");
    expect(h).toHaveLength(1);
    expect(h[0].oldLines).toEqual(["b"]);
    expect(h[0].newLines).toEqual([]);
  });

  it("finds two independent hunks", () => {
    const h = diffLines("a\nb\nc\nd\ne", "A\nb\nc\nD\ne");
    expect(h).toHaveLength(2);
    expect(h[0]).toMatchObject({ oldStart: 0, oldLines: ["a"], newLines: ["A"] });
    expect(h[1]).toMatchObject({ oldStart: 3, oldLines: ["d"], newLines: ["D"] });
  });
});

describe("applyHunks", () => {
  const oldText = "a\nb\nc\nd\ne";
  const newText = "A\nb\nc\nD\ne";
  const hunks = diffLines(oldText, newText);

  it("accepting all hunks reproduces the new text", () => {
    expect(applyHunks(oldText, hunks, [true, true])).toBe(newText);
  });

  it("rejecting all hunks reproduces the old text", () => {
    expect(applyHunks(oldText, hunks, [false, false])).toBe(oldText);
  });

  it("accepting only the first hunk mixes old and new", () => {
    expect(applyHunks(oldText, hunks, [true, false])).toBe("A\nb\nc\nd\ne");
  });

  it("accepting only the second hunk mixes the other way", () => {
    expect(applyHunks(oldText, hunks, [false, true])).toBe("a\nb\nc\nD\ne");
  });
});
