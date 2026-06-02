// CodeMirror 6 LSP extensions over the transport in lsp.ts. Provides completion,
// hover, diagnostics, go-to-definition, and rename. Document symbols are fetched
// on demand (fetchSymbols) for the command-palette outline.
import { invoke } from "@tauri-apps/api/core";
import { autocompletion, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { hoverTooltip, EditorView, ViewPlugin, keymap, Decoration, WidgetType, showTooltip, type Command, type DecorationSet, type Tooltip } from "@codemirror/view";
import { linter, forceLinting, type Diagnostic } from "@codemirror/lint";
import { Prec, StateField, StateEffect, type Extension, type Text } from "@codemirror/state";
import { req, fileUri, uriToPath, diagByPath, onDiagnostics, type RawDiag } from "$lib/lsp";
import { askText } from "$lib/dialog";

export interface RefLoc { path: string; line: number; col: number; }
export interface LspNav {
  onOpen: (path: string, line?: number, col?: number) => void;
  onReferences?: (refs: RefLoc[]) => void;
}

function offsetOf(doc: Text, line: number, ch: number): number {
  if (line >= doc.lines) return doc.length;
  const l = doc.line(line + 1);
  return Math.min(l.from + ch, l.to);
}
function lspPos(doc: Text, off: number) {
  const l = doc.lineAt(off);
  return { line: l.number - 1, character: off - l.from };
}

function hoverText(contents: any): string {
  if (contents == null) return "";
  if (typeof contents === "string") return contents;
  if (Array.isArray(contents)) return contents.map(hoverText).filter(Boolean).join("\n\n");
  if (typeof contents.value === "string") return contents.value;
  return "";
}

// Apply LSP text edits to a raw string (for files not open in this editor).
function applyLspEdits(text: string, edits: any[]): string {
  const starts = [0];
  for (let i = 0; i < text.length; i++) if (text[i] === "\n") starts.push(i + 1);
  const off = (l: number, c: number) => (starts[l] ?? text.length) + c;
  const ops = edits
    .map((e) => ({ from: off(e.range.start.line, e.range.start.character), to: off(e.range.end.line, e.range.end.character), insert: e.newText }))
    .sort((a, b) => b.from - a.from);
  for (const o of ops) text = text.slice(0, o.from) + o.insert + text.slice(o.to);
  return text;
}

const KIND: Record<number, string> = {
  2: "method", 3: "function", 4: "class", 5: "property", 6: "variable",
  7: "class", 8: "interface", 9: "namespace", 10: "property", 13: "enum",
  14: "keyword", 15: "text", 21: "constant", 22: "class", 25: "type",
};
const SEV: Record<number, Diagnostic["severity"]> = { 1: "error", 2: "warning", 3: "info", 4: "info" };

export function cmLsp(lang: string, path: string, nav: LspNav): Extension {
  const uri = fileUri(path);

  const completion = autocompletion({
    override: [async (ctx: CompletionContext): Promise<CompletionResult | null> => {
      const line = ctx.state.doc.lineAt(ctx.pos);
      const r = await req(lang, "textDocument/completion", {
        textDocument: { uri }, position: { line: line.number - 1, character: ctx.pos - line.from },
      });
      const items = Array.isArray(r) ? r : r?.items ?? [];
      if (!items.length) return null;
      const word = ctx.matchBefore(/[\w$]+/);
      if (!word && !ctx.explicit) return null;
      return {
        from: word ? word.from : ctx.pos,
        options: items.slice(0, 200).map((it: any) => ({
          label: it.label, detail: it.detail, type: KIND[it.kind] ?? "variable",
          info: hoverText(it.documentation) || undefined, apply: it.insertText ?? it.label,
        })),
      };
    }],
  });

  const hover = hoverTooltip(async (view, pos) => {
    const line = view.state.doc.lineAt(pos);
    const r = await req(lang, "textDocument/hover", {
      textDocument: { uri }, position: { line: line.number - 1, character: pos - line.from },
    });
    const value = hoverText(r?.contents);
    if (!value) return null;
    return { pos, create: () => { const dom = document.createElement("div"); dom.className = "cm-lsp-hover"; dom.textContent = value; return { dom }; } };
  });

  const lint = linter((view): Diagnostic[] => {
    const raw: RawDiag[] = diagByPath.get(path) ?? [];
    return raw.map((d) => {
      const from = offsetOf(view.state.doc, d.line, d.character);
      return { from, to: Math.max(from + 1, offsetOf(view.state.doc, d.endLine, d.endChar)), severity: SEV[d.severity] ?? "info", message: d.message };
    });
  }, { delay: 150 });

  const relint = ViewPlugin.define((view) => {
    const off = onDiagnostics((p) => { if (p === path) forceLinting(view); });
    return { destroy() { off(); } };
  });

  // ── Go to definition (F12 / ⌘-click) ──
  async function gotoDef(view: EditorView, pos: number): Promise<boolean> {
    const line = view.state.doc.lineAt(pos);
    const r = await req(lang, "textDocument/definition", {
      textDocument: { uri }, position: { line: line.number - 1, character: pos - line.from },
    });
    const loc = Array.isArray(r) ? r[0] : r;
    if (!loc) return false;
    const range = loc.range ?? loc.targetSelectionRange ?? loc.targetRange;
    nav.onOpen(uriToPath(loc.uri ?? loc.targetUri), (range?.start?.line ?? 0) + 1, (range?.start?.character ?? 0) + 1);
    return true;
  }
  const defKey: Command = (view) => { gotoDef(view, view.state.selection.main.head); return true; };
  const cmdClick = EditorView.domEventHandlers({
    mousedown(e, view) {
      if (!(e.metaKey || e.ctrlKey)) return false;
      const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
      if (pos == null) return false;
      e.preventDefault();
      gotoDef(view, pos);
      return true;
    },
  });

  // ── Rename (F2) ──
  const renameKey: Command = (view) => {
    const pos = view.state.selection.main.head;
    const w = view.state.wordAt(pos);
    const cur = w ? view.state.sliceDoc(w.from, w.to) : "";
    (async () => {
      const next = await askText({ title: "Rename symbol", value: cur });
      if (next == null || next === cur) return;
      const line = view.state.doc.lineAt(pos);
      const r = await req(lang, "textDocument/rename", {
        textDocument: { uri }, position: { line: line.number - 1, character: pos - line.from }, newName: next,
      });
      const changes: Record<string, any[]> = r?.changes
        ?? Object.fromEntries((r?.documentChanges ?? []).map((dc: any) => [dc.textDocument.uri, dc.edits]));
      for (const [u, edits] of Object.entries(changes)) {
        const p = uriToPath(u);
        if (p === path) {
          const ch = edits
            .map((e: any) => ({ from: offsetOf(view.state.doc, e.range.start.line, e.range.start.character), to: offsetOf(view.state.doc, e.range.end.line, e.range.end.character), insert: e.newText }))
            .sort((a: any, b: any) => a.from - b.from);
          view.dispatch({ changes: ch });
        } else {
          try {
            const text = await invoke<string>("read_file", { path: p });
            await invoke("write_file", { path: p, contents: applyLspEdits(text, edits) });
          } catch { /* skip unwritable */ }
        }
      }
    })();
    return true;
  };

  // Apply an LSP WorkspaceEdit (used by rename + code actions): current file via
  // a transaction, other files read+write on disk.
  async function applyWorkspaceEdit(view: EditorView, edit: any) {
    const changes: Record<string, any[]> = edit?.changes
      ?? Object.fromEntries((edit?.documentChanges ?? []).map((dc: any) => [dc.textDocument.uri, dc.edits]));
    for (const [u, edits] of Object.entries(changes)) {
      const p = uriToPath(u);
      if (p === path) {
        const ch = edits
          .map((e: any) => ({ from: offsetOf(view.state.doc, e.range.start.line, e.range.start.character), to: offsetOf(view.state.doc, e.range.end.line, e.range.end.character), insert: e.newText }))
          .sort((a: any, b: any) => a.from - b.from);
        view.dispatch({ changes: ch });
      } else {
        try {
          const text = await invoke<string>("read_file", { path: p });
          await invoke("write_file", { path: p, contents: applyLspEdits(text, edits) });
        } catch { /* skip unwritable */ }
      }
    }
  }

  // ── Code actions (⌘.) — quick-fix menu anchored at the cursor ──
  const codeActionKey: Command = (view) => {
    (async () => {
      const sel = view.state.selection.main;
      const diags = (diagByPath.get(path) ?? []).map((d) => ({
        range: { start: { line: d.line, character: d.character }, end: { line: d.endLine, character: d.endChar } },
        message: d.message, severity: d.severity,
      }));
      const r = await req(lang, "textDocument/codeAction", {
        textDocument: { uri },
        range: { start: lspPos(view.state.doc, sel.from), end: lspPos(view.state.doc, sel.to) },
        context: { diagnostics: diags },
      });
      const actions = (Array.isArray(r) ? r : []).filter((a: any) => a.edit || a.command);
      if (!actions.length) return;
      showActionMenu(view, sel.head, actions, async (a: any) => {
        if (a.edit) await applyWorkspaceEdit(view, a.edit);
        else if (a.command) await req(lang, "workspace/executeCommand", { command: a.command.command ?? a.command, arguments: a.command.arguments });
      });
    })();
    return true;
  };

  // ── Find references (⇧F12) ──
  const referencesKey: Command = (view) => {
    if (!nav.onReferences) return false;
    (async () => {
      const pos = view.state.selection.main.head;
      const r = await req(lang, "textDocument/references", {
        textDocument: { uri }, position: lspPos(view.state.doc, pos), context: { includeDeclaration: true },
      });
      const refs: RefLoc[] = (Array.isArray(r) ? r : []).map((l: any) => ({
        path: uriToPath(l.uri), line: (l.range?.start?.line ?? 0) + 1, col: (l.range?.start?.character ?? 0) + 1,
      }));
      nav.onReferences!(refs);
    })();
    return true;
  };

  const navKeys = keymap.of([
    { key: "F12", run: defKey, preventDefault: true },
    { key: "F2", run: renameKey, preventDefault: true },
    { key: "Shift-F12", run: referencesKey, preventDefault: true },
    { key: "Mod-.", run: codeActionKey, preventDefault: true },
    { key: "Shift-Alt-f", run: (view) => { formatDoc(lang, path, view, view.state.tabSize); return true; }, preventDefault: true },
  ]);

  return [completion, hover, lint, relint, cmdClick, signatureHelp(lang, uri), Prec.high(navKeys)];
}

// Floating quick-fix menu (plain DOM, themed via .cm-action-menu in app.css).
function showActionMenu(view: EditorView, pos: number, actions: any[], pick: (a: any) => void) {
  const coords = view.coordsAtPos(pos);
  if (!coords) return;
  const menu = document.createElement("div");
  menu.className = "cm-action-menu";
  menu.style.left = `${coords.left}px`;
  menu.style.top = `${coords.bottom + 4}px`;
  const close = () => { menu.remove(); document.removeEventListener("mousedown", onDoc, true); document.removeEventListener("keydown", onKey, true); };
  const onDoc = (e: MouseEvent) => { if (!menu.contains(e.target as Node)) close(); };
  const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") { e.preventDefault(); close(); } };
  for (const a of actions) {
    const b = document.createElement("button");
    b.textContent = a.title ?? "Fix";
    b.onclick = () => { close(); pick(a); };
    menu.appendChild(b);
  }
  document.body.appendChild(menu);
  document.addEventListener("mousedown", onDoc, true);
  document.addEventListener("keydown", onKey, true);
}

// ── Signature help: param popup triggered by "(" and "," ──
const setSig = StateEffect.define<Tooltip | null>();
const sigField = StateField.define<Tooltip | null>({
  create: () => null,
  update(v, tr) { for (const e of tr.effects) if (e.is(setSig)) v = e.value; return v; },
  provide: (f) => showTooltip.from(f),
});
function signatureHelp(lang: string, uri: string): Extension {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const fetcher = EditorView.updateListener.of((u) => {
    if (!u.docChanged) return;
    let trigger = "";
    u.changes.iterChanges((_a, _b, _c, _d, ins) => { const s = ins.toString(); if (s) trigger = s[s.length - 1]; });
    const pos = u.state.selection.main.head;
    if (trigger === ")" ) { u.view.dispatch({ effects: setSig.of(null) }); return; }
    if (trigger !== "(" && trigger !== ",") return;
    clearTimeout(timer);
    timer = setTimeout(async () => {
      const r = await req(lang, "textDocument/signatureHelp", { textDocument: { uri }, position: lspPos(u.state.doc, pos) });
      const sig = r?.signatures?.[r.activeSignature ?? 0];
      if (!sig) { u.view.dispatch({ effects: setSig.of(null) }); return; }
      const tip: Tooltip = {
        pos, above: true,
        create: () => { const dom = document.createElement("div"); dom.className = "cm-sig"; dom.textContent = sig.label; return { dom }; },
      };
      u.view.dispatch({ effects: setSig.of(tip) });
    }, 120);
  });
  return [sigField, fetcher];
}

// ── Inlay hints (toggleable extension) ──
class InlayWidget extends WidgetType {
  label: string; padL: boolean; padR: boolean;
  constructor(label: string, padL: boolean, padR: boolean) { super(); this.label = label; this.padL = padL; this.padR = padR; }
  toDOM() {
    const s = document.createElement("span");
    s.className = "cm-inlay";
    s.textContent = (this.padL ? " " : "") + this.label + (this.padR ? " " : "");
    return s;
  }
  eq(o: InlayWidget) { return o.label === this.label; }
}
const setInlay = StateEffect.define<DecorationSet>();
const inlayField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(v, tr) { v = v.map(tr.changes); for (const e of tr.effects) if (e.is(setInlay)) v = e.value; return v; },
  provide: (f) => EditorView.decorations.from(f),
});
export function cmInlayHints(lang: string, path: string): Extension {
  const uri = fileUri(path);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const fetcher = ViewPlugin.fromClass(class {
    constructor(view: EditorView) { this.schedule(view); }
    update(u: any) { if (u.docChanged || u.viewportChanged) this.schedule(u.view); }
    schedule(view: EditorView) { clearTimeout(timer); timer = setTimeout(() => this.fetch(view), 300); }
    async fetch(view: EditorView) {
      const { from, to } = view.viewport;
      const r = await req(lang, "textDocument/inlayHint", {
        textDocument: { uri }, range: { start: lspPos(view.state.doc, from), end: lspPos(view.state.doc, to) },
      });
      if (!Array.isArray(r)) return;
      const decos = r.map((h: any) => {
        const off = offsetOf(view.state.doc, h.position.line, h.position.character);
        const label = typeof h.label === "string" ? h.label : (h.label ?? []).map((p: any) => p.value).join("");
        return Decoration.widget({ widget: new InlayWidget(label, !!h.paddingLeft, !!h.paddingRight), side: h.kind === 1 ? -1 : 1 }).range(off);
      }).sort((a, b) => a.from - b.from || a.value.startSide - b.value.startSide);
      view.dispatch({ effects: setInlay.of(Decoration.set(decos, true)) });
    }
    destroy() { clearTimeout(timer); }
  });
  return [inlayField, fetcher];
}

