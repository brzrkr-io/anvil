import { writable, get } from "svelte/store";

// Layout preferences that aren't theme/font/density. Persisted in localStorage.

function loadBool(key: string, def: boolean): boolean {
  if (typeof localStorage === "undefined") return def;
  const v = localStorage.getItem(key);
  return v === null ? def : v === "1";
}

// Auto-hide the left activity rail: collapse it off-screen and slide it back in
// when the pointer reaches the left edge (reclaims horizontal space).
export const autoHideRail = writable<boolean>(loadBool("anvil-autohide-rail", false));

export function setAutoHideRail(v: boolean) {
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-autohide-rail", v ? "1" : "0");
  autoHideRail.set(v);
}

export function toggleAutoHideRail() {
  setAutoHideRail(!get(autoHideRail));
}

// Focus dimming (#65): fade inactive workspace panes so the focused pane stands out.
export const focusDimming = writable<boolean>(loadBool("anvil-focus-dim", false));

export function setFocusDimming(v: boolean) {
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-focus-dim", v ? "1" : "0");
  focusDimming.set(v);
}

export function toggleFocusDimming() {
  setFocusDimming(!get(focusDimming));
}

// Auto-cd (#19): when on, the active terminal follows the open file's directory.
export const terminalAutoCd = writable<boolean>(loadBool("anvil-term-autocd", false));
export function toggleTerminalAutoCd() {
  const v = !get(terminalAutoCd);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-term-autocd", v ? "1" : "0");
  terminalAutoCd.set(v);
}
