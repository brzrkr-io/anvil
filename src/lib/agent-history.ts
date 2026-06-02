// Persisted agent run history (Tier 0 #8). Starting a new chat archives the
// finished conversation as a "run" the user can reopen, instead of discarding
// it. Index holds lightweight metadata; each run's messages live under their own
// key so the index stays small. Capped so history can't grow unbounded.
export interface RunMsg { role: string; text?: string }
export interface RunMeta { id: string; ts: number; title: string }

const IDX = "anvil-agent-runs";
const keyOf = (id: string) => `anvil-agent-run:${id}`;
const CAP = 40;

export function listRuns(): RunMeta[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const v = JSON.parse(localStorage.getItem(IDX) || "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

// Derive a short title from the first user message (or fall back).
export function runTitle(messages: RunMsg[]): string {
  const first = messages.find((m) => m.role === "user" && (m.text ?? "").trim());
  const t = (first?.text ?? "").replace(/\s+/g, " ").trim();
  return t ? t.slice(0, 60) : "Agent session";
}

/** Archive a finished conversation. No-op for an empty chat. Returns the meta. */
export function archiveRun(messages: RunMsg[], id: string, ts: number): RunMeta | null {
  if (typeof localStorage === "undefined" || !messages.length) return null;
  const meta: RunMeta = { id, ts, title: runTitle(messages) };
  try {
    localStorage.setItem(keyOf(id), JSON.stringify(messages));
  } catch {
    return null;
  }
  const idx = [meta, ...listRuns().filter((r) => r.id !== id)];
  const kept = idx.slice(0, CAP);
  for (const dropped of idx.slice(CAP)) localStorage.removeItem(keyOf(dropped.id));
  localStorage.setItem(IDX, JSON.stringify(kept));
  return meta;
}

export function loadRun(id: string): RunMsg[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const v = JSON.parse(localStorage.getItem(keyOf(id)) || "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

export function deleteRun(id: string): RunMeta[] {
  if (typeof localStorage !== "undefined") localStorage.removeItem(keyOf(id));
  const idx = listRuns().filter((r) => r.id !== id);
  if (typeof localStorage !== "undefined") localStorage.setItem(IDX, JSON.stringify(idx));
  return idx;
}