// Format the whole document via the language server (textDocument/formatting),
// applying the returned edits to the view. Real, language-aware format-on-save
// (rustfmt / prettier / gofmt / black, whatever the server runs).
export async function formatDoc(lang: string, path: string, view: EditorView, tabSize: number): Promise<void> {
  const r = await req(lang, "textDocument/formatting", {
    textDocument: { uri: fileUri(path) },
    options: { tabSize, insertSpaces: true },
  });
  if (!Array.isArray(r) || !r.length) return;
  const ch = r
    .map((e: any) => ({
      from: offsetOf(view.state.doc, e.range.start.line, e.range.start.character),
      to: offsetOf(view.state.doc, e.range.end.line, e.range.end.character),
      insert: e.newText,
    }))
    .sort((a, b) => a.from - b.from);
  view.dispatch({ changes: ch });
}

export interface OutlineSym { name: string; detail: string; line: number; endLine: number; kind: number; depth: number; }

export async function fetchSymbols(lang: string, path: string): Promise<OutlineSym[]> {
  const r = await req(lang, "textDocument/documentSymbol", { textDocument: { uri: fileUri(path) } });
  const out: OutlineSym[] = [];
  const walk = (syms: any[], depth: number) => {
    for (const s of syms ?? []) {
      const range = s.range ?? s.location?.range;
      out.push({
        name: s.name, detail: s.detail ?? "",
        line: (range?.start?.line ?? 0) + 1, endLine: (range?.end?.line ?? range?.start?.line ?? 0) + 1,
        kind: s.kind ?? 1, depth,
      });
      if (s.children) walk(s.children, depth + 1);
    }
  };
  walk(Array.isArray(r) ? r : [], 0);
  return out;
}

export interface WsSym { name: string; kind: number; container: string; path: string; line: number; }

// Workspace-wide symbol search (LSP workspace/symbol) for go-to-symbol-in-project.
export async function searchWorkspaceSymbols(lang: string, query: string): Promise<WsSym[]> {
  const r = await req(lang, "workspace/symbol", { query });
  return (Array.isArray(r) ? r : []).map((s: any) => ({
    name: s.name,
    kind: s.kind ?? 1,
    container: s.containerName ?? "",
    path: uriToPath(s.location?.uri ?? ""),
    line: (s.location?.range?.start?.line ?? 0) + 1,
  }));
}

// Symbols (in document order) whose range encloses `line` (1-based), outermost
// → innermost. Powers the breadcrumb + sticky-scroll headers.
export function enclosingSymbols(syms: OutlineSym[], line: number): OutlineSym[] {
  return syms
    .filter((s) => s.line <= line && line <= s.endLine)
    .sort((a, b) => a.depth - b.depth || a.line - b.line);
}
