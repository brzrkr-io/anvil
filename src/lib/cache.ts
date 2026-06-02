// Stale-while-revalidate cache for slow page data (kube pods, helm releases, CI
// pipelines, …). Persist the last successful result to localStorage so a page
// can render real data INSTANTLY on load, then refresh in the background — no
// spinner-then-wait. The backend round-trip (kubectl/helm/glab) still happens,
// it just no longer gates first paint.

const PREFIX = "anvil-cache:";

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
