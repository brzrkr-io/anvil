import { writable, get } from "svelte/store";

function loadNum(key: string, def: number, lo: number, hi: number): number {
  if (typeof localStorage === "undefined") return def;
  const n = Number(localStorage.getItem(key));
  return n >= lo && n <= hi ? n : def;
}
function loadBool(key: string, def: boolean): boolean {
  if (typeof localStorage === "undefined") return def;
  const v = localStorage.getItem(key);
  return v === null ? def : v === "1";
}

export type CursorStyle = "block" | "bar" | "underline";
function loadStr<T extends string>(key: string, def: T, allowed: readonly T[]): T {
  if (typeof localStorage === "undefined") return def;
  const v = localStorage.getItem(key) as T | null;
  return v && allowed.includes(v) ? v : def;
}

export const termFontSize = writable<number>(loadNum("anvil-term-fs", 13, 8, 28));
export const termCursorBlink = writable<boolean>(loadBool("anvil-term-blink", true));
export const CURSOR_STYLES: CursorStyle[] = ["block", "bar", "underline"];
export const termCursorStyle = writable<CursorStyle>(loadStr("anvil-term-cursor", "block", CURSOR_STYLES));
export const termLineHeight = writable<number>(loadNum("anvil-term-lh", 1.2, 1.0, 2.0));
export const termLetterSpacing = writable<number>(loadNum("anvil-term-ls", 0, -2, 4));
// Scrollback ceiling (#76): caps per-terminal buffer memory. 0 = no scrollback.
export const termScrollback = writable<number>(loadNum("anvil-term-scrollback", 10000, 0, 200000));
export function setTermScrollback(n: number) {
  const v = Math.max(0, Math.min(200000, Math.round(n)));
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-term-scrollback", String(v));
  termScrollback.set(v);
}

export function setTermFontSize(n: number) {
  const v = Math.max(8, Math.min(28, Math.round(n)));
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-term-fs", String(v));
  termFontSize.set(v);
}
export function bumpTermFontSize(delta: number) {
  setTermFontSize(get(termFontSize) + delta);
}
export function toggleTermBlink() {
  const v = !get(termCursorBlink);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-term-blink", v ? "1" : "0");
  termCursorBlink.set(v);
}
export function setTermCursorStyle(s: CursorStyle) {
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-term-cursor", s);
  termCursorStyle.set(s);
}
export function bumpTermLineHeight(delta: number) {
  const v = Math.round((Math.max(1.0, Math.min(2.0, get(termLineHeight) + delta))) * 100) / 100;
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-term-lh", String(v));
  termLineHeight.set(v);
}
export function bumpTermLetterSpacing(delta: number) {
  const v = Math.max(-2, Math.min(4, get(termLetterSpacing) + delta));
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-term-ls", String(v));
  termLetterSpacing.set(v);
}
