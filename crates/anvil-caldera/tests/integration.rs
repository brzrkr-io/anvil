//! Integration tests for `anvil-caldera`.
//!
//! Covers:
//! 1. Snapshot round-trip from `activity_v0.json` fixture.
//! 2. `detect_project` — with/without `.caldera/project.json`.
//! 3. Connection state machine via a fake HTTP server.
//! 4. Poller integration — starts a fake server, verifies snapshot transitions.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anvil_agent::{Connection, FindingSeverity, RunStatus, Snapshot};
use anvil_caldera::{
    CalderaClient, Poller,
    client::{RawActivity, RawAgentRuns},
    detect::detect_project,
};

// ── 1. Snapshot fixture round-trip ────────────────────────────────────────────

/// Load the captured `/api/activity` fixture and verify it deserialises cleanly.
#[test]
fn activity_fixture_deserialises() {
    let fixture = include_str!("fixtures/activity_v0.json");

    // Parse into the raw intermediate type.
    let raw: RawActivity = serde_json::from_str(fixture).expect("fixture should parse");

    // Two approvals in the fixture.
    assert_eq!(raw.pending_approvals.len(), 2);
    assert_eq!(raw.pending_approvals[0].approval_id, "approval_1716000001");
    assert_eq!(raw.pending_approvals[0].connector, "kubernetes");
    assert_eq!(raw.pending_approvals[1].connector, "github");

    // Two attention findings.
    assert_eq!(raw.attention.len(), 2);
    assert_eq!(raw.attention[0].severity, "warning");
    assert!(raw.attention[0].summary.contains("Changed files"));
}

/// Verify the caldera-local → anvil-agent type conversion produces expected values.
#[test]
fn activity_response_conversion_maps_fields() {
    use anvil_caldera::client::ActivityResponse;

    let fixture = include_str!("fixtures/activity_v0.json");
    let raw: RawActivity = serde_json::from_str(fixture).unwrap();
    let response = ActivityResponse::from(raw);

    // Approvals map correctly.
    assert_eq!(response.approvals.len(), 2);
    assert_eq!(response.approvals[0].approval_id, "approval_1716000001");
    assert_eq!(response.approvals[0].connector, "kubernetes");

    // Findings map: "warning" → Attention.
    assert_eq!(response.findings.len(), 2);
    assert_eq!(response.findings[0].severity, FindingSeverity::Attention);
    assert!(response.findings[0].action.contains("commit"));
}

/// Agent-runs raw parsing.
#[test]
fn agent_runs_fixture_deserialises() {
    let fixture = r#"{
        "schema_version": "caldera.agent_runs.v0",
        "agent_runs": [
            {
                "schema_version": "caldera.agent_run.v0",
                "run_id": "agent_1716000001",
                "task": "Map the repo safely",
                "agent": "caldera-agent",
                "status": "running",
                "workspace_root": "/home/dev/myrepo",
                "created_at": "2026-05-22T10:00:01Z",
                "finished_at": ""
            },
            {
                "schema_version": "caldera.agent_run.v0",
                "run_id": "agent_1716000002",
                "task": "Review PR",
                "agent": "claude",
                "status": "completed",
                "workspace_root": "/home/dev/myrepo",
                "created_at": "2026-05-22T09:00:00Z",
                "finished_at": "2026-05-22T09:30:00Z"
            }
        ]
    }"#;

    let raw: RawAgentRuns = serde_json::from_str(fixture).unwrap();
    assert_eq!(raw.agent_runs.len(), 2);
    assert_eq!(raw.agent_runs[0].run_id, "agent_1716000001");
    assert_eq!(raw.agent_runs[0].status, "running");
}

#[test]
fn agent_runs_conversion_maps_status_and_timestamp() {
    use anvil_caldera::client::{AgentRunsResponse, RawAgentRuns};

    let fixture = r#"{
        "agent_runs": [
            {
                "run_id": "agent_001",
                "task": "Test task",
                "agent": "claude",
                "status": "completed",
                "created_at": "2026-05-22T10:00:01Z"
            }
        ]
    }"#;
    let raw: RawAgentRuns = serde_json::from_str(fixture).unwrap();
    let response = AgentRunsResponse::from(raw);

    assert_eq!(response.runs.len(), 1);
    assert_eq!(response.runs[0].status, RunStatus::Completed);
    // 2026-05-22T10:00:01Z should be > 0.
    assert!(response.runs[0].created_at_unix > 0);
}

