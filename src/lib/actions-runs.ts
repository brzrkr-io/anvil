// Pure helpers for the GitHub Actions runs view (beat-target: PR+CI triage).
// Parses `gh run list --json …` into failing-first rows so "what's failing" is
// the first thing you see.

export type RunState = "fail" | "running" | "pass" | "neutral";

export interface RunRow {
  id: string;
  title: string; // displayTitle (commit/PR title)
  workflow: string; // workflowName
  branch: string;
  event: string;
  state: RunState;
}

interface RawRun {
  databaseId?: number;
  status?: string; // queued | in_progress | completed | …
  conclusion?: string; // success | failure | cancelled | timed_out | …
  displayTitle?: string;
  workflowName?: string;
  headBranch?: string;
  event?: string;
}

const FAIL = new Set(["failure", "timed_out", "cancelled", "action_required", "startup_failure", "stale"]);

export function runState(r: Pick<RawRun, "status" | "conclusion">): RunState {
  if ((r.status ?? "").toLowerCase() !== "completed") return "running";
  const c = (r.conclusion ?? "").toLowerCase();
  if (FAIL.has(c)) return "fail";
  if (c === "success") return "pass";
  return "neutral"; // skipped / neutral / unknown
}

// Sort rank: failing first, then in-flight, then passing, then neutral.
export function runRank(s: RunState): number {
  return { fail: 0, running: 1, pass: 2, neutral: 3 }[s];
}

export function parseRuns(raw: string): RunRow[] {
  let j: unknown;
  try {
    j = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(j)) return [];
  return (j as RawRun[])
    .map((r) => ({
      id: String(r.databaseId ?? ""),
      title: r.displayTitle ?? "(run)",
      workflow: r.workflowName ?? "",
      branch: r.headBranch ?? "",
      event: r.event ?? "",
      state: runState(r),
    }))
    .filter((r) => r.id)
    .sort((a, b) => runRank(a.state) - runRank(b.state) || Number(b.id) - Number(a.id));
}

export function failingRuns(rows: Pick<RunRow, "state">[]): number {
  return rows.filter((r) => r.state === "fail").length;
}
