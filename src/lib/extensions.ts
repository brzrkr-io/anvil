import { writable, get } from "svelte/store";

// Extension model (§F #69). DevOps/Caldera integrations are modelled as
// first-party extensions; the store UI (#71) enables/disables them and the rail
// gates each integration's surface on whether its extension is enabled.

export interface ExtManifest {
  id: string;
  name: string;
  description: string;
  /** Which rail view this extension contributes to (if any). */
  rail?: "devops" | "caldera";
  /** Declared capabilities — surfaced to the user, enforced later by the host. */
  permissions?: string[];
  /** First-party extensions ship enabled; others are "available" to install. */
  builtin: boolean;
}

export const EXTENSIONS: ExtManifest[] = [
  { id: "kubernetes", name: "Kubernetes", description: "Contexts, pods, logs, exec.", rail: "devops", permissions: ["exec:kubectl"], builtin: true },
  { id: "github-actions", name: "GitHub Actions", description: "Workflow runs + re-run.", rail: "devops", permissions: ["exec:gh"], builtin: true },
  { id: "caldera", name: "Caldera", description: "AI control-plane bridge.", rail: "caldera", permissions: ["net:127.0.0.1:4175"], builtin: true },
  { id: "grafana", name: "Grafana", description: "Dashboards (XFO proxy planned).", permissions: ["net"], builtin: false },
  { id: "terraform", name: "Terraform", description: "plan / apply (planned).", permissions: ["exec:terraform"], builtin: false },
  { id: "aws", name: "AWS", description: "Profiles + SSO (planned).", permissions: ["exec:aws"], builtin: false },
];

function load(): Record<string, boolean> {
  if (typeof localStorage === "undefined") return {};
  try { return JSON.parse(localStorage.getItem("anvil-ext") || "{}"); } catch { return {}; }
}

export const extEnabled = writable<Record<string, boolean>>(load());

/** Enabled state with the built-in default applied when unset. */
export function isExtEnabled(id: string, map: Record<string, boolean>): boolean {
  const v = map[id];
  if (v !== undefined) return v;
  return EXTENSIONS.find((x) => x.id === id)?.builtin ?? false;
}

export function toggleExt(id: string) {
  const map = { ...get(extEnabled) };
  map[id] = !isExtEnabled(id, map);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-ext", JSON.stringify(map));
  extEnabled.set(map);
}

/** True if a rail view should be shown (no gating extensions, or ≥1 enabled). */
export function railEnabled(rail: string, map: Record<string, boolean>): boolean {
  const gating = EXTENSIONS.filter((x) => x.rail === rail);
  if (!gating.length) return true;
  return gating.some((x) => isExtEnabled(x.id, map));
}
