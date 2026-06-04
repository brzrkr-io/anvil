import { writable, get } from "svelte/store";

// Window translucency. 1 = fully opaque (no transparency); lower values mix the
// shell surfaces toward transparent so the macOS vibrancy material shows through
// as a blurred backdrop. Drives the `--win-alpha` custom property.
const MIN = 0.3, MAX = 1;

function load(): number {
  if (typeof localStorage === "undefined") return 1;
  const n = Number(localStorage.getItem("anvil-win-opacity"));
  return n >= MIN && n <= MAX ? n : 1;
}

export const windowOpacity = writable<number>(load());

export function applyOpacity(n: number): void {
  const v = Math.round(Math.max(MIN, Math.min(MAX, n)) * 100) / 100;
  if (typeof document !== "undefined") {
    const root = document.documentElement;
    // Surface tint = the slider value directly. The native vibrancy material
    // behind the webview provides the blur and keeps it readable, so we don't
    // floor or add CSS blur here. `translucent` lets the editor/terminal panes
    // go see-through so the blur shows through them too.
    root.style.setProperty("--win-alpha", String(v));
    root.classList.toggle("translucent", v < 1);
  }
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-win-opacity", String(v));
  windowOpacity.set(v);
}

export function initOpacity(): void {
  applyOpacity(get(windowOpacity));
}
