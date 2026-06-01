// #3 Snippets engine. Per-language snippet completions registered via
// languageData so they merge with basicSetup's autocompletion for files
// without an LSP. (LSP-backed files get the server's own snippets, which take
// over via cm-lsp's `override`.) Template syntax is CM6's: ${1:label} fields,
// ${} exit point — Tab cycles.
import { snippetCompletion, type CompletionContext, type CompletionResult, type Completion } from "@codemirror/autocomplete";
import { EditorState, type Extension } from "@codemirror/state";

type Snip = [label: string, template: string, detail?: string];

const SH: Snip[] = [
  ["#!", "#!/usr/bin/env bash\nset -euo pipefail\n\n${}", "bash header"],
  ["for", "for ${1:x} in ${2:list}; do\n\t${}\ndone", "for loop"],
  ["if", "if ${1:cond}; then\n\t${}\nfi", "if"],
  ["fn", "${1:name}() {\n\t${}\n}", "function"],
  ["case", "case \"${1:var}\" in\n\t${2:pat}) ${} ;;\nesac", "case"],
];

const PY: Snip[] = [
  ["def", "def ${1:name}(${2:args}):\n\t${}", "function"],
  ["class", "class ${1:Name}:\n\tdef __init__(self${2:, args}):\n\t\t${}", "class"],
  ["for", "for ${1:x} in ${2:it}:\n\t${}", "for loop"],
  ["main", 'if __name__ == "__main__":\n\t${}', "main guard"],
  ["try", "try:\n\t${1:pass}\nexcept ${2:Exception} as e:\n\t${}", "try/except"],
];

const JS: Snip[] = [
  ["fn", "function ${1:name}(${2:args}) {\n\t${}\n}", "function"],
  ["afn", "const ${1:name} = (${2:args}) => {\n\t${}\n};", "arrow fn"],
  ["for", "for (const ${1:x} of ${2:list}) {\n\t${}\n}", "for-of"],
  ["log", "console.log(${});", "log"],
  ["try", "try {\n\t${1}\n} catch (${2:e}) {\n\t${}\n}", "try/catch"],
];

const RS: Snip[] = [
  ["fn", "fn ${1:name}(${2:args}) ${3:-> ()} {\n\t${}\n}", "function"],
  ["match", "match ${1:expr} {\n\t${2:pat} => ${},\n}", "match"],
  ["impl", "impl ${1:Type} {\n\t${}\n}", "impl block"],
  ["test", "#[test]\nfn ${1:name}() {\n\t${}\n}", "test"],
];

const GO: Snip[] = [
  ["fn", "func ${1:name}(${2:args}) ${3:error} {\n\t${}\n}", "function"],
  ["iferr", "if err != nil {\n\treturn ${1:err}\n}\n${}", "if err"],
  ["for", "for ${1:i} := range ${2:list} {\n\t${}\n}", "for-range"],
  ["main", "func main() {\n\t${}\n}", "main"],
];

const YAML: Snip[] = [
  ["k8s-deploy", "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: ${1:app}\nspec:\n  replicas: ${2:1}\n  selector:\n    matchLabels:\n      app: ${1:app}\n  template:\n    metadata:\n      labels:\n        app: ${1:app}\n    spec:\n      containers:\n        - name: ${1:app}\n          image: ${}", "k8s Deployment"],
  ["k8s-svc", "apiVersion: v1\nkind: Service\nmetadata:\n  name: ${1:app}\nspec:\n  selector:\n    app: ${1:app}\n  ports:\n    - port: ${2:80}\n      targetPort: ${}", "k8s Service"],
];

const TF: Snip[] = [
  ["resource", 'resource "${1:type}" "${2:name}" {\n\t${}\n}', "resource"],
  ["variable", 'variable "${1:name}" {\n\ttype = ${2:string}\n}', "variable"],
  ["output", 'output "${1:name}" {\n\tvalue = ${}\n}', "output"],
];

const DOCKER: Snip[] = [
  ["from", "FROM ${1:image}:${2:tag}\nWORKDIR ${3:/app}\n${}", "FROM"],
];

const BY_EXT: Record<string, Snip[]> = {
  sh: SH, bash: SH, zsh: SH,
  py: PY,
  ts: JS, tsx: JS, js: JS, jsx: JS, mjs: JS, cjs: JS,
  rs: RS,
  go: GO,
  yaml: YAML, yml: YAML,
  tf: TF, hcl: TF,
};

// User snippet pack (#3 / #92): a shareable list persisted in localStorage,
// merged in per-extension. Each entry is { ext, label, template }.
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

function snipsFor(path: string): Completion[] {
  const file = path.split("/").pop()?.toLowerCase() ?? "";
  let set: Snip[] | undefined;
  let ext = "";
  if (file === "dockerfile") { set = DOCKER; ext = "dockerfile"; }
  else {
    ext = file.includes(".") ? file.split(".").pop()! : "";
    set = BY_EXT[ext];
  }
  const builtin = (set ?? []).map(([label, tpl, detail]) => snippetCompletion(tpl, { label, type: "snippet", detail: detail ?? "snippet" }));
  const user = getUserSnippets()
    .filter((u) => u.ext === ext)
    .map((u) => snippetCompletion(u.template, { label: u.label, type: "snippet", detail: "user snippet" }));
  return [...user, ...builtin];
}

export function cmSnippets(path: string): Extension {
  const options = snipsFor(path);
  if (!options.length) return [];
  const source = (ctx: CompletionContext): CompletionResult | null => {
    const word = ctx.matchBefore(/[\w$#!]+/);
    if (!word && !ctx.explicit) return null;
    return { from: word ? word.from : ctx.pos, options, validFor: /^[\w$#!]*$/ };
  };
  return EditorState.languageData.of(() => [{ autocomplete: source }]);
}
