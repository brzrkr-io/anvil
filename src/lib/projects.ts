// Multi-project workspace. A project is just a folder (git repo / dir). One is
// active at a time; the active project's path drives the app's `cwd`, so SCM /
// Explorer / k8s / CI / Terraform all follow it. Terminals are tagged with the
// project they belong to and grouped under it in the sidebar.

import { writable, get } from "svelte/store";

export interface Project {
  id: string;
  name: string;
  path: string;
}

interface Persisted {
  list: Project[];
  active: string;
}

function load(): Persisted {
  if (typeof localStorage === "undefined") return { list: [], active: "" };
  try {
    const r = JSON.parse(localStorage.getItem("anvil-projects") || "{}");
    if (Array.isArray(r.list)) return { list: r.list, active: r.active || r.list[0]?.id || "" };
  } catch { /* ignore */ }
  return { list: [], active: "" };
}

const init = load();
export const projects = writable<Project[]>(init.list);
export const activeProject = writable<string>(init.active);

let counter = 0;
function pid(): string {
  return `p${Date.now().toString(36)}${(counter++).toString(36)}`;
}

function persist(): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem("anvil-projects", JSON.stringify({ list: get(projects), active: get(activeProject) }));
  } catch { /* quota — ignore */ }
}

/// Add a project for `path` (or focus the existing one) and make it active.
export function addProject(path: string): Project {
  const clean = path.replace(/\/+$/, "");
  if (!clean) return { id: "", name: "", path: "" };
  const existing = get(projects).find((p) => p.path === clean);
  if (existing) {
    activeProject.set(existing.id);
    persist();
    return existing;
  }
  const proj: Project = { id: pid(), name: clean.split("/").pop() || clean, path: clean };
  projects.update((l) => [...l, proj]);
  activeProject.set(proj.id);
  persist();
  return proj;
}

export function removeProject(id: string): void {
  projects.update((l) => l.filter((p) => p.id !== id));
  if (get(activeProject) === id) activeProject.set(get(projects)[0]?.id ?? "");
  persist();
}

export function setActiveProject(id: string): void {
  activeProject.set(id);
  persist();
}

export function activeProjectPath(): string {
  const a = get(activeProject);
  return get(projects).find((p) => p.id === a)?.path ?? "";
}
