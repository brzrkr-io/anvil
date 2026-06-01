import { writable, get } from "svelte/store";

// Font customization. UI font drives chrome (--font-ui); mono font drives
// terminal + editor + .mono (--font-mono). Bold toggles set weight per surface.
// Concrete family names (not CSS vars) are exported for xterm/Monaco, which
// measure glyphs and mis-render against a CSS var.

export type UiFont = "IBM Plex Sans" | "Inter" | "IBM Plex Mono" | "Maple Mono";
export type MonoFont =
  | "JetBrains Mono" | "Fira Code" | "Cascadia Code" | "Source Code Pro"
  | "IBM Plex Mono" | "Maple Mono";

export const UI_FONTS: UiFont[] = ["IBM Plex Sans", "Inter", "IBM Plex Mono", "Maple Mono"];
export const MONO_FONTS: MonoFont[] = [
  "JetBrains Mono", "Fira Code", "Cascadia Code", "Source Code Pro", "IBM Plex Mono", "Maple Mono",
];

const monoStack = (f: string) => `"${f}", "Symbols Nerd Font Mono", "SF Mono", Menlo, ui-monospace, monospace`;
const uiStack = (f: string) =>
  f.includes("Sans") ? `"${f}", -apple-system, system-ui, sans-serif` : monoStack(f);

function loadStr<T extends string>(key: string, def: T, allowed: readonly T[]): T {
  if (typeof localStorage === "undefined") return def;
  const v = localStorage.getItem(key) as T | null;
  return v && allowed.includes(v) ? v : def;
}
function loadBool(key: string, def: boolean): boolean {
  if (typeof localStorage === "undefined") return def;
  const v = localStorage.getItem(key);
  return v === null ? def : v === "1";
}

export const uiFont = writable<UiFont>(loadStr("anvil-ui-font", "IBM Plex Sans", UI_FONTS));
export const monoFont = writable<MonoFont>(loadStr("anvil-mono-font", "IBM Plex Mono", MONO_FONTS));
export const editorBold = writable<boolean>(loadBool("anvil-editor-bold", false));
export const termBold = writable<boolean>(loadBool("anvil-term-bold", false));

function applyUi(f: UiFont) {
  if (typeof document !== "undefined") document.documentElement.style.setProperty("--font-ui", uiStack(f));
}
function applyMono(f: MonoFont) {
  if (typeof document !== "undefined") document.documentElement.style.setProperty("--font-mono", monoStack(f));
}

export function setUiFont(f: UiFont) {
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-ui-font", f);
  uiFont.set(f);
  applyUi(f);
}
export function setMonoFont(f: MonoFont) {
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-mono-font", f);
  monoFont.set(f);
  applyMono(f);
}
export function toggleEditorBold() {
  const v = !get(editorBold);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-bold", v ? "1" : "0");
  editorBold.set(v);
}
export function toggleTermBold() {
  const v = !get(termBold);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-term-bold", v ? "1" : "0");
  termBold.set(v);
}

/** Concrete mono family stack for xterm/Monaco. */
export const monoFamily = () => monoStack(get(monoFont));

export function initFonts() {
  applyUi(get(uiFont));
  applyMono(get(monoFont));
}
