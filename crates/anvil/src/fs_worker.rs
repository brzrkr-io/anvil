//! Filesystem background worker — reads a directory's top-level entries and
//! sends a [`DirSnapshot`] back to the main thread.
//!
//! Mirrors `crates/anvil/src/kube.rs`: named thread, `SyncSender` in,
//! `Receiver` out, non-blocking try_send on the return path.
//!
//! Threading model:
//! 1. Main thread: `tx.try_send(path)` on cwd change.
//! 2. Worker: drains the channel (takes the latest), reads dir, sends snapshot.
//! 3. 2-second per-path debounce: repeated identical paths within 2 s are dropped.
//! 4. Main thread per frame: `while let Ok(s) = rx.try_recv()` — mirror kube drain.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A single directory entry produced by the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// A top-level directory listing sent from the worker to the main thread.
#[derive(Debug, Clone)]
pub struct DirSnapshot {
    pub root: PathBuf,
    pub entries: Vec<DirEntry>,
    /// Git status marks per filename (item 10).
    ///
    /// Populated by a `git status --porcelain` shell-out on the worker thread.
    /// Key: filename (basename). Value: `'M'` modified, `'A'` added, `'?'` untracked, `'D'` deleted.
    /// Empty when not in a git repo or the `git` binary is unavailable.
    pub git_marks: HashMap<String, char>,
}

/// Directories to skip entirely.
const SKIP_DIRS: &[&str] = &["target", "node_modules", ".git"];
/// Maximum visible entries before an overflow sentinel is appended.
const ENTRY_CAP: usize = 200;

/// Spawn the fs worker. Returns `(tx, rx)`.
///
/// - `tx`: main thread sends `PathBuf` (cwd) requests.
/// - `rx`: main thread drains `DirSnapshot` results.
pub fn spawn_fs_worker() -> (mpsc::SyncSender<PathBuf>, mpsc::Receiver<DirSnapshot>) {
    let (req_tx, req_rx) = mpsc::sync_channel::<PathBuf>(8);
    let (snap_tx, snap_rx) = mpsc::sync_channel::<DirSnapshot>(8);

    std::thread::Builder::new()
        .name("anvil-fs-worker".to_string())
        .spawn(move || {
            let mut last_sent: Option<(PathBuf, Instant)> = None;

            loop {
                // Block until we have at least one request.
                let first = match req_rx.recv() {
                    Ok(p) => p,
                    Err(_) => return, // sender dropped — exit cleanly
                };

                // Drain remaining queued requests; keep only the latest.
                let mut latest = first;
                while let Ok(p) = req_rx.try_recv() {
                    latest = p;
                }

                // Debounce: skip if we already served this exact path within 2 s.
                let now = Instant::now();
                if let Some((ref prev, sent_at)) = last_sent {
                    if prev == &latest && now.duration_since(sent_at) < Duration::from_secs(2) {
                        continue;
                    }
                }

                let snap = read_dir_snapshot(&latest);
                last_sent = Some((latest, now));
                // Non-blocking: drop if main thread is not consuming.
                let _ = snap_tx.try_send(snap);
            }
        })
        .expect("failed to spawn anvil-fs-worker thread");

    (req_tx, snap_rx)
}

/// Spawn a worker that loads child directories on demand.
///
/// - `tx`: main thread sends a `PathBuf` for each directory that should be
///   expanded for the first time.
/// - `rx`: main thread drains `(PathBuf, DirSnapshot)` pairs — the requested
///   dir plus its listing.  The key is the dir path so the caller can store the
///   snapshot in `child_snapshots`.
pub fn spawn_child_fs_worker() -> (
    mpsc::SyncSender<PathBuf>,
    mpsc::Receiver<(PathBuf, DirSnapshot)>,
) {
    let (req_tx, req_rx) = mpsc::sync_channel::<PathBuf>(32);
    let (snap_tx, snap_rx) = mpsc::sync_channel::<(PathBuf, DirSnapshot)>(32);

    std::thread::Builder::new()
        .name("anvil-child-fs-worker".to_string())
        .spawn(move || {
            loop {
                let path = match req_rx.recv() {
                    Ok(p) => p,
                    Err(_) => return,
                };
                // Drain extras; no debounce needed — each path is unique per expand.
                let snap = read_dir_snapshot(&path);
                let _ = snap_tx.try_send((path, snap));
            }
        })
        .expect("failed to spawn anvil-child-fs-worker thread");

    (req_tx, snap_rx)
}

/// Read the top-level entries of `root` and return a [`DirSnapshot`].
/// On IO error returns an empty snapshot (honest empty state).
pub fn read_dir_snapshot(root: &PathBuf) -> DirSnapshot {
    let entries = read_entries(root).unwrap_or_default();
    let git_marks = read_git_marks(root);
    DirSnapshot {
        root: root.clone(),
        entries,
        git_marks,
    }
}

