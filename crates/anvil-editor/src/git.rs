//! Git gutter integration — NE13.
//!
//! `GitGutter` computes a per-line change marker (+/-/~) by comparing the
//! current buffer text against the HEAD blob for the file's path in the
//! nearest enclosing git repository.
//!
//! Errors (file not in a repo, no HEAD commit, gix I/O) are silently ignored:
//! the gutter returns an empty (all-`None`) result.

use std::path::Path;

use similar::{ChangeTag, TextDiff};

// ── Public types ─────────────────────────────────────────────────────────────

/// The kind of change on a single buffer line relative to HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitChange {
    /// Line is unchanged relative to HEAD.
    None,
    /// Line was added (not present in HEAD).
    Added,
    /// Line was modified (present in HEAD but different content).
    Modified,
    /// A deletion occurred before this line (lines were removed from HEAD).
    /// Painted as a triangle marker on the line that follows the removed block.
    Removed,
}

/// Per-line git change markers for a buffer.
///
/// `per_line[i]` is the change status for buffer line `i`.
/// The vec has exactly `buffer.line_count()` entries.
pub struct GitGutter {
    pub per_line: Vec<GitChange>,
}

impl GitGutter {
    /// Return an empty gutter with `line_count` entries all set to `None`.
    fn empty(line_count: usize) -> Self {
        GitGutter {
            per_line: vec![GitChange::None; line_count],
        }
    }

    /// Compute the git gutter by diffing `buffer_text` against the HEAD blob
    /// for `path`.
    ///
    /// Returns an all-`None` gutter when:
    /// - `path` is not inside a git repository.
    /// - The file has no HEAD blob (e.g. untracked or new file).
    /// - Any `gix` operation fails.
    pub fn compute(buffer_text: &str, path: &Path) -> Self {
        let buffer_lines: Vec<&str> = buffer_text.lines().collect();
        let line_count = buffer_lines.len().max(1);

        let head_text = match read_head_blob(path) {
            Some(t) => t,
            None => return Self::empty(line_count),
        };

        // Diff HEAD text → buffer text.  We want to annotate buffer lines, so
        // we iterate "new file" side of the diff.
        let diff = TextDiff::from_lines(head_text.as_str(), buffer_text);

        let mut per_line = vec![GitChange::None; line_count];
        // Track whether the previous change block contained deletions so we can
        // mark the following buffer line as `Removed`.
        let mut pending_removed = false;

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => {
                    // If deletions preceded this equal line, mark it `Removed`.
                    if pending_removed {
                        let new_idx = change.new_index();
                        if let Some(idx) = new_idx {
                            if idx < per_line.len() && per_line[idx] == GitChange::None {
                                per_line[idx] = GitChange::Removed;
                            }
                        }
                        pending_removed = false;
                    }
                }
                ChangeTag::Insert => {
                    if pending_removed {
                        // Insertion immediately after deletion → Modified.
                        if let Some(idx) = change.new_index() {
                            if idx < per_line.len() {
                                per_line[idx] = GitChange::Modified;
                            }
                        }
                        pending_removed = false;
                    } else {
                        // Pure insertion → Added.
                        if let Some(idx) = change.new_index() {
                            if idx < per_line.len() && per_line[idx] == GitChange::None {
                                per_line[idx] = GitChange::Added;
                            }
                        }
                    }
                }
                ChangeTag::Delete => {
                    pending_removed = true;
                }
            }
        }

        // If the diff ends with trailing deletions (lines removed at end of
        // file), we have nothing to attach them to — leave them unset.

        GitGutter { per_line }
    }
}

// ── Internal: read HEAD blob ──────────────────────────────────────────────────

/// Attempt to read the HEAD blob for `path` from the nearest enclosing git
/// repository.  Returns `None` on any error.
fn read_head_blob(path: &Path) -> Option<String> {
    // Discover the repository.  `discover` walks up from `path` until it finds
    // a `.git` directory (or bare repo).  Use the directory containing the
    // file as the starting point.
    let dir = path.parent().unwrap_or(Path::new("."));
    let repo = gix::discover(dir).ok()?;

    // Compute the relative path of `path` within the work-tree.
    let workdir = repo.workdir()?;
    let rel = path.strip_prefix(workdir).ok()?;

    // Resolve HEAD → commit → tree → blob.
    let head_commit = repo.head_commit().ok()?;
    let tree = head_commit.tree().ok()?;

    // Walk the tree components to find the entry for `rel`.
    let entry = tree.lookup_entry_by_path(rel).ok()??;
    let obj = entry.object().ok()?;
    // Only handle blobs; skip submodules (Commit) and directories (Tree).
    let blob = obj.try_into_blob().ok()?;
    let text = std::str::from_utf8(&blob.data).ok()?.to_owned();
    Some(text)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Helper: init a git repo in `dir`, add+commit `filename` with `content`.
    fn git_commit(dir: &std::path::Path, filename: &str, content: &str) {
        // Configure git identity so it doesn't fail in CI.
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::fs::write(dir.join(filename), content).unwrap();
        Command::new("git")
            .args(["add", filename])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init", "--no-gpg-sign"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    /// Outside a repo → all-None gutter, no panic.
    #[test]
    fn git_gutter_compute_returns_empty_outside_repo() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("foo.txt");
        std::fs::write(&file_path, "line1\nline2\n").unwrap();
        let gutter = GitGutter::compute("line1\nline2\n", &file_path);
        assert!(
            gutter.per_line.iter().all(|c| *c == GitChange::None),
            "outside a repo all entries must be None"
        );
    }

    /// Added line marked `Added`.
    #[test]
    fn git_gutter_added_line_marked_added() {
        let dir = tempfile::tempdir().unwrap();
        // HEAD: "line1\n"
        git_commit(dir.path(), "test.txt", "line1\n");
        let file_path = dir.path().join("test.txt");
        // Buffer: "line1\nnew_line\n"
        let buffer_text = "line1\nnew_line\n";
        let gutter = GitGutter::compute(buffer_text, &file_path);
        // line 0 ("line1") should be None; line 1 ("new_line") should be Added.
        assert_eq!(
            gutter.per_line[0],
            GitChange::None,
            "unchanged line must be None"
        );
        assert_eq!(
            gutter.per_line[1],
            GitChange::Added,
            "appended line must be Added"
        );
    }
}
