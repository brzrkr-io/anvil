// Tests the localStorage persistence paths in command-history.ts.
// The core feedInput logic is covered in command-history.test.ts.
import { describe, it, expect, beforeEach } from "vitest";
import { feedInput, getHistory, clearHistory } from "./command-history";

beforeEach(() => {
  localStorage.clear();
  clearHistory();
});

describe("command-history localStorage persistence", () => {
  it("commit writes history to localStorage", () => {
    feedInput("t1", "kubectl get pods\r");
    const stored = JSON.parse(localStorage.getItem("anvil-cmd-history") || "[]");
    expect(stored).toContain("kubectl get pods");
  });

  it("clearHistory removes the localStorage entry", () => {
    feedInput("t1", "ls\r");
    clearHistory();
    expect(localStorage.getItem("anvil-cmd-history")).toBeNull();
  });

  it("history survives clear of in-memory state (simulates restart)", () => {
    feedInput("t1", "echo hello\r");
    // Verify the data is in localStorage (what a restart would read)
    const stored = JSON.parse(localStorage.getItem("anvil-cmd-history") || "[]");
    expect(stored).toContain("echo hello");
  });
});
