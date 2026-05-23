//! Background poller: keeps a `Snapshot` fresh by polling caldera-local every
//! 2 seconds. Supports a "kick" channel for immediate re-poll after an action.
//!
//! Connection state machine:
//!
//! ```text
//!  start
//!    │
//!    ├── detect_project(cwd) == false  →  connection: NoProject
//!    │
//!    ├── GET /health fails (ECONNREFUSED) →  connection: Offline
//!    │
//!    ├── GET /api/project → enabled == false  →  connection: Disabled
//!    ├── GET /api/project missing / error     →  connection: NoProject
//!    │
//!    └── GET /api/activity + GET /api/agent-runs
//!              success  →  connection: Live (data populated)
//!              error    →  connection: ErrorState (data zeroed)
//! ```
//!
//! On any non-`Live` transition, data fields are zeroed so the UI never
//! renders stale data alongside an error state.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anvil_agent::{Connection, Snapshot};

use crate::CalderaClient;
use crate::client::ProjectResponse;
use crate::detect::detect_project;

/// How often the poller wakes up if no kick arrives.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Messages sent to the worker thread.
enum Msg {
    Kick,
    Stop,
}

/// A background poller that keeps a `Snapshot` current.
///
/// Call `stop()` during app shutdown to join the worker thread cleanly.
pub struct Poller {
    snapshot: Arc<Mutex<Snapshot>>,
    tx: Option<SyncSender<Msg>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Poller {
    /// Start the background polling thread.
    ///
    /// `endpoint` — the caldera-local base URL (e.g. `"http://127.0.0.1:4175"`).
    /// `repo_root` — the workspace root; used for `detect_project` on each cycle.
    pub fn start(endpoint: impl Into<String>, repo_root: PathBuf) -> Self {
        let endpoint = endpoint.into();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let snap_clone = Arc::clone(&snapshot);

        // Bounded channel: one pending message is enough.
        let (tx, rx) = mpsc::sync_channel::<Msg>(1);

        let handle = thread::spawn(move || {
            let client = CalderaClient::new(endpoint).with_timeout(Duration::from_secs(4));
            loop {
                let new_snap = poll_once(&client, &repo_root);
                if let Ok(mut guard) = snap_clone.lock() {
                    *guard = new_snap;
                }
                // Wait for kick, stop, or timeout.
                match rx.recv_timeout(POLL_INTERVAL) {
                    Ok(Msg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(Msg::Kick) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });

        Self {
            snapshot,
            tx: Some(tx),
            handle: Some(handle),
        }
    }

    /// Return a cheap clone of the latest snapshot.
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Trigger an immediate re-poll. Best-effort — silently ignored if the
    /// channel is full (one pending kick is already queued).
    pub fn kick(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(Msg::Kick);
        }
    }

    /// Signal the worker thread to stop and block until it exits.
    pub fn stop(&mut self) {
        // Send Stop, then drop the sender so the thread unblocks even if Stop
        // arrives while the thread is mid-poll.
        if let Some(tx) = self.tx.take() {
            match tx.try_send(Msg::Stop) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {}
            }
            // Dropping tx here disconnects the channel, guaranteeing the
            // worker exits on the next recv_timeout call.
            drop(tx);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        // Best-effort cleanup if stop() was not called explicitly.
        if let Some(tx) = self.tx.take() {
            let _ = tx.try_send(Msg::Stop);
        }
        // Do NOT join in drop — could panic if drop happens on the worker
        // thread itself, or cause unexpected blocking in normal teardown.
    }
}

/// Run one full poll cycle. Returns a `Snapshot` reflecting the current state.
fn poll_once(client: &CalderaClient, repo_root: &Path) -> Snapshot {
    // Gate: is a caldera project even configured for this repo?
    if !detect_project(repo_root) {
        return Snapshot {
            connection: Connection::NoProject,
            ..Default::default()
        };
    }

    // Liveness probe.
    if client.health().is_err() {
        return Snapshot {
            connection: Connection::Offline,
            ..Default::default()
        };
    }

    // Project enabled check.
    match client.project() {
        Err(_) => {
            return Snapshot {
                connection: Connection::NoProject,
                ..Default::default()
            };
        }
        Ok(ProjectResponse { project: None }) => {
            return Snapshot {
                connection: Connection::NoProject,
                ..Default::default()
            };
        }
        Ok(ProjectResponse {
            project: Some(ref p),
        }) if !p.enabled => {
            return Snapshot {
                connection: Connection::Disabled,
                ..Default::default()
            };
        }
        Ok(_) => {}
    }

    // Fetch activity (approvals + findings) and agent runs.
    let activity = match client.activity() {
        Ok(a) => a,
        Err(_) => {
            return Snapshot {
                connection: Connection::ErrorState,
                ..Default::default()
            };
        }
    };

    let runs_response = match client.agent_runs() {
        Ok(r) => r,
        Err(_) => {
            return Snapshot {
                connection: Connection::ErrorState,
                ..Default::default()
            };
        }
    };

    // Compute derived counts.
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let running_count = runs_response
        .runs
        .iter()
        .filter(|r| r.status == anvil_agent::RunStatus::Running)
        .count() as u8;

    let pending_approvals_count = activity.approvals.len() as u8;

    let attention_count = activity
        .findings
        .iter()
        .filter(|f| {
            f.severity == anvil_agent::FindingSeverity::Attention
                || f.severity == anvil_agent::FindingSeverity::Risk
                || f.severity == anvil_agent::FindingSeverity::Failure
        })
        .count() as u8;

    Snapshot {
        connection: Connection::Live,
        runs: runs_response.runs,
        approvals: activity.approvals,
        findings: activity.findings,
        running_count,
        pending_approvals_count,
        attention_count,
        polled_at_unix: now_unix,
    }
}
