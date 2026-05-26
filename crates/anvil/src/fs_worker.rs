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

/// Flags controlling which entries are included in a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterFlags {
    /// When true, dot-prefix entries are included (except always-skipped dirs).
    pub show_hidden: bool,
    /// When true, entries matching `.gitignore` / `.git/info/exclude` are
    /// included. When false (the default), gitignored entries are hidden.
    pub show_gitignored: bool,
}

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

/// Spawn the fs worker. Returns `(tx, rx, filter_tx)`.
///
/// - `tx`: main thread sends `PathBuf` (cwd) requests.
/// - `rx`: main thread drains `DirSnapshot` results.
/// - `filter_tx`: main thread sends the current `FilterFlags`; the worker
///   uses the latest value it has seen when serving the next request.
pub fn spawn_fs_worker() -> (
    mpsc::SyncSender<PathBuf>,
    mpsc::Receiver<DirSnapshot>,
    mpsc::SyncSender<FilterFlags>,
) {
    let (req_tx, req_rx) = mpsc::sync_channel::<PathBuf>(8);
    let (snap_tx, snap_rx) = mpsc::sync_channel::<DirSnapshot>(8);
    let (filter_tx, filter_rx) = mpsc::sync_channel::<FilterFlags>(4);

    std::thread::Builder::new()
        .name("anvil-fs-worker".to_string())
        .spawn(move || {
            let mut last_sent: Option<(PathBuf, Instant)> = None;
            let mut flags = FilterFlags {
                show_hidden: false,
                show_gitignored: false,
            };

            loop {
                // Drain any pending filter flag updates.
                while let Ok(f) = filter_rx.try_recv() {
                    flags = f;
                }

                // Block until we have at least one request.
                let first = match req_rx.recv() {
                    Ok(p) => p,
                    Err(_) => return, // sender dropped — exit cleanly
                };

                // Drain any pending flag updates that arrived while blocked.
                while let Ok(f) = filter_rx.try_recv() {
                    flags = f;
                }

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

                let snap = read_dir_snapshot(&latest, flags);
                last_sent = Some((latest, now));
                // Non-blocking: drop if main thread is not consuming.
                let _ = snap_tx.try_send(snap);
            }
        })
        .expect("failed to spawn anvil-fs-worker thread");

    (req_tx, snap_rx, filter_tx)
}

/// Request type for the child-fs worker — `(directory_path, filter_flags)`.
pub type ChildFsRequest = (PathBuf, FilterFlags);

/// Response type for the child-fs worker — `(directory_path, snapshot)`.
pub type ChildFsResponse = (PathBuf, DirSnapshot);

/// Spawn a worker that loads child directories on demand.
///
/// - `tx`: main thread sends a `PathBuf` for each directory that should be
///   expanded for the first time.
/// - `rx`: main thread drains `(PathBuf, DirSnapshot)` pairs — the requested
///   dir plus its listing.  The key is the dir path so the caller can store the
///   snapshot in `child_snapshots`.
pub fn spawn_child_fs_worker() -> (
    mpsc::SyncSender<ChildFsRequest>,
    mpsc::Receiver<ChildFsResponse>,
) {
    let (req_tx, req_rx) = mpsc::sync_channel::<ChildFsRequest>(32);
    let (snap_tx, snap_rx) = mpsc::sync_channel::<ChildFsResponse>(32);

    std::thread::Builder::new()
        .name("anvil-child-fs-worker".to_string())
        .spawn(move || {
            loop {
                let (path, flags) = match req_rx.recv() {
                    Ok(p) => p,
                    Err(_) => return,
                };
                // Drain extras; no debounce needed — each path is unique per expand.
                let snap = read_dir_snapshot(&path, flags);
                let _ = snap_tx.try_send((path, snap));
            }
        })
        .expect("failed to spawn anvil-child-fs-worker thread");

    (req_tx, snap_rx)
}

/// Read the top-level entries of `root` and return a [`DirSnapshot`].
/// On IO error returns an empty snapshot (honest empty state).
///
/// Includes `git_marks`. May block for hundreds of ms on large repos because
/// `git status --porcelain` is synchronous. Always call from a worker thread.
pub fn read_dir_snapshot(root: &PathBuf, flags: FilterFlags) -> DirSnapshot {
    let entries = read_entries(root, flags).unwrap_or_default();
    let git_marks = read_git_marks(root);
    DirSnapshot {
        root: root.clone(),
        entries,
        git_marks,
    }
}

