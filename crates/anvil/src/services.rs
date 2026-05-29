use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anvil_render::agent_panel::{GitState, SectionId};

// ── Git worker ───────────────────────────────────────────────────────────────

pub(crate) struct GitResult {
    pub(crate) state: GitState,
    pub(crate) branch: String,
    pub(crate) dirty: u32,
    pub(crate) ahead: u32,
    pub(crate) behind: u32,
    pub(crate) head_short: String,
    pub(crate) head_subject: String,
    /// Locally-listening TCP ports detected at the time of the git query.
    pub(crate) ports: Vec<u16>,
    /// Detected project kind: "rust", "node", or "make". None if unrecognised.
    pub(crate) project_kind: Option<String>,
}

/// Result sent from the recent-files worker to the main thread.
pub(crate) struct RecentResult {
    pub(crate) files: Vec<String>,
}

// ── Gutter worker (T1) ──────────────────────────────────────────────────────

/// Request sent to the git-gutter worker.
pub(crate) struct GutterRequest {
    pub(crate) buffer_id: anvil_editor::BufferId,
    pub(crate) path: std::path::PathBuf,
    pub(crate) text: String,
}

/// Result returned by the git-gutter worker.
pub(crate) struct GutterResult {
    pub(crate) buffer_id: anvil_editor::BufferId,
    pub(crate) gutter: anvil_editor::GitGutter,
}

// ── Blame worker (T2) ───────────────────────────────────────────────────────

/// Request sent to the git-blame worker.
pub(crate) struct BlameRequest {
    pub(crate) path: std::path::PathBuf,
    /// 1-indexed line number (git blame -L uses 1-based).
    pub(crate) line: usize,
}

/// Parsed blame entry returned by the blame worker.
pub(crate) struct BlameEntry {
    pub(crate) author: String,
    /// Relative human-readable time, e.g. "3 hours ago".
    pub(crate) time_relative: String,
    /// Short commit hash (first 7 chars).
    pub(crate) short_hash: String,
}

/// Result returned by the git-blame worker.
pub(crate) struct BlameResult {
    pub(crate) path: std::path::PathBuf,
    pub(crate) line: usize,
    /// `None` means "Not Committed Yet".
    pub(crate) entry: Option<BlameEntry>,
}

/// Sent from the file-watcher thread to the main thread when a tracked buffer's
/// on-disk file changes (item 27).
pub(crate) struct FileWatchEvent {
    /// The buffer whose backing file changed.
    pub(crate) buffer_id: anvil_editor::BufferId,
}

/// Detect locally-listening TCP ports via `lsof`.
///
/// Cached for 2 s to avoid hammering lsof on every HUD tick. Skips ports
/// below 1024 (system) and the well-known noise ports 5353 (mDNS) and 7000
/// (AirPlay).
pub(crate) fn detect_ports() -> Vec<u16> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static PORT_CACHE: Mutex<Option<(Instant, Vec<u16>)>> = Mutex::new(None);
    const PORT_CACHE_TTL: Duration = Duration::from_secs(2);

    if let Ok(guard) = PORT_CACHE.lock() {
        if let Some((ts, ref ports)) = *guard {
            if ts.elapsed() < PORT_CACHE_TTL {
                return ports.clone();
            }
        }
    }

    let ports = detect_ports_uncached();
    if let Ok(mut guard) = PORT_CACHE.lock() {
        *guard = Some((Instant::now(), ports.clone()));
    }
    ports
}

