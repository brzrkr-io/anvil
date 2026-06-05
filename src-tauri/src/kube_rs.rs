// Live Kubernetes data via kube-rs (the API server), replacing the kubectl
// subprocess + 5s text-poll. A supervised task per kind builds a Client for the
// Anvil-selected view-context, runs a reflector+watcher stream, and pushes shaped
// rows over the existing `kube://<kind>` event — emitting only on change.
// Connection/auth state rides in the payload `error`; the frontend's auth-gated
// `conn` state machine renders it. EKS exec auth (`aws eks get-token`) is handled
// by kube-rs from the kubeconfig.
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::TryStreamExt;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::api::{Api, ListParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::runtime::{reflector, watcher};
use kube::{Client, Config};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

#[derive(Serialize, Clone)]
pub struct PodRow {
    pub ns: String,
    pub name: String,
    pub ready: String,
    pub status: String,
    pub restarts: String,
    pub age: String,
}

// ── Pod → row mapping ──────────────────────────────────────────────────────

fn pod_ready(pod: &Pod) -> String {
    match pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
    {
        Some(cs) => format!("{}/{}", cs.iter().filter(|c| c.ready).count(), cs.len()),
        None => "0/0".into(),
    }
}

fn pod_restarts(pod: &Pod) -> i32 {
    pod.status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
        .map(|cs| cs.iter().map(|c| c.restart_count).sum())
        .unwrap_or(0)
}

/// kubectl-style status: a container's waiting/terminated reason
/// (CrashLoopBackOff, ImagePullBackOff, Completed, OOMKilled, …) wins over the
/// pod phase; a pod under deletion reads "Terminating".
fn pod_status(pod: &Pod) -> String {
    if pod.metadata.deletion_timestamp.is_some() {
        return "Terminating".into();
    }
    let status = pod.status.as_ref();
    let phase = status
        .and_then(|s| s.phase.clone())
        .unwrap_or_else(|| "Unknown".into());
    if let Some(cstats) = status.and_then(|s| s.container_statuses.as_ref()) {
        for cs in cstats {
            if let Some(state) = &cs.state {
                if let Some(w) = &state.waiting {
                    if let Some(r) = &w.reason {
                        if !r.is_empty() {
                            return r.clone();
                        }
                    }
                }
                if let Some(t) = &state.terminated {
                    if let Some(r) = &t.reason {
                        if !r.is_empty() {
                            return r.clone();
                        }
                    }
                }
            }
        }
    }
    phase
}

fn pod_age(ts: Option<&Time>) -> String {
    let Some(t) = ts else { return String::new() };
    // `Time.0` is a jiff::Timestamp (k8s-openapi 0.27); compare epoch seconds.
    let now = k8s_openapi::jiff::Timestamp::now().as_second();
    let secs = (now - t.0.as_second()).max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

// Surface broken workloads first (not-running), then high restart counts.
fn pod_rank(status: &str, restarts: i32) -> u8 {
    const BROKEN: [&str; 10] = [
        "Error",
        "CrashLoop",
        "Failed",
        "Evicted",
        "ImagePull",
        "Pending",
        "Unknown",
        "Init:",
        "Terminating",
        "OOMKilled",
    ];
    if BROKEN.iter().any(|b| status.contains(b)) {
        return 0;
    }
    if restarts > 0 {
        return 1;
    }
    2
}

/// Shape, sort (broken-first), and cap a set of pods for the UI.
fn rows_from(pods: &[Arc<Pod>]) -> Vec<PodRow> {
    let mut rows: Vec<(PodRow, i32)> = pods
        .iter()
        .map(|p| {
            let restarts = pod_restarts(p);
            (
                PodRow {
                    ns: p.metadata.namespace.clone().unwrap_or_default(),
                    name: p.metadata.name.clone().unwrap_or_default(),
                    ready: pod_ready(p),
                    status: pod_status(p),
                    restarts: restarts.to_string(),
                    age: pod_age(p.metadata.creation_timestamp.as_ref()),
                },
                restarts,
            )
        })
        .collect();
    rows.sort_by(|(a, ar), (b, br)| {
        pod_rank(&a.status, *ar)
            .cmp(&pod_rank(&b.status, *br))
            .then_with(|| br.cmp(ar))
            .then_with(|| a.name.cmp(&b.name))
    });
    rows.truncate(400);
    rows.into_iter().map(|(r, _)| r).collect()
}

fn hash_rows(rows: &[PodRow]) -> u64 {
    let mut h = DefaultHasher::new();
    serde_json::to_string(rows).unwrap_or_default().hash(&mut h);
    h.finish()
}

// ── Client + supervised watch ──────────────────────────────────────────────

/// Build a kube Client for a kubeconfig context (empty = ambient current-context).
async fn client_for(ctx: &str) -> Result<Client, String> {
    let kc = Kubeconfig::read().map_err(|e| e.to_string())?;
    let cfg = Config::from_custom_kubeconfig(
        kc,
        &KubeConfigOptions {
            context: (!ctx.is_empty()).then(|| ctx.to_string()),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    Client::try_from(cfg).map_err(|e| e.to_string())
}

/// Fired by `kube_set_view_context` so a running watch reconnects to the newly
/// selected cluster instead of streaming the old one.
static CTX_CHANGED: OnceLock<Notify> = OnceLock::new();
pub fn ctx_changed() -> &'static Notify {
    CTX_CHANGED.get_or_init(Notify::new)
}

// One watch task per kind; the value is its stop signal.
static WATCHERS: OnceLock<Mutex<HashMap<&'static str, Arc<Notify>>>> = OnceLock::new();
fn watchers() -> &'static Mutex<HashMap<&'static str, Arc<Notify>>> {
    WATCHERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Start the live pods watch (idempotent). Streams `kube://pods` deltas for the
/// selected view-context; reconnects on context change or stream error.
pub fn start_pod_watch(app: AppHandle) {
    let stop = {
        let mut reg = watchers().lock().unwrap();
        if reg.contains_key("pods") {
            return;
        }
        let stop = Arc::new(Notify::new());
        reg.insert("pods", stop.clone());
        stop
    };
    tauri::async_runtime::spawn(pod_watch_loop(app, stop));
}

pub fn stop_pod_watch() {
    if let Some(stop) = watchers().lock().unwrap().remove("pods") {
        stop.notify_waiters();
    }
}

async fn pod_watch_loop(app: AppHandle, stop: Arc<Notify>) {
    const TOPIC: &str = "kube://pods";
    loop {
        let ctx = crate::kube::view_context_value();
        let client = match client_for(&ctx).await {
            Ok(c) => c,
            Err(e) => {
                let _ = app.emit(TOPIC, json!({ "rows": [], "error": e }));
                tokio::select! {
                    _ = stop.notified() => return,
                    _ = ctx_changed().notified() => continue,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => continue,
                }
            }
        };
        let api: Api<Pod> = Api::all(client);
        let (reader, writer) = reflector::store::<Pod>();
        let stream = reflector(writer, watcher(api, watcher::Config::default()));
        tokio::pin!(stream);
        let mut last_hash = 0u64; // fresh per (re)connect — first state always emits
        loop {
            tokio::select! {
                _ = stop.notified() => return,
                _ = ctx_changed().notified() => break, // rebuild for the new context
                step = stream.try_next() => match step {
                    Ok(Some(_ev)) => {
                        let rows = rows_from(&reader.state());
                        let h = hash_rows(&rows);
                        if h != last_hash {
                            last_hash = h;
                            let _ = app.emit(TOPIC, json!({ "rows": rows, "error": "" }));
                        }
                    }
                    Ok(None) => break, // stream ended → reconnect
                    Err(e) => {
                        let _ = app.emit(TOPIC, json!({ "rows": [], "error": e.to_string() }));
                        tokio::select! {
                            _ = stop.notified() => return,
                            _ = ctx_changed().notified() => {}
                            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
                        }
                        break;
                    }
                }
            }
        }
    }
}

/// One-shot shaped list for instant first paint + the Refresh button.
pub async fn pod_snapshot() -> Value {
    let ctx = crate::kube::view_context_value();
    match client_for(&ctx).await {
        Err(e) => json!({ "rows": [], "error": e }),
        Ok(client) => {
            let api: Api<Pod> = Api::all(client);
            match api.list(&ListParams::default()).await {
                Ok(list) => {
                    let arcs: Vec<Arc<Pod>> = list.items.into_iter().map(Arc::new).collect();
                    json!({ "rows": rows_from(&arcs), "error": "" })
                }
                Err(e) => json!({ "rows": [], "error": e.to_string() }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateWaiting, ContainerStatus, PodStatus,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn cs(ready: bool, restarts: i32, waiting: Option<&str>) -> ContainerStatus {
        ContainerStatus {
            ready,
            restart_count: restarts,
            state: waiting.map(|r| ContainerState {
                waiting: Some(ContainerStateWaiting {
                    reason: Some(r.into()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn pod(ns: &str, name: &str, phase: &str, cstats: Vec<ContainerStatus>) -> Arc<Pod> {
        Arc::new(Pod {
            metadata: ObjectMeta {
                namespace: Some(ns.into()),
                name: Some(name.into()),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some(phase.into()),
                container_statuses: Some(cstats),
                ..Default::default()
            }),
            ..Default::default()
        })
    }

    #[test]
    fn ready_restarts_and_status_reason() {
        let p = pod(
            "prod",
            "api-x",
            "Running",
            vec![cs(true, 2, None), cs(false, 5, Some("CrashLoopBackOff"))],
        );
        assert_eq!(pod_ready(&p), "1/2");
        assert_eq!(pod_restarts(&p), 7);
        // container waiting reason overrides the "Running" phase.
        assert_eq!(pod_status(&p), "CrashLoopBackOff");
    }

    #[test]
    fn phase_used_when_no_container_reason() {
        let p = pod("default", "ok", "Running", vec![cs(true, 0, None)]);
        assert_eq!(pod_status(&p), "Running");
        assert_eq!(pod_ready(&p), "1/1");
    }

    #[test]
    fn rows_sort_broken_then_restarts_then_healthy() {
        let pods = vec![
            pod("default", "ok-1", "Running", vec![cs(true, 0, None)]),
            pod(
                "prod",
                "crash-1",
                "Running",
                vec![cs(false, 8, Some("CrashLoopBackOff"))],
            ),
            pod("default", "restarted", "Running", vec![cs(true, 3, None)]),
        ];
        let rows = rows_from(&pods);
        assert_eq!(rows[0].name, "crash-1"); // broken first
        assert_eq!(rows[1].name, "restarted"); // then restarts > 0
        assert_eq!(rows[2].name, "ok-1"); // healthy last
    }
}
