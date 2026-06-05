import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { parsePlanSummary, planBadge, lineClass, parsePlanJson } from "./iac-plan.js";

describe("parsePlanSummary", () => {
  it("parses a changes plan line", () => {
    const out = "...\nPlan: 3 to add, 1 to change, 2 to destroy.\n";
    expect(parsePlanSummary(out)).toEqual({ add: 3, change: 1, destroy: 2, none: false });
  });

  it("reports no changes when the plan is clean", () => {
    expect(parsePlanSummary("No changes. Your infrastructure matches the configuration.")).toEqual({
      add: 0, change: 0, destroy: 0, none: true,
    });
  });

  it("does not report no-changes for non-plan output", () => {
    expect(parsePlanSummary("No changes detected in validate", false)).toBeNull();
  });

  it("returns null when there's no plan result", () => {
    expect(parsePlanSummary("Initializing the backend...")).toBeNull();
  });
});

describe("planBadge", () => {
  it("is drift when a plan has changes", () => {
    expect(planBadge({ add: 1, change: 0, destroy: 0, none: false })).toBe("drift");
  });
  it("is clean for a no-changes plan", () => {
    expect(planBadge({ add: 0, change: 0, destroy: 0, none: true })).toBe("clean");
  });
  it("is empty with no result", () => {
    expect(planBadge(null)).toBe("");
    expect(planBadge(undefined)).toBe("");
  });
});

describe("lineClass", () => {
  it("classifies add / destroy / change / replace lines", () => {
    expect(lineClass("  + resource")).toBe("add");
    expect(lineClass("  - resource")).toBe("del");
    expect(lineClass("  ~ resource")).toBe("chg");
    expect(lineClass("-/+ resource")).toBe("rec");
  });
  it("classifies error and success lines", () => {
    expect(lineClass("Error: invalid")).toBe("err");
    expect(lineClass("Apply complete!")).toBe("ok");
  });
  it("returns empty for ordinary lines", () => {
    expect(lineClass("  some text")).toBe("");
  });
});

describe("parsePlanJson", () => {
  const doc = JSON.stringify({
    resource_changes: [
      { address: "aws_s3_bucket.a", type: "aws_s3_bucket", name: "a",
        change: { actions: ["create"], before: null, after: { bucket: "my-bucket", acl: "private" }, after_unknown: { id: true } } },
      { address: "aws_instance.web", type: "aws_instance", name: "web",
        change: { actions: ["delete", "create"], before: { ami: "ami-old", tags: { env: "dev" } }, after: { ami: "ami-new", tags: { env: "dev" } }, after_unknown: {} } },
      { address: "aws_db.x", type: "aws_db", name: "x",
        change: { actions: ["update"], before: { size: 10 }, after: { size: 20 }, after_unknown: {} } },
      { address: "aws_sg.old", type: "aws_sg", name: "old",
        change: { actions: ["delete"], before: { name: "old" }, after: null, after_unknown: {} } },
      { address: "aws_noop.n", type: "aws_noop", name: "n",
        change: { actions: ["no-op"], before: {}, after: {}, after_unknown: {} } },
      { address: "data.aws_ami.r", type: "aws_ami", name: "r",
        change: { actions: ["read"], before: null, after: {}, after_unknown: {} } },
    ],
  });

  it("counts create / update / replace / delete and drops no-op + read", () => {
    const t = parsePlanJson(doc)!;
    expect(t.counts).toEqual({ create: 1, update: 1, replace: 1, delete: 1 });
    expect(t.changes).toHaveLength(4); // no-op + read excluded
  });

  it("detects a replace from [delete, create] and orders most-impactful first", () => {
    const t = parsePlanJson(doc)!;
    expect(t.changes.map((c) => c.action)).toEqual(["replace", "delete", "update", "create"]);
    expect(t.changes[0].address).toBe("aws_instance.web");
  });

  it("diffs only changed attributes; unchanged nested values are skipped", () => {
    const replace = parsePlanJson(doc)!.changes[0];
    // tags are identical before/after → excluded; only ami changes.
    expect(replace.attrs).toEqual([{ key: "ami", before: "ami-old", after: "ami-new", unknown: false }]);
  });

  it("marks computed values as known-after-apply", () => {
    const create = parsePlanJson(doc)!.changes.find((c) => c.address === "aws_s3_bucket.a")!;
    const id = create.attrs.find((a) => a.key === "id")!;
    expect(id).toEqual({ key: "id", before: "null", after: "(known after apply)", unknown: true });
  });

  it("returns null for invalid or non-plan JSON", () => {
    expect(parsePlanJson("{not json")).toBeNull();
    expect(parsePlanJson('{"format_version":"1.0"}')).toBeNull();
  });
});

// Parse REAL `terraform show -json` documents (terraform v1.14.5, hashicorp/local)
// captured in e2e/fixtures, so a future terraform schema change that breaks the
// parser is caught here rather than only in the live UI.
describe("parsePlanJson — real terraform output", () => {
  // Vitest runs from the package root; fixtures are committed so this needs no
  // terraform at test time (real captures from terraform v1.14.5, hashicorp/local).
  const fixture = (n: string) => readFileSync(`e2e/fixtures/${n}`, "utf8");

  it("parses a real create plan (2× local_file)", () => {
    const t = parsePlanJson(fixture("tf-create.json"))!;
    expect(t.counts).toEqual({ create: 2, update: 0, replace: 0, delete: 0 });
    expect(t.changes.map((c) => c.action)).toEqual(["create", "create"]);
    const alpha = t.changes.find((c) => c.address === "local_file.alpha")!;
    expect(alpha.attrs.find((a) => a.key === "content")?.after).toBe("hello");
  });

  it("parses a real replace+destroy plan, replace ordered first", () => {
    const t = parsePlanJson(fixture("tf-replace.json"))!;
    expect(t.counts).toEqual({ create: 0, update: 0, replace: 1, delete: 1 });
    expect(t.changes[0].action).toBe("replace");
    expect(t.changes[0].address).toBe("local_file.alpha");
    expect(t.changes[1].action).toBe("delete");
    // content forces the replace; computed hashes come back known-after-apply.
    const content = t.changes[0].attrs.find((a) => a.key === "content")!;
    expect(content).toMatchObject({ before: "hello", after: "HELLO-CHANGED" });
    expect(t.changes[0].attrs.some((a) => a.unknown)).toBe(true);
  });
});
