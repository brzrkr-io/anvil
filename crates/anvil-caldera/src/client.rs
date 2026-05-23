//! Intermediate response types for caldera-local endpoints.
//!
//! These types reflect the actual JSON shapes returned by caldera-local and
//! are converted to `anvil_agent` canonical types before leaving this module.
//! This insulates the rest of the crate from caldera-local JSON quirks
//! (e.g. `"warning"` severity, ISO timestamps for runs, etc.).

use anvil_agent::{AgentRunRow, ApprovalRow, FindingRow, FindingSeverity, RunStatus};
use serde::{Deserialize, Serialize};

// ── Activity response ─────────────────────────────────────────────────────────

/// Parsed `/api/activity` response.
///
/// Converted from caldera-local's JSON; fields not needed by the agent panel
/// are dropped.
pub struct ActivityResponse {
    pub approvals: Vec<ApprovalRow>,
    pub findings: Vec<FindingRow>,
}

/// Raw `/api/activity` JSON shape from caldera-local.
#[derive(Debug, Deserialize)]
pub struct RawActivity {
    #[serde(default)]
    pub pending_approvals: Vec<RawApproval>,
    #[serde(default)]
    pub attention: Vec<RawFinding>,
}

#[derive(Debug, Deserialize)]
pub struct RawApproval {
    #[serde(default)]
    pub approval_id: String,
    #[serde(default)]
    pub connector: String,
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct RawFinding {
    #[serde(default)]
    pub severity: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub recommended_action: String,
}

impl From<RawActivity> for ActivityResponse {
    fn from(raw: RawActivity) -> Self {
        let approvals = raw
            .pending_approvals
            .into_iter()
            .map(|a| ApprovalRow {
                approval_id: a.approval_id,
                connector: a.connector,
                pattern: a.pattern,
                reason: a.reason,
            })
            .collect();

        let findings = raw
            .attention
            .into_iter()
            .map(|f| FindingRow {
                severity: map_severity(&f.severity),
                summary: f.summary,
                action: f.recommended_action,
            })
            .collect();

        ActivityResponse {
            approvals,
            findings,
        }
    }
}

/// Map caldera-local severity strings to `FindingSeverity`.
///
/// caldera-local currently uses `"warning"` and `"critical"`.
/// Unknown values map to `Info` (safe default).
fn map_severity(s: &str) -> FindingSeverity {
    match s {
        "warning" => FindingSeverity::Attention,
        "critical" | "failure" | "error" => FindingSeverity::Failure,
        "risk" => FindingSeverity::Risk,
        "attention" => FindingSeverity::Attention,
        "info" => FindingSeverity::Info,
        _ => FindingSeverity::Info,
    }
}

// ── Agent-runs response ───────────────────────────────────────────────────────

/// Parsed `/api/agent-runs` response.
pub struct AgentRunsResponse {
    pub runs: Vec<AgentRunRow>,
}

#[derive(Debug, Deserialize)]
pub struct RawAgentRuns {
    #[serde(default)]
    pub agent_runs: Vec<RawAgentRun>,
}

#[derive(Debug, Deserialize)]
pub struct RawAgentRun {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub status: String,
    /// ISO 8601 timestamp string, e.g. "2026-05-22T10:00:01Z".
    #[serde(default)]
    pub created_at: String,
}

impl From<RawAgentRuns> for AgentRunsResponse {
    fn from(raw: RawAgentRuns) -> Self {
        let runs = raw
            .agent_runs
            .into_iter()
            .map(|r| AgentRunRow {
                run_id: r.run_id,
                agent: r.agent,
                task: r.task,
                status: map_run_status(&r.status),
                created_at_unix: parse_iso_unix(&r.created_at),
            })
            .collect();
        AgentRunsResponse { runs }
    }
}

/// Map caldera-local status strings to `RunStatus`.
fn map_run_status(s: &str) -> RunStatus {
    match s {
        "running" | "started" => RunStatus::Running,
        "completed" | "prepared" => RunStatus::Completed,
        "failed" => RunStatus::Failed,
        "abandoned" => RunStatus::Abandoned,
        _ => RunStatus::Unknown,
    }
}

/// Parse an ISO 8601 UTC timestamp to a Unix timestamp (seconds).
///
/// Accepts the `"2026-05-22T10:00:01Z"` format produced by caldera-local.
/// Returns 0 on any parse failure rather than propagating an error.
fn parse_iso_unix(s: &str) -> i64 {
    // Format: YYYY-MM-DDTHH:MM:SSZ
    let s = s.trim_end_matches('Z');
    let parts: Vec<&str> = s.splitn(2, 'T').collect();
    if parts.len() != 2 {
        return 0;
    }
    let date_parts: Vec<u32> = parts[0].split('-').filter_map(|p| p.parse().ok()).collect();
    let time_parts: Vec<u32> = parts[1].split(':').filter_map(|p| p.parse().ok()).collect();
    if date_parts.len() < 3 || time_parts.len() < 3 {
        return 0;
    }
    let (year, month, day) = (
        date_parts[0] as i64,
        date_parts[1] as i64,
        date_parts[2] as i64,
    );
    let (hour, min, sec) = (
        time_parts[0] as i64,
        time_parts[1] as i64,
        time_parts[2] as i64,
    );
    // Days from epoch (1970-01-01) via a simple Gregorian formula.
    let days = days_from_epoch(year, month, day);
    days * 86400 + hour * 3600 + min * 60 + sec
}

/// Compute days since 1970-01-01 (Unix epoch) for a Gregorian date.
fn days_from_epoch(year: i64, month: i64, day: i64) -> i64 {
    // Rata Die algorithm (proleptic Gregorian).
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let rd = era * 146097 + doe - 719468; // offset to Unix epoch
    rd
}

// ── Project response ──────────────────────────────────────────────────────────

/// Parsed `/api/project` response.
pub struct ProjectResponse {
    pub project: Option<ProjectInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectInfo {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct RawProject {
    pub project: Option<ProjectInfo>,
}

impl From<RawProject> for ProjectResponse {
    fn from(raw: RawProject) -> Self {
        ProjectResponse {
            project: raw.project,
        }
    }
}

// ── POST request bodies ───────────────────────────────────────────────────────

/// Body for `POST /api/approvals`.
#[derive(Debug, Serialize)]
pub struct ApprovalRequest {
    pub connector: String,
    pub pattern: String,
    pub reason: String,
    pub ttl_seconds: u32,
}

/// Body for `POST /api/findings/ack`.
#[derive(Debug, Serialize)]
pub struct AckFindingRequest {
    pub code: String,
    pub reason: String,
}

/// Body for `POST /api/task-handoff`.
#[derive(Debug, Serialize)]
pub struct StartRunRequest {
    pub task: String,
    pub agent: String,
}