pub(crate) fn detect_ports_uncached() -> Vec<u16> {
    const SKIP: &[u16] = &[5353, 7000];
    let Ok(out) = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut ports: Vec<u16> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines().skip(1) {
        // Each line has fields separated by whitespace. The NAME column
        // (last field) looks like "*:3000" or "127.0.0.1:8080".
        let Some(name) = line.split_whitespace().last() else {
            continue;
        };
        let Some(port_str) = name.rsplit(':').next() else {
            continue;
        };
        let Ok(port) = port_str.parse::<u16>() else {
            continue;
        };
        if port < 1024 || SKIP.contains(&port) {
            continue;
        }
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    ports.sort_unstable();
    ports
}

/// Detect project kind by checking for well-known marker files in `cwd`.
/// Returns "rust", "node", or "make" for the first match, or None.
pub(crate) fn detect_project_kind(cwd: &std::path::Path) -> Option<String> {
    if cwd.join("Cargo.toml").exists() {
        return Some("rust".to_string());
    }
    if cwd.join("package.json").exists() {
        return Some("node".to_string());
    }
    if cwd.join("pyproject.toml").exists() {
        return Some("python".to_string());
    }
    if cwd.join("go.mod").exists() {
        return Some("go".to_string());
    }
    if cwd.join("Makefile").exists() {
        return Some("make".to_string());
    }
    if cwd.join(".git").is_dir() {
        return Some("git".to_string());
    }
    None
}

pub(crate) fn has_project_marker_in_or_above(cwd: &Path) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    for dir in cwd.ancestors() {
        if detect_project_kind(dir).is_some() {
            return true;
        }
        if home.as_deref() == Some(dir) {
            break;
        }
    }
    false
}

/// Walk `cwd` up to depth `max_depth`, collecting (mtime, path) for regular
/// files (skipping hidden dirs and known noise dirs like `target/`,
/// `node_modules/`, `.git/`). Returns the top-`n` most recently modified
/// files as absolute path strings.
pub(crate) fn recent_files_in_dir(cwd: &std::path::Path, n: usize) -> Vec<String> {
    use std::time::SystemTime;
    const SKIP_DIRS: &[&str] = &["target", "node_modules", ".git"];

    let mut entries: Vec<(SystemTime, String)> = Vec::new();
    walk_dir_for_recent(cwd, 0, 3, &mut entries, SKIP_DIRS);
    entries.sort_by_key(|e| std::cmp::Reverse(e.0));
    entries.into_iter().take(n).map(|(_, p)| p).collect()
}

pub(crate) fn walk_dir_for_recent(
    dir: &std::path::Path,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<(std::time::SystemTime, String)>,
    skip_dirs: &[&str],
) {
    use std::fs;
    if depth > max_depth {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || skip_dirs.contains(&name_str.as_ref()) {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if ft.is_dir() {
            if depth < max_depth {
                walk_dir_for_recent(&path, depth + 1, max_depth, out, skip_dirs);
            }
        } else if ft.is_file() {
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    if let Some(s) = path.to_str() {
                        out.push((mtime, s.to_string()));
                    }
                }
            }
        }
    }
}

/// Path used to persist the user's HUD section order.
///
/// Lives under `~/.config/anvil/` (XDG-ish) so it survives across launches
/// without touching the main TOML config (the config crate doesn't have a
/// writer yet). One section token per line, in display order.
pub(crate) fn hud_section_order_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = std::path::PathBuf::from(home);
    p.push(".config");
    p.push("anvil");
    Some(p.join("section_order.txt"))
}

pub(crate) fn load_hud_section_order() -> Option<Vec<SectionId>> {
    let path = hud_section_order_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let order: Vec<SectionId> = text
        .lines()
        .filter_map(|line| SectionId::from_token(line.trim()))
        .collect();
    if order.is_empty() {
        return None;
    }
    Some(order)
}

/// X11: load user snippets from `~/.config/anvil/snippets.toml`.
///
/// Expected format:
/// ```toml
/// [snippet.fn]
/// body = "fn ${1:name}() {\n    $0\n}"
///
/// [snippet.impl]
/// body = "impl ${1:Type} {\n    $0\n}"
/// ```
///
/// Returns an empty map when the file does not exist or cannot be parsed.
pub(crate) fn load_snippets() -> HashMap<String, String> {
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return HashMap::new(),
    };
    let path = home.join(".config").join("anvil").join("snippets.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return HashMap::new(),
    };
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(_) => return HashMap::new(),
    };
    let mut snippets = HashMap::new();
    if let Some(toml::Value::Table(snippet_table)) = table.get("snippet") {
        for (trigger, val) in snippet_table {
            if let toml::Value::Table(entry) = val {
                if let Some(toml::Value::String(body)) = entry.get("body") {
                    snippets.insert(trigger.clone(), body.clone());
                }
            }
        }
    }
    snippets
}

