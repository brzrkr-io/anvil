import { describe, it, expect, beforeEach, vi } from "vitest";
import { recordPrompt, recordExit, getBlocks, clearBlocks, lastExit } from "./command-blocks";

beforeEach(() => {
  clearBlocks("term1");
  clearBlocks("term2");
  lastExit.set(null);
});

describe("recordPrompt / recordExit / getBlocks", () => {
  it("associates a prompt line with its exit code", () => {
    recordPrompt("term1", 5);
    recordExit("term1", 0);
    const blocks = getBlocks("term1");
    expect(blocks).toHaveLength(1);
    expect(blocks[0].promptLine).toBe(5);
    expect(blocks[0].exit).toBe(0);
  });

  it("records non-zero exit codes (command failure)", () => {
    recordPrompt("term1", 10);
    recordExit("term1", 1);
    expect(getBlocks("term1")[0].exit).toBe(1);
  });

  it("falls back to promptLine 0 when no prior recordPrompt", () => {
    recordExit("term1", 0);
    expect(getBlocks("term1")[0].promptLine).toBe(0);
  });

  it("accumulates multiple blocks per terminal", () => {
    recordPrompt("term1", 1);
    recordExit("term1", 0);
    recordPrompt("term1", 4);
    recordExit("term1", 2);
    expect(getBlocks("term1")).toHaveLength(2);
  });

  it("tracks terminals independently", () => {
    recordPrompt("term1", 1);
    recordExit("term1", 0);
    recordPrompt("term2", 7);
    recordExit("term2", 1);
    expect(getBlocks("term1")).toHaveLength(1);
    expect(getBlocks("term2")).toHaveLength(1);
    expect(getBlocks("term2")[0].promptLine).toBe(7);
  });

  it("updates lastExit store on each exit", () => {
    let current: number | null = null;
    const unsub = lastExit.subscribe((v) => { current = v; });
    recordPrompt("term1", 0);
    recordExit("term1", 42);
    expect(current).toBe(42);
    unsub();
  });
});

describe("clearBlocks", () => {
  it("removes all blocks and pending prompt for the given terminal", () => {
    recordPrompt("term1", 3);
    recordExit("term1", 0);
    clearBlocks("term1");
    expect(getBlocks("term1")).toHaveLength(0);
  });

  it("does not affect other terminals", () => {
    recordPrompt("term2", 1);
    recordExit("term2", 0);
    clearBlocks("term1");
    expect(getBlocks("term2")).toHaveLength(1);
  });

  it("getBlocks returns empty array for an unknown terminal id", () => {
    expect(getBlocks("never-seen")).toHaveLength(0);
  });
});
