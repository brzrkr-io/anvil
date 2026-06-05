// Classify a raw kubectl/cluster error into a category + a friendly message, so
// the UI can tell the user whether to re-auth, fix permissions, or check the
// network — instead of dumping a raw stderr line. (#5)

export type K8sErrorKind = "auth" | "rbac" | "network" | "none" | "other";

const AUTH_RE =
  /expired|credentials|unauthorized|not logged in|sso session|reauthenticate|InvalidIdentityToken|token has expired|failed to get token/i;
const RBAC_RE = /forbidden|cannot (list|get|watch)|is forbidden/i;
const NET_RE =
  /timeout|timed out|connection refused|no route to host|dial tcp|i\/o timeout|unreachable|could not resolve|EOF|TLS handshake/i;

export function classifyK8sError(raw: string | null | undefined): K8sErrorKind {
  const e = (raw ?? "").trim();
  if (!e) return "none";
  // Auth wins: an expired token often also reads as forbidden downstream.
  if (AUTH_RE.test(e)) return "auth";
  if (RBAC_RE.test(e)) return "rbac";
  if (NET_RE.test(e)) return "network";
  return "other";
}

// Parse `kubectl get ns -o name` output into clean namespace names. On an auth
// failure the backend returns the kubectl error TEXT instead of a list; keep only
// valid RFC1123 label names so the error's words can't render as bogus namespace
// options — and (critically) so repeated error lines can't collide as duplicate
// keys in the namespace <select>'s keyed each block (each_key_duplicate crash).
export function parseNamespaces(raw: string | null | undefined): string[] {
  return (raw ?? "")
    .split("\n")
    .map((l) => l.trim().replace(/^namespace\//, ""))
    .filter((l) => /^[a-z0-9][a-z0-9-]*$/.test(l));
}

export function friendlyK8sError(raw: string | null | undefined): string {
  switch (classifyK8sError(raw)) {
    case "auth":
      return "Cloud credentials expired or missing.";
    case "rbac":
      return "Access denied (RBAC) — this context can't list resources here.";
    case "network":
      return "Can't reach the cluster — check your network / VPN, then Retry.";
    case "none":
      return "";
    default:
      return (raw ?? "").trim();
  }
}
