// Lets the agent read terminal output (#32). Each live Terminal registers a
// reader that returns its recent parsed buffer text (ANSI already interpreted by
// xterm). Decoupled so AgentPanel never holds a terminal reference.
import { writable } from "svelte/store";

const readers = new Map<string, () => string>();

export function registerTerminal(id: string, read: () => string) {
  readers.set(id, read);
}
export function unregisterTerminal(id: string) {
  readers.delete(id);
}
export function readTerminal(id: string): string {
  try { return readers.get(id)?.() ?? ""; } catch { return ""; }
}

// Broadcast input (#15): when on, a keystroke in any terminal is mirrored to
// every live terminal. `liveTerminals()` is the set of registered PTY ids.
export const broadcastInput = writable(false);
export function liveTerminals(): string[] { return [...readers.keys()]; }