pub(crate) fn save_hud_section_order(order: &[SectionId]) {
    let Some(path) = hud_section_order_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body: String = order
        .iter()
        .map(|s| format!("{}\n", s.token()))
        .collect::<String>();
    let _ = std::fs::write(&path, body);
}

/// One-shot `git log -1 --format=%h %s` against `cwd`. Returns (sha, subject)
/// Return the local wall-clock time as `"HH:MM"` using libc `localtime_r`.
pub(crate) fn local_hhmm() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    let mut tm = libc::tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null_mut(),
    };
    // SAFETY: secs is a valid time_t; tm is stack-allocated and we own it.
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// or empty strings on failure / non-repo. Bounded: takes <10ms in practice.
pub(crate) fn git_head_oneline(cwd: &std::path::Path) -> (String, String) {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--format=%h %s"])
        .current_dir(cwd)
        .output();
    let Ok(out) = output else {
        return (String::new(), String::new());
    };
    if !out.status.success() {
        return (String::new(), String::new());
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next().unwrap_or("").trim_end();
    match line.split_once(' ') {
        Some((sha, subject)) => (sha.to_string(), subject.to_string()),
        None => (line.to_string(), String::new()),
    }
}

// ── Z1/Z2/Z3/Z5/Z6/Z8/Z12: git / gh shell-outs ──────────────────────────────

/// Run `git status --porcelain` and return staged/unstaged file lists.
pub(crate) fn git_status_files(cwd: &std::path::Path) -> (Vec<ScmFile>, Vec<ScmFile>) {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output();
    let Ok(out) = output else {
        return (Vec::new(), Vec::new());
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    for line in text.lines() {
        if line.len() < 4 {
            continue;
        }
        let x = line.chars().next().unwrap_or(' ');
        let y = line.chars().nth(1).unwrap_or(' ');
        let path_part = line[3..].trim_matches('"');
        // Rename: take destination.
        let path_str = if let Some(arrow) = path_part.find(" -> ") {
            &path_part[arrow + 4..]
        } else {
            path_part
        };
        let abs = cwd.join(path_str);
        // Index (X col) — staged.
        if x != ' ' && x != '?' {
            let mark = match x {
                'A' => 'A',
                'D' => 'D',
                _ => 'M',
            };
            staged.push(ScmFile {
                path: abs.clone(),
                mark,
                staged: true,
            });
        }
        // Work-tree (Y col) — unstaged or untracked.
        if y != ' ' || x == '?' {
            let mark = if x == '?' && y == '?' {
                '?'
            } else {
                match y {
                    'D' => 'D',
                    _ => 'M',
                }
            };
            unstaged.push(ScmFile {
                path: abs,
                mark,
                staged: false,
            });
        }
    }
    (staged, unstaged)
}

/// Run `git branch` and return (branch names, index of current branch).
pub(crate) fn git_branch_list(cwd: &std::path::Path) -> (Vec<String>, Option<usize>) {
    let output = std::process::Command::new("git")
        .args(["branch"])
        .current_dir(cwd)
        .output();
    let Ok(out) = output else {
        return (Vec::new(), None);
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut branches = Vec::new();
    let mut current = None;
    for (i, line) in text.lines().enumerate() {
        if let Some(stripped) = line.strip_prefix("* ") {
            current = Some(i);
            branches.push(stripped.trim().to_string());
        } else {
            branches.push(line.trim().to_string());
        }
    }
    (branches, current)
}

/// Run `git log --oneline -50` and return entries.
pub(crate) fn git_log_entries(cwd: &std::path::Path) -> Vec<GitLogEntry> {
    let output = std::process::Command::new("git")
        .args(["log", "--oneline", "-50"])
        .current_dir(cwd)
        .output();
    let Ok(out) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter_map(|line| {
            let (hash, rest) = line.split_once(' ')?;
            Some(GitLogEntry {
                hash: hash.to_string(),
                subject: rest.to_string(),
            })
        })
        .collect()
}

/// Run `git stash list` and return stash entries.
pub(crate) fn git_stash_list(cwd: &std::path::Path) -> Vec<StashEntry> {
    let output = std::process::Command::new("git")
        .args(["stash", "list"])
        .current_dir(cwd)
        .output();
    let Ok(out) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .enumerate()
        .map(|(i, line)| StashEntry {
            idx: i,
            message: line.to_string(),
        })
        .collect()
}

/// Run `gh pr list --json number,title` and return entries.
/// Returns empty vec when gh is not on PATH or the command fails.
pub(crate) fn gh_pr_list(cwd: &std::path::Path) -> Vec<PrEntry> {
    let output = std::process::Command::new("gh")
        .args(["pr", "list", "--json", "number,title"])
        .current_dir(cwd)
        .output();
    let Ok(out) = output else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // Minimal JSON parse — look for {"number":N,"title":"..."} objects.
    // Use a simple regex-free approach: split on },{.
    parse_gh_pr_json(&text)
}

pub(crate) fn parse_gh_pr_json(text: &str) -> Vec<PrEntry> {
    // Expected format: [{"number":1,"title":"foo"},...].
    // Extract each {...} object naively.
    let mut entries = Vec::new();
    let mut depth = 0i32;
    let mut obj_start = None;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' => {
                depth += 1;
                if depth == 1 {
                    obj_start = Some(i);
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = obj_start.take() {
                        let obj: String = chars[start..=i].iter().collect();
                        if let Some(e) = parse_gh_pr_obj(&obj) {
                            entries.push(e);
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    entries
}

pub(crate) fn parse_gh_pr_obj(obj: &str) -> Option<PrEntry> {
    let number = extract_json_number(obj, "number")?;
    let title = extract_json_string(obj, "title").unwrap_or_default();
    Some(PrEntry { number, title })
}

pub(crate) fn extract_json_number(obj: &str, key: &str) -> Option<u32> {
    let needle = format!("\"{}\":", key);
    let start = obj.find(&needle)? + needle.len();
    let rest = obj[start..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

pub(crate) fn extract_json_string(obj: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\":\"", key);
    let start = obj.find(&needle)? + needle.len();
    let rest = &obj[start..];
    // Read until unescaped closing quote.
    let mut result = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            result.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            break;
        } else {
            result.push(ch);
        }
    }
    Some(result)
}

/// Run `git add <path>` to stage a file. Returns Ok(()) on success.
pub(crate) fn git_add(cwd: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .args(["add", "--", &path.to_string_lossy()])
        .current_dir(cwd)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Run `git reset HEAD <path>` to unstage a file.
pub(crate) fn git_reset_head(cwd: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .args(["reset", "HEAD", "--", &path.to_string_lossy()])
        .current_dir(cwd)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Run `git commit -m <msg>`.
pub(crate) fn git_commit(cwd: &std::path::Path, msg: &str) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(cwd)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim_end().to_owned())
    }
}

/// Run `git checkout <branch>`.
pub(crate) fn git_checkout(cwd: &std::path::Path, branch: &str) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .args(["checkout", branch])
        .current_dir(cwd)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim_end().to_owned())
    }
}

/// Run `git diff HEAD -- <path>` (unstaged) or `git diff --cached -- <path>`
/// (staged) and return the diff text.  Returns an empty string on error.
pub(crate) fn git_diff_for_file(cwd: &std::path::Path, path: &str, staged: bool) -> String {
    let mut cmd = std::process::Command::new("git");
    if staged {
        cmd.args(["diff", "--cached", "--", path]);
    } else {
        cmd.args(["diff", "HEAD", "--", path]);
    }
    cmd.current_dir(cwd)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
}

/// Run `git stash apply stash@{N}`.
pub(crate) fn git_stash_apply(cwd: &std::path::Path, idx: usize) -> Result<(), String> {
    let stash_ref = format!("stash@{{{idx}}}");
    let out = std::process::Command::new("git")
        .args(["stash", "apply", &stash_ref])
        .current_dir(cwd)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim_end().to_owned())
    }
}

