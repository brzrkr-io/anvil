// Command-palette frequency ranking (roadmap H74). Tracks how often each
// command (by label) is run and floats the most-used to the top, so the palette
// learns your habits. Ties keep the original (curated) order — stable sort.

const KEY = "anvil-cmd-usage";
type Usage = Record<string, number>;

function load(): Usage {
  if (typeof localStorage === "undefined") return {};
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}

export function bumpUsage(label: string): void {
  if (typeof localStorage === "undefined") return;
  const u = load();
  u[label] = (u[label] || 0) + 1;
  localStorage.setItem(KEY, JSON.stringify(u));
}

// Sort by usage desc, original index asc for ties (stable). Returns a new array.
export function rankItems<T extends { label: string }>(items: T[]): T[] {
  const u = load();
  return items
    .map((it, i) => ({ it, i }))
    .sort((a, b) => (u[b.it.label] || 0) - (u[a.it.label] || 0) || a.i - b.i)
    .map((x) => x.it);
}

// Wrap an item's run() to record a use first, so ranking self-updates.
export function withTracking<T extends { label: string; run: () => void }>(item: T): T {
  return { ...item, run: () => { bumpUsage(item.label); item.run(); } };
}
