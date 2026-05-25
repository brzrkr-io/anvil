//! CI status types. Populated by a background `gh-ci` poller (future work).

/// Current state of the last CI run on the active branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CiState {
    Running,
    Ok,
    Failed,
    Unknown,
}

/// Snapshot of CI state for the active branch.
#[derive(Clone, Debug)]
pub struct CiStatus {
    pub state: CiState,
    pub branch: String,
    pub duration_s: u32,
    pub open_prs: u32,
    pub pr_url: String,
}
