import { describe, it, expect, beforeEach } from "vitest";
import { feedInput, getHistory, clearHistory } from "./command-history";

beforeEach(() => clearHistory());

describe("feedInput / getHistory", () => {
  it("commits a line when Enter (\\r) is received", () => {
    feedInput("term1", "ls -la\r");
    expect(getHistory()).toContain("ls -la");
  });

  it("commits a line when \\n is received", () => {
    feedInput("term1", "pwd\n");
    expect(getHistory()).toContain("pwd");
  });

  it("does not commit empty lines", () => {
    feedInput("term1", "\r");
    expect(getHistory()).toHaveLength(0);
  });

  it("handles backspace (\\x7f) correctly before commit", () => {
    feedInput("term1", "gti\x7f\x7fit status\r");
    expect(getHistory()).toContain("git status");
  });

  it("handles \\b as backspace", () => {
    feedInput("term1", "ab\bc\r");
    expect(getHistory()).toContain("ac");
  });

  it("Ctrl-C (\\x03) discards the current line buffer", () => {
    feedInput("term1", "oops\x03");
    feedInput("term1", "\r");
    // After Ctrl-C the buffer is cleared; the Enter commits an empty line which is dropped
    expect(getHistory()).toHaveLength(0);
  });

  it("Ctrl-U (\\x15) discards the current line buffer", () => {
    feedInput("term1", "typo\x15correct\r");
    expect(getHistory()).toContain("correct");
    expect(getHistory()).not.toContain("typo");
  });

  it("deduplicates consecutive identical commands", () => {
    feedInput("term1", "ls\r");
    feedInput("term1", "ls\r");
    const h = getHistory();
    expect(h.filter((c) => c === "ls")).toHaveLength(1);
  });

  it("allows the same command after a different one", () => {
    feedInput("term1", "ls\r");
    feedInput("term1", "pwd\r");
    feedInput("term1", "ls\r");
    const h = getHistory();
    expect(h.filter((c) => c === "ls")).toHaveLength(2);
  });

  it("ignores control characters below space", () => {
    feedInput("term1", "\x01\x02\r");
    // nothing printable was typed
    expect(getHistory()).toHaveLength(0);
  });

  it("drops lines longer than 400 characters", () => {
    feedInput("term1", "a".repeat(401) + "\r");
    expect(getHistory()).toHaveLength(0);
  });

  it("tracks buffers per terminal id independently", () => {
    feedInput("termA", "echo a");
    feedInput("termB", "echo b");
    feedInput("termA", "\r");
    feedInput("termB", "\r");
    const h = getHistory();
    expect(h).toContain("echo a");
    expect(h).toContain("echo b");
  });
});

describe("clearHistory", () => {
  it("empties the history array", () => {
    feedInput("term1", "ls\r");
    clearHistory();
    expect(getHistory()).toHaveLength(0);
  });
});