/// Run `git push` in a background thread; result sent back via channel.
pub(crate) fn spawn_git_push(
    cwd: std::path::PathBuf,
    tx: std::sync::mpsc::Sender<Result<(), String>>,
) {
    std::thread::spawn(move || {
        let out = std::process::Command::new("git")
            .arg("push")
            .current_dir(&cwd)
            .output();
        let result = match out {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim_end().to_owned()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(result);
    });
}

/// Run `git pull` in a background thread; result sent back via channel.
pub(crate) fn spawn_git_pull(
    cwd: std::path::PathBuf,
    tx: std::sync::mpsc::Sender<Result<(), String>>,
) {
    std::thread::spawn(move || {
        let out = std::process::Command::new("git")
            .arg("pull")
            .current_dir(&cwd)
            .output();
        let result = match out {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim_end().to_owned()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(result);
    });
}

/// Run `git show <hash>` and return the output as a String.
pub(crate) fn git_show(cwd: &std::path::Path, hash: &str) -> String {
    let out = std::process::Command::new("git")
        .args(["show", hash])
        .current_dir(cwd)
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => format!("git show {hash}: failed"),
    }
}

// ── git blame shell-out (T2) ─────────────────────────────────────────────────

/// Run `git blame -L<line>,<line> --porcelain <path>` and parse the first entry.
///
/// `line` is 0-indexed (converted to 1-indexed for git).
/// Returns `None` when git is unavailable, the file is untracked, or the line
/// has not yet been committed ("0000…" hash).
pub(crate) fn run_git_blame(path: &std::path::Path, line: usize) -> Option<BlameEntry> {
    let git_line = line + 1; // git blame uses 1-based lines
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let output = std::process::Command::new("git")
        .args([
            "blame",
            &format!("-L{git_line},{git_line}"),
            "--porcelain",
            &path.to_string_lossy(),
        ])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_blame_porcelain(&text)
}

/// Parse a single entry from `git blame --porcelain` output.
///
/// The porcelain format starts with: `<40-char-hash> <orig-line> <final-line> <count>`
/// Followed by key/value lines until the actual source line (prefixed with `\t`).
///
/// Returns `None` when the commit is "not yet committed" (all-zero hash).
pub(crate) fn parse_blame_porcelain(text: &str) -> Option<BlameEntry> {
    let mut lines = text.lines();
    let header = lines.next()?;
    let hash = header.split_whitespace().next()?;
    // All-zero hash means uncommitted.
    if hash.chars().all(|c| c == '0') {
        return None;
    }
    let short_hash = &hash[..hash.len().min(7)];

    let mut author = String::new();
    let mut author_time: u64 = 0;

    for line in lines {
        if line.starts_with('\t') {
            break; // source line — done
        }
        if let Some(val) = line.strip_prefix("author ") {
            author = val.to_string();
        } else if let Some(val) = line.strip_prefix("author-time ") {
            author_time = val.parse().unwrap_or(0);
        }
    }

    let time_relative = blame_relative_time(author_time);

    Some(BlameEntry {
        author,
        time_relative,
        short_hash: short_hash.to_string(),
    })
}

/// Format a UNIX timestamp as a human-readable relative time string.
pub(crate) fn blame_relative_time(unix_secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs = now.saturating_sub(unix_secs);
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        let m = secs / 60;
        format!("{m} min ago")
    } else if secs < 86400 {
        let h = secs / 3600;
        format!("{h} hr ago")
    } else if secs < 86400 * 30 {
        let d = secs / 86400;
        format!("{d} days ago")
    } else if secs < 86400 * 365 {
        let mo = secs / (86400 * 30);
        format!("{mo} mo ago")
    } else {
        let y = secs / (86400 * 365);
        format!("{y} yr ago")
    }
}

