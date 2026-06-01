import { writable } from "svelte/store";

export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  text: string;
}

export const toasts = writable<Toast[]>([]);

let seq = 0;

export function toast(text: string, kind: Toast["kind"] = "info", ttl = 3500): void {
  const id = ++seq;
  toasts.update((all) => [...all, { id, kind, text }]);
  setTimeout(() => dismiss(id), ttl);
}

export function dismiss(id: number): void {
  toasts.update((all) => all.filter((t) => t.id !== id));
}
