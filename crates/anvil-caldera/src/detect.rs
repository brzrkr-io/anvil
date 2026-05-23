//! Repo opt-in detection: walks up from `cwd` looking for
//! `.caldera/project.json` with `"enabled": true`.
//!
//! This is a pure file-system gate. It runs offline and is cheap enough to
//! call on every poller wake-up to handle the user switching projects.

use std::path::Path;

/// Walk up from `cwd`, looking for `.caldera/project.json` with
/// `"enabled": true`.
///
/// Returns `true` when a project file with `enabled: true` is found.
/// Returns `false` when:
/// - no `.caldera/project.json` is found anywhere in the ancestor chain, or
/// - the file is found but `enabled` is absent or `false`, or
/// - the file cannot be read or parsed.
pub fn detect_project(cwd: &Path) -> bool {
    let mut dir = cwd;
    loop {
        let candidate = dir.join(".caldera/project.json");
        if candidate.exists() {
            return is_enabled(&candidate);
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return false,
        }
    }
}

fn is_enabled(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    value
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
