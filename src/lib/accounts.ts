// Accounts / credentials. Secrets live in the macOS Keychain (via Rust
// `secret_*` commands); non-secret config in localStorage. Nothing sensitive
// is ever written to disk in plaintext by the app.

import { invoke } from "@tauri-apps/api/core";

export interface AccountField {
  key: string;
  label: string;
  secret: boolean;
  placeholder: string;
  hint?: string;
}

export const ACCOUNTS: AccountField[] = [
  { key: "llm-endpoint", label: "LLM endpoint", secret: false, placeholder: "http://localhost:1234/v1", hint: "OpenAI-compatible base URL" },
  { key: "llm-key", label: "LLM API key", secret: true, placeholder: "sk-…", hint: "remote providers only — local LM Studio needs none" },
  { key: "github-token", label: "GitHub token", secret: true, placeholder: "ghp_…", hint: "optional; the gh CLI usually handles auth" },
  { key: "aws-profile", label: "AWS profile", secret: false, placeholder: "default", hint: "named profile for kubectl / aws" },
  { key: "grafana-url", label: "Observability URL", secret: false, placeholder: "https://grafana…/d/…", hint: "embedded dashboard in DevOps → Observability" },
];

const cfgKey = (k: string) => `anvil-acct-${k}`;

export async function getValue(f: AccountField): Promise<string> {
  if (f.secret) {
    try { return await invoke<string>("secret_get", { key: f.key }); } catch { return ""; }
  }
  return typeof localStorage !== "undefined" ? localStorage.getItem(cfgKey(f.key)) ?? "" : "";
}

export async function setValue(f: AccountField, value: string): Promise<void> {
  if (f.secret) { await invoke("secret_set", { key: f.key, value }); }
  else if (typeof localStorage !== "undefined") localStorage.setItem(cfgKey(f.key), value);
}

export async function clearValue(f: AccountField): Promise<void> {
  if (f.secret) { try { await invoke("secret_delete", { key: f.key }); } catch { /* ignore */ } }
  else if (typeof localStorage !== "undefined") localStorage.removeItem(cfgKey(f.key));
}

// Convenience: resolve the LLM base URL + API key for the agent.
export async function llmCreds(): Promise<{ base: string; apiKey: string }> {
  const find = (k: string) => ACCOUNTS.find((f) => f.key === k)!;
  return {
    base: await getValue(find("llm-endpoint")),
    apiKey: await getValue(find("llm-key")),
  };
}

export async function hasValue(f: AccountField): Promise<boolean> {
  if (f.secret) {
    try { return await invoke<boolean>("secret_has", { key: f.key }); } catch { return false; }
  }
  return !!(typeof localStorage !== "undefined" && localStorage.getItem(cfgKey(f.key)));
}
