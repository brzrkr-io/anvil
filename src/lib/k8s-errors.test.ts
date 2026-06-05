import { describe, it, expect } from "vitest";
import { classifyK8sError, friendlyK8sError, parseNamespaces } from "./k8s-errors";

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

  it("flags a missing CLI (exec not found) as tooling, not auth", () => {
    // The real EKS-in-a-GUI failure: contains "credentials" but is a PATH bug.
    expect(classifyK8sError("getting credentials: exec: executable aws not found")).toBe("tooling");
    expect(classifyK8sError("kubectl: command not found")).toBe("tooling");
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

describe("parseNamespaces", () => {
  it("keeps valid namespace names", () => {
    expect(parseNamespaces("default\nkube-system\nflux-system")).toEqual([
      "default", "kube-system", "flux-system",
    ]);
  });

  it("strips a namespace/ prefix", () => {
    expect(parseNamespaces("namespace/default\nnamespace/cert-manager")).toEqual([
      "default", "cert-manager",
    ]);
  });

  // The regression: kubectl returns the SSO error TEXT on expired creds. Split
  // into lines it produced duplicate strings that crashed the keyed <select>
  // (each_key_duplicate at indexes 5 and 6). parseNamespaces must drop it whole,
  // even when the line repeats — no bogus options, nothing to collide.
  it("drops kubectl auth-error text entirely (even when repeated)", () => {
    const sso =
      "The SSO session associated with this profile has expired or is otherwise invalid. To refresh this SSO session run aws sso login with the corresponding profile.";
    expect(parseNamespaces(`${sso}\n${sso}`)).toEqual([]);
    expect(parseNamespaces("error: You must be logged in to the server (Unauthorized)")).toEqual([]);
    expect(parseNamespaces("")).toEqual([]);
  });
});
