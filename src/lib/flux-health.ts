// Pure helpers for the Flux GitOps panel (beat-target: GitOps reconcile loop).
// Kept out of the component so the sort/summary logic is unit-testable.

export type Ready = "ok" | "fail" | "unknown";

export interface FluxLike {
  name: string;
  ns: string;
  ready: Ready;
  suspended: boolean;
  message: string;
  revision: string;
}

// Sort rank: broken things first so the eye lands on what needs attention.
// failing < suspended < unknown < ok. (Suspended is deliberate, so it ranks
// below an outright failure but above healthy.)
export function healthRank(it: Pick<FluxLike, "ready" | "suspended">): number {
  if (it.ready === "fail") return 0;
  if (it.suspended) return 1;
  if (it.ready === "unknown") return 2;
  return 3;
}

// Comparator: by health rank, then ns+name alphabetically within a rank.
export function byHealth(a: FluxLike, b: FluxLike): number {
  return healthRank(a) - healthRank(b) || (a.ns + a.name).localeCompare(b.ns + b.name);
}

// Count of outright-failing items (suspended is not counted as failing).
export function failingCount(items: Pick<FluxLike, "ready">[]): number {
  return items.filter((i) => i.ready === "fail").length;
}

// Collapse a multi-line condition message to a single trimmed line for inline
// display; the row's title attribute still carries the full text.
export function oneLine(msg: string): string {
  return msg.replace(/\s+/g, " ").trim();
}

// git "main@sha1:abcd1234…" or "sha256:…" → keep ref prefix + 7 hex chars.
export function shortRev(r: string): string {
  const m = r.match(/^(.*?[@:])?([0-9a-f]{7,})/i);
  if (m) return `${m[1] ?? ""}${m[2].slice(0, 7)}`;
  return r.length > 24 ? r.slice(0, 24) + "…" : r;
}
