import { describe, it, expect, beforeEach } from "vitest";
import { getSnippets, addSnippet, removeSnippet } from "./snippets.js";

beforeEach(() => localStorage.clear());

describe("snippets", () => {
  it("returns defaults when nothing is stored", () => {
    const s = getSnippets();
    expect(s.length).toBeGreaterThan(0);
    expect(s.some((x) => x.command.startsWith("kubectl"))).toBe(true);
  });

  it("adds a snippet with a fresh id and persists it", () => {
    addSnippet("test", "echo hi");
    const s = getSnippets();
    const added = s.find((x) => x.label === "test")!;
    expect(added.command).toBe("echo hi");
    expect(added.id).toBeGreaterThan(0);
  });

  it("ignores blank label or command", () => {
    const before = getSnippets().length;
    addSnippet("", "echo");
    addSnippet("x", "   ");
    expect(getSnippets().length).toBe(before);
  });

  it("removes by id", () => {
    addSnippet("temp", "ls");
    const id = getSnippets().find((x) => x.label === "temp")!.id;
    removeSnippet(id);
    expect(getSnippets().some((x) => x.id === id)).toBe(false);
  });

  it("falls back to defaults on corrupt storage", () => {
    localStorage.setItem("anvil-snippets", "{{bad");
    expect(getSnippets().length).toBeGreaterThan(0);
  });
});
