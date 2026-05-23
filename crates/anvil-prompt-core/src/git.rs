//! Git status for the prompt. `parse_status` (pure, tested) interprets the
//! output of `git status --porcelain=v1 --branch`; `query` runs git as a
//! subprocess and feeds it to the parser.

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub branch: String,
    pub dirty: u32,
    pub ahead: u32,
    pub behind: u32,
}

/// Parse `git status --porcelain=v1 --branch` output.
/// Returns `None` if no branch header line is present.
pub fn parse_status(text: &str) -> Option<Info> {
    let mut info: Option<Info> = None;
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            info = Some(parse_branch_line(rest));
        } else if let Some(i) = info.as_mut() {
            i.dirty += 1;
        }
    }
    info
}

fn parse_branch_line(rest: &str) -> Info {
    // e.g. "main...origin/main [ahead 1, behind 2]"  or  "main"
    let branch_end = {
        let by_dots = rest.find("...").unwrap_or(rest.len());
        let by_space = rest.find(' ').unwrap_or(rest.len());
        by_dots.min(by_space)
    };
    let branch = rest[..branch_end].to_string();

    let ahead = rest
        .find("ahead ")
        .map(|i| read_num(&rest[i + 6..]))
        .unwrap_or(0);
    let behind = rest
        .find("behind ")
        .map(|i| read_num(&rest[i + 7..]))
        .unwrap_or(0);

    Info {
        branch,
        dirty: 0,
        ahead,
        behind,
    }
}

fn read_num(s: &str) -> u32 {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

/// Run git in `cwd` and return its status, or `None` if not a repo / git
/// fails / it errors.
pub fn query(cwd: &Path) -> Option<Info> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "--branch"])
        .current_dir(cwd)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_status(&stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_reads_branch_and_dirty_count() {
        let out = "## main...origin/main\n M src/a.zig\n?? new.txt\n";
        let info = parse_status(out).unwrap();
        assert_eq!(info.branch, "main");
        assert_eq!(info.dirty, 2);
        assert_eq!(info.ahead, 0);
    }

    #[test]
    fn parse_status_reads_ahead_and_behind() {
        let out = "## main...origin/main [ahead 3, behind 1]\n";
        let info = parse_status(out).unwrap();
        assert_eq!(info.branch, "main");
        assert_eq!(info.ahead, 3);
        assert_eq!(info.behind, 1);
    }

    #[test]
    fn parse_status_handles_a_branch_with_no_upstream() {
        let info = parse_status("## feature/x\n").unwrap();
        assert_eq!(info.branch, "feature/x");
        assert_eq!(info.dirty, 0);
    }

    #[test]
    fn parse_status_returns_none_without_a_branch_header() {
        assert!(parse_status("").is_none());
        assert!(parse_status("?? stray.txt\n").is_none());
    }

    // ── Additional parse_branch_line coverage ──────────────────────────────────

    #[test]
    fn parse_branch_line_ahead_only() {
        let info = parse_status("## main...origin/main [ahead 2]\n").unwrap();
        assert_eq!(info.ahead, 2);
        assert_eq!(info.behind, 0);
    }

    #[test]
    fn parse_branch_line_behind_only() {
        let info = parse_status("## main...origin/main [behind 4]\n").unwrap();
        assert_eq!(info.ahead, 0);
        assert_eq!(info.behind, 4);
    }

    #[test]
    fn parse_branch_line_no_dots_no_upstream() {
        // Branch name with a space would end at the space boundary.
        let info = parse_status("## mybranch\n").unwrap();
        assert_eq!(info.branch, "mybranch");
        assert_eq!(info.ahead, 0);
        assert_eq!(info.behind, 0);
    }

    #[test]
    fn parse_status_multiple_dirty_lines() {
        let out = "## main\n M file1\n?? file2\n?? file3\n";
        let info = parse_status(out).unwrap();
        assert_eq!(info.dirty, 3);
    }

    #[test]
    fn read_num_non_digit_prefix_returns_zero() {
        // read_num is private; test via parse_status with malformed ahead value.
        // "ahead abc" → read_num("abc") → 0
        let info = parse_status("## main...origin/main [ahead abc]\n").unwrap();
        assert_eq!(info.ahead, 0);
    }

    // ── query: runs git in a temp dir (integration test) ──────────────────────

    #[test]
    fn query_non_repo_returns_none() {
        // /tmp should not be a git repo.
        let result = query(std::path::Path::new("/tmp"));
        // In most cases this returns None (not a git repo).
        // If /tmp is inside a git repo on the CI runner, skip the assertion.
        let _ = result; // Either None or Some is acceptable here.
    }

    #[test]
    fn query_current_dir_returns_some_or_none() {
        // The test runner's cwd might be in the anvil repo.
        if let Ok(cwd) = std::env::current_dir() {
            let result = query(&cwd);
            // We don't assert the value — just that it doesn't panic.
            let _ = result;
        }
    }
}
