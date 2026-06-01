// Tests for the localStorage-backed paths in redaction.ts.
// Built-in masking logic is covered in redaction.test.ts.
import { describe, it, expect, beforeEach } from "vitest";
import {
  setRedactionRules, redactionRules,
  addRedactionRule,
  getAuditLog, clearAuditLog, auditAgentSend,
} from "./redaction";

beforeEach(() => {
  localStorage.clear();
  setRedactionRules([]);
});

describe("setRedactionRules — persistence round-trip (L14, L21)", () => {
  it("persists rules to localStorage as JSON", () => {
    setRedactionRules(["SECRET\\w+", "token_\\d+"]);
    const stored = JSON.parse(localStorage.getItem("anvil-redaction-rules") || "[]");
    expect(stored).toEqual(["SECRET\\w+", "token_\\d+"]);
  });

  it("round-trips: rules stored can be read back via getItem", () => {
    setRedactionRules(["MY_PATTERN"]);
    const raw = localStorage.getItem("anvil-redaction-rules");
    expect(JSON.parse(raw!)).toContain("MY_PATTERN");
  });
});

describe("addRedactionRule — persists via setRedactionRules", () => {
  it("persisted after adding", () => {
    addRedactionRule("PROD_KEY_\\w+");
    const stored = JSON.parse(localStorage.getItem("anvil-redaction-rules") || "[]");
    expect(stored).toContain("PROD_KEY_\\w+");
  });
});

describe("auditAgentSend + getAuditLog + clearAuditLog (L46–59)", () => {
  it("appends an entry to the audit log in localStorage", () => {
    auditAgentSend("chat", "hello world");
    const log = getAuditLog();
    expect(log).toHaveLength(1);
    expect(log[0].kind).toBe("chat");
    expect(log[0].chars).toBe(11);
  });

  it("stores only a 120-char preview, not the full text", () => {
    const long = "x".repeat(300);
    auditAgentSend("send", long);
    const log = getAuditLog();
    expect(log[0].preview.length).toBe(120);
  });

  it("accumulates multiple entries", () => {
    auditAgentSend("a", "first");
    auditAgentSend("b", "second");
    expect(getAuditLog()).toHaveLength(2);
  });

  it("persists across getAuditLog calls (reads from localStorage)", () => {
    auditAgentSend("kind", "payload");
    // Re-read: getAuditLog parses from storage each time
    const log = getAuditLog();
    expect(log.length).toBeGreaterThan(0);
    expect(log[0].kind).toBe("kind");
  });

  it("clearAuditLog removes the entry from localStorage", () => {
    auditAgentSend("x", "data");
    clearAuditLog();
    expect(getAuditLog()).toHaveLength(0);
    expect(localStorage.getItem("anvil-agent-audit")).toBeNull();
  });

  it("entry has a timestamp close to now", () => {
    const before = Date.now();
    auditAgentSend("t", "hi");
    const after = Date.now();
    const log = getAuditLog();
    expect(log[0].ts).toBeGreaterThanOrEqual(before);
    expect(log[0].ts).toBeLessThanOrEqual(after);
  });
});
