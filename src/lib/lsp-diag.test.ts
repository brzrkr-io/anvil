// Tests for the wireDiagnostics path in lsp.ts.
// This file is isolated so it gets its own module instance of lsp.ts, ensuring
// diagWired starts as false and the listen callback can be captured.
import { describe, it, expect, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => true),
}));

let capturedListenCallback: ((e: { payload: unknown }) => void) | null = null;

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_channel: string, cb: (e: { payload: unknown }) => void) => {
    capturedListenCallback = cb;
    return () => {};
  }),
}));

vi.mock("$lib/diagnostics", () => ({
  setFileProblems: vi.fn(),
}));

import { ensureLsp, diagByPath, onDiagnostics } from "./lsp";
import { setFileProblems } from "$lib/diagnostics";

const mockSetFileProblems = vi.mocked(setFileProblems);

describe("wireDiagnostics — diagnostics event processing", () => {
  it("triggers listen when ensureLsp succeeds and wires the diagnostic callback", async () => {
    await ensureLsp("isolated-lang", "/project");
    expect(capturedListenCallback).not.toBeNull();
  });

  it("processes a full diagnostics event: updates diagByPath, calls setFileProblems, notifies listeners", () => {
    const notified: string[] = [];
    const unsub = onDiagnostics((p) => notified.push(p));

    capturedListenCallback!({
      payload: {
        uri: "file:///project/app.ts",
        diagnostics: [
          {
            range: { start: { line: 4, character: 2 }, end: { line: 4, character: 10 } },
            message: "Type error",
            severity: 1,
          },
        ],
      },
    });

    const raw = diagByPath.get("/project/app.ts");
    expect(raw).toHaveLength(1);
    expect(raw![0].message).toBe("Type error");
    expect(raw![0].line).toBe(4);
    expect(raw![0].severity).toBe(1);
    expect(mockSetFileProblems).toHaveBeenCalledWith(
      "/project/app.ts",
      [{ path: "/project/app.ts", line: 5, message: "Type error", severity: 1 }]
    );
    expect(notified).toContain("/project/app.ts");

    unsub();
    diagByPath.delete("/project/app.ts");
  });

  it("ignores a diagnostics event with no URI field", () => {
    mockSetFileProblems.mockClear();
    capturedListenCallback!({ payload: { diagnostics: [] } });
    expect(mockSetFileProblems).not.toHaveBeenCalled();
  });

  it("uses severity 1 as default when severity is absent", () => {
    capturedListenCallback!({
      payload: {
        uri: "file:///project/b.ts",
        diagnostics: [
          { range: { start: { line: 0, character: 0 }, end: { line: 0, character: 5 } }, message: "warn" },
        ],
      },
    });
    const raw = diagByPath.get("/project/b.ts");
    expect(raw![0].severity).toBe(1);
    diagByPath.delete("/project/b.ts");
  });

  it("uses endLine same as start.line when end is absent", () => {
    capturedListenCallback!({
      payload: {
        uri: "file:///project/c.ts",
        diagnostics: [
          { range: { start: { line: 3, character: 0 } }, message: "err" },
        ],
      },
    });
    const raw = diagByPath.get("/project/c.ts");
    expect(raw![0].endLine).toBe(3);
    diagByPath.delete("/project/c.ts");
  });
});
