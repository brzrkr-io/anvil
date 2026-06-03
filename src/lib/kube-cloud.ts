// Detect which cloud a kubeconfig context belongs to and produce the right
// re-auth commands, so the Kubernetes auth banner works across AWS/GCP/Azure
// instead of assuming AWS. (#8 multi-cloud auth)

export type Cloud = "aws" | "gcp" | "azure" | "unknown";

export interface ReauthAction {
  cmd: string;
  label: string;
}

/** Best-effort cloud detection from a context name. */
export function detectCloud(context: string): Cloud {
  const c = (context || "").trim();
  if (!c) return "unknown";
  if (c.startsWith("gke_")) return "gcp";
  if (/^arn:aws:eks:|:aws:|(^|[-_])eks([-_]|$)/i.test(c)) return "aws";
  // AKS contexts from `az aks get-credentials` are usually just the cluster
  // name with no prefix, so only treat an explicit hint as Azure.
  if (/(^|[-_])aks([-_]|$)|azure/i.test(c)) return "azure";
  return "unknown";
}

/** Parse the EKS cluster name out of an ARN context, else use the context. */
function eksCluster(context: string): string {
  const m = /cluster\/(.+)$/.exec(context);
  return m ? m[1] : context;
}

/** Parse `gke_<project>_<location>_<cluster>` into its parts. */
function gkeParts(context: string): { project: string; location: string; cluster: string } | null {
  const m = /^gke_([^_]+)_([^_]+)_(.+)$/.exec(context);
  return m ? { project: m[1], location: m[2], cluster: m[3] } : null;
}

/**
 * Ordered re-auth actions for a context: first the credential login, then the
 * kubeconfig refresh. Falls back to AWS SSO when the cloud is unknown.
 */
export function reauthActions(context: string): ReauthAction[] {
  switch (detectCloud(context)) {
    case "gcp": {
      const g = gkeParts(context);
      const refresh = g
        ? `gcloud container clusters get-credentials ${g.cluster} --location ${g.location} --project ${g.project}`
        : "gcloud container clusters get-credentials";
      return [
        { cmd: "gcloud auth login", label: "gcloud auth login" },
        { cmd: refresh, label: "refresh kubeconfig" },
      ];
    }
    case "azure":
      return [
        { cmd: "az login", label: "az login" },
        { cmd: `az aks get-credentials --name ${context}`, label: "refresh kubeconfig" },
      ];
    case "aws":
    default:
      return [
        { cmd: "aws sso login", label: "aws sso login" },
        { cmd: `aws eks update-kubeconfig --name "${eksCluster(context)}"`, label: "refresh kubeconfig" },
      ];
  }
}
