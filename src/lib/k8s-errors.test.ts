import { describe, it, expect } from "vitest";
import { classifyK8sError, friendlyK8sError } from "./k8s-errors";

describe("classifyK8sError (#5)", () => {
  it("flags expired/SSO credentials as auth", () => {
    expect(classifyK8sError("error: You must be logged in (InvalidIdentityToken)")).toBe("auth");
    expect(classifyK8sError("the SSO session has expired")).toBe("auth");
  });

  it("flags forbidden / cannot-list as rbac", () => {
    expect(classifyK8sError('pods is forbidden: User cannot list resource "pods"')).toBe("rbac");
  });

  it("flags unreachable/timeout as network", () => {
    expect(classifyK8sError("dial tcp 10.0.0.1:443: i/o timeout")).toBe("network");
    expect(classifyK8sError("Unable to connect: connection refused")).toBe("network");
  });

  it("auth takes priority over a downstream forbidden", () => {
    expect(classifyK8sError("token has expired; pods is forbidden")).toBe("auth");
  });

  it("empty is none; unknown is other", () => {
    expect(classifyK8sError("")).toBe("none");
    expect(classifyK8sError("   ")).toBe("none");
    expect(classifyK8sError("some weird error")).toBe("other");
  });
});

describe("friendlyK8sError", () => {
  it("maps each kind to readable copy and passes through unknowns", () => {
    expect(friendlyK8sError("token has expired")).toMatch(/credentials/i);
    expect(friendlyK8sError("forbidden")).toMatch(/RBAC/);
    expect(friendlyK8sError("i/o timeout")).toMatch(/reach the cluster/i);
    expect(friendlyK8sError("")).toBe("");
    expect(friendlyK8sError("raw thing")).toBe("raw thing");
  });
});