pub(crate) fn blame_popup_text(entry: &Option<BlameEntry>) -> Option<String> {
    entry
        .as_ref()
        .map(|e| format!("{} · {} · {}", e.author, e.time_relative, e.short_hash))
}
// ── Z1: Source-control panel (Cmd+Shift+G) ────────────────────────────────────

/// A single file row in the source-control panel.
#[derive(Clone, Debug)]
pub(crate) struct ScmFile {
    /// Absolute path.
    pub(crate) path: PathBuf,
    /// 'M' modified, 'A' added, '?' untracked, 'D' deleted.
    pub(crate) mark: char,
    /// True = staged (index), false = unstaged / untracked.
    pub(crate) staged: bool,
}

/// A single stash entry (Z5).
#[derive(Clone, Debug)]
pub(crate) struct StashEntry {
    /// stash@{N} index used by git_stash_apply.
    pub(crate) idx: usize,
    /// Message, e.g. "WIP on main: ...".
    pub(crate) message: String,
}

/// PR list entry (Z12).
#[derive(Clone, Debug)]
pub(crate) struct PrEntry {
    pub(crate) number: u32,
    pub(crate) title: String,
}

/// State for the source-control panel overlay (Z1/Z2/Z3/Z5/Z7/Z12).
pub(crate) struct ScmPanel {
    /// Staged files (index-modified).
    pub(crate) staged: Vec<ScmFile>,
    /// Unstaged / untracked files.
    pub(crate) unstaged: Vec<ScmFile>,
    /// Currently selected row (0-based across both sections).
    pub(crate) selected: usize,
    /// Commit message text input (Z3).
    pub(crate) commit_msg: String,
    /// Whether the commit-message input has keyboard focus (Z3).
    pub(crate) commit_input_active: bool,
    /// Stash entries (Z5). Empty when none / not in a repo.
    pub(crate) stashes: Vec<StashEntry>,
    /// Whether the stash section is expanded.
    pub(crate) stashes_expanded: bool,
    /// PR entries from `gh` (Z12). Empty when gh not available.
    pub(crate) prs: Vec<PrEntry>,
    /// Whether the PR section is expanded.
    pub(crate) prs_expanded: bool,
}

