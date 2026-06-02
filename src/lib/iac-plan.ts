// Pure helpers for the Terraform/Terragrunt panel (beat-target: IaC plan/apply).
// Kept out of the component so plan parsing + line classification are testable.

export interface PlanSummary {
  add: number;
  change: number;
  destroy: number;
  none: boolean; // true = plan ran and reported no changes
}

// Parse a `Plan: N to add, N to change, N to destroy` line, or a no-changes
// plan. `isPlan` guards the no-changes case so validate/init output that happens
// to contain "No changes" isn't misread as an empty plan. Returns null when the
// text carries no recognizable plan result.
export function parsePlanSummary(output: string, isPlan = true): PlanSummary | null {
  const m = output.match(/Plan:\s+(\d+)\s+to add,\s+(\d+)\s+to change,\s+(\d+)\s+to destroy/);
  if (m) return { add: +m[1], change: +m[2], destroy: +m[3], none: false };
  if (isPlan && /No changes\.|infrastructure matches/i.test(output)) {
    return { add: 0, change: 0, destroy: 0, none: true };
  }
  return null;
}

// One-word summary for a stack-row badge: "clean" (no changes), "drift" (has
// changes), or "" (no plan result yet).
export function planBadge(s: PlanSummary | null | undefined): "clean" | "drift" | "" {
  if (!s) return "";
  return s.none ? "clean" : "drift";
}

// Classify a plan/apply output line for diff colorization.
export function lineClass(l: string): string {
  const t = l.replace(/^\s+/, "");
  if (t.startsWith("-/+") || t.startsWith("+/-")) return "rec";
  if (t.startsWith("+")) return "add";
  if (t.startsWith("- ") || t === "-") return "del";
  if (t.startsWith("~")) return "chg";
  if (/^(Error:|╷|│\s*Error|✗)/.test(t)) return "err";
  if (/^(Success!|✓|Apply complete|No changes)/.test(t)) return "ok";
  return "";
}
