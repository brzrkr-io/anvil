//! Action helpers: POST request builders for approval, finding ack, and run
//! start. Each function issues the HTTP request through `CalderaClient` and
//! returns the raw JSON body string on success.
//!
//! These are fire-and-forget from the poller's perspective: the caller should
//! call `Poller::kick` after an action so the next poll picks up the new state.

use crate::{CalderaClient, CalderaError};
use serde_json::json;

/// Create a scoped one-use approval.
///
/// `ttl_seconds` sets how long the approval is valid.
pub fn approve(
    client: &CalderaClient,
    connector: &str,
    pattern: &str,
    reason: &str,
    ttl_seconds: u32,
) -> Result<String, CalderaError> {
    let body = json!({
        "connector": connector,
        "pattern": pattern,
        "reason": reason,
        "ttl_seconds": ttl_seconds,
    });
    client.post_raw("/api/approvals", &body.to_string())
}

/// Acknowledge a reviewed finding by its code.
///
/// Acknowledged findings are hidden from the active dashboard view while their
/// original audit evidence is preserved.
pub fn ack_finding(
    client: &CalderaClient,
    code: &str,
    reason: &str,
) -> Result<String, CalderaError> {
    let body = json!({
        "code": code,
        "reason": reason,
    });
    client.post_raw("/api/findings/ack", &body.to_string())
}

/// Start a new agent run via the task-handoff endpoint.
///
/// Returns the raw JSON response body. The caller should call `Poller::kick`
/// after this so the snapshot reflects the new run.
pub fn start_run(client: &CalderaClient, task: &str, agent: &str) -> Result<String, CalderaError> {
    let body = json!({
        "task": task,
        "agent": agent,
    });
    client.post_raw("/api/task-handoff", &body.to_string())
}