impl ScmPanel {
    pub(crate) fn new() -> Self {
        Self {
            staged: Vec::new(),
            unstaged: Vec::new(),
            selected: 0,
            commit_msg: String::new(),
            commit_input_active: false,
            stashes: Vec::new(),
            stashes_expanded: false,
            prs: Vec::new(),
            prs_expanded: false,
        }
    }

    /// Total selectable file rows (staged + unstaged).
    pub(crate) fn total_rows(&self) -> usize {
        self.staged.len() + self.unstaged.len()
    }

    /// Returns the selected ScmFile (if any).
    pub(crate) fn selected_file(&self) -> Option<&ScmFile> {
        let n_staged = self.staged.len();
        if self.selected < n_staged {
            self.staged.get(self.selected)
        } else {
            self.unstaged.get(self.selected - n_staged)
        }
    }
}

// ── Z6: Branch switcher (Cmd+Shift+B) ────────────────────────────────────────

/// State for the branch-switcher palette (Z6).
pub(crate) struct BranchSwitcher {
    /// All local branches, output of `git branch`.
    pub(crate) branches: Vec<String>,
    /// Filter text.
    pub(crate) query: String,
    /// Currently selected row in the *filtered* list.
    pub(crate) selected: usize,
}

impl BranchSwitcher {
    pub(crate) fn filtered(&self) -> Vec<usize> {
        let q = self.query.to_ascii_lowercase();
        self.branches
            .iter()
            .enumerate()
            .filter(|(_, b)| q.is_empty() || b.to_ascii_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }
}

// ── Z8: Git-log palette (Cmd+K Cmd+G) ────────────────────────────────────────

/// One entry from `git log --oneline -50`.
#[derive(Clone, Debug)]
pub(crate) struct GitLogEntry {
    pub(crate) hash: String,
    pub(crate) subject: String,
}

/// State for the git-log palette (Z8).
pub(crate) struct GitLogPalette {
    pub(crate) entries: Vec<GitLogEntry>,
    pub(crate) selected: usize,
}
