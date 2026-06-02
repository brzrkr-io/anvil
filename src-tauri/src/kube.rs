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
    let mut cmd = std::process::Command::new("kubectl");
    cmd.args(args);
    let profile = aws_profile().lock().unwrap().clone();
    if !profile.is_empty() {
        cmd.env("AWS_PROFILE", &profile);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && !stderr.is_empty() {
        combined.push_str(&stderr);
    }
    Ok(combined)
}

#[tauri::command]
pub fn kube_contexts() -> Result<String, String> {
    kubectl(&["config", "get-contexts", "-o", "name"])
}

/// #50 Server-side diff of a manifest (`kubectl diff -f <path>`). Exit code 1
/// just means "differences found", so we return the combined output regardless.
#[tauri::command]
pub fn kube_diff(path: String) -> Result<String, String> {
    let out = kubectl(&["diff", "-f", &path])?;
    Ok(if out.trim().is_empty() {
        "(no differences)".into()
    } else {
        out
    })
}

/// #50 Apply a manifest after the user has approved the diff.
#[tauri::command]
pub fn kube_apply(path: String) -> Result<String, String> {
    kubectl(&["apply", "-f", &path])
}

#[tauri::command]
pub fn kube_current_context() -> Result<String, String> {
    kubectl(&["config", "current-context"])
}

#[tauri::command]
pub fn kube_use_context(name: String) -> Result<String, String> {
    kubectl(&["config", "use-context", &name])
}

/// Namespaces in the current context (#49).
#[tauri::command]
pub fn kube_namespaces() -> Result<String, String> {
    kubectl(&["get", "ns", "-o", "name"]).map(|s| {
        s.lines()
            .map(|l| l.trim_start_matches("namespace/"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// The namespace bound to the current context (defaults to "default").
#[tauri::command]
pub fn kube_current_namespace() -> Result<String, String> {
    let ns = kubectl(&["config", "view", "--minify", "-o", "jsonpath={..namespace}"])?;
    Ok(if ns.trim().is_empty() {
        "default".into()
    } else {
        ns.trim().to_string()
    })
}

/// Pin the namespace on the current context (#49).
#[tauri::command]
pub fn kube_set_namespace(namespace: String) -> Result<String, String> {
    kubectl(&[
        "config",
        "set-context",
        "--current",
        "--namespace",
        &namespace,
    ])
}

#[tauri::command]
pub fn kube_pods(context: String) -> Result<String, String> {
    if context.is_empty() {
        kubectl(&["get", "pods", "-A"])
    } else {
        kubectl(&["--context", &context, "get", "pods", "-A"])
    }
}

#[tauri::command]
pub fn kube_logs(context: String, namespace: String, pod: String) -> Result<String, String> {
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
}

/// #46 In-pane multiplexed logs across pods matching a label selector.
#[tauri::command]
pub fn kube_logs_selector(
    context: String,
    namespace: String,
    selector: String,
) -> Result<String, String> {
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
}

/// `kubectl describe pod` (#74).
#[tauri::command]
pub fn kube_describe(context: String, namespace: String, pod: String) -> Result<String, String> {
    let mut args: Vec<&str> = Vec::new();
    if !context.is_empty() {
        args.push("--context");
        args.push(&context);
    }
    args.extend(["describe", "pod", "-n", &namespace, &pod]);
    kubectl(&args)
}

/// `kubectl delete pod` (#74). The controller recreates it — a quick restart.
#[tauri::command]
pub fn kube_delete_pod(context: String, namespace: String, pod: String) -> Result<String, String> {
    let mut args: Vec<&str> = Vec::new();
    if !context.is_empty() {
        args.push("--context");
        args.push(&context);
    }
    args.extend(["delete", "pod", "-n", &namespace, &pod]);
    kubectl(&args)
}

/// `kubectl rollout restart` for a pod's owning deployment, best-effort (#74).
#[tauri::command]
pub fn kube_restart(
    context: String,
    namespace: String,
    deployment: String,
) -> Result<String, String> {
    let dep = format!("deployment/{deployment}");
    let mut args: Vec<&str> = Vec::new();
    if !context.is_empty() {
        args.push("--context");
        args.push(&context);
    }
    args.extend(["rollout", "restart", "-n", &namespace, &dep]);
    kubectl(&args)
}

#[tauri::command]
pub fn kube_pf_start(
    context: String,
    namespace: String,
    pod: String,
    ports: String,
) -> Result<String, String> {
    let mut cmd = std::process::Command::new("kubectl");
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
}

#[tauri::command]
pub fn kube_pf_list() -> Result<String, String> {
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
}

#[tauri::command]
pub fn kube_pf_stop(pid: u32) -> Result<(), String> {
    if let Some((mut c, _)) = port_forwards().lock().unwrap().remove(&pid) {
        let _ = c.kill();
        let _ = c.wait();
    }
    Ok(())
}
