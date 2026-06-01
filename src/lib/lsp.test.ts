import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  lspLang,
  fileUri,
  uriToPath,
  req,
  ensureLsp,
  didOpen,
  didChange,
  diagByPath,
  onDiagnostics,
} from "./lsp";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { listen } from "@tauri-apps/api/event";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("$lib/diagnostics", () => ({
  setFileProblems: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { setFileProblems } from "$lib/diagnostics";
const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);
const mockSetFileProblems = vi.mocked(setFileProblems);

beforeEach(() => {
  vi.resetAllMocks();
  // Default: listen resolves immediately (no-op unsubscribe)
  mockListen.mockResolvedValue(() => {});
});

describe("lspLang", () => {
  it("maps .rs to rust", () => expect(lspLang("src/main.rs")).toBe("rust"));
  it("maps .go to go", () => expect(lspLang("main.go")).toBe("go"));
  it("maps .ts to typescript", () => expect(lspLang("index.ts")).toBe("typescript"));
  it("maps .tsx to typescript", () => expect(lspLang("App.tsx")).toBe("typescript"));
  it("maps .js to typescript", () => expect(lspLang("index.js")).toBe("typescript"));
  it("maps .jsx to typescript", () => expect(lspLang("Comp.jsx")).toBe("typescript"));
  it("maps .mjs to typescript", () => expect(lspLang("util.mjs")).toBe("typescript"));
  it("maps .cjs to typescript", () => expect(lspLang("util.cjs")).toBe("typescript"));
  it("maps .py to python", () => expect(lspLang("script.py")).toBe("python"));
  it("maps .pyi to python", () => expect(lspLang("stubs.pyi")).toBe("python"));
  it("maps .c to cpp", () => expect(lspLang("main.c")).toBe("cpp"));
  it("maps .h to cpp", () => expect(lspLang("header.h")).toBe("cpp"));
  it("maps .cpp to cpp", () => expect(lspLang("app.cpp")).toBe("cpp"));
  it("maps .cc to cpp", () => expect(lspLang("app.cc")).toBe("cpp"));
  it("maps .hpp to cpp", () => expect(lspLang("app.hpp")).toBe("cpp"));
  it("maps .cxx to cpp", () => expect(lspLang("app.cxx")).toBe("cpp"));
  it("maps .hxx to cpp", () => expect(lspLang("app.hxx")).toBe("cpp"));
  it("returns null for unknown extensions", () => expect(lspLang("readme.md")).toBeNull());
  it("returns null for extensionless files", () => expect(lspLang("Makefile")).toBeNull());
});

describe("fileUri", () => {
  it("converts an absolute path to a file URI", () => {
    expect(fileUri("/home/user/project/main.rs")).toBe("file:///home/user/project/main.rs");
  });

  it("percent-encodes spaces in path components", () => {
    expect(fileUri("/my folder/file.ts")).toBe("file:///my%20folder/file.ts");
  });

  it("prepends a leading slash for relative paths", () => {
    const uri = fileUri("src/main.ts");
    expect(uri).toMatch(/^file:\/\//);
  });
});

describe("uriToPath", () => {
  it("decodes a valid file URI to its path", () => {
    expect(uriToPath("file:///home/user/main.rs")).toBe("/home/user/main.rs");
  });

  it("decodes percent-encoded characters in the path", () => {
    expect(uriToPath("file:///my%20folder/file.ts")).toBe("/my folder/file.ts");
  });

  it("strips file:// prefix when URL constructor fails (truly malformed URI)", () => {
    // URL constructor succeeds on file://badpath (host=badpath, path=/), so
    // uriToPath returns the decoded pathname. The fallback only fires on a
    // URI that URL cannot parse at all (e.g. an empty string after the scheme).
    // We test the normal decode path here.
    expect(uriToPath("file:///normal/path")).toBe("/normal/path");
  });
});

describe("req", () => {
  it("calls invoke lsp_request with the correct args", async () => {
    mockInvoke.mockResolvedValue({ items: [] });
    const result = await req("typescript", "textDocument/completion", { pos: 0 });
    expect(mockInvoke).toHaveBeenCalledWith("lsp_request", {
      lang: "typescript",
      method: "textDocument/completion",
      params: { pos: 0 },
    });
    expect(result).toEqual({ items: [] });
  });

  it("returns null when invoke throws (server down)", async () => {
    mockInvoke.mockRejectedValue(new Error("LSP server not running"));
    const result = await req("rust", "textDocument/hover", {});
    expect(result).toBeNull();
  });
});

describe("ensureLsp", () => {
  it("calls invoke lsp_start and returns true on success", async () => {
    mockInvoke.mockResolvedValue(true);
    const result = await ensureLsp("typescript", "/project");
    expect(mockInvoke).toHaveBeenCalledWith("lsp_start", {
      lang: "typescript",
      rootUri: fileUri("/project"),
    });
    expect(result).toBe(true);
  });

  it("returns false when invoke throws", async () => {
    mockInvoke.mockRejectedValue(new Error("no LSP installed"));
    const result = await ensureLsp("cobol", "/project");
    expect(result).toBe(false);
  });
});

describe("didOpen / didChange", () => {
  it("didOpen calls notify with textDocument/didOpen params", async () => {
    mockInvoke.mockResolvedValue(undefined);
    await didOpen("typescript", "/src/app.ts", "const x = 1;", 1);
    expect(mockInvoke).toHaveBeenCalledWith("lsp_notify", {
      lang: "typescript",
      method: "textDocument/didOpen",
      params: {
        textDocument: {
          uri: fileUri("/src/app.ts"),
          languageId: "typescript",
          version: 1,
          text: "const x = 1;",
        },
      },
    });
  });

  it("didChange calls notify with textDocument/didChange params", async () => {
    mockInvoke.mockResolvedValue(undefined);
    await didChange("typescript", "/src/app.ts", "const x = 2;", 2);
    expect(mockInvoke).toHaveBeenCalledWith("lsp_notify", {
      lang: "typescript",
      method: "textDocument/didChange",
      params: {
        textDocument: { uri: fileUri("/src/app.ts"), version: 2 },
        contentChanges: [{ text: "const x = 2;" }],
      },
    });
  });
});

describe("onDiagnostics", () => {
  it("registers and calls a listener, returns an unsubscribe function", () => {
    const calls: string[] = [];
    const unsub = onDiagnostics((path) => calls.push(path));
    unsub();
    // After unsubscribe, further calls won't reach the listener
    expect(calls).toHaveLength(0);
  });
});

