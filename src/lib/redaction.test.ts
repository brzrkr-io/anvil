import { describe, it, expect, beforeEach } from "vitest";
import {
  applyRedaction,
  addRedactionRule,
  removeRedactionRule,
  setRedactionRules,
  redactionRules,
} from "./redaction";

function resetRules() {
  setRedactionRules([]);
}

describe("applyRedaction — built-in secrets must not survive", () => {
  beforeEach(resetRules);

  it("masks AWS access key IDs", () => {
    const out = applyRedaction("key=AKIAIOSFODNN7EXAMPLE123456");
    expect(out).not.toContain("AKIAIOSFODNN7EXAMPLE123456");
  });

  it("masks GitHub PATs", () => {
    const out = applyRedaction("token ghp_abcdefghijklmnopqrstuvwxyz012345");
    expect(out).not.toContain("abcdefghijklmnopqrstuvwxyz012345");
    expect(out).toContain("ghp_****REDACTED");
  });

  it("masks OpenAI-style sk- API keys", () => {
    const out = applyRedaction("Authorization: Bearer sk-ABCDEFGHIJKLMNOPQRSTUVWX");
    expect(out).not.toContain("ABCDEFGHIJKLMNOPQRSTUVWX");
    expect(out).toContain("sk-****REDACTED");
  });

  it("masks Slack tokens", () => {
    const out = applyRedaction("xoxb-123456789-abcdefghijklmn");
    expect(out).not.toContain("123456789-abcdefghijklmn");
  });

  it("masks PEM private key blocks", () => {
    const pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA\n-----END RSA PRIVATE KEY-----";
    expect(applyRedaction(pem)).toBe("****REDACTED PRIVATE KEY****");
  });

  it("masks password= values", () => {
    const out = applyRedaction("password=hunter2hunter");
    expect(out).not.toContain("hunter2hunter");
  });

  it("does not alter ordinary code or prose", () => {
    const plain = "function hello() { return 42; }";
    expect(applyRedaction(plain)).toBe(plain);
  });
});

describe("user-defined redaction rules", () => {
  beforeEach(resetRules);

  it("addRedactionRule causes matching text to be masked", () => {
    addRedactionRule("INTERNAL_TOKEN_\\w+");
    const out = applyRedaction("send INTERNAL_TOKEN_abc123 over wire");
    expect(out).not.toContain("INTERNAL_TOKEN_abc123");
    expect(out).toContain("****REDACTED****");
  });

  it("removeRedactionRule stops masking", () => {
    addRedactionRule("TOPSECRET");
    removeRedactionRule("TOPSECRET");
    let rules: string[] = [];
    redactionRules.subscribe((v) => { rules = v; })();
    expect(rules).not.toContain("TOPSECRET");
    const out = applyRedaction("TOPSECRET value");
    expect(out).toContain("TOPSECRET");
  });

  it("ignores blank rule sources", () => {
    addRedactionRule("   ");
    let rules: string[] = [];
    redactionRules.subscribe((v) => { rules = v; })();
    expect(rules).toHaveLength(0);
  });

  it("rejects invalid regex without throwing", () => {
    expect(() => addRedactionRule("[invalid")).not.toThrow();
    let rules: string[] = [];
    redactionRules.subscribe((v) => { rules = v; })();
    expect(rules).toHaveLength(0);
  });

  it("deduplicates rules when the same pattern is added twice", () => {
    addRedactionRule("MYSECRET");
    addRedactionRule("MYSECRET");
    let rules: string[] = [];
    redactionRules.subscribe((v) => { rules = v; })();
    expect(rules.filter((r) => r === "MYSECRET")).toHaveLength(1);
  });
});
