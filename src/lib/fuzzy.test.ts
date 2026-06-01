import { describe, it, expect } from "vitest";
import { score, rank } from "./fuzzy.js";

describe("score", () => {
  it("returns a number for a valid subsequence match", () => {
    const s = score("foobar", "fb");
    expect(typeof s).toBe("number");
    expect(s).not.toBeNull();
  });

  it("returns null for a non-subsequence query", () => {
    expect(score("foobar", "xyz")).toBeNull();
    expect(score("abc", "abcd")).toBeNull();
  });

  it("returns 0 for an empty query", () => {
    expect(score("anything", "")).toBe(0);
  });

  it("earlier matches score lower (better) than later matches", () => {
    // "ab" in "abXX" starts at position 0 vs "ab" in "XXab" starting at 2
    const early = score("abXX", "ab");
    const late = score("XXab", "ab");
    expect(early).not.toBeNull();
    expect(late).not.toBeNull();
    expect(early!).toBeLessThan(late!);
  });

  it("tighter matches score lower than spread-out matches", () => {
    // "ab" consecutive vs "a_b" with a gap
    const tight = score("ab", "ab");
    const spread = score("a_b", "ab");
    expect(tight).not.toBeNull();
    expect(spread).not.toBeNull();
    expect(tight!).toBeLessThan(spread!);
  });

  it("is case-insensitive", () => {
    expect(score("FOO", "foo")).not.toBeNull();
    expect(score("foo", "FOO")).not.toBeNull();
  });

  it("returns null for empty text with a non-empty query", () => {
    expect(score("", "a")).toBeNull();
  });
});

describe("rank", () => {
  const items = ["foobar", "baz", "fb", "far", "Foobar2"];

  it("filters out non-matching items", () => {
    const result = rank(items, "xyz", (s) => s);
    expect(result).toHaveLength(0);
  });

  it("returns items ordered by score (best match first)", () => {
    // "fb" is a tight subsequence match — should rank ahead of "foobar"
    const result = rank(items, "fb", (s) => s);
    expect(result[0]).toBe("fb");
  });

  it("returns all items on empty query (preserving original order up to limit)", () => {
    const result = rank(items, "", (s) => s);
    expect(result).toHaveLength(items.length);
    // Original order preserved for equal scores
    expect(result).toEqual(items);
  });

  it("respects the limit parameter", () => {
    const result = rank(items, "", (s) => s, 2);
    expect(result).toHaveLength(2);
  });

  it("uses the key function to extract the string to match against", () => {
    const objs = [{ name: "alpha" }, { name: "beta" }, { name: "gamma" }];
    const result = rank(objs, "b", (o) => o.name);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("beta");
  });

  it("returns items unchanged when query matches everything", () => {
    const simple = ["a", "b", "c"];
    // empty string matches all
    const result = rank(simple, "", (s) => s);
    expect(result).toEqual(simple);
  });
});