/// Snapshot `#[serde(default)]` tolerance — partial JSON should deserialise cleanly.
#[test]
fn snapshot_partial_json_uses_defaults() {
    let json = r#"{"connection": "offline"}"#;
    let s: Snapshot = serde_json::from_str(json).unwrap();
    assert_eq!(s.connection, Connection::Offline);
    assert_eq!(s.running_count, 0);
    assert!(s.runs.is_empty());
    assert!(s.approvals.is_empty());
    assert!(s.findings.is_empty());
}

// ── 2. detect_project ─────────────────────────────────────────────────────────

#[test]
fn detect_returns_false_when_no_project_file() {
    let dir = tempdir();
    assert!(!detect_project(&dir));
}

#[test]
fn detect_returns_true_when_enabled() {
    let dir = tempdir();
    write_project_json(&dir, true);
    assert!(detect_project(&dir));
}

#[test]
fn detect_returns_false_when_disabled() {
    let dir = tempdir();
    write_project_json(&dir, false);
    assert!(!detect_project(&dir));
}

#[test]
fn detect_walks_up_to_parent() {
    let dir = tempdir();
    write_project_json(&dir, true);
    // Detect from a nested subdirectory.
    let nested = dir.join("a/b/c");
    std::fs::create_dir_all(&nested).unwrap();
    assert!(detect_project(&nested));
}

#[test]
fn detect_stops_at_disabled_ancestor() {
    let dir = tempdir();
    write_project_json(&dir, false);
    let nested = dir.join("deep");
    std::fs::create_dir_all(&nested).unwrap();
    assert!(!detect_project(&nested));
}

// ── 3. Connection state machine via fake HTTP server ─────────────────────────

/// Helper: spawn a fake HTTP server that handles one request then exits.
/// Returns the bound address and a join handle.
fn fake_server(response: &'static str) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (addr, handle)
}

const PROJECT_ENABLED: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"project\":{\"enabled\":true,\"project_name\":\"test\",\"mode\":\"local\"},\"config_path\":\".caldera/project.json\"}";

#[test]
fn connection_refused_produces_offline() {
    // Bind then drop the listener immediately so the port is closed.
    let addr = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };

    let client =
        CalderaClient::new(format!("http://{}", addr)).with_timeout(Duration::from_millis(500));
    let err = client.health();
    assert!(err.is_err(), "expected error on refused connection");
}

#[test]
fn http_500_produces_error_state() {
    let (addr, server) = fake_server(
        "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\n\r\n{\"error\":\"oops\"}",
    );
    let client =
        CalderaClient::new(format!("http://{}", addr)).with_timeout(Duration::from_millis(500));
    let err = client.health().unwrap_err();
    assert!(
        matches!(err, anvil_caldera::CalderaError::Http { status: 500, .. }),
        "expected HTTP 500 error, got {err:?}"
    );
    server.join().unwrap();
}

#[test]
fn project_disabled_flag_is_parsed() {
    let (addr, server) = fake_server(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"project\":{\"enabled\":false,\"project_name\":\"test\",\"mode\":\"local\"},\"config_path\":\".caldera/project.json\"}",
    );
    let client =
        CalderaClient::new(format!("http://{}", addr)).with_timeout(Duration::from_millis(500));
    let resp = client.project().unwrap();
    assert!(!resp.project.unwrap().enabled);
    server.join().unwrap();
}

#[test]
fn project_enabled_flag_is_parsed() {
    let (addr, server) = fake_server(PROJECT_ENABLED);
    let client =
        CalderaClient::new(format!("http://{}", addr)).with_timeout(Duration::from_millis(500));
    let resp = client.project().unwrap();
    assert!(resp.project.unwrap().enabled);
    server.join().unwrap();
}

#[test]
fn activity_response_parsed_from_server() {
    let body = r#"{"schema_version":"caldera.activity.v0","pending_approvals":[{"approval_id":"appr_001","connector":"k8s","pattern":"restart *","reason":"test approval"}],"recent_jobs":[],"recent_audit_events":[],"attention":[]}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    // leak so it's 'static
    let response: &'static str = Box::leak(response.into_boxed_str());
    let (addr, server) = fake_server(response);
    let client =
        CalderaClient::new(format!("http://{}", addr)).with_timeout(Duration::from_millis(500));
    let resp = client.activity().unwrap();
    assert_eq!(resp.approvals.len(), 1);
    assert_eq!(resp.approvals[0].connector, "k8s");
    server.join().unwrap();
}

