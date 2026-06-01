import { writable, get } from "svelte/store";

// Editable keymap (#82): user-defined shortcuts for the core actions, layered
// additively over the built-in defaults (so a custom binding never breaks the
// defaults). Persisted in localStorage.

export interface KeyAction { id: string; label: string; defaultCombo: string }

// The remappable global actions (must match the KEY_FNS table in +page.svelte).
export const KEY_ACTIONS: KeyAction[] = [
  { id: "new-terminal", label: "New terminal", defaultCombo: "⌘T" },
  { id: "reopen-tab", label: "Reopen closed tab", defaultCombo: "⌘⇧T" },
  { id: "command-palette", label: "Command palette", defaultCombo: "⌘K" },
  { id: "go-to-file", label: "Go to file", defaultCombo: "⌘P" },
  { id: "recent-files", label: "Recent files", defaultCombo: "⌘E" },
  { id: "open-file", label: "Open file", defaultCombo: "⌘O" },
  { id: "open-folder", label: "Open folder", defaultCombo: "⌘⇧O" },
  { id: "new-window", label: "New window", defaultCombo: "⌘N" },
  { id: "split-terminal", label: "Split / unsplit terminal", defaultCombo: "⌘D" },
  { id: "bottom-dock", label: "Toggle bottom terminal", defaultCombo: "⌘J" },
  { id: "explorer", label: "Toggle Explorer", defaultCombo: "⌘B" },
  { id: "zen", label: "Zen / terminal mode", defaultCombo: "⌘." },
  { id: "search", label: "Search workspace", defaultCombo: "⌘⇧F" },
];

/** Format a KeyboardEvent as a display combo, e.g. "⌘⇧P". */
export function comboOf(e: KeyboardEvent): string {
  const p: string[] = [];
  if (e.metaKey) p.push("⌘");
  if (e.ctrlKey) p.push("⌃");
  if (e.altKey) p.push("⌥");
  if (e.shiftKey) p.push("⇧");
  let k = e.key;
  if (k === " ") k = "Space";
  else if (k.length === 1) k = k.toUpperCase();
  p.push(k);
  return p.join("");
}

function load(): Record<string, string> {
  if (typeof localStorage === "undefined") return {};
  try { return JSON.parse(localStorage.getItem("anvil-keymap") || "{}"); } catch { return {}; }
}

export const keyOverrides = writable<Record<string, string>>(load());

export function setKeyOverride(id: string, combo: string) {
  const m = { ...get(keyOverrides), [id]: combo };
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-keymap", JSON.stringify(m));
  keyOverrides.set(m);
}
export function clearKeyOverride(id: string) {
  const m = { ...get(keyOverrides) };
  delete m[id];
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-keymap", JSON.stringify(m));
  keyOverrides.set(m);
}
export function comboFor(id: string, overrides: Record<string, string>): string {
  return overrides[id] ?? KEY_ACTIONS.find((a) => a.id === id)?.defaultCombo ?? "";
}

// #89 Keybinding presets. Each preset is a set of overrides for the global
// actions, matching that editor's muscle memory. "Anvil (default)" clears all
// overrides. (Editor-internal keys follow CodeMirror's own keymap.)
export const KEY_PRESETS: Record<string, Record<string, string>> = {
  "Anvil (default)": {},
  "VS Code": {
    "command-palette": "⌘⇧P",
    "go-to-file": "⌘P",
    "search": "⌘⇧F",
    "explorer": "⌘B",
    "split-terminal": "⌘\\",
  },
  "Zed": {
    "command-palette": "⌘⇧P",
    "go-to-file": "⌘P",
    "search": "⌘⇧F",
    "explorer": "⌘B",
  },
};

export function applyKeymapPreset(name: string): boolean {
  const p = KEY_PRESETS[name];
  if (!p) return false;
  if (typeof localStorage !== "undefined") localStorage.setItem("anvil-keymap", JSON.stringify(p));
  keyOverrides.set({ ...p });
  return true;
}
