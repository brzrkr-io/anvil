import { writable, get } from "svelte/store";

// Window translucency. 1 = fully opaque (no transparency); lower values mix the
// shell surfaces toward transparent so the macOS vibrancy material shows through
// as a blurred backdrop. Drives the `--win-alpha` custom property.
const MIN = 0.5, MAX = 1;

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
    // Floor the *visual* surface alpha so the theme color always dominates the
    // backdrop — otherwise the desktop washes straight through. Slider 0.5..1
    // maps to surface 0.66..1; the raw slider value still drives the store.
    const surface = v >= 1 ? 1 : 0.66 + (v - MIN) * ((1 - 0.66) / (1 - MIN));
    const blur = v >= 1 ? 0 : Math.round((1 - v) * 64);
    root.style.setProperty("--win-alpha", surface.toFixed(3));
    root.style.setProperty("--win-blur", `${blur}px`);
    // Flag for the editor/terminal panes to drop their opaque backgrounds so the
    // frosted shell shows through uniformly (not just the chrome).
    root.classList.toggle("translucent", v < 1);
  }
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-win-opacity", String(v));
  windowOpacity.set(v);
}

export function initOpacity(): void {
  applyOpacity(get(windowOpacity));
}
