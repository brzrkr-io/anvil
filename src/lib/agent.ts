// Pure helpers for the AI agent panel, isolated for unit testing.

// Mask common credential shapes so secrets never leave the machine in context.
export function redact(s: string): string {
  return s
    // AWS access-key ids: AKIA (long-term) + ASIA (temporary / STS session creds).
    .replace(/(AKIA|ASIA)[0-9A-Z]{16}/g, "$1****REDACTED")
    // Google API keys (AIza + 35 chars) — common in a DevOps shell/agent context.
    .replace(/AIza[0-9A-Za-z\-_]{35}/g, "AIza****REDACTED")
    .replace(/(ghp|gho|ghs|ghr|github_pat)_[A-Za-z0-9_]{20,}/g, "$1_****REDACTED")
    .replace(/sk-[A-Za-z0-9]{20,}/g, "sk-****REDACTED")
    .replace(/xox[baprs]-[A-Za-z0-9-]{10,}/g, "xox*-****REDACTED")
    .replace(/-----BEGIN[^-]+PRIVATE KEY-----[\s\S]*?-----END[^-]+PRIVATE KEY-----/g, "****REDACTED PRIVATE KEY****")
    // `Authorization: Bearer <token>` headers (curl -H, kubeconfig, etc.).
    .replace(/(authorization:\s*bearer\s+)[\w.\-]{8,}/gi, "$1****REDACTED")
    .replace(/((?:password|passwd|secret|token|api[_-]?key|access[_-]?key)\s*[:=]\s*["']?)([^\s"']{6,})/gi, "$1****REDACTED");
}

// Parse a plan code block into clean step strings (strip bullets / numbering).
export function planSteps(t: string): string[] {
  return t.split("\n").map((l) => l.replace(/^\s*(?:[-*]|\d+[.)])\s*/, "").trim()).filter(Boolean);
}
