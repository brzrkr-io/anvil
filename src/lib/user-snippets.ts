// User snippet pack (#3 / #92): a shareable list persisted in localStorage,
// merged into the editor's completions per-extension by cm-snippets. These are
// pure storage helpers with NO CodeMirror dependency, so the command palette can
// import them without dragging the CodeMirror vendor chunk into the cold-start
// graph (cm-snippets.ts itself still pulls CodeMirror for the completion source).
export type UserSnippet = { ext: string; label: string; template: string };
const USER_KEY = "anvil-user-snippets";

export function getUserSnippets(): UserSnippet[] {
  if (typeof localStorage === "undefined") return [];
  try { return JSON.parse(localStorage.getItem(USER_KEY) || "[]"); } catch { return []; }
}
function saveUserSnippets(list: UserSnippet[]) {
  if (typeof localStorage !== "undefined") localStorage.setItem(USER_KEY, JSON.stringify(list));
}
export function addUserSnippet(s: UserSnippet) {
  if (!s.ext || !s.label || !s.template) return;
  const next = [...getUserSnippets().filter((x) => !(x.ext === s.ext && x.label === s.label)), s];
  saveUserSnippets(next);
}
export function removeUserSnippet(ext: string, label: string) {
  saveUserSnippets(getUserSnippets().filter((x) => !(x.ext === ext && x.label === label)));
}
