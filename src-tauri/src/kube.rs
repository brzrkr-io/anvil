use std::collections::HashMap;
use std::sync::Mutex;

use crate::shared::aws_profile;

// #48 Managed kubectl port-forwards, keyed by child PID so they can be listed
// and stopped from the UI (a real managed list, not a fire-and-forget terminal).
static PORT_FORWARDS: std::sync::OnceLock<Mutex<HashMap<u32, (std::process::Child, String)>>> =
    std::sync::OnceLock::new();
fn port_forwards() -> &'static Mutex<HashMap<u32, (std::process::Child, String)>> {
    PORT_FORWARDS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn kubectl(args: &[&str]) -> Result<String, String> {
    let mut cmd = crate::shared::command("kubectl");
    cmd.args(args);
    let profile = aws_profile().lock().unwrap().clone();
    if !profile.is_empty() {
        cmd.env("AWS_PROFILE", &profile);
    }
    let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && !stderr.is_empty() {
        combined.push_str(&stderr);
    }
    Ok(combined)
}

#[tauri::command]
pub async fn kube_contexts() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| kubectl(&["config", "get-contexts", "-o", "name"]))
        .await
        .map_err(|e| e.to_string())?
}

/// #50 Server-side diff of a manifest (`kubectl diff -f <path>`). Exit code 1
/// just means "differences found", so we return the combined output regardless.
#[tauri::command]
pub async fn kube_diff(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let out = kubectl(&["diff", "-f", &path])?;
        Ok(if out.trim().is_empty() {
            "(no differences)".into()
        } else {
            out
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

// Imperative `kubectl apply` removed by design: Anvil is strict GitOps —
// cluster changes land via a git commit that Flux reconciles, never an ad-hoc
// apply. `kube_diff` (read-only) stays for inspecting drift.

#[tauri::command]
pub async fn kube_current_context() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| kubectl(&["config", "current-context"]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn kube_use_context(name: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || kubectl(&["config", "use-context", &name]))
        .await
        .map_err(|e| e.to_string())?
}

/// Namespaces in the current context (#49).
#[tauri::command]
pub async fn kube_namespaces() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        kubectl(&["get", "ns", "-o", "name"]).map(|s| {
            s.lines()
                .map(|l| l.trim_start_matches("namespace/"))
                .collect::<Vec<_>>()
                .join("\n")
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The namespace bound to the current context (defaults to "default").
#[tauri::command]
pub async fn kube_current_namespace() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let ns = kubectl(&["config", "view", "--minify", "-o", "jsonpath={..namespace}"])?;
        Ok(if ns.trim().is_empty() {
            "default".into()
        } else {
            ns.trim().to_string()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Pin the namespace on the current context (#49).
#[tauri::command]
pub async fn kube_set_namespace(namespace: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        kubectl(&[
            "config",
            "set-context",
            "--current",
            "--namespace",
            &namespace,
        ])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #16 Node capacity: `kubectl top nodes` (live CPU/mem usage, needs
/// metrics-server) plus `kubectl get nodes -o wide` (Ready/roles/version). Both
/// are best-effort; if metrics-server is absent the top section explains that.
#[tauri::command]
pub async fn kube_nodes(context: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ctx: Vec<&str> = if context.is_empty() {
            vec![]
        } else {
            vec!["--context", &context]
        };
        let mut top_args = ctx.clone();
        top_args.extend(["top", "nodes"]);
        let top = kubectl(&top_args).unwrap_or_else(|e| format!("(metrics unavailable: {e})"));
        let mut get_args = ctx;
        get_args.extend(["get", "nodes", "-o", "wide"]);
        let get = kubectl(&get_args)?;
        Ok(format!(
            "# USAGE (kubectl top nodes)\n{top}\n\n# NODES (kubectl get nodes -o wide)\n{get}"
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn kube_pods(context: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if context.is_empty() {
            kubectl(&["get", "pods", "-A"])
        } else {
            kubectl(&["--context", &context, "get", "pods", "-A"])
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #14 Rollout status across the cluster: `kubectl get deploy -A` — the
/// READY / UP-TO-DATE / AVAILABLE columns are the live rollout progress (e.g.
/// READY 2/3 = a rollout in flight).
#[tauri::command]
pub async fn kube_deployments(context: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if context.is_empty() {
            kubectl(&["get", "deploy", "-A"])
        } else {
            kubectl(&["--context", &context, "get", "deploy", "-A"])
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn kube_logs(context: String, namespace: String, pod: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args: Vec<&str> = Vec::new();
        if !context.is_empty() {
            args.push("--context");
            args.push(&context);
        }
        args.push("logs");
        args.push("-n");
        args.push(&namespace);
        args.push("--tail=300");
        args.push(&pod);
        kubectl(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #46 In-pane multiplexed logs across pods matching a label selector.
#[tauri::command]
pub async fn kube_logs_selector(
    context: String,
    namespace: String,
    selector: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args: Vec<&str> = Vec::new();
        if !context.is_empty() {
            args.push("--context");
            args.push(&context);
        }
        args.extend_from_slice(&[
            "logs",
            "-n",
            &namespace,
            "-l",
            &selector,
            "--all-containers",
            "--prefix",
            "--tail=200",
        ]);
        kubectl(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #13 Events for a specific object (pod/deploy/…) — the fastest "why" for a
/// crash/pending without leaving the panel.
#[tauri::command]
pub async fn kube_events(
    context: String,
    namespace: String,
    object: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args: Vec<&str> = Vec::new();
        if !context.is_empty() {
            args.push("--context");
            args.push(&context);
        }
        let selector = format!("involvedObject.name={object}");
        args.extend([
            "get",
            "events",
            "-n",
            &namespace,
            "--field-selector",
            &selector,
            "--sort-by=.lastTimestamp",
        ]);
        kubectl(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `kubectl describe pod` (#74).
#[tauri::command]
pub async fn kube_describe(
    context: String,
    namespace: String,
    pod: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args: Vec<&str> = Vec::new();
        if !context.is_empty() {
            args.push("--context");
            args.push(&context);
        }
        args.extend(["describe", "pod", "-n", &namespace, &pod]);
        kubectl(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `kubectl delete pod` (#74). The controller recreates it — a quick restart.
#[tauri::command]
pub async fn kube_delete_pod(
    context: String,
    namespace: String,
    pod: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args: Vec<&str> = Vec::new();
        if !context.is_empty() {
            args.push("--context");
            args.push(&context);
        }
        args.extend(["delete", "pod", "-n", &namespace, &pod]);
        kubectl(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `kubectl rollout restart` for a pod's owning deployment, best-effort (#74).
#[tauri::command]
pub async fn kube_restart(
    context: String,
    namespace: String,
    deployment: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dep = format!("deployment/{deployment}");
        let mut args: Vec<&str> = Vec::new();
        if !context.is_empty() {
            args.push("--context");
            args.push(&context);
        }
        args.extend(["rollout", "restart", "-n", &namespace, &dep]);
        kubectl(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn kube_pf_start(
    context: String,
    namespace: String,
    pod: String,
    ports: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = crate::shared::command("kubectl");
        if !context.is_empty() {
            cmd.args(["--context", &context]);
        }
        cmd.args(["port-forward", "-n", &namespace, &pod, &ports]);
        let profile = aws_profile().lock().unwrap().clone();
        if !profile.is_empty() {
            cmd.env("AWS_PROFILE", &profile);
        }
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = cmd.spawn().map_err(|e| e.to_string())?;
        let pid = child.id();
        let desc = format!("{namespace}/{pod} {ports}");
        port_forwards().lock().unwrap().insert(pid, (child, desc));
        Ok(pid.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn kube_pf_list() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut map = port_forwards().lock().unwrap();
        let dead: Vec<u32> = map
            .iter_mut()
            .filter_map(|(pid, (c, _))| matches!(c.try_wait(), Ok(Some(_))).then_some(*pid))
            .collect();
        for d in dead {
            map.remove(&d);
        }
        Ok(map
            .iter()
            .map(|(pid, (_, desc))| format!("{pid}\t{desc}"))
            .collect::<Vec<_>>()
            .join("\n"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn kube_pf_stop(pid: u32) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some((mut c, _)) = port_forwards().lock().unwrap().remove(&pid) {
            let _ = c.kill();
            let _ = c.wait();
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── AWS SSO / per-context cloud auth (#8 follow-up) ───────────────────────────
// EKS kubeconfig entries bake the AWS profile + region + cluster into the user's
// `exec` block (e.g. `aws --region us-east-2 eks get-token --cluster-name X`,
// env AWS_PROFILE=dev-core). Surfacing those lets the UI offer a PRECISE
// `aws sso login --profile <P>` (or --sso-session) and a live auth check, instead
// of a profile-blind `aws sso login` that authenticates the wrong identity.

#[derive(serde::Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ContextCloud {
    pub cloud: String,   // aws | gcp | azure | unknown
    pub profile: String, // AWS_PROFILE from the context's exec, if any
    pub region: String,
    pub cluster: String,
    pub sso_session: String, // ~/.aws/config sso_session for that profile, if any
    pub authed: bool,        // `aws sts get-caller-identity` succeeded for it
}

/// Find the `sso_session = <name>` for `[profile <name>]` in an `~/.aws/config`
/// body. Pure (no IO) so it's unit-testable.
fn aws_profile_sso_session(cfg: &str, profile: &str) -> String {
    let header = format!("[profile {profile}]");
    let mut in_section = false;
    for line in cfg.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = t == header;
            continue;
        }
        if in_section {
            if let Some(rest) = t.strip_prefix("sso_session") {
                return rest.trim_start_matches([' ', '=']).trim().to_string();
            }
        }
    }
    String::new()
}

/// Inspect one kubeconfig context: which cloud, and (for AWS) the profile /
/// region / cluster / `sso_session` its exec-credential uses, plus whether that
/// identity currently has valid credentials.
#[tauri::command]
pub async fn kube_context_cloud(context: String) -> Result<ContextCloud, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cc = ContextCloud::default();
        let raw = kubectl(&[
            "config",
            "view",
            "--minify",
            "--context",
            &context,
            "-o",
            "json",
        ])?;
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let exec = &v["users"][0]["user"]["exec"];
            if let Some(envs) = exec["env"].as_array() {
                for e in envs {
                    if e["name"].as_str() == Some("AWS_PROFILE") {
                        cc.profile = e["value"].as_str().unwrap_or_default().to_string();
                    }
                }
            }
            if let Some(args) = exec["args"].as_array() {
                let args: Vec<&str> = args.iter().filter_map(|a| a.as_str()).collect();
                for w in args.windows(2) {
                    match w[0] {
                        "--region" => cc.region = w[1].to_string(),
                        "--cluster-name" => cc.cluster = w[1].to_string(),
                        _ => {}
                    }
                }
                let cmd = exec["command"].as_str().unwrap_or_default();
                if cmd == "aws" || args.iter().any(|a| a.contains("eks")) {
                    cc.cloud = "aws".into();
                } else if cmd.contains("gke") || cmd.contains("gcloud") {
                    cc.cloud = "gcp".into();
                } else if cmd.contains("kubelogin") || args.iter().any(|a| a.contains("azure")) {
                    cc.cloud = "azure".into();
                }
            }
        }
        if cc.cloud.is_empty() {
            cc.cloud = if context.starts_with("arn:aws") || context.contains("eks") {
                "aws"
            } else if context.starts_with("gke_") {
                "gcp"
            } else {
                "unknown"
            }
            .into();
        }
        if cc.cloud == "aws" {
            if !cc.profile.is_empty() {
                if let Some(home) = std::env::var_os("HOME") {
                    let path = std::path::Path::new(&home).join(".aws/config");
                    if let Ok(txt) = std::fs::read_to_string(path) {
                        cc.sso_session = aws_profile_sso_session(&txt, &cc.profile);
                    }
                }
            }
            // Live auth probe for exactly this identity.
            let mut c = crate::shared::command("aws");
            c.args(["sts", "get-caller-identity", "--output", "json"]);
            if !cc.profile.is_empty() {
                c.args(["--profile", &cc.profile]);
            }
            cc.authed = crate::shared::exec_capture(c, 8).is_ok_and(|o| o.status.success());
        }
        Ok(cc)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod cloud_tests {
    use super::aws_profile_sso_session;

    const CONFIG: &str = "\
[profile dev-core]
sso_session = corp-sso
region = us-east-2

[profile other]
region = us-west-2
";

    #[test]
    fn finds_sso_session_for_profile() {
        assert_eq!(aws_profile_sso_session(CONFIG, "dev-core"), "corp-sso");
    }
    #[test]
    fn empty_when_profile_has_no_sso_session() {
        assert_eq!(aws_profile_sso_session(CONFIG, "other"), "");
    }
    #[test]
    fn empty_when_profile_absent() {
        assert_eq!(aws_profile_sso_session(CONFIG, "nope"), "");
    }
}
