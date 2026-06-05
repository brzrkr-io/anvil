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

// ── Structured plan (terraform show -json) ────────────────────────────────
// Rich plan review: parse the machine-readable plan document into a per-resource
// add/change/destroy/replace tree with attribute-level diffs, instead of scraping
// human stdout. Schema: https://developer.hashicorp.com/terraform/internals/json-format

export type PlanAction = "create" | "update" | "delete" | "replace" | "read" | "noop";

export interface AttrDiff {
  key: string;
  before: string;
  after: string;
  unknown: boolean; // value is "(known after apply)"
}

export interface ResourceChange {
  address: string;
  module: string;
  type: string;
  name: string;
  action: PlanAction;
  attrs: AttrDiff[];
}

export interface PlanTree {
  changes: ResourceChange[];
  counts: { create: number; update: number; replace: number; delete: number };
}

// terraform encodes a replace as ["delete","create"] or ["create","delete"]
// (create-before-destroy). A bare ["create"|"update"|"delete"|"read"|"no-op"]
// is the simple action.
function actionOf(actions: unknown): PlanAction {
  const a = Array.isArray(actions) ? (actions as string[]) : [];
  if (a.includes("create") && a.includes("delete")) return "replace";
  if (a.length === 1) {
    if (a[0] === "create") return "create";
    if (a[0] === "update") return "update";
    if (a[0] === "delete") return "delete";
    if (a[0] === "read") return "read";
  }
  return "noop"; // ["no-op"] or anything unrecognized
}

function fmtVal(v: unknown): string {
  if (v === null || v === undefined) return "null";
  if (typeof v === "string") return v;
  return JSON.stringify(v);
}

// Diff before→after attribute maps, surfacing only keys that actually change
// (or become computed). `after_unknown[k] === true` ⇒ "(known after apply)".
function diffAttrs(
  before: Record<string, unknown> | null,
  after: Record<string, unknown> | null,
  unknown: Record<string, unknown> | null,
): AttrDiff[] {
  const b = before ?? {}, a = after ?? {}, u = unknown ?? {};
  const keys = [...new Set([...Object.keys(b), ...Object.keys(a), ...Object.keys(u)])].sort();
  const out: AttrDiff[] = [];
  for (const k of keys) {
    const isUnknown = u[k] === true;
    if (!isUnknown && JSON.stringify(b[k]) === JSON.stringify(a[k])) continue;
    out.push({ key: k, before: fmtVal(b[k]), after: isUnknown ? "(known after apply)" : fmtVal(a[k]), unknown: isUnknown });
  }
  return out;
}

// Most-impactful first: replace, delete, update, create.
const ACTION_RANK: Record<PlanAction, number> = { replace: 0, delete: 1, update: 2, create: 3, read: 4, noop: 5 };

// Parse a `terraform show -json <planfile>` document. Returns null if it isn't a
// recognizable plan JSON (caller falls back to raw text). no-op/read resources
// are dropped — only real changes appear in the tree.
export function parsePlanJson(json: string): PlanTree | null {
  let doc: unknown;
  try { doc = JSON.parse(json); } catch { return null; }
  const rc = (doc as { resource_changes?: unknown[] })?.resource_changes;
  if (!Array.isArray(rc)) return null;
  const counts = { create: 0, update: 0, replace: 0, delete: 0 };
  const changes: ResourceChange[] = [];
  for (const r of rc as Record<string, unknown>[]) {
    const change = (r.change ?? {}) as Record<string, unknown>;
    const action = actionOf(change.actions);
    if (action === "noop" || action === "read") continue;
    if (action in counts) (counts as Record<string, number>)[action]++;
    changes.push({
      address: String(r.address ?? ""),
      module: String(r.module_address ?? ""),
      type: String(r.type ?? ""),
      name: String(r.name ?? ""),
      action,
      attrs: diffAttrs(
        change.before as Record<string, unknown> | null,
        change.after as Record<string, unknown> | null,
        change.after_unknown as Record<string, unknown> | null,
      ),
    });
  }
  changes.sort((x, y) => ACTION_RANK[x.action] - ACTION_RANK[y.action] || x.address.localeCompare(y.address));
  return { changes, counts };
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
