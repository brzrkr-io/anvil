// Map a file path to a CodeMirror 6 language extension (replaces Monaco's
// built-in language detection). Returns [] for unknown types (plain text).
import type { Extension } from "@codemirror/state";
import { StreamLanguage } from "@codemirror/language";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { markdown } from "@codemirror/lang-markdown";
import { rust } from "@codemirror/lang-rust";
import { python } from "@codemirror/lang-python";
import { sql } from "@codemirror/lang-sql";
import { xml } from "@codemirror/lang-xml";
import { yaml } from "@codemirror/lang-yaml";
import { cpp } from "@codemirror/lang-cpp";
import { go } from "@codemirror/lang-go";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { lua } from "@codemirror/legacy-modes/mode/lua";
import { ruby } from "@codemirror/legacy-modes/mode/ruby";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";

export function cmLang(path: string): Extension[] {
  const file = path.split("/").pop()?.toLowerCase() ?? "";
  if (file === "dockerfile") return [StreamLanguage.define(dockerFile)];
  if (file === "makefile") return [];
  const ext = file.includes(".") ? file.split(".").pop()! : "";
  switch (ext) {
    case "ts": return [javascript({ typescript: true })];
    case "tsx": return [javascript({ typescript: true, jsx: true })];
    case "jsx": return [javascript({ jsx: true })];
    case "js": case "mjs": case "cjs": return [javascript()];
    case "json": return [json()];
    case "css": case "scss": case "less": return [css()];
    case "html": case "htm": case "svelte": case "vue": return [html()];
    case "md": case "markdown": return [markdown()];
    case "rs": return [rust()];
    case "py": return [python()];
    case "sql": return [sql()];
    case "xml": return [xml()];
    case "yaml": case "yml": return [yaml()];
    case "c": case "h": case "cpp": case "hpp": case "cc": case "cxx": return [cpp()];
    case "go": return [go()];
    case "sh": case "bash": case "zsh": case "fish": return [StreamLanguage.define(shell)];
    case "toml": return [StreamLanguage.define(toml)];
    case "ini": case "conf": case "properties": case "env": return [StreamLanguage.define(properties)];
    case "lua": return [StreamLanguage.define(lua)];
    case "rb": return [StreamLanguage.define(ruby)];
    default: return [];
  }
}
