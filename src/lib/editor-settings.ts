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

export const editorFontSize = writable<number>(loadNum("anvil-editor-fs", 13, 8, 32));
export const editorTabSize = writable<number>(loadNum("anvil-editor-tab", 2, 1, 8));
export const editorWordWrap = writable<boolean>(loadBool("anvil-editor-wrap", false));
export const editorMinimap = writable<boolean>(loadBool("anvil-editor-minimap", true));
export const editorLigatures = writable<boolean>(loadBool("anvil-editor-ligatures", true));
export const editorLineHeight = writable<number>(loadNum("anvil-editor-lh", 1.55, 1.0, 2.4));
export const editorLetterSpacing = writable<number>(loadNum("anvil-editor-ls", 0, -2, 4));
export const editorStickyScroll = writable<boolean>(loadBool("anvil-editor-sticky", true));
export const editorInlayHints = writable<boolean>(loadBool("anvil-editor-inlay", false));
export const editorFormatOnSave = writable<boolean>(loadBool("anvil-editor-fos", true));
export const editorBlameAlways = writable<boolean>(loadBool("anvil-editor-blame", false));
export const editorGhostText = writable<boolean>(loadBool("anvil-editor-ghost", false));
// #34 Ghost-text source: 'lsp' (top completion) or 'llm' (model continuation).
export const editorGhostSource = writable<"lsp" | "llm">(
  (typeof localStorage !== "undefined" && (localStorage.getItem("anvil-editor-ghost-src") as "lsp" | "llm")) || "lsp",
);
export function setGhostSource(s: "lsp" | "llm") {
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-ghost-src", s);
  editorGhostSource.set(s);
}
// Transient: set a 1-based line number to scroll the active editor to it (#37).
export const editorGoto = writable<number | null>(null);

export function setEditorFontSize(n: number) {
  const v = Math.max(8, Math.min(32, Math.round(n)));
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-fs", String(v));
  editorFontSize.set(v);
}

export function bumpEditorFontSize(delta: number) {
  setEditorFontSize(get(editorFontSize) + delta);
}

export function setEditorTabSize(n: number) {
  const v = Math.max(1, Math.min(8, Math.round(n)));
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-tab", String(v));
  editorTabSize.set(v);
}

export function toggleWordWrap() {
  const v = !get(editorWordWrap);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-wrap", v ? "1" : "0");
  editorWordWrap.set(v);
}

export function toggleMinimap() {
  const v = !get(editorMinimap);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-minimap", v ? "1" : "0");
  editorMinimap.set(v);
}

export function toggleLigatures() {
  const v = !get(editorLigatures);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-ligatures", v ? "1" : "0");
  editorLigatures.set(v);
}

export function bumpEditorLineHeight(delta: number) {
  const v = Math.round((Math.max(1.0, Math.min(2.4, get(editorLineHeight) + delta))) * 100) / 100;
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-lh", String(v));
  editorLineHeight.set(v);
}
export function bumpEditorLetterSpacing(delta: number) {
  const v = Math.max(-2, Math.min(4, get(editorLetterSpacing) + delta));
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-ls", String(v));
  editorLetterSpacing.set(v);
}
export function toggleStickyScroll() {
  const v = !get(editorStickyScroll);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-sticky", v ? "1" : "0");
  editorStickyScroll.set(v);
}
export function toggleInlayHints() {
  const v = !get(editorInlayHints);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-inlay", v ? "1" : "0");
  editorInlayHints.set(v);
}
export function toggleFormatOnSave() {
  const v = !get(editorFormatOnSave);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-fos", v ? "1" : "0");
  editorFormatOnSave.set(v);
}
export function toggleBlameAlways() {
  const v = !get(editorBlameAlways);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-blame", v ? "1" : "0");
  editorBlameAlways.set(v);
}
// #89 Vim editor mode (CodeMirror vim keybindings).
export const editorVimMode = writable<boolean>(loadBool("anvil-editor-vim", false));
export function toggleVimMode() {
  const v = !get(editorVimMode);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-vim", v ? "1" : "0");
  editorVimMode.set(v);
}

export function toggleGhostText() {
  const v = !get(editorGhostText);
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-editor-ghost", v ? "1" : "0");
  editorGhostText.set(v);
}
