//! Agent-state data types for the Anvil agent surface (AG1 phase).
//! This file is types-only — no thread, no networking, no runtime dependency.
//! AG2 will add a background poller that populates `Snapshot` from the local
//! Caldera API.

/// Connection state between Anvil and caldera-local.
pub const Connection = enum {
    not_installed,
    no_project,
    disabled,
    offline,
    error_state,
    live,
};

/// Status of a single agent run.
pub const RunStatus = enum {
    running,
    completed,
    failed,
    abandoned,
    unknown,
};

/// One row in the agent-runs table.
pub const AgentRunRow = struct {
    run_id: [24]u8 = undefined,
    run_id_len: u8 = 0,
    agent: [24]u8 = undefined,
    agent_len: u8 = 0,
    task: [80]u8 = undefined,
    task_len: u8 = 0,
    status: RunStatus = .unknown,
    created_at_unix: i64 = 0,
};

/// One pending approval row.
pub const ApprovalRow = struct {
    approval_id: [24]u8 = undefined,
    approval_id_len: u8 = 0,
    connector: [32]u8 = undefined,
    connector_len: u8 = 0,
    pattern: [80]u8 = undefined,
    pattern_len: u8 = 0,
    reason: [80]u8 = undefined,
    reason_len: u8 = 0,
};

/// Severity levels that map to semantic status colors.
pub const FindingSeverity = enum { info, attention, risk, failure };

/// One finding row (attention item / risk / failure surfaced by an agent).
pub const FindingRow = struct {
    severity: FindingSeverity = .info,
    summary: [80]u8 = undefined,
    summary_len: u8 = 0,
    action: [80]u8 = undefined,
    action_len: u8 = 0,
};

/// A complete snapshot of agent state. Plain data — no allocations.
/// `connection = .not_installed` is the safe zero-value default.
pub const Snapshot = struct {
    connection: Connection = .not_installed,
    runs: [8]AgentRunRow = undefined,
    runs_len: u8 = 0,
    approvals: [6]ApprovalRow = undefined,
    approvals_len: u8 = 0,
    findings: [8]FindingRow = undefined,
    findings_len: u8 = 0,
    running_count: u8 = 0,
    pending_approvals_count: u8 = 0,
    attention_count: u8 = 0,
    polled_at_unix: i64 = 0,
};
