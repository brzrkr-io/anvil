import { describe, it, expect, beforeEach } from "vitest";
import { getProfiles, saveProfile, deleteProfile, getActiveProfile, setActiveProfile, profileSummary } from "./profiles.js";

describe("profiles", () => {
  beforeEach(() => localStorage.clear());

  it("upserts by name and keeps the list sorted", () => {
    saveProfile({ name: "prod", kubeContext: "p" });
    saveProfile({ name: "dev", kubeContext: "d" });
    saveProfile({ name: "prod", kubeContext: "p2" }); // replace
    const list = getProfiles();
    expect(list.map((p) => p.name)).toEqual(["dev", "prod"]);
    expect(list.find((p) => p.name === "prod")!.kubeContext).toBe("p2");
  });

  it("ignores a nameless profile", () => {
    saveProfile({ name: "  " });
    expect(getProfiles()).toEqual([]);
  });

  it("deleting the active profile clears the active marker", () => {
    saveProfile({ name: "stage" });
    setActiveProfile("stage");
    expect(getActiveProfile()).toBe("stage");
    deleteProfile("stage");
    expect(getActiveProfile()).toBeNull();
    expect(getProfiles()).toEqual([]);
  });

  it("tolerates corrupt storage", () => {
    localStorage.setItem("anvil-env-profiles", "{bad");
    expect(getProfiles()).toEqual([]);
  });

  it("summarizes what a profile switches", () => {
    expect(profileSummary({ name: "x", kubeContext: "c", awsProfile: "a", namespace: "n" }))
      .toBe("ctx c · aws a · ns n");
  });
});
