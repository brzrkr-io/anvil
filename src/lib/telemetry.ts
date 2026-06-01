// #93 Telemetry — strictly local-first. Events are appended to a capped ring in
// localStorage ONLY when the user opts in, and are never transmitted anywhere.
// The user can view or clear them at any time.
import { writable, get } from "svelte/store";

const KEY = "anvil-telemetry";
const ENABLED_KEY = "anvil-telemetry-on";
const CAP = 500;

export type TelemetryEvent = { ts: number; name: string; data?: Record<string, unknown> };

function loadBool(key: string, def: boolean): boolean {
  if (typeof localStorage === "undefined") return def;
  const v = localStorage.getItem(key);
  return v === null ? def : v === "1";
}

export const telemetryEnabled = writable<boolean>(loadBool(ENABLED_KEY, false));

export function setTelemetry(v: boolean) {
  if (typeof localStorage !== "undefined") localStorage.setItem(ENABLED_KEY, v ? "1" : "0");
  telemetryEnabled.set(v);
}
export function toggleTelemetry() { setTelemetry(!get(telemetryEnabled)); }

export function getEvents(): TelemetryEvent[] {
  if (typeof localStorage === "undefined") return [];
  try { return JSON.parse(localStorage.getItem(KEY) || "[]"); } catch { return []; }
}

export function clearEvents() {
  if (typeof localStorage !== "undefined") localStorage.removeItem(KEY);
}

// No-op unless the user opted in. `ts` is supplied by the caller's clock at log
// time (Date.now is fine in app code; this module just records what it's given).
export function logEvent(name: string, data?: Record<string, unknown>) {
  if (!get(telemetryEnabled)) return;
  try {
    const evs = getEvents();
    evs.push({ ts: Date.now(), name, data });
    localStorage.setItem(KEY, JSON.stringify(evs.slice(-CAP)));
  } catch { /* ignore quota / serialization errors */ }
}
