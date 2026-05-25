//! kubectl context background worker.
//!
//! Spawns a thread that polls `kubectl config current-context` and
//! `kubectl config view --minify` every 30 seconds, then sends the result
//! over a sync channel. If `kubectl` is not on PATH the thread exits
//! immediately and silently.

use std::sync::mpsc;
use std::time::Duration;

use anvil_prompt_core::{EnvKind, KubeCtx};

/// Spawn the kube worker. Returns immediately; the worker runs on its own thread.
/// The worker sends `KubeCtx` updates through `tx`. If `kubectl` is absent the
/// thread exits after logging once to stderr.
pub fn spawn_kube_worker(tx: mpsc::SyncSender<KubeCtx>) {
    std::thread::Builder::new()
        .name("anvil-kube".to_string())
        .spawn(move || {
            // Check for kubectl once.
            if std::process::Command::new("which")
                .arg("kubectl")
                .output()
                .map(|o| !o.status.success())
                .unwrap_or(true)
            {
                eprintln!("anvil-kube: kubectl not found, disabling");
                return;
            }

            let mut last_cluster = String::new();

            loop {
                if let Some(ctx) = poll_once() {
                    let first = last_cluster.is_empty();
                    if first || ctx.cluster != last_cluster {
                        eprintln!(
                            "anvil-kube: {} / {} ({:?})",
                            ctx.cluster, ctx.namespace, ctx.env_kind
                        );
                        last_cluster = ctx.cluster.clone();
                    }
                    // Non-blocking send; drop if the receiver is gone or channel full.
                    let _ = tx.try_send(ctx);
                }
                std::thread::sleep(Duration::from_secs(30));
            }
        })
        .expect("failed to spawn anvil-kube thread");
}

/// Run kubectl queries and build a `KubeCtx`. Returns `None` on any error.
fn poll_once() -> Option<KubeCtx> {
    let cluster_out = std::process::Command::new("kubectl")
        .args(["config", "current-context"])
        .output()
        .ok()?;
    if !cluster_out.status.success() {
        return None;
    }
    let cluster = String::from_utf8_lossy(&cluster_out.stdout)
        .trim()
        .to_string();
    if cluster.is_empty() {
        return None;
    }

    let ns_out = std::process::Command::new("kubectl")
        .args([
            "config",
            "view",
            "--minify",
            "-o",
            "jsonpath={.contexts[0].context.namespace}",
        ])
        .output()
        .ok()?;
    let namespace = if ns_out.status.success() {
        let s = String::from_utf8_lossy(&ns_out.stdout).trim().to_string();
        if s.is_empty() {
            "default".to_string()
        } else {
            s
        }
    } else {
        "default".to_string()
    };

    let env_kind = EnvKind::from_cluster_name(&cluster);
    Some(KubeCtx {
        cluster,
        namespace,
        env_kind,
    })
}
