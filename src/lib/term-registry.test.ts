import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  registerTerminal,
  unregisterTerminal,
  readTerminal,
  broadcastInput,
  liveTerminals,
} from "./term-registry";

// Clean up all registered terminals between tests
beforeEach(() => {
  for (const id of liveTerminals()) {
    unregisterTerminal(id);
  }
  broadcastInput.set(false);
});

describe("registerTerminal / readTerminal", () => {
  it("registered reader is called by readTerminal", () => {
    registerTerminal("t1", () => "output from t1");
    expect(readTerminal("t1")).toBe("output from t1");
  });

  it("returns empty string for an unknown terminal id", () => {
    expect(readTerminal("no-such-id")).toBe("");
  });

  it("returns empty string when the reader throws", () => {
    registerTerminal("throwing", () => { throw new Error("read failed"); });
    expect(readTerminal("throwing")).toBe("");
  });
});

describe("unregisterTerminal", () => {
  it("removed terminal returns empty string from readTerminal", () => {
    registerTerminal("t2", () => "content");
    unregisterTerminal("t2");
    expect(readTerminal("t2")).toBe("");
  });

  it("is a no-op for an id that was never registered", () => {
    expect(() => unregisterTerminal("ghost")).not.toThrow();
  });
});

describe("liveTerminals", () => {
  it("returns an empty array when no terminals are registered", () => {
    expect(liveTerminals()).toHaveLength(0);
  });

  it("lists all currently registered terminal ids", () => {
    registerTerminal("ta", () => "");
    registerTerminal("tb", () => "");
    const ids = liveTerminals();
    expect(ids).toContain("ta");
    expect(ids).toContain("tb");
    expect(ids).toHaveLength(2);
  });

  it("does not include unregistered terminals", () => {
    registerTerminal("keep", () => "");
    registerTerminal("gone", () => "");
    unregisterTerminal("gone");
    expect(liveTerminals()).toEqual(["keep"]);
  });
});

describe("broadcastInput store", () => {
  it("defaults to false", () => {
    expect(get(broadcastInput)).toBe(false);
  });

  it("can be set to true", () => {
    broadcastInput.set(true);
    expect(get(broadcastInput)).toBe(true);
  });
});
