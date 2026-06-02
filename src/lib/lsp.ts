// Frontend half of the LSP bridge: starts a server per language, pushes document
// sync notifications, and exposes a transport (req/notify) the CodeMirror LSP
// extensions (cm-lsp.ts) build on. Diagnostics flow into both the per-path map
// (for inline squiggles) and the monaco-free Problems store.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writable } from "svelte/store";
import { setFileProblems } from "$lib/diagnostics";

const started = new Map<string, boolean>(); // lang -> available
let diagWired = false;

// Reactive per-language server state for the status indicator.
export type LspState = "down" | "starting" | "up";
export const lspStatus = writable<Record<string, LspState>>({});
function setStatus(lang: string, s: LspState) {
  lspStatus.update((m) => ({ ...m, [lang]: s }));
}

export function lspLang(path: string): string | null {
  const ext = path.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "rs": return "rust";
    case "go": return "go";
    case "ts": case "tsx": case "js": case "jsx": case "mjs": case "cjs": return "typescript";
    case "py": case "pyi": return "python";
    case "c": case "h": case "cpp": case "cc": case "hpp": case "cxx": case "hxx": return "cpp";
    default: return null;
  }
}

// file:// URI helpers (replacing monaco.Uri).
export function fileUri(p: string): string {
  const enc = p.split("/").map(encodeURIComponent).join("/");
  return "file://" + (p.startsWith("/") ? enc : "/" + enc);
}
export function uriToPath(uri: string): string {
  try { return decodeURIComponent(new URL(uri).pathname); }
  catch { return uri.replace(/^file:\/\//, ""); }
}

export async function req(lang: string, method: string, params: unknown): Promise<any> {
  try { return await invoke("lsp_request", { lang, method, params }); }
  catch { return null; }
}
async function notify(lang: string, method: string, params: unknown): Promise<void> {
  try { await invoke("lsp_notify", { lang, method, params }); } catch { /* server down */ }
}

/// Start the server for `lang` (rooted at `rootDir`) if not already up.
/// Returns false when no usable server is installed — caller just carries on.
export async function ensureLsp(lang: string, rootDir: string): Promise<boolean> {
  if (started.has(lang)) return started.get(lang)!;
  setStatus(lang, "starting");
  let ok = false;
  try { ok = await invoke<boolean>("lsp_start", { lang, rootUri: fileUri(rootDir) }); } catch { ok = false; }
  started.set(lang, ok);
  setStatus(lang, ok ? "up" : "down");
  if (ok) wireDiagnostics();
  return ok;
}

/// Restart the server for `lang` (used by the status indicator's click action).
export async function restartLsp(lang: string, rootDir: string): Promise<boolean> {
  started.delete(lang);
  try { await invoke("lsp_stop", { lang }); } catch { /* may not be running */ }
  return ensureLsp(lang, rootDir);
}

export function didOpen(lang: string, path: string, text: string, version: number) {
  return notify(lang, "textDocument/didOpen", {
    textDocument: { uri: fileUri(path), languageId: lang, version, text },
  });
}
export function didChange(lang: string, path: string, text: string, version: number) {
  return notify(lang, "textDocument/didChange", {
    textDocument: { uri: fileUri(path), version },
    contentChanges: [{ text }],
  });
}

// ── Diagnostics: per-path raw store + listeners for CM linting ──
export interface RawDiag {
  line: number; character: number; endLine: number; endChar: number; message: string; severity: number;
}
export const diagByPath = new Map<string, RawDiag[]>();
const diagListeners = new Set<(path: string) => void>();
export function onDiagnostics(fn: (path: string) => void): () => void {
  diagListeners.add(fn);
  return () => diagListeners.delete(fn);
}

function wireDiagnostics() {
  if (diagWired) return;
  diagWired = true;
  listen<{ uri: string; diagnostics: any[] }>("lsp://diagnostics", (e) => {
    const p = e.payload;
    if (!p?.uri) return;
    const path = uriToPath(p.uri);
    const raw: RawDiag[] = (p.diagnostics ?? []).map((d) => ({
      line: d.range?.start?.line ?? 0,
      character: d.range?.start?.character ?? 0,
      endLine: d.range?.end?.line ?? d.range?.start?.line ?? 0,
      endChar: d.range?.end?.character ?? 0,
      message: d.message,
      severity: d.severity ?? 1,
    }));
    diagByPath.set(path, raw);
    setFileProblems(path, raw.map((r) => ({ path, line: r.line + 1, message: r.message, severity: r.severity })));
    diagListeners.forEach((fn) => fn(path));
  });
}
