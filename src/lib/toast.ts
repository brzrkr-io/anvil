import { writable } from "svelte/store";

export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  text: string;
}

// A persisted notification record: every toast is also archived here so the
// user can review what scrolled past (the Notification Center, #94).
export interface Notification {
  id: number;
  kind: Toast["kind"];
  text: string;
  ts: number; // epoch ms
  read: boolean;
}

export const toasts = writable<Toast[]>([]);
export const notifications = writable<Notification[]>(loadHistory());

const HISTORY_KEY = "anvil-notifications";
const HISTORY_CAP = 200;

function loadHistory(): Notification[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    const arr = raw ? (JSON.parse(raw) as Notification[]) : [];
    return Array.isArray(arr) ? arr : [];
  } catch {
    return [];
  }
}

function saveHistory(all: Notification[]): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(all));
  } catch {
    /* quota — best-effort */
  }
}

let seq = 0;

// `now` is injectable so tests are deterministic (Date.now is otherwise impure).
export function toast(
  text: string,
  kind: Toast["kind"] = "info",
  ttl = 3500,
  now: number = Date.now(),
): void {
  const id = ++seq;
  toasts.update((all) => [...all, { id, kind, text }]);
  notifications.update((all) => {
    const next = [{ id, kind, text, ts: now, read: false }, ...all].slice(0, HISTORY_CAP);
    saveHistory(next);
    return next;
  });
  setTimeout(() => dismiss(id), ttl);
}

export function dismiss(id: number): void {
  toasts.update((all) => all.filter((t) => t.id !== id));
}

export function markAllRead(): void {
  notifications.update((all) => {
    const next = all.map((n) => (n.read ? n : { ...n, read: true }));
    saveHistory(next);
    return next;
  });
}

export function clearNotifications(): void {
  notifications.set([]);
  saveHistory([]);
}
