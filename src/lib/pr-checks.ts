// Pure helpers for the PR list with CI status (beat-target: PR+CI triage).
// Rolls up a PR's statusCheckRollup into one state so failing PRs surface first.

export type CheckState = "fail" | "pending" | "pass" | "none";

// A single entry in gh's statusCheckRollup is either a CheckRun (has
// status/conclusion) or a StatusContext (has state). Normalize either to a state.
interface RollupItem {
  __typename?: string;
  status?: string; // CheckRun: QUEUED | IN_PROGRESS | COMPLETED | ...
  conclusion?: string; // CheckRun: SUCCESS | FAILURE | NEUTRAL | SKIPPED | ...
  state?: string; // StatusContext: SUCCESS | FAILURE | ERROR | PENDING | ...
}

const FAIL_CONCLUSIONS = new Set(["FAILURE", "TIMED_OUT", "CANCELLED", "ACTION_REQUIRED", "STARTUP_FAILURE", "STALE"]);
const FAIL_STATES = new Set(["FAILURE", "ERROR"]);
const PENDING_STATES = new Set(["PENDING", "EXPECTED"]);

export function checkState(it: RollupItem): CheckState {
  // StatusContext (legacy commit statuses)
  if (it.state) {
    const s = it.state.toUpperCase();
    if (FAIL_STATES.has(s)) return "fail";
    if (PENDING_STATES.has(s)) return "pending";
    if (s === "SUCCESS") return "pass";
    return "none";
  }
  // CheckRun
  const status = (it.status ?? "").toUpperCase();
  if (status && status !== "COMPLETED") return "pending"; // QUEUED, IN_PROGRESS, WAITING, …
  const c = (it.conclusion ?? "").toUpperCase();
  if (FAIL_CONCLUSIONS.has(c)) return "fail";
  if (c === "SUCCESS") return "pass";
  return "none"; // NEUTRAL, SKIPPED, or unknown → not counted against health
}

// Worst-wins rollup across all checks: any fail → fail; else any pending →
// pending; else any pass → pass; else none (no checks configured).
export function rollupChecks(items: RollupItem[] | null | undefined): CheckState {
  if (!items || !items.length) return "none";
  let sawPending = false;
  let sawPass = false;
  for (const it of items) {
    const s = checkState(it);
    if (s === "fail") return "fail";
    if (s === "pending") sawPending = true;
    else if (s === "pass") sawPass = true;
  }
  return sawPending ? "pending" : sawPass ? "pass" : "none";
}

// Sort rank: failing PRs first, then pending, then passing, then no-checks.
export function prRank(state: CheckState): number {
  return { fail: 0, pending: 1, pass: 2, none: 3 }[state];
}

export interface PrRow {
  num: string;
  title: string;
  branch: string;
  draft: boolean;
  checks: CheckState;
}

interface RawPr {
  number: number;
  title: string;
  headRefName?: string;
  isDraft?: boolean;
  statusCheckRollup?: RollupItem[] | null;
}

// Parse `gh pr list --json …` output into rows sorted failing-first. Returns
// [] on anything that isn't a JSON array (e.g. a gh error string).
export function parsePrRows(raw: string): PrRow[] {
  let j: unknown;
  try {
    j = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(j)) return [];
  return (j as RawPr[])
    .map((p) => ({
      num: String(p.number),
      title: p.title ?? String(p.number),
      branch: p.headRefName ?? "",
      draft: p.isDraft === true,
      checks: rollupChecks(p.statusCheckRollup),
    }))
    .sort((a, b) => prRank(a.checks) - prRank(b.checks) || Number(a.num) - Number(b.num));
}