// ── 4. Poller integration test ────────────────────────────────────────────────

/// A multi-request fake server that serves a fixed number of requests then
/// exits. Responses are served round-robin across accepted connections.
///
/// The server thread exits once `max_requests` connections have been handled,
/// which prevents the thread from blocking `poller.stop()` indefinitely.
struct ReusableServer {
    addr: std::net::SocketAddr,
    _handle: thread::JoinHandle<()>,
}

impl ReusableServer {
    /// Serve `max_requests` connections, cycling through `responses`.
    fn new(responses: Vec<String>, max_requests: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        // Short accept timeout so the server thread exits quickly after the
        // poller stops connecting.
        listener.set_nonblocking(false).ok();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut idx = 0;
            for _ in 0..max_requests {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = [0u8; 4096];
                        let _ = stream.read(&mut buf);
                        let resp = &responses[idx % responses.len()];
                        idx += 1;
                        let _ = stream.write_all(resp.as_bytes());
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            addr,
            _handle: handle,
        }
    }
}

#[test]
fn poller_transitions_through_live_state() {
    // The fake server serves health → project → activity → agent-runs for
    // every poll cycle. We use a round-robin over a fixed sequence.
    let responses = vec![
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}",
            r#"{"status":"ok","service":"caldera-local"}"#
        ),
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}",
            r#"{"project":{"enabled":true,"project_name":"test","mode":"local"},"config_path":".caldera/project.json"}"#
        ),
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}",
            r#"{"schema_version":"caldera.activity.v0","pending_approvals":[{"approval_id":"appr_001","connector":"k8s","pattern":"restart *","reason":"approved"}],"recent_jobs":[],"recent_audit_events":[],"attention":[]}"#
        ),
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}",
            r#"{"agent_runs":[{"run_id":"run_001","task":"Do work","agent":"claude","status":"running","created_at":"2026-05-22T10:00:00Z"}]}"#
        ),
    ];

    // 4 responses per poll cycle × 3 cycles max = 12 connections.
    let server = ReusableServer::new(responses, 12);

    // Set up a temp dir with a caldera project enabled.
    let repo = tempdir();
    write_project_json(&repo, true);

    let endpoint = format!("http://{}", server.addr);
    let mut poller = Poller::start(endpoint, repo);

    // Give the poller time to complete its first cycle.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut got_live = false;
    while std::time::Instant::now() < deadline {
        let snap = poller.snapshot();
        if snap.connection == Connection::Live {
            got_live = true;
            assert_eq!(snap.approvals.len(), 1);
            assert_eq!(snap.approvals[0].connector, "k8s");
            assert_eq!(snap.runs.len(), 1);
            assert_eq!(snap.running_count, 1);
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(got_live, "poller never reached Live state within deadline");
    poller.stop();
}

#[test]
fn poller_without_project_stays_no_project() {
    // No project.json → NoProject state, regardless of whether a server is up.
    let repo = tempdir();
    // No .caldera/project.json written.

    // Bind but never accept — connection refused path.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let endpoint = format!("http://{}", addr);
    let mut poller = Poller::start(endpoint, repo.clone());

    // Kick immediately so the first cycle runs fast.
    poller.kick();

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut saw_no_project = false;
    while std::time::Instant::now() < deadline {
        let snap = poller.snapshot();
        if snap.connection == Connection::NoProject {
            saw_no_project = true;
            break;
        }
        thread::sleep(Duration::from_millis(30));
    }
    assert!(
        saw_no_project,
        "expected NoProject, got {:?}",
        poller.snapshot().connection
    );
    poller.stop();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "anvil_caldera_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_project_json(dir: &PathBuf, enabled: bool) {
    let caldera_dir = dir.join(".caldera");
    std::fs::create_dir_all(&caldera_dir).unwrap();
    let json = format!(
        r#"{{"schema_version":"caldera.project.v0","enabled":{},"project_name":"Test","mode":"local","adapter_preflight":"/api/task-handoff"}}"#,
        enabled
    );
    std::fs::write(caldera_dir.join("project.json"), json).unwrap();
}