/// Like [`read_dir_snapshot`] but skips the `git status` shell-out. Fast and
/// safe to call on the main thread (e.g. for the synchronous startup seed).
/// The worker thread will repopulate `git_marks` on the next async refresh.
pub fn read_dir_snapshot_fast(root: &PathBuf, flags: FilterFlags) -> DirSnapshot {
    let entries = read_entries(root, flags).unwrap_or_default();
    DirSnapshot {
        root: root.clone(),
        entries,
        git_marks: HashMap::new(),
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

/// Build a gitignore matcher for `root` using `.gitignore` and
/// `.git/info/exclude`. Returns `None` when neither file exists or the root
/// is not a git repo.
fn build_gitignore(root: &Path) -> Option<ignore::gitignore::Gitignore> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    let gi_path = root.join(".gitignore");
    if gi_path.exists() {
        let _ = builder.add(gi_path);
    }
    let exclude_path = root.join(".git").join("info").join("exclude");
    if exclude_path.exists() {
        let _ = builder.add(exclude_path);
    }
    builder.build().ok()
}

/// Read, filter, sort, and cap entries. Returns `Err` on IO failure.
///
/// Filtering is controlled by `flags`:
/// - `show_hidden`: when false, dot-prefix entries are skipped.
/// - `show_gitignored`: when false, entries matching `.gitignore` /
///   `.git/info/exclude` are skipped.
///
/// `SKIP_DIRS` (`.git`, `target`, `node_modules`) are always excluded.
fn read_entries(root: &PathBuf, flags: FilterFlags) -> Result<Vec<DirEntry>, std::io::Error> {
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();

    // Build gitignore matcher once for the whole listing.
    let gitignore = if !flags.show_gitignored {
        build_gitignore(root)
    } else {
        None
    };

    for entry in std::fs::read_dir(root)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden (dot-prefix) entries unless show_hidden is set.
        if !flags.show_hidden && name_str.starts_with('.') {
            continue;
        }

        // Resolve symlinks one hop for is_dir classification.
        let meta = entry.metadata().or_else(|_| entry.path().metadata());
        let is_dir = meta.map(|m| m.is_dir()).unwrap_or(false);

        // Skip blacklisted directory names.
        if is_dir && SKIP_DIRS.contains(&name_str.as_ref()) {
            continue;
        }

        // Skip gitignored entries (S1).
        if let Some(ref gi) = gitignore {
            let path = entry.path();
            let matched = gi.matched(&path, is_dir);
            if matched.is_ignore() {
                continue;
            }
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

    fn default_flags() -> FilterFlags {
        FilterFlags {
            show_hidden: false,
            show_gitignored: false,
        }
    }

    fn show_hidden_flags() -> FilterFlags {
        FilterFlags {
            show_hidden: true,
            show_gitignored: false,
        }
    }

    /// 3 files + 1 dir → dirs first, then files, both alphabetical.
    #[test]
    fn sort_dirs_before_files_alphabetical() {
        let root = make_test_dir("sort");

        fs::write(root.join("zebra.txt"), "").unwrap();
        fs::write(root.join("apple.txt"), "").unwrap();
        fs::write(root.join("mango.txt"), "").unwrap();
        fs::create_dir(root.join("src")).unwrap();

        let snap = read_dir_snapshot(&root, default_flags());

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
        let snap = read_dir_snapshot(&path, default_flags());
        assert_eq!(snap.root, path);
        assert!(
            snap.entries.is_empty(),
            "expected empty entries for bad path"
        );
    }

    /// Hidden files (dot-prefix) are excluded when show_hidden is false.
    #[test]
    fn hidden_files_excluded() {
        let root = make_test_dir("hidden");

        fs::write(root.join(".hidden"), "").unwrap();
        fs::write(root.join("visible.txt"), "").unwrap();

        let snap = read_dir_snapshot(&root, default_flags());
        assert!(snap.entries.iter().all(|e| !e.name.starts_with('.')));
        assert_eq!(snap.entries.len(), 1);
        assert_eq!(snap.entries[0].name, "visible.txt");

        let _ = fs::remove_dir_all(&root);
    }

    /// When show_hidden is true, dot-prefix files appear (except SKIP_DIRS).
    #[test]
    fn hidden_files_included_when_show_hidden() {
        let root = make_test_dir("show_hidden");

        fs::write(root.join(".env"), "").unwrap();
        fs::write(root.join("visible.txt"), "").unwrap();
        // .git is always skipped even when show_hidden is true.
        fs::create_dir(root.join(".git")).unwrap();

        let snap = read_dir_snapshot_fast(&root, show_hidden_flags());
        let names: Vec<&str> = snap.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&".env"),
            ".env should be visible with show_hidden"
        );
        assert!(names.contains(&"visible.txt"));
        assert!(!names.contains(&".git"), ".git always skipped");

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

        let snap = read_dir_snapshot(&root, default_flags());
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

    /// S1: gitignored files are hidden when show_gitignored is false.
    #[test]
    fn gitignored_files_hidden_by_default() {
        let root = make_test_dir("gitignore_hide");

        // Write a .gitignore that ignores *.log
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(root.join("app.log"), "").unwrap();
        fs::write(root.join("main.rs"), "").unwrap();

        let snap = read_dir_snapshot_fast(&root, default_flags());
        let names: Vec<&str> = snap.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            !names.contains(&"app.log"),
            "app.log should be hidden (gitignored), got {names:?}"
        );
        assert!(
            names.contains(&"main.rs"),
            "main.rs should be visible, got {names:?}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// S1: gitignored files are shown when show_gitignored is true.
    #[test]
    fn gitignored_files_shown_when_flag_set() {
        let root = make_test_dir("gitignore_show");

        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(root.join("app.log"), "").unwrap();
        fs::write(root.join("main.rs"), "").unwrap();

        let flags = FilterFlags {
            show_hidden: false,
            show_gitignored: true,
        };
        let snap = read_dir_snapshot_fast(&root, flags);
        let names: Vec<&str> = snap.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"app.log"),
            "app.log should be visible with show_gitignored, got {names:?}"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
