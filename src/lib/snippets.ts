// Command snippets — quick reusable shell commands run into the active terminal
// from the palette. Stored in localStorage; ships with a few DevOps defaults.

export interface Snippet {
  id: number;
  label: string;
  command: string;
}

const KEY = "anvil-snippets";

const DEFAULTS: Snippet[] = [
  { id: 1, label: "k8s: pods (all namespaces)", command: "kubectl get pods -A" },
  { id: 2, label: "k8s: failing pods", command: "kubectl get pods -A --field-selector=status.phase!=Running" },
  { id: 3, label: "flux: reconcile all", command: "flux reconcile source git flux-system" },
  { id: 4, label: "git: prune merged branches", command: "git branch --merged | grep -vE '^\\*|main|master' | xargs -r git branch -d" },
  { id: 5, label: "docker: prune", command: "docker system prune -f" },
];

export function getSnippets(): Snippet[] {
  if (typeof localStorage === "undefined") return DEFAULTS;
  const raw = localStorage.getItem(KEY);
  if (raw === null) return DEFAULTS;
  try {
    const v = JSON.parse(raw);
    return Array.isArray(v) ? v : DEFAULTS;
  } catch {
    return DEFAULTS;
  }
}

function save(list: Snippet[]) {
  if (typeof localStorage !== "undefined") localStorage.setItem(KEY, JSON.stringify(list));
}

function nextId(list: Snippet[]): number {
  return list.reduce((m, s) => Math.max(m, s.id), 0) + 1;
}

export function addSnippet(label: string, command: string): Snippet[] {
  const l = label.trim();
  const c = command.trim();
  if (!l || !c) return getSnippets();
  const list = getSnippets();
  const next = [...list, { id: nextId(list), label: l, command: c }];
  save(next);
  return next;
}

export function removeSnippet(id: number): Snippet[] {
  const next = getSnippets().filter((s) => s.id !== id);
  save(next);
  return next;
}
