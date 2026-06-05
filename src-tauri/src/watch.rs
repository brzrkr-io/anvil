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

// Pods now stream live via kube-rs (`kube_rs.rs`); the kubectl text parser that
// used to live here is gone. The watcher below still serves Flux over kubectl.

// ── Flux (GitOps) ─────────────────────────────────────────────────────────
const KUSTOMIZATIONS: &str = "kustomizations.kustomize.toolkit.fluxcd.io";
const HELMRELEASES: &str = "helmreleases.helm.toolkit.fluxcd.io";
const SOURCES: &str = "gitrepositories.source.toolkit.fluxcd.io,ocirepositories.source.toolkit.fluxcd.io,helmrepositories.source.toolkit.fluxcd.io,helmcharts.source.toolkit.fluxcd.io,buckets.source.toolkit.fluxcd.io";
const IMAGES: &str = "imagerepositories.image.toolkit.fluxcd.io,imagepolicies.image.toolkit.fluxcd.io,imageupdateautomations.image.toolkit.fluxcd.io";

#[derive(Serialize)]
struct FluxRow {
    name: String,
    ns: String,
    #[serde(rename = "apiKind")]
    api_kind: String,
    ready: String,
    suspended: bool,
    revision: String,
    message: String,
    source: String,
    deps: usize,
    #[serde(rename = "dependsOn")]
    depends_on: Vec<String>,
}

fn flux_absent(raw: &str) -> bool {
    let l = raw.to_lowercase();
    l.contains("the server doesn't have a resource type")
        || l.contains("no matches for kind")
        || l.contains("notfound")
        || l.contains("could not find the requested resource")
}

fn health_rank(ready: &str, suspended: bool) -> u8 {
    if ready == "fail" {
        0
    } else if suspended {
        1
    } else if ready == "unknown" {
        2
    } else {
        3
    }
}

