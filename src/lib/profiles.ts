// Per-environment profiles (roadmap Tier 0 #4). A profile bundles the settings
// that define "where am I working" — kube context, AWS profile, namespace,
// GitLab host — so switching all surfaces at once is one action instead of four.
// Pure storage + CRUD; the UI applies a profile by invoking the relevant
// backend commands. Non-secret, so localStorage is fine.

export interface EnvProfile {
  name: string;
  kubeContext?: string;
  awsProfile?: string;
  namespace?: string;
  gitlabHost?: string;
}

const KEY = "anvil-env-profiles";
const ACTIVE_KEY = "anvil-env-active";

export function getProfiles(): EnvProfile[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

function write(list: EnvProfile[]): void {
  if (typeof localStorage !== "undefined") localStorage.setItem(KEY, JSON.stringify(list));
}

/** Insert or replace a profile by name (case-sensitive). Returns the new list. */
export function saveProfile(p: EnvProfile): EnvProfile[] {
  if (!p.name.trim()) return getProfiles();
  const list = getProfiles().filter((x) => x.name !== p.name);
  list.push(p);
  list.sort((a, b) => a.name.localeCompare(b.name));
  write(list);
  return list;
}

export function deleteProfile(name: string): EnvProfile[] {
  const list = getProfiles().filter((x) => x.name !== name);
  write(list);
  if (getActiveProfile() === name) setActiveProfile(null);
  return list;
}

export function getActiveProfile(): string | null {
  if (typeof localStorage === "undefined") return null;
  return localStorage.getItem(ACTIVE_KEY);
}

export function setActiveProfile(name: string | null): void {
  if (typeof localStorage === "undefined") return;
  if (name) localStorage.setItem(ACTIVE_KEY, name);
  else localStorage.removeItem(ACTIVE_KEY);
}

/** Short one-line summary of what a profile switches, for the palette hint. */
export function profileSummary(p: EnvProfile): string {
  return [
    p.kubeContext && `ctx ${p.kubeContext}`,
    p.awsProfile && `aws ${p.awsProfile}`,
    p.namespace && `ns ${p.namespace}`,
    p.gitlabHost && `glab ${p.gitlabHost}`,
  ]
    .filter(Boolean)
    .join(" · ");
}
