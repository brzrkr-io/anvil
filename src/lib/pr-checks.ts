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
  base: string; // target branch (#27 stacked PRs)
  draft: boolean;
  checks: CheckState;
}

interface RawPr {
  number: number;
  title: string;
  headRefName?: string;
  baseRefName?: string;
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
      base: p.baseRefName ?? "",
      draft: p.isDraft === true,
      checks: rollupChecks(p.statusCheckRollup),
    }))
    .sort((a, b) => prRank(a.checks) - prRank(b.checks) || Number(a.num) - Number(b.num));
}

export type StackedPr = PrRow & { depth: number };

// #27 Stacked PRs: a PR is "stacked" when its base branch is another open PR's
// head branch (it sits on top of that PR instead of the default branch).
// Reorders rows so each child follows its parent, indented by `depth`, while
// roots keep their incoming (failing-first) order. Orphans/cycles fall back to
// depth 0 so no PR is ever dropped.
export function orderStacks(rows: PrRow[]): StackedPr[] {
  const heads = new Set(rows.map((r) => r.branch).filter(Boolean));
  const childrenOf = new Map<string, PrRow[]>(); // parent head branch → child rows
  const roots: PrRow[] = [];
  for (const r of rows) {
    if (r.base && heads.has(r.base) && r.base !== r.branch) {
      const arr = childrenOf.get(r.base) ?? [];
      arr.push(r);
      childrenOf.set(r.base, arr);
    } else {
      roots.push(r);
    }
  }
  const out: StackedPr[] = [];
  const seen = new Set<string>();
  const walk = (r: PrRow, depth: number) => {
    if (seen.has(r.num)) return; // guard against cycles
    seen.add(r.num);
    out.push({ ...r, depth });
    for (const c of childrenOf.get(r.branch) ?? []) walk(c, depth + 1);
  };
  for (const r of roots) walk(r, 0);
  // Any row not reached (e.g. its parent was itself a child in a cycle) lands flat.
  for (const r of rows) if (!seen.has(r.num)) out.push({ ...r, depth: 0 });
  return out;
}
