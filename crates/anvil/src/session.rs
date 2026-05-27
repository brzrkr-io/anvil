//! Session persistence — item 19.
//!
//! Saves and restores per-workspace UI state so Anvil picks up where it left
//! off after a relaunch.  State is written to
//! `~/.config/anvil/sessions/<cwd_hash>.json` where `<cwd_hash>` is the
//! 16-character lower-hex hash of the workspace cwd produced by
//! `std::collections::hash_map::DefaultHasher`.
//!
//! The format is plain JSON serialised by serde_json.  No external crate is
//! needed beyond what is already in the workspace.
//!
//! Failure modes
//! - Missing session file → silent no-op; caller gets `None`.
//! - Malformed JSON → logged once to stderr; caller gets `None`.
//! - Write error → logged once to stderr; best-effort, never panics.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── Data types ────────────────────────────────────────────────────────────────

/// All UI state that survives a relaunch.
///
/// Stored as a flat JSON object; adding new optional fields is
/// backwards-compatible.
/// Bump on any default change that should invalidate older saved sessions.
/// Older sessions with `version < CURRENT_VERSION` are ignored on load.
pub const CURRENT_VERSION: u32 = 3;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionState {
    /// Schema version. Sessions with `version < CURRENT_VERSION` are ignored
    /// on load to prevent stale defaults from clobbering new ones.
    #[serde(default)]
    pub version: u32,

    /// Global UI scale multiplier (Cmd+=/Cmd+- zoom).  Default 1.0.
    pub ui_scale: f64,

    /// Explorer sidebar width in logical points.  Default 300.
    pub left_dock_w_pt: f64,

    /// Layout mode: `"terminal"` or `"ide"`.  Default `"terminal"`.
    pub layout_mode: String,

    /// Editor-over-drawer ratio for the IDE vertical split (0.0–1.0).
    /// 0.0 = use the hard-coded default; non-zero overrides.
    pub editor_split_ratio: f64,

    /// Expanded directory paths in the Explorer.
    pub expanded_dirs: Vec<PathBuf>,

    /// Open buffer paths per pane, keyed by stringified pane id.
    /// Each entry is the ordered list of open file paths for that pane.
    pub open_buffers: Vec<PaneSession>,

    /// Recently-opened workspace directories (item 30).
    /// Capped at 20; most-recent first.  Persisted across runs.
    #[serde(default)]
    pub recent_projects: Vec<PathBuf>,

    /// H4: font-only scale multiplier (Cmd+Opt+=/- zoom).
    /// Applied on top of `font_size_pt * window_scale`; does NOT scale
    /// dock widths, chrome heights, or row heights.  Default 1.0.
    #[serde(default)]
    pub font_scale: f64,

    /// Q56: whether the Explorer shows dot-prefix (hidden) entries.
    /// Default false.
    #[serde(default)]
    pub show_hidden_files: bool,

    /// Q22: per-buffer language overrides. Key is the file path (as a string);
    /// value is the LSP language-id (e.g. `"rust"`).
    #[serde(default)]
    pub language_overrides: std::collections::HashMap<String, String>,
}

/// Per-pane buffer state.
#[derive(Debug, Serialize, Deserialize)]
pub struct PaneSession {
    /// Stringified pane id (used only to order sessions deterministically;
    /// actual restore opens the paths in the main editor pane, not per-pane).
    pub pane_id: u64,
    /// Ordered list of open file paths.
    pub paths: Vec<PathBuf>,
    /// The active (focused) buffer path, if any.
    pub active_path: Option<PathBuf>,
}

// ── Path helpers ──────────────────────────────────────────────────────────────

/// Compute the session file path for `cwd`.
///
/// Returns `None` when `HOME` is not set.
pub fn session_path(cwd: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut dir = PathBuf::from(home);
    dir.push(".config");
    dir.push("anvil");
    dir.push("sessions");

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cwd.hash(&mut hasher);
    let hash = hasher.finish();
    let filename = format!("{hash:016x}.json");

    Some(dir.join(filename))
}

// ── I/O ───────────────────────────────────────────────────────────────────────

/// Write `state` to the session file for `cwd`.
///
/// Creates parent directories if needed.  Logs and returns on error (never
/// panics).
pub fn save_session(cwd: &Path, state: &SessionState) {
    let Some(path) = session_path(cwd) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = match serde_json::to_string_pretty(state) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("anvil-session: failed to serialise session: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(&path, json.as_bytes()) {
        eprintln!("anvil-session: failed to write {}: {e}", path.display());
    }
}

