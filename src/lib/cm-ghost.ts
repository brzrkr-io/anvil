// #34 Inline ghost-text completion. A single grey suggestion is rendered after
// the cursor; Tab accepts it, Escape dismisses. The suggestion source is
// pluggable — here it's the LSP top completion item that strictly extends the
// word being typed. (An LLM source can replace `suggest` without touching the
// mechanism.)
import { StateField, StateEffect, type Extension, Prec } from "@codemirror/state";
import { EditorView, Decoration, type DecorationSet, WidgetType, ViewPlugin, type ViewUpdate, keymap } from "@codemirror/view";
import { completionStatus } from "@codemirror/autocomplete";
import { req, fileUri } from "$lib/lsp";
import { invoke } from "@tauri-apps/api/core";
import { llmCreds } from "$lib/accounts";

type Ghost = { from: number; text: string };

const setGhost = StateEffect.define<Ghost | null>();

const ghostField = StateField.define<Ghost | null>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(setGhost)) return e.value;
    // Any doc change or cursor move invalidates a stale ghost until refreshed.
    if (value && (tr.docChanged || tr.selection)) return null;
    return value;
  },
});

class GhostWidget extends WidgetType {
  text: string;
  constructor(text: string) { super(); this.text = text; }
  eq(other: GhostWidget) { return other.text === this.text; }
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-ghost";
    span.textContent = this.text;
    return span;
  }
  get estimatedHeight() { return -1; }
  ignoreEvent() { return false; }
}

const ghostDeco = EditorView.decorations.compute([ghostField], (state) => {
  const g = state.field(ghostField);
  if (!g || !g.text) return Decoration.none;
  return Decoration.set([Decoration.widget({ widget: new GhostWidget(g.text), side: 1 }).range(g.from)]);
});

export function acceptGhost(view: EditorView): boolean {
  const g = view.state.field(ghostField, false);
  if (!g || !g.text) return false;
  view.dispatch({
    changes: { from: g.from, insert: g.text },
    selection: { anchor: g.from + g.text.length },
    effects: setGhost.of(null),
  });
  return true;
}

function clearGhost(view: EditorView): boolean {
  const g = view.state.field(ghostField, false);
  if (!g) return false;
  view.dispatch({ effects: setGhost.of(null) });
  return true;
}

// Driver: debounce after typing, ask the LSP for the top completion, and if it
// strictly extends the current word, show the remainder as a ghost.
function ghostDriver(lang: string, path: string): Extension {
  const uri = fileUri(path);
  return ViewPlugin.fromClass(class {
    timer: ReturnType<typeof setTimeout> | null = null;
    update(u: ViewUpdate) {
      if (!u.docChanged) return;
      // Only the user typing forward at a single cursor triggers a suggestion.
      const sel = u.state.selection.main;
      if (!sel.empty) return;
      if (this.timer) clearTimeout(this.timer);
      const view = u.view;
      this.timer = setTimeout(() => this.run(view), 140);
    }
    async run(view: EditorView) {
      const state = view.state;
      // Don't fight the autocomplete popup.
      if (completionStatus(state) === "active") return;
      const pos = state.selection.main.head;
      const line = state.doc.lineAt(pos);
      const before = state.sliceDoc(line.from, pos);
      const wm = /[\w$]+$/.exec(before);
      const word = wm ? wm[0] : "";
      if (!word) return;
      let items: any[] = [];
      try {
        const r = await req(lang, "textDocument/completion", {
          textDocument: { uri },
          position: { line: line.number - 1, character: pos - line.from },
        });
        items = Array.isArray(r) ? r : r?.items ?? [];
      } catch { return; }
      if (view.state.selection.main.head !== pos) return; // moved while awaiting
      const label = (it: any) => (typeof it.insertText === "string" ? it.insertText : it.label) as string;
      const hit = items
        .map(label)
        .filter((l) => typeof l === "string" && l.length > word.length && l.startsWith(word) && /^[\w$]+$/.test(l))
        .sort((a, b) => a.length - b.length)[0];
      if (!hit) { clearGhost(view); return; }
      view.dispatch({ effects: setGhost.of({ from: pos, text: hit.slice(word.length) }) });
    }
    destroy() { if (this.timer) clearTimeout(this.timer); }
  });
}

// LLM-sourced driver: ask a model for a single-line continuation after the
// cursor (slower, so debounced harder). Uses the configured utility model.
function ghostDriverLLM(): Extension {
  return ViewPlugin.fromClass(class {
    timer: ReturnType<typeof setTimeout> | null = null;
    update(u: ViewUpdate) {
      if (!u.docChanged || !u.state.selection.main.empty) return;
      if (this.timer) clearTimeout(this.timer);
      const view = u.view;
      this.timer = setTimeout(() => this.run(view), 450);
    }
    async run(view: EditorView) {
      const state = view.state;
      if (completionStatus(state) === "active") return;
      const pos = state.selection.main.head;
      const before = state.sliceDoc(Math.max(0, pos - 2000), pos);
      if (!before.trim()) return;
      try {
        const { base, apiKey } = await llmCreds();
        const models = await invoke<string[]>("llm_models", { base, apiKey }).catch(() => [] as string[]);
        const util = (typeof localStorage !== "undefined" && localStorage.getItem("anvil-util-model")) || "";
        const model = (util && models.includes(util) ? util : models[0]) ?? "";
        const reply = await invoke<string>("llm_chat", {
          model,
          messages: [
            { role: "system", content: "You are an inline code-completion engine. Output ONLY the continuation of the code right after the cursor — no explanation, no markdown fences, a single short line." },
            { role: "user", content: before },
          ],
          base,
          apiKey,
        });
        if (view.state.selection.main.head !== pos) return;
        const ghost = (reply.replace(/^```[\w]*\n?|```$/g, "").split("\n")[0] || "").slice(0, 160);
        if (ghost.trim()) view.dispatch({ effects: setGhost.of({ from: pos, text: ghost }) });
      } catch { /* no model / offline */ }
    }
    destroy() { if (this.timer) clearTimeout(this.timer); }
  });
}

export function cmGhost(lang: string, path: string, source: "lsp" | "llm" = "lsp"): Extension {
  return [
    ghostField,
    ghostDeco,
    source === "llm" ? ghostDriverLLM() : ghostDriver(lang, path),
    // High precedence so Tab accepts a ghost before indent/snippet handling, but
    // only when one is present (otherwise it returns false and falls through).
    Prec.highest(keymap.of([
      { key: "Tab", run: acceptGhost },
      { key: "Escape", run: clearGhost },
    ])),
  ];
}