/// Run `git status --porcelain` in `root` and return a map of basename → status char.
///
/// Runs synchronously on the worker thread. Fast for typical project sizes.
/// Returns an empty map when not in a git repo or `git` is unavailable.
fn read_git_marks(root: &Path) -> HashMap<String, char> {
    use std::process::Command;

    let output = match Command::new("git")
        .args(["-C", &root.to_string_lossy(), "status", "--porcelain"])
        .output()
    {
        Ok(o) if o.status.success() || !o.stdout.is_empty() => o,
        _ => return HashMap::new(),
    };

    let mut marks = HashMap::new();
    let text = match std::str::from_utf8(&output.stdout) {
        Ok(s) => s,
        Err(_) => return marks,
    };

    for line in text.lines() {
        // Porcelain format: "XY path" or "XY orig -> path" for renames.
        if line.len() < 4 {
            continue;
        }
        let xy = &line[..2];
        // For renames (R), take the destination path after " -> ".
        let path_part = if let Some(arrow) = line[3..].find(" -> ") {
            &line[3 + arrow + 4..]
        } else {
            line[3..].trim_start_matches('"').trim_end_matches('"')
        };
        // Only care about direct children of root (no subpath slash).
        let basename = if let Some(slash) = path_part.find('/') {
            &path_part[..slash]
        } else {
            path_part
        };
        if basename.is_empty() {
            continue;
        }
        // Determine mark from XY columns.
        // Index (X) takes precedence over work-tree (Y) unless it's untracked.
        let mark = if xy == "??" {
            '?'
        } else if xy.starts_with('D') || xy.ends_with('D') {
            'D'
        } else if xy.starts_with('A') {
            'A'
        } else {
            'M' // modified in any form (M, R, C, U, …)
        };
        marks.insert(basename.to_string(), mark);
    }
    marks
}

/// Read, filter, sort, and cap entries. Returns `Err` on IO failure.
fn read_entries(root: &PathBuf) -> Result<Vec<DirEntry>, std::io::Error> {
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(root)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden (dot-prefix) entries.
        if name_str.starts_with('.') {
            continue;
        }

        // Resolve symlinks one hop for is_dir classification.
        let meta = entry.metadata().or_else(|_| entry.path().metadata());
        let is_dir = meta.map(|m| m.is_dir()).unwrap_or(false);

        // Skip blacklisted directory names.
        if is_dir && SKIP_DIRS.contains(&name_str.as_ref()) {
            continue;
        }

        if is_dir {
            dirs.push(name_str.into_owned());
        } else {
            files.push(name_str.into_owned());
        }
    }

    // Sort each group case-insensitively.
    dirs.sort_by_key(|a: &String| a.to_lowercase());
    files.sort_by_key(|a: &String| a.to_lowercase());

    // Merge: dirs first, then files.
    let total = dirs.len() + files.len();
    let mut result: Vec<DirEntry> = Vec::with_capacity(total.min(ENTRY_CAP + 1));

    let dir_entries = dirs.into_iter().map(|n| DirEntry {
        name: n,
        is_dir: true,
    });
    let file_entries = files.into_iter().map(|n| DirEntry {
        name: n,
        is_dir: false,
    });

    for entry in dir_entries.chain(file_entries) {
        if result.len() < ENTRY_CAP {
            result.push(entry);
        } else {
            let overflow = total - ENTRY_CAP;
            result.push(DirEntry {
                name: format!("\u{2026}{overflow} more"),
                is_dir: false,
            });
            break;
        }
    }

    Ok(result)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create an isolated temp subdirectory unique to this test run.
    fn make_test_dir(suffix: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("anvil_fs_test_{suffix}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("create test dir");
        base
    }

    /// 3 files + 1 dir → dirs first, then files, both alphabetical.
    #[test]
    fn sort_dirs_before_files_alphabetical() {
        let root = make_test_dir("sort");

        fs::write(root.join("zebra.txt"), "").unwrap();
        fs::write(root.join("apple.txt"), "").unwrap();
        fs::write(root.join("mango.txt"), "").unwrap();
        fs::create_dir(root.join("src")).unwrap();

        let snap = read_dir_snapshot(&root);

        assert_eq!(snap.root, root);
        assert!(!snap.entries.is_empty());

        // First entry must be the dir.
        assert!(
            snap.entries[0].is_dir,
            "first entry should be dir, got: {:?}",
            snap.entries[0]
        );
        assert_eq!(snap.entries[0].name, "src");

        // Remaining entries are files in alphabetical order.
        let file_names: Vec<&str> = snap.entries[1..].iter().map(|e| e.name.as_str()).collect();
        assert_eq!(file_names, &["apple.txt", "mango.txt", "zebra.txt"]);

        let _ = fs::remove_dir_all(&root);
    }

    /// Unreadable path → empty entries, no panic.
    #[test]
    fn unreadable_path_empty_no_panic() {
        let path = PathBuf::from("/nonexistent/path/that/does/not/exist");
        let snap = read_dir_snapshot(&path);
        assert_eq!(snap.root, path);
        assert!(
            snap.entries.is_empty(),
            "expected empty entries for bad path"
        );
    }

    /// Hidden files (dot-prefix) are excluded.
    #[test]
    fn hidden_files_excluded() {
        let root = make_test_dir("hidden");

        fs::write(root.join(".hidden"), "").unwrap();
        fs::write(root.join("visible.txt"), "").unwrap();

        let snap = read_dir_snapshot(&root);
        assert!(snap.entries.iter().all(|e| !e.name.starts_with('.')));
        assert_eq!(snap.entries.len(), 1);
        assert_eq!(snap.entries[0].name, "visible.txt");

        let _ = fs::remove_dir_all(&root);
    }

    /// `target`, `node_modules`, `.git` dirs are skipped.
    #[test]
    fn skip_dirs_excluded() {
        let root = make_test_dir("skip");

        for name in &["target", "node_modules", ".git"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        fs::write(root.join("Cargo.toml"), "").unwrap();

        let snap = read_dir_snapshot(&root);
        let names: Vec<&str> = snap.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(!names.contains(&"target"), "target should be excluded");
        assert!(
            !names.contains(&"node_modules"),
            "node_modules should be excluded"
        );
        assert!(!names.contains(&".git"), ".git should be excluded");
        assert!(names.contains(&"Cargo.toml"));

        let _ = fs::remove_dir_all(&root);
    }
}
