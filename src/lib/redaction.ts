// #44 Redaction rules + audit log. Layers user-defined patterns on top of the
// built-in credential masking (agent.ts `redact`), and keeps a local-only audit
// trail of what the agent was sent (counts + a redacted preview, never raw
// secrets, never transmitted).
import { writable, get } from "svelte/store";
import { redact } from "$lib/agent";

const RULES_KEY = "anvil-redaction-rules";
const AUDIT_KEY = "anvil-agent-audit";
const AUDIT_CAP = 300;

function loadRules(): string[] {
  if (typeof localStorage === "undefined") return [];
  try { return JSON.parse(localStorage.getItem(RULES_KEY) || "[]"); } catch { return []; }
}

// Each rule is a regex source string; matches are replaced with ****REDACTED****.
export const redactionRules = writable<string[]>(loadRules());

export function setRedactionRules(rules: string[]) {
  if (typeof localStorage !== "undefined") localStorage.setItem(RULES_KEY, JSON.stringify(rules));
  redactionRules.set(rules);
}
export function addRedactionRule(src: string) {
  if (!src.trim()) return;
  try { new RegExp(src); } catch { return; } // reject invalid regex
  const next = [...get(redactionRules).filter((r) => r !== src), src];
  setRedactionRules(next);
}
export function removeRedactionRule(src: string) {
  setRedactionRules(get(redactionRules).filter((r) => r !== src));
}

// Built-in masking first, then user rules. Invalid user rules are skipped.
export function applyRedaction(s: string): string {
  let out = redact(s);
  for (const src of get(redactionRules)) {
    try { out = out.replace(new RegExp(src, "g"), "****REDACTED****"); } catch { /* skip bad rule */ }
  }
  return out;
}

export type AuditEntry = { ts: number; kind: string; chars: number; preview: string };

export function getAuditLog(): AuditEntry[] {
  if (typeof localStorage === "undefined") return [];
  try { return JSON.parse(localStorage.getItem(AUDIT_KEY) || "[]"); } catch { return []; }
}
export function clearAuditLog() {
  if (typeof localStorage !== "undefined") localStorage.removeItem(AUDIT_KEY);
}

// Record an outbound agent payload. `text` is already redacted by the caller;
// we store only a short preview so the audit itself can't leak anything.
export function auditAgentSend(kind: string, text: string) {
  try {
    const log = getAuditLog();
    log.push({ ts: Date.now(), kind, chars: text.length, preview: text.slice(0, 120) });
    localStorage.setItem(AUDIT_KEY, JSON.stringify(log.slice(-AUDIT_CAP)));
  } catch { /* ignore */ }
}
