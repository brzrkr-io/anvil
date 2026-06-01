// #37 Background agents queue — a simple FIFO of agent prompts plus a status
// surface. Tasks are drained one at a time into the agent (via agentSeed); the
// queue depth drives a status chip so backgrounded work stays visible.
import { writable, get } from "svelte/store";

export type QueuedTask = { id: number; prompt: string };

export const agentQueue = writable<QueuedTask[]>([]);
let seq = 0;

export function enqueueAgent(prompt: string) {
  if (!prompt.trim()) return;
  agentQueue.update((q) => [...q, { id: ++seq, prompt: prompt.trim() }]);
}

export function dequeueAgent(): QueuedTask | null {
  const q = get(agentQueue);
  if (!q.length) return null;
  agentQueue.set(q.slice(1));
  return q[0];
}

export function removeQueued(id: number) {
  agentQueue.update((q) => q.filter((t) => t.id !== id));
}

export function clearQueue() { agentQueue.set([]); }
