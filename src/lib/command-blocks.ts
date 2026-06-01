// #12 Command blocks — capture OSC 133 shell-integration marks (prompt /
// command / output / exit) per terminal. This is the data foundation for
// collapsing per-command output and jumping between prompts; the exit code also
// drives a status indicator.
import { writable } from "svelte/store";

export type Block = { promptLine: number; exit: number };

const blocks = new Map<string, Block[]>();
let pending = new Map<string, number>(); // id -> prompt line awaiting exit

// Last command's exit code (null = none yet / still running).
export const lastExit = writable<number | null>(null);

export function recordPrompt(id: string, line: number) {
  pending.set(id, line);
}

export function recordExit(id: string, exit: number) {
  const promptLine = pending.get(id) ?? 0;
  const list = blocks.get(id) ?? [];
  list.push({ promptLine, exit });
  if (list.length > 1000) list.shift();
  blocks.set(id, list);
  lastExit.set(exit);
}

export function getBlocks(id: string): Block[] { return blocks.get(id) ?? []; }
export function clearBlocks(id: string) { blocks.delete(id); pending.delete(id); }
