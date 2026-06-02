import { describe, it, expect, beforeEach } from "vitest";
import { askText, askConfirm, activeDialog } from "./dialog.js";
import { get } from "svelte/store";

describe("dialog", () => {
  beforeEach(() => activeDialog.set(null));

  it("askText publishes a text request and resolves with the entered value", async () => {
    const p = askText({ title: "New file", placeholder: "name" });
    const req = get(activeDialog);
    expect(req?.kind).toBe("text");
    expect(req?.title).toBe("New file");
    (req as { resolve: (v: string | null) => void }).resolve("hello.ts");
    expect(await p).toBe("hello.ts");
  });

  it("askText resolves null when cancelled", async () => {
    const p = askText({ title: "x" });
    (get(activeDialog) as { resolve: (v: string | null) => void }).resolve(null);
    expect(await p).toBeNull();
  });

  it("askConfirm carries the danger flag and resolves a boolean", async () => {
    const p = askConfirm({ title: "Delete?", danger: true });
    const req = get(activeDialog);
    expect(req?.kind).toBe("confirm");
    expect((req as { danger?: boolean }).danger).toBe(true);
    (req as { resolve: (v: boolean) => void }).resolve(true);
    expect(await p).toBe(true);
  });
});