/// Load and parse the session file for `cwd`.
///
/// Returns `None` silently when the file does not exist.
/// Logs once and returns `None` on parse error.
pub fn load_session(cwd: &Path) -> Option<SessionState> {
    let path = session_path(cwd)?;
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return None, // file not found → silent no-op
    };
    match serde_json::from_slice::<SessionState>(&bytes) {
        Ok(s) => {
            if s.version < CURRENT_VERSION {
                eprintln!(
                    "anvil-session: {} has version {} < {}; ignoring and starting fresh",
                    path.display(),
                    s.version,
                    CURRENT_VERSION
                );
                return None;
            }
            Some(s)
        }
        Err(e) => {
            eprintln!(
                "anvil-session: failed to parse {}, starting fresh: {e}",
                path.display()
            );
            None
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_path_is_16_hex_chars() {
        let cwd = Path::new("/home/user/project");
        if let Some(p) = session_path(cwd) {
            let stem = p.file_stem().unwrap().to_str().unwrap();
            assert_eq!(stem.len(), 16);
            assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
        }
        // If HOME is unset the function returns None, which is valid.
    }

    #[test]
    fn session_path_is_stable() {
        let cwd = Path::new("/home/user/project");
        // Same cwd must produce the same path on every call.
        assert_eq!(session_path(cwd), session_path(cwd));
    }

    #[test]
    fn session_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();

        let state = SessionState {
            version: CURRENT_VERSION,
            ui_scale: 1.25,
            font_scale: 1.1,
            left_dock_w_pt: 320.0,
            layout_mode: "ide".to_string(),
            editor_split_ratio: 0.72,
            expanded_dirs: vec![PathBuf::from("/home/user/project/src")],
            open_buffers: vec![PaneSession {
                pane_id: 1,
                paths: vec![PathBuf::from("/home/user/project/src/main.rs")],
                active_path: Some(PathBuf::from("/home/user/project/src/main.rs")),
            }],
            recent_projects: vec![PathBuf::from("/home/user/project")],
            ..Default::default()
        };

        save_session(cwd, &state);
        let loaded = load_session(cwd).expect("session must load after save");

        assert!((loaded.ui_scale - 1.25).abs() < f64::EPSILON);
        assert!((loaded.left_dock_w_pt - 320.0).abs() < f64::EPSILON);
        assert_eq!(loaded.layout_mode, "ide");
        assert!((loaded.editor_split_ratio - 0.72).abs() < f64::EPSILON);
        assert_eq!(
            loaded.expanded_dirs,
            vec![PathBuf::from("/home/user/project/src")]
        );
        assert_eq!(loaded.open_buffers.len(), 1);
        assert_eq!(loaded.open_buffers[0].pane_id, 1);
        assert_eq!(
            loaded.open_buffers[0].active_path,
            Some(PathBuf::from("/home/user/project/src/main.rs"))
        );
    }

    #[test]
    fn load_session_missing_file_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        // No file written — must return None silently.
        assert!(load_session(cwd).is_none());
    }

    #[test]
    fn load_session_corrupt_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        // Write garbage to the expected path.
        if let Some(path) = session_path(cwd) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"not valid json").unwrap();
        }
        // Must return None (logs to stderr, does not panic).
        assert!(load_session(cwd).is_none());
    }

    #[test]
    fn session_recent_projects_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let state = SessionState {
            version: CURRENT_VERSION,
            ui_scale: 1.0,
            font_scale: 1.0,
            left_dock_w_pt: 300.0,
            layout_mode: "terminal".to_string(),
            editor_split_ratio: 0.0,
            expanded_dirs: vec![],
            open_buffers: vec![],
            recent_projects: vec![
                PathBuf::from("/home/user/project-a"),
                PathBuf::from("/home/user/project-b"),
            ],
            ..Default::default()
        };
        save_session(cwd, &state);
        let loaded = load_session(cwd).expect("must load");
        assert_eq!(
            loaded.recent_projects,
            vec![
                PathBuf::from("/home/user/project-a"),
                PathBuf::from("/home/user/project-b"),
            ]
        );
    }

    #[test]
    fn session_missing_recent_projects_defaults_to_empty() {
        // Old session JSON without the `recent_projects` field should
        // deserialise fine (serde default).
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let old_json = r#"{"version":3,"ui_scale":1.0,"left_dock_w_pt":300.0,"layout_mode":"terminal","editor_split_ratio":0.0,"expanded_dirs":[],"open_buffers":[]}"#;
        if let Some(path) = session_path(cwd) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, old_json.as_bytes()).unwrap();
        }
        let loaded = load_session(cwd).expect("must load");
        assert!(
            loaded.recent_projects.is_empty(),
            "old session without recent_projects must deserialise to empty vec"
        );
    }

    /// H4: `font_scale` persists and restores correctly.
    #[test]
    fn session_font_scale_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let state = SessionState {
            version: CURRENT_VERSION,
            ui_scale: 1.0,
            font_scale: 1.3,
            left_dock_w_pt: 300.0,
            layout_mode: "terminal".to_string(),
            editor_split_ratio: 0.0,
            expanded_dirs: vec![],
            open_buffers: vec![],
            recent_projects: vec![],
            ..Default::default()
        };
        save_session(cwd, &state);
        let loaded = load_session(cwd).expect("must load");
        assert!(
            (loaded.font_scale - 1.3).abs() < f64::EPSILON,
            "font_scale must round-trip; got {}",
            loaded.font_scale
        );
    }

    /// H4: old session JSON without `font_scale` defaults to 0.0 (treated as
    /// "not set" by restore_session → keeps the runtime default of 1.0).
    #[test]
    fn session_font_scale_missing_defaults_to_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let old_json = r#"{"version":3,"ui_scale":1.0,"left_dock_w_pt":300.0,"layout_mode":"terminal","editor_split_ratio":0.0,"expanded_dirs":[],"open_buffers":[]}"#;
        if let Some(path) = session_path(cwd) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, old_json.as_bytes()).unwrap();
        }
        let loaded = load_session(cwd).expect("must load");
        assert_eq!(
            loaded.font_scale, 0.0,
            "missing font_scale should default to 0.0"
        );
    }
}
