// Stale-while-revalidate cache for slow page data (kube pods, helm releases, CI
// pipelines, …). Persist the last successful result to localStorage so a page
// can render real data INSTANTLY on load, then refresh in the background — no
// spinner-then-wait. The backend round-trip (kubectl/helm/glab) still happens,
// it just no longer gates first paint.
//
// VERSIONED: the prefix carries a schema version. When a cached payload's SHAPE
// changes (e.g. pods went from raw text → Pod[]), bump CACHE_VERSION — old
// entries are then ignored AND pruned, so a stale value can never be fed back
// into a component as the wrong type (which previously crashed a keyed {#each}).
const CACHE_VERSION = 2;
const PREFIX = `anvil-cache:v${CACHE_VERSION}:`;
const LEGACY_PREFIX = "anvil-cache:";

// One-time prune of any cache entry not matching the current version (runs when
// this module is first imported).
if (typeof localStorage !== "undefined") {
  try {
    for (let i = localStorage.length - 1; i >= 0; i--) {
      const k = localStorage.key(i);
      if (k && k.startsWith(LEGACY_PREFIX) && !k.startsWith(PREFIX)) localStorage.removeItem(k);
    }
  } catch {
    /* ignore */
  }
}

export function readCache<T>(key: string): T | null {
  if (typeof localStorage === "undefined") return null;
  try {
    const raw = localStorage.getItem(PREFIX + key);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}

export function writeCache<T>(key: string, value: T): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(PREFIX + key, JSON.stringify(value));
  } catch {
    /* quota / serialization — ignore, cache is best-effort */
  }
}
