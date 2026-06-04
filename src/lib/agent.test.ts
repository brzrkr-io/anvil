import { describe, it, expect } from "vitest";
import { redact, planSteps } from "./agent";

describe("redact", () => {
  it("masks AWS access key IDs so creds never reach the model", () => {
    expect(redact("aws_key = AKIAIOSFODNN7EXAMPLE")).not.toContain("AKIAIOSFODNN7EXAMPLE");
  });

  it("masks GitHub PATs", () => {
    const out = redact("Use ghp_abcdefghijklmnopqrstuvwxyz0123456789 here");
    expect(out).toContain("ghp_****REDACTED");
    expect(out).not.toContain("abcdefghijklmnopqrstuvwxyz0123456789");
  });

  it("masks OpenAI-style sk- keys", () => {
    expect(redact("sk-ABCDEFGHIJKLMNOPQRSTUVWX")).toContain("sk-****REDACTED");
  });

  it("masks key=value secrets regardless of casing", () => {
    expect(redact("PASSWORD=hunter2hunter")).toContain("****REDACTED");
    expect(redact('api_key: "s3cr3tValue123"')).toContain("****REDACTED");
  });

  it("masks PEM private key blocks", () => {
    const pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----";
    expect(redact(pem)).toBe("****REDACTED PRIVATE KEY****");
  });

  it("masks AWS temporary (STS) access key IDs", () => {
    expect(redact("AWS_ACCESS_KEY_ID=ASIAY34FZKBOKMUTVV7A")).not.toContain("ASIAY34FZKBOKMUTVV7A");
  });

  it("masks Google API keys", () => {
    const k = "AIza" + "x".repeat(35);
    expect(redact(`gkey=${k}`)).not.toContain(k);
  });

  it("masks Authorization: Bearer tokens", () => {
    const out = redact('curl -H "Authorization: Bearer eyJabc.def.ghi1234567"');
    expect(out).toContain("****REDACTED");
    expect(out).not.toContain("eyJabc.def.ghi1234567");
  });

  it("leaves ordinary code untouched", () => {
    const code = "function add(a, b) { return a + b; }";
    expect(redact(code)).toBe(code);
  });
});

describe("planSteps", () => {
  it("strips bullets and numbering", () => {
    expect(planSteps("1. build\n2) test\n- ship")).toEqual(["build", "test", "ship"]);
  });

  it("drops blank lines", () => {
    expect(planSteps("a\n\n  \nb")).toEqual(["a", "b"]);
  });
});
