//! Activity-snapshot types: the agent-domain data contract for the Anvil
//! agent panel.  These mirror `src/caldera/poller.zig` and are the single
//! owned definition of the agent schema consumed by `anvil-caldera` (HTTP
//! client) and `anvil-control` / `anvil-render` (agent panel).
//!
//! Design notes:
//! - Fixed-size Zig buffers (`[24]u8` + length field) are replaced with
//!   idiomatic `String` / `Vec<T>` — the allocation constraint was a Zig
//!   ergonomics workaround, not a semantic requirement.
//! - `#[serde(default)]` on struct fields and `#[serde(deny_unknown_fields)]`
//!   is intentionally NOT applied at the struct level so that schema additions
//!   in caldera-local are silently ignored rather than crashing.

use serde::{Deserialize, Serialize};

/// Connection state between Anvil and caldera-local.
///
/// `NotInstalled` is the zero/default value, matching the Zig `.not_installed`
/// zero variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    #[default]
    NotInstalled,
    NoProject,
    Disabled,
    Offline,
    ErrorState,
    Live,
}

/// Status of a single agent run.
///
/// `Unknown` is the zero/default value, matching the Zig `.unknown` zero
/// variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    Abandoned,
    #[default]
    Unknown,
}

/// Severity levels that map to semantic status colors.
///
/// `Info` is the zero/default value, matching the Zig `.info` zero variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    #[default]
    Info,
    Attention,
    Risk,
    Failure,
}

/// One row in the agent-runs table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRunRow {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub status: RunStatus,
    #[serde(default)]
    pub created_at_unix: i64,
}

/// One pending approval row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRow {
    #[serde(default)]
    pub approval_id: String,
    #[serde(default)]
    pub connector: String,
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub reason: String,
}

/// One finding row (attention item / risk / failure surfaced by an agent).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingRow {
    #[serde(default)]
    pub severity: FindingSeverity,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub action: String,
}

/// A complete snapshot of agent state.
///
/// `connection = Connection::NotInstalled` is the safe zero-value default,
/// matching the Zig `Snapshot` default.  `Vec` fields replace the Zig fixed
/// arrays + count fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub connection: Connection,
    #[serde(default)]
    pub runs: Vec<AgentRunRow>,
    #[serde(default)]
    pub approvals: Vec<ApprovalRow>,
    #[serde(default)]
    pub findings: Vec<FindingRow>,
    #[serde(default)]
    pub running_count: u8,
    #[serde(default)]
    pub pending_approvals_count: u8,
    #[serde(default)]
    pub attention_count: u8,
    #[serde(default)]
    pub polled_at_unix: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Connection round-trip ---

    #[test]
    fn connection_roundtrip_not_installed() {
        let v = Connection::NotInstalled;
        let json = serde_json::to_string(&v).unwrap();
        let back: Connection = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn connection_roundtrip_live() {
        let v = Connection::Live;
        let json = serde_json::to_string(&v).unwrap();
        let back: Connection = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn connection_default_is_not_installed() {
        assert_eq!(Connection::default(), Connection::NotInstalled);
    }

    // --- RunStatus round-trip ---

    #[test]
    fn run_status_roundtrip_all_variants() {
        for v in [
            RunStatus::Running,
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Abandoned,
            RunStatus::Unknown,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let back: RunStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back, "round-trip failed for {v:?}");
        }
    }

    #[test]
    fn run_status_default_is_unknown() {
        assert_eq!(RunStatus::default(), RunStatus::Unknown);
    }

    // --- FindingSeverity round-trip ---

    #[test]
    fn finding_severity_roundtrip_all_variants() {
        for v in [
            FindingSeverity::Info,
            FindingSeverity::Attention,
            FindingSeverity::Risk,
            FindingSeverity::Failure,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let back: FindingSeverity = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back, "round-trip failed for {v:?}");
        }
    }

    #[test]
    fn finding_severity_default_is_info() {
        assert_eq!(FindingSeverity::default(), FindingSeverity::Info);
    }

    // --- Snapshot round-trip ---

    #[test]
    fn snapshot_default_connection_is_not_installed() {
        let s = Snapshot::default();
        assert_eq!(s.connection, Connection::NotInstalled);
        assert!(s.runs.is_empty());
        assert!(s.approvals.is_empty());
        assert!(s.findings.is_empty());
    }

    #[test]
    fn snapshot_roundtrip() {
        let s = Snapshot {
            connection: Connection::Live,
            runs: vec![AgentRunRow {
                run_id: "run-001".into(),
                agent: "lmstudio".into(),
                task: "Review PR".into(),
                status: RunStatus::Running,
                created_at_unix: 1_716_000_000,
            }],
            approvals: vec![ApprovalRow {
                approval_id: "appr-001".into(),
                connector: "github".into(),
                pattern: "push *".into(),
                reason: "CI gate".into(),
            }],
            findings: vec![FindingRow {
                severity: FindingSeverity::Risk,
                summary: "Dependency outdated".into(),
                action: "Run cargo update".into(),
            }],
            running_count: 1,
            pending_approvals_count: 1,
            attention_count: 1,
            polled_at_unix: 1_716_000_010,
        };

        let json = serde_json::to_string(&s).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    // --- Unknown-field tolerance ---

    #[test]
    fn snapshot_ignores_unknown_fields() {
        let json = r#"{
            "connection": "live",
            "runs": [],
            "approvals": [],
            "findings": [],
            "running_count": 0,
            "pending_approvals_count": 0,
            "attention_count": 0,
            "polled_at_unix": 0,
            "future_field_from_caldera": "ignored"
        }"#;
        let s: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(s.connection, Connection::Live);
    }

    #[test]
    fn agent_run_row_ignores_unknown_fields() {
        let json = r#"{
            "run_id": "r1",
            "agent": "claude",
            "task": "do things",
            "status": "completed",
            "created_at_unix": 0,
            "extra_future_field": true
        }"#;
        let row: AgentRunRow = serde_json::from_str(json).unwrap();
        assert_eq!(row.run_id, "r1");
        assert_eq!(row.status, RunStatus::Completed);
    }

    #[test]
    fn approval_row_ignores_unknown_fields() {
        let json = r#"{
            "approval_id": "a1",
            "connector": "github",
            "pattern": "*",
            "reason": "test",
            "new_field": 99
        }"#;
        let row: ApprovalRow = serde_json::from_str(json).unwrap();
        assert_eq!(row.approval_id, "a1");
    }

    #[test]
    fn finding_row_ignores_unknown_fields() {
        let json = r#"{
            "severity": "failure",
            "summary": "broken",
            "action": "fix it",
            "added_later": null
        }"#;
        let row: FindingRow = serde_json::from_str(json).unwrap();
        assert_eq!(row.severity, FindingSeverity::Failure);
    }

    // --- Partial / missing fields use serde defaults ---

    #[test]
    fn snapshot_missing_fields_use_defaults() {
        let json = r#"{"connection": "offline"}"#;
        let s: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(s.connection, Connection::Offline);
        assert_eq!(s.running_count, 0);
        assert_eq!(s.polled_at_unix, 0);
        assert!(s.runs.is_empty());
    }
}
