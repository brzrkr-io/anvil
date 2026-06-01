import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  ACCOUNTS,
  getValue,
  setValue,
  clearValue,
  hasValue,
  llmCreds,
  type AccountField,
} from "./accounts";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
  localStorage.clear();
  vi.resetAllMocks();
});

// Non-secret field for localStorage tests
const endpointField = ACCOUNTS.find((f) => f.key === "llm-endpoint")!;
// Secret field for keychain tests
const apiKeyField = ACCOUNTS.find((f) => f.key === "llm-key")!;

describe("ACCOUNTS metadata", () => {
  it("has at least one non-secret field (llm-endpoint)", () => {
    expect(endpointField.secret).toBe(false);
    expect(endpointField.label).toBeTruthy();
    expect(endpointField.placeholder).toBeTruthy();
  });

  it("has at least one secret field (llm-key)", () => {
    expect(apiKeyField.secret).toBe(true);
  });

  it("every field has a key, label, and placeholder", () => {
    for (const f of ACCOUNTS) {
      expect(f.key).toBeTruthy();
      expect(f.label).toBeTruthy();
      expect(f.placeholder).toBeTruthy();
    }
  });
});

describe("getValue — non-secret", () => {
  it("returns empty string when nothing stored", async () => {
    expect(await getValue(endpointField)).toBe("");
  });

  it("returns the value stored in localStorage", async () => {
    localStorage.setItem("anvil-acct-llm-endpoint", "http://localhost:1234/v1");
    expect(await getValue(endpointField)).toBe("http://localhost:1234/v1");
  });
});

describe("getValue — secret", () => {
  it("calls invoke secret_get and returns the value", async () => {
    mockInvoke.mockResolvedValue("sk-supersecret");
    const result = await getValue(apiKeyField);
    expect(mockInvoke).toHaveBeenCalledWith("secret_get", { key: "llm-key" });
    expect(result).toBe("sk-supersecret");
  });

  it("returns empty string when invoke throws (keychain unavailable)", async () => {
    mockInvoke.mockRejectedValue(new Error("keychain locked"));
    const result = await getValue(apiKeyField);
    expect(result).toBe("");
  });
});

describe("setValue — non-secret", () => {
  it("stores the value in localStorage", async () => {
    await setValue(endpointField, "http://example.com/v1");
    expect(localStorage.getItem("anvil-acct-llm-endpoint")).toBe("http://example.com/v1");
  });

  it("does not call invoke for non-secret fields", async () => {
    await setValue(endpointField, "x");
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});

describe("setValue — secret", () => {
  it("calls invoke secret_set with the field key and value", async () => {
    mockInvoke.mockResolvedValue(undefined);
    await setValue(apiKeyField, "sk-newkey");
    expect(mockInvoke).toHaveBeenCalledWith("secret_set", { key: "llm-key", value: "sk-newkey" });
  });
});

describe("clearValue — non-secret", () => {
  it("removes the value from localStorage", async () => {
    localStorage.setItem("anvil-acct-llm-endpoint", "http://localhost");
    await clearValue(endpointField);
    expect(localStorage.getItem("anvil-acct-llm-endpoint")).toBeNull();
  });
});

describe("clearValue — secret", () => {
  it("calls invoke secret_delete", async () => {
    mockInvoke.mockResolvedValue(undefined);
    await clearValue(apiKeyField);
    expect(mockInvoke).toHaveBeenCalledWith("secret_delete", { key: "llm-key" });
  });

  it("does not throw when invoke fails (key already absent)", async () => {
    mockInvoke.mockRejectedValue(new Error("not found"));
    await expect(clearValue(apiKeyField)).resolves.toBeUndefined();
  });
});

describe("hasValue — non-secret", () => {
  it("returns false when localStorage has no value", async () => {
    expect(await hasValue(endpointField)).toBe(false);
  });

  it("returns true when localStorage has a non-empty value", async () => {
    localStorage.setItem("anvil-acct-llm-endpoint", "http://localhost");
    expect(await hasValue(endpointField)).toBe(true);
  });
});

describe("hasValue — secret", () => {
  it("calls invoke secret_has and returns true", async () => {
    mockInvoke.mockResolvedValue(true);
    const result = await hasValue(apiKeyField);
    expect(mockInvoke).toHaveBeenCalledWith("secret_has", { key: "llm-key" });
    expect(result).toBe(true);
  });

  it("returns false when invoke throws", async () => {
    mockInvoke.mockRejectedValue(new Error("error"));
    expect(await hasValue(apiKeyField)).toBe(false);
  });
});

describe("llmCreds", () => {
  it("returns base URL from localStorage and apiKey from keychain", async () => {
    localStorage.setItem("anvil-acct-llm-endpoint", "http://my-llm/v1");
    mockInvoke.mockResolvedValue("sk-mykeyvalue");
    const creds = await llmCreds();
    expect(creds.base).toBe("http://my-llm/v1");
    expect(creds.apiKey).toBe("sk-mykeyvalue");
  });

  it("returns empty strings when nothing is configured", async () => {
    mockInvoke.mockRejectedValue(new Error("no key"));
    const creds = await llmCreds();
    expect(creds.base).toBe("");
    expect(creds.apiKey).toBe("");
  });
});
