import { writable, get } from "svelte/store";

export type Density = "compact" | "regular";

export const density = writable<Density>("regular");

export function applyDensity(d: Density) {
  document.documentElement.dataset.density = d;
  localStorage.setItem("anvil-density", d);
  density.set(d);
}

export function initDensity() {
  const saved = localStorage.getItem("anvil-density") as Density | null;
  applyDensity(saved === "compact" ? "compact" : "regular");
}

export function toggleDensity() {
  applyDensity(get(density) === "compact" ? "regular" : "compact");
}