/// Parse `kubectl get <flux-crd> -A -o json` into rows. Returns (rows, present);
/// present=false when the cluster has no such CRD.
fn parse_flux(raw: &str) -> (Vec<FluxRow>, bool) {
    if flux_absent(raw) {
        return (vec![], false);
    }
    let j: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return (vec![], false),
    };
    let items = j["items"].as_array().cloned().unwrap_or_default();
    let mut rows: Vec<FluxRow> = items
        .iter()
        .map(|it| {
            let st = &it["status"];
            let sp = &it["spec"];
            let ready_cond = st["conditions"]
                .as_array()
                .and_then(|cs| cs.iter().find(|c| c["type"] == "Ready"));
            let ready = match ready_cond {
                None => "unknown",
                Some(c) if c["status"] == "True" => "ok",
                Some(_) => "fail",
            };
            let revision = st["lastAppliedRevision"]
                .as_str()
                .or_else(|| st["lastAttemptedRevision"].as_str())
                .or_else(|| st["artifact"]["revision"].as_str())
                .unwrap_or("")
                .to_string();
            let source = sp["sourceRef"]["name"]
                .as_str()
                .or_else(|| sp["chart"]["spec"]["sourceRef"]["name"].as_str())
                .or_else(|| sp["chartRef"]["name"].as_str())
                .unwrap_or("")
                .to_string();
            FluxRow {
                name: it["metadata"]["name"].as_str().unwrap_or("?").to_string(),
                ns: it["metadata"]["namespace"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                api_kind: it["kind"].as_str().unwrap_or("").to_string(),
                ready: ready.to_string(),
                suspended: sp["suspend"] == Value::Bool(true),
                revision,
                message: ready_cond
                    .and_then(|c| c["message"].as_str())
                    .unwrap_or("")
                    .to_string(),
                source,
                deps: sp["dependsOn"].as_array().map_or(0, std::vec::Vec::len),
                depends_on: sp["dependsOn"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|d| d["name"].as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        health_rank(&a.ready, a.suspended)
            .cmp(&health_rank(&b.ready, b.suspended))
            .then_with(|| (a.ns.clone() + &a.name).cmp(&(b.ns.clone() + &b.name)))
    });
    (rows, true)
}

fn flux_crd(tab: &str) -> Option<&'static str> {
    match tab {
        "kustomizations" => Some(KUSTOMIZATIONS),
        "helmreleases" => Some(HELMRELEASES),
        "sources" => Some(SOURCES),
        "images" => Some(IMAGES),
        _ => None,
    }
}

// Short-TTL memo for `kubectl get <crd> -A -o json`. The Flux list watcher and
// the health watcher both want kustomizations/helmreleases each cycle; within
// the TTL they share one fetch instead of forking kubectl twice. TTL < the poll
// interval so each cycle still gets fresh data.
fn flux_get_cached(crd: &'static str) -> Result<String, String> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, (std::time::Instant, String)>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    const TTL: Duration = Duration::from_millis(2500);
    if let Some((at, raw)) = cache.lock().unwrap().get(crd) {
        if at.elapsed() < TTL {
            return Ok(raw.clone());
        }
    }
    let raw = kubectl(&["get", crd, "-A", "-o", "json"])?;
    cache
        .lock()
        .unwrap()
        .insert(crd, (std::time::Instant::now(), raw.clone()));
    Ok(raw)
}

fn snapshot_flux(tab: &str) -> Value {
    let Some(crd) = flux_crd(tab) else {
        return json!({ "rows": [], "present": false, "error": format!("unknown flux tab: {tab}") });
    };
    match flux_get_cached(crd) {
        Ok(raw) if is_auth_err(&raw) => {
            json!({ "rows": [], "present": true, "error": "Cloud credentials expired or missing." })
        }
        Ok(raw) => {
            let (rows, present) = parse_flux(&raw);
            json!({ "rows": rows, "present": present, "error": "" })
        }
        Err(e) => json!({ "rows": [], "present": false, "error": e }),
    }
}

/// Cluster-wide failing count for the rail badge (Kustomizations + `HelmReleases`).
fn snapshot_flux_health() -> Value {
    let mut failing = 0usize;
    let mut present = false;
    for crd in [KUSTOMIZATIONS, HELMRELEASES] {
        if let Ok(raw) = flux_get_cached(crd) {
            let (rows, ok) = parse_flux(&raw);
            if ok {
                present = true;
                failing += rows.iter().filter(|r| r.ready == "fail").count();
            }
        }
    }
    json!({ "failing": failing, "present": present })
}

fn is_auth_err(s: &str) -> bool {
    // A SUCCESSFUL `kubectl ... -o json` response is valid JSON whose DATA may
    // contain words like "credentials"/"token"/"expired" (a resource name, a
    // status message). Never flag a JSON body as an auth error — real auth
    // failures come back as plain-text stderr, not JSON. (This was the bug: a
    // kustomization JSON containing such a word showed a false "creds expired".)
    let t = s.trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        return false;
    }
    let l = s.to_lowercase();
    // A missing CLI ("exec: executable aws not found") contains "credentials" but
    // is a PATH problem, not expired auth — let it pass through as a raw error so
    // the UI shows the real cause instead of a false "credentials expired".
    if l.contains("not found") || l.contains("no such file") || l.contains("command not found") {
        return false;
    }
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
        "flux:health" => snapshot_flux_health(),
        k if k.starts_with("flux:") => snapshot_flux(&k["flux:".len()..]),
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
    // Pods are served by the live kube-rs watch (server-push stream), not the
    // kubectl text poll below.
    if kind == "pods" {
        crate::kube_rs::start_pod_watch(app);
        return;
    }
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
        let mut backoff = 0u32;
        let mut idle = 0u32;
        while !stop.load(Ordering::Relaxed) {
            let payload = snapshot(&kind);
            let has_err = payload
                .get("error")
                .and_then(|e| e.as_str())
                .is_some_and(|s| !s.is_empty());
            let h = hash_payload(&payload);
            if h == last {
                idle = (idle + 1).min(2);
            } else {
                last = h;
                idle = 0;
                let _ = app.emit(&topic, payload);
            }
            // Two adaptive slowdowns, error wins:
            //  - errors: back off hard (up to 16×, cap 60s) so a dead cluster
            //    stops forking kubectl every interval.
            //  - stable data: ease off (up to 4×, cap 20s) so an idle cluster is
            //    light on CPU; any change snaps straight back to the base rate.
            let (mult, cap) = if has_err {
                backoff = (backoff + 1).min(4);
                (1u64 << backoff, 60_000)
            } else {
                backoff = 0;
                (1u64 << idle, 20_000)
            };
            let effective = interval.saturating_mul(mult).min(cap);
            // Sleep in small slices so a stop is honored promptly.
            let mut waited = 0u64;
            while waited < effective && !stop.load(Ordering::Relaxed) {
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
    if kind == "pods" {
        crate::kube_rs::stop_pod_watch();
        return;
    }
    if let Some(s) = watchers().lock().unwrap().get(&kind) {
        s.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::is_auth_err;

    #[test]
    fn json_body_is_never_auth_error() {
        // The bug: a successful `-o json` response whose DATA contains auth-ish
        // words (a kustomization name, a status message) was flagged as expired.
        let json = r#"{"apiVersion":"v1","items":[{"kind":"Kustomization","metadata":{"name":"sso-credentials"},"status":{"conditions":[{"message":"token expired upstream"}]}}]}"#;
        assert!(!is_auth_err(json));
    }

    #[test]
    fn plain_text_auth_error_still_detected() {
        assert!(is_auth_err(
            "error: You must be logged in to the server (the server has asked for credentials)"
        ));
        assert!(is_auth_err(
            "the SSO session associated with this profile has expired"
        ));
    }

    #[test]
    fn missing_cli_is_not_auth() {
        assert!(!is_auth_err(
            "Unable to connect: getting credentials: exec: executable aws not found"
        ));
    }
}

/// One-shot shaped snapshot for instant first paint and the Refresh button.
#[tauri::command]
pub async fn kube_snapshot(kind: String) -> Result<Value, String> {
    if kind == "pods" {
        return Ok(crate::kube_rs::pod_snapshot().await);
    }
    tauri::async_runtime::spawn_blocking(move || snapshot(&kind))
        .await
        .map_err(|e| e.to_string())
}
