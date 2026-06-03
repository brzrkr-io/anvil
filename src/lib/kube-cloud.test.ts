import { describe, it, expect } from "vitest";
import { detectCloud, reauthActions } from "./kube-cloud";

describe("detectCloud (#8)", () => {
  it("detects GKE by the gke_ prefix", () => {
    expect(detectCloud("gke_my-proj_us-central1_prod")).toBe("gcp");
  });
  it("detects EKS by arn or eks token", () => {
    expect(detectCloud("arn:aws:eks:us-east-1:123456789:cluster/prod")).toBe("aws");
    expect(detectCloud("prod-eks-cluster")).toBe("aws");
  });
  it("detects AKS only on an explicit hint", () => {
    expect(detectCloud("prod-aks")).toBe("azure");
    expect(detectCloud("just-a-name")).toBe("unknown");
  });
});

describe("reauthActions (#8)", () => {
  it("builds gcloud commands with parsed project/location/cluster", () => {
    const a = reauthActions("gke_my-proj_us-central1_prod");
    expect(a[0].cmd).toBe("gcloud auth login");
    expect(a[1].cmd).toBe(
      "gcloud container clusters get-credentials prod --location us-central1 --project my-proj",
    );
  });
  it("parses the EKS cluster name out of an ARN", () => {
    const a = reauthActions("arn:aws:eks:us-east-1:123:cluster/prod-cluster");
    expect(a[0].cmd).toBe("aws sso login");
    expect(a[1].cmd).toBe('aws eks update-kubeconfig --name "prod-cluster"');
  });
  it("falls back to AWS SSO for an unknown context", () => {
    expect(reauthActions("mystery")[0].cmd).toBe("aws sso login");
  });
  it("uses az for Azure contexts", () => {
    const a = reauthActions("prod-aks");
    expect(a[0].cmd).toBe("az login");
    expect(a[1].cmd).toContain("az aks get-credentials");
  });

  it("uses the context's AWS profile for a precise sso login + refresh", () => {
    const a = reauthActions("arn:aws:eks:us-east-2:511:cluster/firemon-core-eks", {
      cloud: "aws", profile: "dev-core", region: "us-east-2", cluster: "firemon-core-eks",
    });
    expect(a[0].cmd).toBe("aws sso login --profile dev-core");
    expect(a[1].cmd).toBe(
      'aws eks update-kubeconfig --name "firemon-core-eks" --region us-east-2 --profile dev-core',
    );
  });

  it("prefers --sso-session when the profile shares one (re-auths all profiles)", () => {
    const a = reauthActions("arn:aws:eks:us-east-2:511:cluster/x", {
      cloud: "aws", profile: "dev-core", ssoSession: "corp-sso",
    });
    expect(a[0].cmd).toBe("aws sso login --sso-session corp-sso");
  });
});
