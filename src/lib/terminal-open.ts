// #20 Terminal file-path links — a terminal publishes a clicked file path here;
// the shell (+page) opens it in the editor (optionally at a line).
import { writable } from "svelte/store";

export const terminalOpenPath = writable<{ path: string; line?: number } | null>(null);
