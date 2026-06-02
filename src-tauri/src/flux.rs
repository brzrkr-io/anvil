// FluxCD (GitOps) surface. Reads come from `kubectl ... -o json` (stable, easy to
// parse) and mutations go through the `flux` CLI (correct reconcile/suspend
// semantics). AWS_PROFILE is applied via the shared kubectl helper for EKS auth.
use crate::kube::kubectl;

fn flux(args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("flux");
    cmd.args(args);
    let profile = crate::shared::aws_profile().lock().unwrap().clone();
    if !profile.is_empty() {
        cmd.env("AWS_PROFILE", &profile);
    }
    let out = crate::shared::exec_capture(cmd, 120).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "flux not found in PATH".to_string()
        } else {
            e.to_string()
        }
    })?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.is_empty() {
        s.push_str(&err);
    }
    Ok(s)
}

// Fully-qualified CRD groups so we never collide with same-named CRDs.
const KUSTOMIZATIONS: &str = "kustomizations.kustomize.toolkit.fluxcd.io";
const HELMRELEASES: &str = "helmreleases.helm.toolkit.fluxcd.io";
const SOURCES: &str = "gitrepositories.source.toolkit.fluxcd.io,ocirepositories.source.toolkit.fluxcd.io,helmrepositories.source.toolkit.fluxcd.io,helmcharts.source.toolkit.fluxcd.io,buckets.source.toolkit.fluxcd.io";
// A9: Flux image-automation CRDs (read-only listing).
const IMAGES: &str = "imagerepositories.image.toolkit.fluxcd.io,imagepolicies.image.toolkit.fluxcd.io,imageupdateautomations.image.toolkit.fluxcd.io";

/// Cluster-wide Flux objects of `kind` as `kubectl ... -o json`. `kind` is one of
/// "kustomizations" | "helmreleases" | "sources"; the frontend reads the standard
/// status conditions, `spec.suspend`, and `status.lastAppliedRevision`.
#[tauri::command]
pub async fn flux_get(kind: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = match kind.as_str() {
            "kustomizations" => KUSTOMIZATIONS,
            "helmreleases" => HELMRELEASES,
            "sources" => SOURCES,
            "images" => IMAGES,
            _ => return Err(format!("unknown flux kind: {kind}")),
        };
        kubectl(&["get", target, "-A", "-o", "json"])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `flux check` — verifies the flux CLI, controllers, and CRDs are present so the
/// UI can show a clear "Flux not installed / unreachable" state.
#[tauri::command]
pub async fn flux_check() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| flux(&["check"]))
        .await
        .map_err(|e| e.to_string())?
}

// flux's object kinds. Allow-listed so a frontend value can't inject flags/args.
fn flux_kind(kind: &str) -> Result<Vec<&'static str>, String> {
    Ok(match kind {
        "kustomization" => vec!["kustomization"],
        "helmrelease" => vec!["helmrelease"],
        "source git" => vec!["source", "git"],
        "source oci" => vec!["source", "oci"],
        "source helm" => vec!["source", "helm"],
        "source chart" => vec!["source", "chart"],
        "source bucket" => vec!["source", "bucket"],
        _ => return Err(format!("unsupported flux kind: {kind}")),
    })
}

/// `flux reconcile <kind> <name> -n <ns> [--with-source]` — pull the latest from
/// the source and re-apply now (the GitOps "force sync"). Non-destructive.
#[tauri::command]
pub async fn flux_reconcile(
    kind: String,
    name: String,
    namespace: String,
    with_source: bool,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args = vec!["reconcile"];
        args.extend(flux_kind(&kind)?);
        args.push(&name);
        args.push("-n");
        args.push(&namespace);
        if with_source {
            args.push("--with-source");
        }
        flux(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `flux suspend <kind> <name> -n <ns>` — pause reconciliation (freeze).
#[tauri::command]
pub async fn flux_suspend(kind: String, name: String, namespace: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args = vec!["suspend"];
        args.extend(flux_kind(&kind)?);
        args.push(&name);
        args.push("-n");
        args.push(&namespace);
        flux(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `flux resume <kind> <name> -n <ns>` — un-pause reconciliation.
#[tauri::command]
pub async fn flux_resume(kind: String, name: String, namespace: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args = vec!["resume"];
        args.extend(flux_kind(&kind)?);
        args.push(&name);
        args.push("-n");
        args.push(&namespace);
        flux(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::flux_kind;

    #[test]
    fn flux_kind_allowlist() {
        assert_eq!(flux_kind("kustomization").unwrap(), vec!["kustomization"]);
        assert_eq!(flux_kind("source oci").unwrap(), vec!["source", "oci"]);
        assert!(flux_kind("--all").is_err());
        assert!(flux_kind("kustomization; rm -rf /").is_err());
    }
}
