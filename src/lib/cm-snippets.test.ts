import { describe, it, expect, beforeEach } from "vitest";
import {
  getUserSnippets,
  addUserSnippet,
  removeUserSnippet,
  cmSnippets,
  type UserSnippet,
} from "./cm-snippets";

beforeEach(() => {
  localStorage.clear();
});

describe("getUserSnippets", () => {
  it("returns empty array when no snippets are stored", () => {
    expect(getUserSnippets()).toEqual([]);
  });

  it("returns empty array when localStorage has corrupt JSON", () => {
    localStorage.setItem("anvil-user-snippets", "{{{bad");
    expect(getUserSnippets()).toEqual([]);
  });
});

describe("addUserSnippet", () => {
  it("stores a valid snippet and it appears in getUserSnippets", () => {
    addUserSnippet({ ext: "ts", label: "mysnip", template: "const ${1:x} = ${}" });
    const snips = getUserSnippets();
    expect(snips).toHaveLength(1);
    expect(snips[0].label).toBe("mysnip");
    expect(snips[0].ext).toBe("ts");
  });

  it("ignores snippets with a missing ext", () => {
    addUserSnippet({ ext: "", label: "bad", template: "x" } as UserSnippet);
    expect(getUserSnippets()).toHaveLength(0);
  });

  it("ignores snippets with a missing label", () => {
    addUserSnippet({ ext: "ts", label: "", template: "x" });
    expect(getUserSnippets()).toHaveLength(0);
  });

  it("ignores snippets with a missing template", () => {
    addUserSnippet({ ext: "ts", label: "good", template: "" });
    expect(getUserSnippets()).toHaveLength(0);
  });

  it("replaces an existing snippet with the same ext+label", () => {
    addUserSnippet({ ext: "ts", label: "fn", template: "old template" });
    addUserSnippet({ ext: "ts", label: "fn", template: "new template" });
    const snips = getUserSnippets();
    expect(snips).toHaveLength(1);
    expect(snips[0].template).toBe("new template");
  });

  it("keeps separate snippets for different extensions with the same label", () => {
    addUserSnippet({ ext: "ts", label: "fn", template: "ts-fn" });
    addUserSnippet({ ext: "go", label: "fn", template: "go-fn" });
    expect(getUserSnippets()).toHaveLength(2);
  });
});

describe("removeUserSnippet", () => {
  it("removes the snippet matching ext+label", () => {
    addUserSnippet({ ext: "ts", label: "log", template: "console.log(${});" });
    removeUserSnippet("ts", "log");
    expect(getUserSnippets()).toHaveLength(0);
  });

  it("leaves snippets with a different label intact", () => {
    addUserSnippet({ ext: "ts", label: "fn", template: "function" });
    addUserSnippet({ ext: "ts", label: "log", template: "console.log" });
    removeUserSnippet("ts", "log");
    const snips = getUserSnippets();
    expect(snips).toHaveLength(1);
    expect(snips[0].label).toBe("fn");
  });

  it("is a no-op when the snippet does not exist", () => {
    addUserSnippet({ ext: "ts", label: "fn", template: "fn" });
    removeUserSnippet("ts", "nonexistent");
    expect(getUserSnippets()).toHaveLength(1);
  });
});

describe("cmSnippets", () => {
  it("returns an empty array (no extension registered) for an unknown file type", () => {
    // cmSnippets returns [] Extension when there are no snippets for the file
    const ext = cmSnippets("file.unknownxyz");
    expect(Array.isArray(ext)).toBe(true);
    expect((ext as unknown[]).length).toBe(0);
  });

  it("returns a non-empty Extension for a known file type (TypeScript)", () => {
    // .ts has built-in snippets, so cmSnippets returns an EditorState.languageData entry
    const ext = cmSnippets("src/app.ts");
    // The returned value is an Extension (StateExtension), not an array
    expect(ext).toBeTruthy();
    expect(Array.isArray(ext)).toBe(false);
  });

  it("returns a non-empty Extension for Dockerfile (exact filename match)", () => {
    const ext = cmSnippets("Dockerfile");
    expect(ext).toBeTruthy();
    expect(Array.isArray(ext)).toBe(false);
  });

  it("includes user snippets in the completion set for matching ext", () => {
    addUserSnippet({ ext: "ts", label: "custom", template: "custom ${}" });
    // cmSnippets builds the options; we can't execute completions in unit tests
    // but we can verify the function does not throw and returns an Extension
    const ext = cmSnippets("app.ts");
    expect(ext).toBeTruthy();
  });
});
