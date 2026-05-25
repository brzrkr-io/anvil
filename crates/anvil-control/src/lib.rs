//! Control surface: dual-transport (keyboard + agent) input handling.

pub mod bridge;

/// Re-export the editor action enum so callers can `use anvil_control::EditorAction`
/// without depending directly on `anvil-workspace`.
pub use anvil_workspace::editor_pane::EditorAction;

// Re-export AI-native buffer types so caldera (external) can depend on this
// crate alone rather than on `anvil-editor` directly (NE14).
pub use anvil_editor::BufferId;
pub use anvil_editor::buffer::{
    AgentRevision, Edit, EditProposal, GhostTextSpan, Position, ProposalError, ProposalStatus,
};
pub use anvil_workspace::layout::PaneId;

/// Inbound messages from an external agent (caldera or similar).
///
/// These differ from [`bridge::Inbound`] (which is web-bridge only).
/// An external agent constructs these variants and hands them to the host
/// process; `main.rs` routes each variant to the appropriate buffer API.
///
/// No actual wire protocol is defined here — that is caldera-side work.
/// This enum defines the Rust API surface the agent bridge must satisfy.
#[derive(Debug, Clone)]
pub enum AgentInbound {
    /// Request that an edit be staged as a proposal on the focused buffer.
    AgentPropose {
        pane_id: PaneId,
        agent_id: String,
        edit: Edit,
        rationale: Option<String>,
    },
    /// Set a ghost-text suggestion for the focused buffer at `anchor`.
    AgentSetGhost {
        pane_id: PaneId,
        agent_id: String,
        anchor: Position,
        text: String,
    },
    /// Clear all ghost-text suggestions on the focused buffer.
    AgentClearGhost { pane_id: PaneId },
}

use anvil_agent::{SessionStartRequest, SessionStartResponse};
use anvil_caldera::{CalderaClient, CalderaError};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct AiSessionBroker {
    client: CalderaClient,
}

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("Caldera session broker failed: {0}")]
    Caldera(#[from] CalderaError),
}

impl AiSessionBroker {
    pub fn new(client: CalderaClient) -> Self {
        Self { client }
    }

    pub fn localhost() -> Self {
        Self::new(CalderaClient::localhost())
    }

    pub fn prepare_repo_session(
        &self,
        task: impl Into<String>,
        agent: impl Into<String>,
    ) -> Result<SessionStartResponse, ControlError> {
        let request = SessionStartRequest::new(task, agent);
        Ok(self.client.start_session(&request)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn broker_prepares_session_through_caldera_client() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("POST /api/sessions/start HTTP/1.1"));
            assert!(request.contains("\"agent\":\"codex\""));

            let body = r##"{
              "schema_version": "caldera.session_start.v0",
              "session_id": "agent_2",
              "task": "Open repo",
              "agent": "codex",
              "status": "prepared",
              "handoff_path": ".forge/agent-run-handoffs/agent_2.md",
              "handoff_markdown": "# Caldera AI Session",
              "context_cache": {
                "status": "ready",
                "source_state": "current",
                "refresh_reason": "test",
                "estimated_tokens_saved": 1
              },
              "launch": {
                "mode": "prepared",
                "command": ["codex"],
                "shell_command": "codex",
                "working_directory": "/repo",
                "handoff_file": "/repo/.forge/agent-run-handoffs/agent_2.md",
                "environment": {},
                "instructions": "Start Codex"
              },
              "run": {
                "schema_version": "caldera.agent_run.v0",
                "run_id": "agent_2",
                "task": "Open repo",
                "agent": "codex",
                "status": "prepared",
                "workspace_root": "/repo",
                "created_at": "2026-05-22T00:00:00Z",
                "finished_at": ""
              },
              "next": []
            }"##;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let broker = AiSessionBroker::new(CalderaClient::new(format!("http://{}", addr)));
        let session = broker.prepare_repo_session("Open repo", "codex").unwrap();

        assert!(session.is_prepared());
        assert_eq!(session.launch.shell_command, "codex");
        server.join().unwrap();
    }

    #[test]
    fn localhost_broker_connects_to_default_endpoint() {
        // localhost() should construct successfully (no connection made on construction).
        let _broker = AiSessionBroker::localhost();
        // Just verifies no panic at construction time.
    }
}
