import { writable } from "svelte/store";

// Aggregated LSP diagnostics across all files, for the Problems list (#23).
// Deliberately monaco-free so consumers (e.g. the command palette) can read it
// without pulling Monaco into the startup bundle.

export type Problem = { path: string; line: number; message: string; severity: number };

const byFile = new Map<string, Problem[]>();
export const problems = writable<Problem[]>([]);

export function setFileProblems(path: string, items: Problem[]) {
  if (items.length) byFile.set(path, items);
  else byFile.delete(path);
  problems.set([...byFile.values()].flat().sort((a, b) => a.severity - b.severity));
}
