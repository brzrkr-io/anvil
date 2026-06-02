import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  addRedactionRule, removeRedactionRule, setRedactionRules, applyRedaction,
  auditAgentSend, getAuditLog, clearAuditLog, redactionRules,
} from "./redaction.js";
import { bumpUsage, rankItems, withTracking } from "./palette-rank.js";
import { checkState, rollupChecks, parsePrRows, prRank } from "./pr-checks.js";
import { terraformInvestigation, githubInvestigation, investigationPrompt, actionsInvestigation } from "./agent-ops.js";

beforeEach(() => { localStorage.clear(); setRedactionRules([]); });

describe("redaction", () => {
  it("rejects an invalid regex rule but keeps valid ones (and de-dupes)", () => {
    addRedactionRule("(unclosed");
    expect(get(redactionRules)).toEqual([]);
    addRedactionRule("token-\\d+");
    addRedactionRule("token-\\d+"); // duplicate
    expect(get(redactionRules)).toEqual(["token-\\d+"]);
    removeRedactionRule("token-\\d+");
    expect(get(redactionRules)).toEqual([]);
  });

  it("applies user rules and skips a corrupt one without throwing", () => {
    setRedactionRules(["hunter2", "(bad"]); // second is invalid
    expect(applyRedaction("pw=hunter2")).toContain("****REDACTED****");
  });

  it("audit log records a preview only, and clears", () => {
    auditAgentSend("prompt", "x".repeat(500));
    const log = getAuditLog();
    expect(log).toHaveLength(1);
    expect(log[0].chars).toBe(500);
    expect(log[0].preview.length).toBeLessThanOrEqual(120);
    clearAuditLog();
    expect(getAuditLog()).toEqual([]);
  });

  it("audit log tolerates corrupt storage", () => {
    localStorage.setItem("anvil-agent-audit", "{bad");
    expect(getAuditLog()).toEqual([]);
  });
});

describe("palette-rank", () => {
  it("floats the most-used command to the top, stable on ties", () => {
    bumpUsage("Build"); bumpUsage("Build"); bumpUsage("Test");
    const ranked = rankItems([{ label: "Deploy" }, { label: "Test" }, { label: "Build" }]);
    expect(ranked.map((r) => r.label)).toEqual(["Build", "Test", "Deploy"]);
  });

  it("withTracking records a use when the wrapped command runs", () => {
    let ran = false;
    const wrapped = withTracking({ label: "X", run: () => (ran = true) });
    wrapped.run();
    expect(ran).toBe(true);
    expect(rankItems([{ label: "Y" }, { label: "X" }])[0].label).toBe("X");
  });

  it("tolerates corrupt usage storage", () => {
    localStorage.setItem("anvil-cmd-usage", "{bad");
    expect(rankItems([{ label: "A" }])).toHaveLength(1);
  });
});

describe("pr-checks", () => {
  it("normalizes legacy StatusContext states", () => {
    expect(checkState({ state: "ERROR" })).toBe("fail");
    expect(checkState({ state: "PENDING" })).toBe("pending");
    expect(checkState({ state: "SUCCESS" })).toBe("pass");
    expect(checkState({ state: "WEIRD" })).toBe("none");
  });
  it("normalizes CheckRun status/conclusion", () => {
    expect(checkState({ status: "IN_PROGRESS" })).toBe("pending");
    expect(checkState({ status: "COMPLETED", conclusion: "FAILURE" })).toBe("fail");
    expect(checkState({ status: "COMPLETED", conclusion: "SUCCESS" })).toBe("pass");
    expect(checkState({ status: "COMPLETED", conclusion: "SKIPPED" })).toBe("none");
  });
  it("rolls up worst-wins and ranks failing first", () => {
    expect(rollupChecks([])).toBe("none");
    expect(rollupChecks([{ conclusion: "SUCCESS", status: "COMPLETED" }, { status: "QUEUED" }])).toBe("pending");
    expect(rollupChecks([{ conclusion: "SUCCESS", status: "COMPLETED" }])).toBe("pass");
    expect(prRank("fail")).toBeLessThan(prRank("pass"));
  });
  it("parses gh json into failing-first rows and ignores non-arrays", () => {
    expect(parsePrRows("not json")).toEqual([]);
    expect(parsePrRows('{"x":1}')).toEqual([]);
    const rows = parsePrRows(JSON.stringify([
      { number: 2, title: "ok", statusCheckRollup: [{ conclusion: "SUCCESS", status: "COMPLETED" }] },
      { number: 3, title: "bad", statusCheckRollup: [{ conclusion: "FAILURE", status: "COMPLETED" }] },
    ]));
    expect(rows[0].num).toBe("3"); // failing first
    expect(rows[0].checks).toBe("fail");
  });
});

describe("agent-ops builders", () => {
  it("terraform investigation labels repo root and omits empty detail", () => {
    const p = terraformInvestigation(".", "terraform plan");
    expect(p).toContain("(repo root)");
    expect(p).not.toContain("Known failure detail");
  });
  it("includes trimmed detail when provided", () => {
    const p = investigationPrompt({ tool: "terraform", subject: "x", diagnose: "plan", detail: "  boom  " });
    expect(p).toContain("boom");
  });
  it("github investigation adds a log-failed follow-up only with a branch", () => {
    expect(githubInvestigation("42", "feat")).toContain("--log-failed");
    expect(githubInvestigation("42", "")).not.toContain("xargs");
  });
  it("actions investigation names workflow + branch when present, omits when not", () => {
    const full = actionsInvestigation("9", "CI", "main");
    expect(full).toContain("(CI)");
    expect(full).toContain("on main");
    const bareSubject = actionsInvestigation("9", "", "").split("\n\n")[0];
    expect(bareSubject).toBe("GitHub Actions run 9 has failing CI checks.");
  });
});
