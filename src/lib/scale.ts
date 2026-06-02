import { writable, get } from "svelte/store";

// Global UI zoom — scales the entire app (chrome, editor, terminal) at once.
// Drives a `--scale` custom property; `.app` uses CSS `zoom` and inverse-sizes
// (width/height = 100vw/vh ÷ scale) so it fills the viewport without overflow.
// `zoom` (not `transform: scale`) is required so pointer coordinates stay in the
// same CSS-pixel grid as the content — otherwise editor selection and click-
// anchored popovers drift from the cursor.
const MIN = 0.7, MAX = 1.8, STEP = 0.1;

function load(): number {
  if (typeof localStorage === "undefined") return 1;
  const n = Number(localStorage.getItem("anvil-ui-scale"));
  return n >= MIN && n <= MAX ? n : 1;
}

export const uiScale = writable<number>(load());

export function applyScale(n: number): void {
  const v = Math.round(Math.max(MIN, Math.min(MAX, n)) * 100) / 100;
  if (typeof document !== "undefined") {
    document.documentElement.style.setProperty("--scale", String(v));
    // Only engage the transform when actually zoomed. At 100% the transform is
    // dropped entirely so the terminal's WebGL canvas renders natively (no
    // GPU resample) — much faster.
    if (v === 1) delete document.documentElement.dataset.scaled;
    else document.documentElement.dataset.scaled = "1";
  }
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-ui-scale", String(v));
  uiScale.set(v);
}

export function initScale(): void {
  applyScale(get(uiScale));
}
export function bumpScale(delta: number): void {
  applyScale(get(uiScale) + delta * STEP);
}
export function resetScale(): void {
  applyScale(1);
}
