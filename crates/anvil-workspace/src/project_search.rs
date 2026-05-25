//! Project-wide text search — NE12.
//!
//! Uses the ripgrep building blocks (`ignore`, `grep-regex`, `grep-searcher`)
//! to scan a directory tree and collect line matches. No shell-out; pure Rust.
//!
//! # Caps
//! - Max hits returned: `MAX_HITS` (1000).
//! - Max file size scanned: `MAX_FILE_BYTES` (1 MiB).
//! - Max walk depth: `MAX_DEPTH` (8).

use std::path::{Path, PathBuf};

use grep_regex::RegexMatcher;
use grep_searcher::Searcher;
use grep_searcher::sinks::UTF8;
use ignore::WalkBuilder;

/// Maximum number of hits returned from a single scan.
pub const MAX_HITS: usize = 1000;
/// Maximum file size (in bytes) that will be scanned. Larger files are skipped.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024; // 1 MiB
/// Maximum walk depth below the root.
pub const MAX_DEPTH: usize = 8;

/// A single line match found by [`ProjectSearch::scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSearchHit {
    /// Path to the file containing the match.
    pub path: PathBuf,
    /// 1-indexed line number of the match.
    pub line: usize,
    /// Byte column of the first matching byte on the line.
    pub col: usize,
    /// Trimmed preview of the matching line text.
    pub preview: String,
}

/// State for a project-wide search session.
///
/// Callers set the query and root, then call [`scan`](ProjectSearch::scan).
/// Results accumulate in [`hits`](ProjectSearch::hits).
pub struct ProjectSearch {
    /// The query string used for the last scan.
    pub query: String,
    /// The directory root used for the last scan.
    pub root: PathBuf,
    /// Collected hits, capped at [`MAX_HITS`].
    pub hits: Vec<ProjectSearchHit>,
    /// True while a scan is running. Always false after [`scan`] returns,
    /// because v1 is synchronous. Reserved for the future async upgrade.
    pub running: bool,
    /// Index of the currently selected hit (for keyboard navigation).
    pub selected: usize,
    /// True when the overlay is open.
    pub visible: bool,
}

impl ProjectSearch {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            root: PathBuf::new(),
            hits: Vec::new(),
            running: false,
            selected: 0,
            visible: false,
        }
    }

    /// Synchronous scan.
    ///
    /// Walks `root` (respecting `.gitignore`, `.ignore`, `.git/info/exclude`)
    /// and collects lines matching `query` (interpreted as a regex) up to
    /// [`MAX_HITS`]. Skips files larger than [`MAX_FILE_BYTES`]. Walk depth is
    /// capped at [`MAX_DEPTH`].
    ///
    /// Results replace any previous `hits`. `query` and `root` fields are
    /// updated to the supplied values. If `query` is empty or fails to compile
    /// as a regex, hits are cleared and the function returns early.
    pub fn scan(&mut self, query: &str, root: &Path) {
        self.query = query.to_string();
        self.root = root.to_path_buf();
        self.hits.clear();
        self.selected = 0;

        if query.is_empty() {
            return;
        }

        let matcher = match RegexMatcher::new(query) {
            Ok(m) => m,
            Err(_) => return,
        };

        let walker = WalkBuilder::new(root)
            .max_depth(Some(MAX_DEPTH))
            .standard_filters(true) // respects .gitignore, .ignore, skip hidden
            .require_git(false) // honor .gitignore even outside a git repo
            .build();

        'walk: for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            // Only scan regular files.
            let file_type = entry.file_type();
            if file_type.is_none_or(|ft| !ft.is_file()) {
                continue;
            }
            // Size guard: skip files larger than MAX_FILE_BYTES.
            if let Ok(meta) = entry.metadata() {
                if meta.len() > MAX_FILE_BYTES {
                    continue;
                }
            }

            let path = entry.path().to_path_buf();

            let _ = Searcher::new().search_path(
                &matcher,
                &path,
                UTF8(|line_num, line| {
                    if self.hits.len() >= MAX_HITS {
                        // Signal the searcher to stop by returning false.
                        return Ok(false);
                    }
                    // Find byte column of first match on the line.
                    let col = grep_matcher::Matcher::find(&matcher, line.as_bytes())
                        .ok()
                        .flatten()
                        .map(|m| m.start())
                        .unwrap_or(0);
                    self.hits.push(ProjectSearchHit {
                        path: path.clone(),
                        line: line_num as usize,
                        col,
                        preview: line.trim().to_string(),
                    });
                    Ok(true)
                }),
            );

            if self.hits.len() >= MAX_HITS {
                break 'walk;
            }
        }
    }

    /// Move selection down one row (clamped at last hit).
    pub fn select_next(&mut self) {
        if !self.hits.is_empty() && self.selected + 1 < self.hits.len() {
            self.selected += 1;
        }
    }

    /// Move selection up one row (clamped at 0).
    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Return the currently selected hit, or `None` when there are no hits.
    pub fn current_hit(&self) -> Option<&ProjectSearchHit> {
        self.hits.get(self.selected)
    }

    /// Open the overlay. Callers should immediately call [`scan`] with the
    /// current query and working directory.
    pub fn open(&mut self) {
        self.visible = true;
        self.selected = 0;
    }

    /// Close and reset the overlay.
    pub fn close(&mut self) {
        self.visible = false;
        self.hits.clear();
        self.query.clear();
        self.selected = 0;
    }
}

impl Default for ProjectSearch {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn write_file(dir: &Path, name: &str, content: &str) {
        let p = dir.join(name);
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn project_search_finds_literal_in_root() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "a.txt", "hello world\nfoo bar\n");
        write_file(dir.path(), "b.txt", "no match here\n");

        let mut ps = ProjectSearch::new();
        ps.scan("hello", dir.path());

        assert_eq!(ps.hits.len(), 1, "expected exactly 1 hit");
        assert_eq!(ps.hits[0].line, 1, "hit should be on line 1");
        assert!(
            ps.hits[0].preview.contains("hello"),
            "preview should contain query"
        );
    }

    #[test]
    fn project_search_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        // Write a .gitignore that excludes "ignored.txt".
        write_file(dir.path(), ".gitignore", "ignored.txt\n");
        write_file(dir.path(), "ignored.txt", "needle\n");
        write_file(dir.path(), "visible.txt", "no match\n");

        let mut ps = ProjectSearch::new();
        ps.scan("needle", dir.path());

        assert_eq!(
            ps.hits.len(),
            0,
            "ignored.txt should be excluded by .gitignore"
        );
    }

    #[test]
    fn project_search_caps_hits_at_1000() {
        let dir = tempfile::tempdir().unwrap();
        // Write a single file with 1500 lines each containing the query.
        let content: String = (0..1500).map(|_| "match_me\n").collect();
        write_file(dir.path(), "big.txt", &content);

        let mut ps = ProjectSearch::new();
        ps.scan("match_me", dir.path());

        assert_eq!(
            ps.hits.len(),
            MAX_HITS,
            "hits must be capped at MAX_HITS (1000)"
        );
    }
}
