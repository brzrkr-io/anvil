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
pub(crate) fn poll_once(client: &CalderaClient, repo_root: &Path) -> Snapshot {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Spin up a mock TCP server that serves each body in `responses` in order
    /// (one connection per body), then returns the base URL.
    fn mock_multi(responses: Vec<&'static str>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for body in responses {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        });
        (format!("http://{}", addr), handle)
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("anvil_poller_test_{}_{}", name, std::process::id()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_project_json(dir: &Path, enabled: bool) {
        let caldera = dir.join(".caldera");
        fs::create_dir_all(&caldera).unwrap();
        fs::write(
            caldera.join("project.json"),
            format!(r#"{{"enabled":{enabled}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn poll_once_no_project_file_returns_no_project() {
        let dir = tmp_dir("no_project");
        // No .caldera/project.json — detect_project returns false.
        let client = CalderaClient::new("http://127.0.0.1:1"); // never reached
        let snap = poll_once(&client, &dir);
        assert_eq!(snap.connection, Connection::NoProject);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_once_offline_server_returns_offline() {
        let dir = tmp_dir("offline");
        write_project_json(&dir, true);
        // Port 1 always refuses — health() will Err → Offline.
        let client = CalderaClient::new("http://127.0.0.1:1");
        let snap = poll_once(&client, &dir);
        assert_eq!(snap.connection, Connection::Offline);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_once_project_disabled_returns_disabled() {
        let dir = tmp_dir("disabled");
        write_project_json(&dir, true);

        // health response, then project with enabled=false
        let health_body = r#"{"status":"ok","service":"caldera-local"}"#;
        let project_body = r#"{"project":{"name":"x","enabled":false}}"#;
        let (url, handle) = mock_multi(vec![health_body, project_body]);

        let client = CalderaClient::new(url);
        let snap = poll_once(&client, &dir);
        assert_eq!(snap.connection, Connection::Disabled);
        handle.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_once_project_null_returns_no_project() {
        let dir = tmp_dir("null_project");
        write_project_json(&dir, true);

        let health_body = r#"{"status":"ok","service":"caldera-local"}"#;
        let project_body = r#"{"project":null}"#;
        let (url, handle) = mock_multi(vec![health_body, project_body]);

        let client = CalderaClient::new(url);
        let snap = poll_once(&client, &dir);
        assert_eq!(snap.connection, Connection::NoProject);
        handle.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_once_live_returns_snapshot_with_counts() {
        let dir = tmp_dir("live");
        write_project_json(&dir, true);

        let health_body = r#"{"status":"ok","service":"caldera-local"}"#;
        let project_body = r#"{"project":{"name":"x","enabled":true}}"#;
        let activity_body = r#"{
            "pending_approvals":[
                {"approval_id":"ap1","connector":"git","pattern":"*.rs","reason":"CI"}
            ],
            "attention":[
                {"severity":"risk","summary":"s","recommended_action":""},
                {"severity":"failure","summary":"f","recommended_action":""}
            ]
        }"#;
        let runs_body = r#"{"agent_runs":[
            {"run_id":"r1","agent":"codex","task":"t","status":"running","created_at":"2026-01-01T00:00:00Z"}
        ]}"#;

        let (url, handle) = mock_multi(vec![health_body, project_body, activity_body, runs_body]);

        let client = CalderaClient::new(url);
        let snap = poll_once(&client, &dir);
        assert_eq!(snap.connection, Connection::Live);
        assert_eq!(snap.pending_approvals_count, 1);
        assert_eq!(snap.attention_count, 2);
        assert_eq!(snap.running_count, 1);
        handle.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poller_start_stop_cleanly() {
        let dir = tmp_dir("poller_stop");
        // No project file → poll_once immediately returns NoProject without network.
        let mut poller = Poller::start("http://127.0.0.1:1", dir.clone());
        let snap = poller.snapshot();
        // Just verify it returns a snapshot without panic.
        let _ = snap.connection;
        poller.stop();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poller_kick_does_not_panic() {
        let dir = tmp_dir("poller_kick");
        let poller = Poller::start("http://127.0.0.1:1", dir.clone());
        poller.kick();
        poller.kick(); // second kick is silently ignored (channel full)
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_once_project_error_returns_no_project() {
        let dir = tmp_dir("proj_err");
        write_project_json(&dir, true);

        let health_body = r#"{"status":"ok","service":"caldera-local"}"#;
        // Invalid JSON for project → client.project() returns Err → NoProject
        let project_body = "not json";
        let (url, handle) = mock_multi(vec![health_body, project_body]);

        let client = CalderaClient::new(url);
        let snap = poll_once(&client, &dir);
        assert_eq!(snap.connection, Connection::NoProject);
        handle.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_once_activity_error_returns_error_state() {
        let dir = tmp_dir("activity_err");
        write_project_json(&dir, true);

        let health_body = r#"{"status":"ok","service":"caldera-local"}"#;
        let project_body = r#"{"project":{"name":"x","enabled":true}}"#;
        // Invalid JSON for activity → Err → ErrorState
        let activity_body = "not json";
        let (url, handle) = mock_multi(vec![health_body, project_body, activity_body]);

        let client = CalderaClient::new(url);
        let snap = poll_once(&client, &dir);
        assert_eq!(snap.connection, Connection::ErrorState);
        handle.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_once_agent_runs_error_returns_error_state() {
        let dir = tmp_dir("runs_err");
        write_project_json(&dir, true);

        let health_body = r#"{"status":"ok","service":"caldera-local"}"#;
        let project_body = r#"{"project":{"name":"x","enabled":true}}"#;
        let activity_body = r#"{"pending_approvals":[],"attention":[]}"#;
        // Invalid JSON for agent_runs → Err → ErrorState
        let runs_body = "not json";
        let (url, handle) = mock_multi(vec![health_body, project_body, activity_body, runs_body]);

        let client = CalderaClient::new(url);
        let snap = poll_once(&client, &dir);
        assert_eq!(snap.connection, Connection::ErrorState);
        handle.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
