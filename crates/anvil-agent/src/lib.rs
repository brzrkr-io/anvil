//! Agent schema: shared message types for AI-native tooling.

pub mod snapshot;
pub use snapshot::{
    AgentRunRow, ApprovalRow, Connection, FindingRow, FindingSeverity, RunStatus, Snapshot,
};

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStartRequest {
    pub task: String,
    pub agent: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStartResponse {
    pub schema_version: String,
    pub session_id: String,
    pub task: String,
    pub agent: String,
    pub status: String,
    pub handoff_path: String,
    pub handoff_markdown: String,
    pub context_cache: ContextCacheSummary,
    pub launch: LaunchContract,
    pub run: AgentRun,
    #[serde(default)]
    pub next: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentSessionResponse {
    pub schema_version: String,
    pub session: Option<AgentRun>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextCacheSummary {
    pub status: String,
    pub source_state: String,
    pub refresh_reason: String,
    #[serde(default)]
    pub estimated_tokens_saved: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchContract {
    pub mode: String,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub shell_command: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub handoff_file: String,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub instructions: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRun {
    pub schema_version: String,
    pub run_id: String,
    pub task: String,
    pub agent: String,
    pub status: String,
    pub workspace_root: String,
    pub created_at: String,
    #[serde(default)]
    pub finished_at: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub handoff_path: String,
    #[serde(default)]
    pub context_cache_status: String,
    #[serde(default)]
    pub launch: LaunchContract,
}

impl SessionStartRequest {
    pub fn new(task: impl Into<String>, agent: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            agent: agent.into(),
        }
    }
}

impl SessionStartResponse {
    pub fn is_prepared(&self) -> bool {
        self.status == "prepared" && self.run.status == "prepared"
    }

    pub fn launch_command(&self) -> &[String] {
        &self.launch.command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_start_contract() {
        let body = r##"{
          "schema_version": "caldera.session_start.v0",
          "session_id": "agent_1",
          "task": "Review repo",
          "agent": "lmstudio",
          "status": "prepared",
          "handoff_path": ".forge/agent-run-handoffs/agent_1.md",
          "handoff_markdown": "# Caldera AI Session",
          "context_cache": {
            "status": "ready",
            "source_state": "current",
            "refresh_reason": "rust-runtime-refresh",
            "estimated_tokens_saved": 1200
          },
          "launch": {
            "mode": "prepared",
            "command": ["caldera-local", "ask"],
            "shell_command": "caldera-local ask",
            "working_directory": "/repo",
            "handoff_file": "/repo/.forge/agent-run-handoffs/agent_1.md",
            "environment": {"CALDERA_SESSION_ID": "agent_1"},
            "instructions": "Run local model"
          },
          "run": {
            "schema_version": "caldera.agent_run.v0",
            "run_id": "agent_1",
            "task": "Review repo",
            "agent": "lmstudio",
            "status": "prepared",
            "workspace_root": "/repo",
            "created_at": "2026-05-22T00:00:00Z",
            "finished_at": ""
          },
          "next": []
        }"##;

        let parsed: SessionStartResponse = serde_json::from_str(body).unwrap();

        assert!(parsed.is_prepared());
        assert_eq!(parsed.session_id, "agent_1");
        assert_eq!(parsed.launch_command(), ["caldera-local", "ask"]);
    }
}
