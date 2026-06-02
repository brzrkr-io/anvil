import { describe, it, expect } from "vitest";
import { containerState, parseContainers, runningCount } from "./docker-ps.js";

describe("containerState", () => {
  it("uses the State field when present", () => {
    expect(containerState({ State: "running" })).toBe("running");
    expect(containerState({ State: "exited" })).toBe("exited");
  });
  it("falls back to the Status string", () => {
    expect(containerState({ Status: "Up 3 minutes" })).toBe("running");
    expect(containerState({ Status: "Up 2 hours (Paused)" })).toBe("paused");
    expect(containerState({ Status: "Exited (0) 5 minutes ago" })).toBe("exited");
  });
});

describe("parseContainers", () => {
  const raw = [
    JSON.stringify({ ID: "aaaaaaaaaaaa1", Names: "web", Image: "nginx", State: "running", Status: "Up 1m", Ports: "80/tcp" }),
    JSON.stringify({ ID: "bbbbbbbbbbbb2", Names: "db", Image: "pg", Status: "Exited (1) ago" }),
    "garbage line",
  ].join("\n");

  it("sorts running before exited and trims ids", () => {
    const rows = parseContainers(raw);
    expect(rows.map((r) => r.name)).toEqual(["web", "db"]);
    expect(rows[0].id).toHaveLength(12);
    expect(rows[1].state).toBe("exited");
  });
  it("skips unparseable lines and empties", () => {
    expect(parseContainers("\n\nnot json\n")).toEqual([]);
  });
});

describe("runningCount", () => {
  it("counts only running", () => {
    expect(runningCount([{ state: "running" }, { state: "exited" }, { state: "running" }])).toBe(2);
  });
});
