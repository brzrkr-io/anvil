// Backend data layer (scaling architecture). Instead of every component polling
// kubectl and parsing big JSON on the UI thread, a single background watcher per
// resource kind fetches + parses + sorts + caps in Rust (off the UI thread) and
// PUSHES shaped, render-ready rows to the frontend over a `kube://<kind>` event,
// coalesced so it only emits when the data actually changed. The frontend becomes
// a dumb subscriber: no polling, no parse, no jank.
//
// The fetch currently shells out to kubectl (keeps EKS/exec auth working exactly
// as before); it's isolated in `snapshot_*` so it can later be swapped for a
// long-lived kube-rs informer without touching the frontend or this plumbing.
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::kube::kubectl;

#[derive(Serialize)]
struct PodRow {
    ns: String,
    name: String,
    ready: String,
    status: String,
    restarts: String,
    age: String,
}

// Surface broken workloads first (not-running), then high restart counts.
fn pod_rank(status: &str, restarts: &str) -> u8 {
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
    if restarts.parse::<i64>().unwrap_or(0) > 0 {
        return 1;
    }
    2
}

fn parse_pods(text: &str) -> Vec<PodRow> {
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    if !header.starts_with("NAMESPACE") {
        return vec![];
    }
    let mut rows: Vec<PodRow> = lines
        .filter_map(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            if t.len() < 5 || t[1].is_empty() {
                return None;
            }
            Some(PodRow {
                ns: t[0].into(),
                name: t[1].into(),
                ready: t[2].into(),
                status: t[3].into(),
                restarts: t[4].into(),
                age: t.last().copied().unwrap_or("").into(),
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        pod_rank(&a.status, &a.restarts)
            .cmp(&pod_rank(&b.status, &b.restarts))
            .then_with(|| {
                b.restarts
                    .parse::<i64>()
                    .unwrap_or(0)
                    .cmp(&a.restarts.parse::<i64>().unwrap_or(0))
            })
            .then_with(|| a.name.cmp(&b.name))
    });
    rows.truncate(400);
    rows
}

fn is_auth_err(s: &str) -> bool {
    let l = s.to_lowercase();
    [
        "expired",
        "credentials",
        "unauthorized",
        "not logged in",
        "sso session",
        "reauthenticate",
        "invalididentitytoken",
        "token has expired",
        "failed to get token",
    ]
    .iter()
    .any(|p| l.contains(p))
}

/// `{ rows, error }` payload for a kind. Blocking (runs kubectl) — call from a
/// worker thread, never the UI thread.
fn snapshot(kind: &str) -> Value {
    match kind {
        "pods" => match kubectl(&["get", "pods", "-A"]) {
            Ok(text) if is_auth_err(&text) => {
                json!({ "rows": [], "error": "Cloud credentials expired or missing." })
            }
            Ok(text) => json!({ "rows": parse_pods(&text), "error": "" }),
            Err(e) => json!({ "rows": [], "error": e }),
        },
        _ => json!({ "rows": [], "error": format!("unknown watch kind: {kind}") }),
    }
}

fn hash_payload(v: &Value) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.to_string().hash(&mut h);
    h.finish()
}

static WATCHERS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
fn watchers() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    WATCHERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Start a background watcher for `kind` (idempotent — a second call is a no-op
/// while one is running). Emits `kube://<kind>` with `{rows,error}` whenever the
/// data changes, at most every `interval_ms`.
#[tauri::command]
pub fn kube_watch_start(app: AppHandle, kind: String, interval_ms: u64) {
    {
        let mut reg = watchers().lock().unwrap();
        if reg.contains_key(&kind) {
            return;
        }
        reg.insert(kind.clone(), Arc::new(AtomicBool::new(false)));
    }
    let stop = watchers().lock().unwrap().get(&kind).unwrap().clone();
    let topic = format!("kube://{kind}");
    let interval = interval_ms.max(1000);
    std::thread::spawn(move || {
        let mut last = 0u64;
        while !stop.load(Ordering::Relaxed) {
            let payload = snapshot(&kind);
            let h = hash_payload(&payload);
            if h != last {
                last = h;
                let _ = app.emit(&topic, payload);
            }
            // Sleep in small slices so a stop is honored promptly.
            let mut waited = 0u64;
            while waited < interval && !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(150));
                waited += 150;
            }
        }
        watchers().lock().unwrap().remove(&kind);
    });
}

/// Signal a watcher to stop (it tears down within ~150ms and frees its slot).
#[tauri::command]
pub fn kube_watch_stop(kind: String) {
    if let Some(s) = watchers().lock().unwrap().get(&kind) {
        s.store(true, Ordering::Relaxed);
    }
}

/// One-shot shaped snapshot for instant first paint and the Refresh button.
#[tauri::command]
pub async fn kube_snapshot(kind: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || snapshot(&kind))
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{parse_pods, pod_rank};

    #[test]
    fn broken_pods_sort_first() {
        let text = "NAMESPACE NAME READY STATUS RESTARTS AGE\n\
                    default ok-1 1/1 Running 0 4d\n\
                    prod crash-1 0/1 CrashLoopBackOff 8 12m\n\
                    default restarted 1/1 Running 3 1d";
        let rows = parse_pods(text);
        assert_eq!(rows[0].name, "crash-1"); // broken first
        assert_eq!(rows[1].name, "restarted"); // then restarts > 0
        assert_eq!(rows[2].name, "ok-1");
    }

    #[test]
    fn rank_orders_broken_then_restarts_then_healthy() {
        assert_eq!(pod_rank("CrashLoopBackOff", "0"), 0);
        assert_eq!(pod_rank("Running", "5"), 1);
        assert_eq!(pod_rank("Running", "0"), 2);
    }

    #[test]
    fn non_table_text_yields_no_rows() {
        assert!(parse_pods("error: something").is_empty());
    }
}
