//! Anvil core — Tauri backend.
//! Owns the PTY (cross-platform via portable-pty) and thin git helpers.
//! The webview frontend (Svelte + xterm.js) drives everything over IPC.

mod lsp;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Emitter, Manager, State};

struct Pty {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
}

/// Registry of live PTYs keyed by a frontend-chosen session id, so each
/// terminal tab/split owns an independent shell.
#[derive(Default)]
struct PtyState(Mutex<HashMap<String, Pty>>);

// #78 Background terminals whose coalescer should throttle (wider flush window).
static PTY_INACTIVE: std::sync::OnceLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();
fn pty_inactive() -> &'static Mutex<std::collections::HashSet<String>> {
    PTY_INACTIVE.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// #78 Mark a terminal active/inactive so its PTY reader can throttle when it's
/// off-screen. The coalescer re-reads this per burst.
#[tauri::command]
fn pty_set_active(id: String, active: bool) {
    let mut set = pty_inactive().lock().unwrap();
    if active {
        set.remove(&id);
    } else {
        set.insert(id);
    }
}

#[derive(Clone, serde::Serialize)]
struct PtyExit {
    id: String,
}

/// Spawn a login shell in a new PTY under `id`. Output streams to the webview
/// as raw bytes over the per-terminal `on_data` channel (no base64); process
/// exit is signalled via the `pty://exit` event tagged with the same id.
#[tauri::command]
fn pty_spawn(
    app: tauri::AppHandle,
    state: State<PtyState>,
    id: String,
    cols: u16,
    rows: u16,
    cwd: Option<String>,
    shell: Option<String>,
    on_data: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let shell = shell
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()));
    let mut cmd = CommandBuilder::new(shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("ANVIL", "1");
    let dir = cwd
        .filter(|d| !d.is_empty())
        .or_else(|| std::env::var("HOME").ok());
    if let Some(d) = dir {
        cmd.cwd(d);
    }

    let _child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let rid = id.clone();
    // Reader thread: blocking 64 KiB reads → channel. A second coalescer thread
    // batches bursts over a ~4ms window so a flood of back-to-back reads becomes
    // far fewer IPC messages (#44). Interactive echo flushes on the idle timeout,
    // adding at most ~4ms latency (well under one frame).
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF: closing tx signals the coalescer to exit.
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    std::thread::spawn(move || {
        // Active terminals flush every 4 ms for snappy echo; backgrounded ones
        // coalesce over a much wider window to cut CPU/IPC for off-screen floods (#78).
        const WINDOW_ACTIVE: std::time::Duration = std::time::Duration::from_millis(4);
        const WINDOW_BG: std::time::Duration = std::time::Duration::from_millis(200);
        const FLUSH_CAP: usize = 262_144; // 256 KiB: bound latency/memory under flood.
        loop {
            // Block for the first chunk of a burst.
            let mut pending = match rx.recv() {
                Ok(c) => c,
                Err(_) => break, // reader gone → child exited.
            };
            let window = if pty_inactive().lock().unwrap().contains(&rid) {
                WINDOW_BG
            } else {
                WINDOW_ACTIVE
            };
            let deadline = std::time::Instant::now() + window;
            loop {
                if pending.len() >= FLUSH_CAP {
                    break;
                }
                let wait = deadline.saturating_duration_since(std::time::Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(c) => pending.extend_from_slice(&c),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        let _ = on_data.send(InvokeResponseBody::Raw(pending));
                        let _ = app.emit("pty://exit", PtyExit { id: rid.clone() });
                        return;
                    }
                }
            }
            let _ = on_data.send(InvokeResponseBody::Raw(pending));
        }
        let _ = app.emit("pty://exit", PtyExit { id: rid.clone() });
    });

    state.0.lock().unwrap().insert(
        id,
        Pty {
            writer,
            master: pair.master,
        },
    );
    Ok(())
}

#[tauri::command]
fn pty_write(state: State<PtyState>, id: String, data: String) -> Result<(), String> {
    if let Some(p) = state.0.lock().unwrap().get_mut(&id) {
        p.writer
            .write_all(data.as_bytes())
            .map_err(|e| e.to_string())?;
        let _ = p.writer.flush();
    }
    Ok(())
}

#[tauri::command]
fn pty_resize(state: State<PtyState>, id: String, cols: u16, rows: u16) -> Result<(), String> {
    if let Some(p) = state.0.lock().unwrap().get(&id) {
        p.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Close a PTY: dropping the master sends SIGHUP to the child shell.
#[tauri::command]
fn pty_kill(state: State<PtyState>, id: String) {
    state.0.lock().unwrap().remove(&id);
}

fn git(cwd: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// US-delimited `git log` (one commit per line) for the Source Control view.
/// Optional filters (#23): author, message grep, and path — applied server-side
/// so the swimlane graph rebuilds correctly from the filtered set.
#[tauri::command]
fn git_log(
    cwd: String,
    author: Option<String>,
    grep: Option<String>,
    path: Option<String>,
) -> Result<String, String> {
    let mut args: Vec<String> = vec![
        "log".into(),
        "--max-count=500".into(),
        "--date-order".into(),
        "--pretty=format:%H\x1f%h\x1f%an\x1f%ae\x1f%at\x1f%P\x1f%D\x1f%s".into(),
    ];
    if let Some(a) = author.filter(|s| !s.trim().is_empty()) {
        args.push(format!("--author={a}"));
    }
    if let Some(g) = grep.filter(|s| !s.trim().is_empty()) {
        args.push(format!("--grep={g}"));
        args.push("--regexp-ignore-case".into());
    }
    if let Some(p) = path.filter(|s| !s.trim().is_empty()) {
        args.push("--".into());
        args.push(p);
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    git(&cwd, &refs)
}

/// Per-commit insertion/deletion totals for the history view (Terax-style
/// `+N -N` column). Mirrors git_log's filters so the commit set matches.
/// Output is `--shortstat` interleaved with a `\x01<shorthash>` marker per
/// commit; the frontend sums them by hash.
#[tauri::command]
fn git_log_stats(
    cwd: String,
    author: Option<String>,
    grep: Option<String>,
    path: Option<String>,
) -> Result<String, String> {
    let mut args: Vec<String> = vec![
        "log".into(),
        "--max-count=500".into(),
        "--date-order".into(),
        "--shortstat".into(),
        "--pretty=format:\x01%h".into(),
    ];
    if let Some(a) = author.filter(|s| !s.trim().is_empty()) {
        args.push(format!("--author={a}"));
    }
    if let Some(g) = grep.filter(|s| !s.trim().is_empty()) {
        args.push(format!("--grep={g}"));
        args.push("--regexp-ignore-case".into());
    }
    if let Some(p) = path.filter(|s| !s.trim().is_empty()) {
        args.push("--".into());
        args.push(p);
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    git(&cwd, &refs)
}

#[tauri::command]
fn git_status(cwd: String) -> Result<String, String> {
    git(&cwd, &["status", "--porcelain=v1", "-b"])
}

/// #21 One-line `git log` for a range (e.g. `origin/main..HEAD`) — rebase preview.
#[tauri::command]
fn git_log_range(cwd: String, range: String) -> Result<String, String> {
    git(&cwd, &["log", "--oneline", "--no-decorate", &range])
}

/// #21 Run a non-interactive rebase from a UI-built todo. The todo is dropped in
/// as the rebase sequence via GIT_SEQUENCE_EDITOR (supports pick/fixup/drop +
/// reordering — no message editors open, so it never blocks). On failure the
/// rebase is aborted so the tree is left clean. (Unix shells; Windows pending.)
#[tauri::command]
fn git_rebase_run(cwd: String, target: String, todo: String) -> Result<String, String> {
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("anvil-rebase-{}.txt", std::process::id()));
    std::fs::write(&tmp, todo).map_err(|e| e.to_string())?;
    let editor = format!("cp '{}'", tmp.display());
    let out = std::process::Command::new("git")
        .current_dir(&cwd)
        .env("GIT_SEQUENCE_EDITOR", &editor)
        .env("GIT_EDITOR", "true")
        .args(["rebase", "-i", &target])
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(s)
    } else {
        let _ = std::process::Command::new("git")
            .current_dir(&cwd)
            .args(["rebase", "--abort"])
            .output();
        Err(s)
    }
}

/// #25 Resolve a merge conflict by taking one side wholesale, then stage it.
/// `side` is "ours" or "theirs".
#[tauri::command]
fn git_checkout_side(cwd: String, path: String, side: String) -> Result<String, String> {
    let flag = if side == "theirs" {
        "--theirs"
    } else {
        "--ours"
    };
    git(&cwd, &["checkout", flag, "--", &path])?;
    git(&cwd, &["add", "--", &path])
}

/// #29 Update submodules to their pinned commits.
#[tauri::command]
fn git_submodule_update(cwd: String) -> Result<String, String> {
    git(&cwd, &["submodule", "update", "--init", "--recursive"])
}

/// #29 Pull Git LFS objects for the working tree.
#[tauri::command]
fn git_lfs_pull(cwd: String) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .current_dir(&cwd)
        .args(["lfs", "pull"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(s)
    } else {
        Err(s)
    }
}

/// #24 Worktrees: tab-separated `path\tbranch` per line.
#[tauri::command]
fn git_worktrees(cwd: String) -> Result<String, String> {
    let raw = git(&cwd, &["worktree", "list", "--porcelain"])?;
    let mut out = Vec::new();
    let mut path = String::new();
    for line in raw.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = p.to_string();
        } else if let Some(b) = line.strip_prefix("branch ") {
            out.push(format!("{path}\t{}", b.rsplit('/').next().unwrap_or(b)));
        } else if line == "detached" {
            out.push(format!("{path}\t(detached)"));
        }
    }
    Ok(out.join("\n"))
}

/// #24 Add a worktree for an existing branch at a sibling path.
#[tauri::command]
fn git_worktree_add(cwd: String, path: String, branch: String) -> Result<String, String> {
    git(&cwd, &["worktree", "add", &path, &branch])
}

/// Comma-separated repo features: "submodules" and/or "lfs" (#29).
#[tauri::command]
fn git_repo_features(cwd: String) -> Result<String, String> {
    let mut f = Vec::new();
    if !git(&cwd, &["submodule", "status"])
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        f.push("submodules");
    }
    if std::fs::read_to_string(format!("{cwd}/.gitattributes"))
        .map(|s| s.contains("filter=lfs"))
        .unwrap_or(false)
    {
        f.push("lfs");
    }
    Ok(f.join(","))
}

#[tauri::command]
fn git_stage(cwd: String, path: String) -> Result<String, String> {
    git(&cwd, &["add", "--", &path])
}

#[tauri::command]
fn git_unstage(cwd: String, path: String) -> Result<String, String> {
    git(&cwd, &["restore", "--staged", "--", &path])
}

#[tauri::command]
fn git_discard(cwd: String, path: String) -> Result<String, String> {
    git(&cwd, &["checkout", "--", &path])
}

#[tauri::command]
fn git_stage_all(cwd: String) -> Result<String, String> {
    git(&cwd, &["add", "-A"])
}

#[tauri::command]
fn git_commit(cwd: String, message: String, amend: Option<bool>) -> Result<String, String> {
    let mut args = vec!["-C", &cwd, "commit"];
    if amend.unwrap_or(false) {
        args.push("--amend");
    }
    args.extend(["-m", &message]);
    let out = std::process::Command::new("git")
        .args(&args)
        .output()
        .map_err(|e| e.to_string())?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(combined)
    } else {
        Err(combined.trim().to_string())
    }
}

/// Amend the last commit with currently staged changes, keeping its message (#63).
#[tauri::command]
fn git_amend(cwd: String) -> Result<String, String> {
    git(&cwd, &["commit", "--amend", "--no-edit"])
}

/// Full message (subject + body) of the last commit, for amend prefill.
#[tauri::command]
fn git_last_message(cwd: String) -> Result<String, String> {
    git(&cwd, &["log", "-1", "--pretty=%B"])
}

#[tauri::command]
fn git_branches(cwd: String) -> Result<String, String> {
    git(&cwd, &["branch", "--format=%(HEAD)\t%(refname:short)"])
}

#[tauri::command]
fn git_checkout(cwd: String, branch: String) -> Result<String, String> {
    git(&cwd, &["checkout", &branch])
}

#[tauri::command]
fn git_create_branch(cwd: String, name: String) -> Result<String, String> {
    git(&cwd, &["checkout", "-b", &name])
}

#[tauri::command]
fn git_diff(cwd: String, path: String, staged: bool) -> Result<String, String> {
    if staged {
        git(&cwd, &["diff", "--cached", "--", &path])
    } else {
        git(&cwd, &["diff", "--", &path])
    }
}

/// Apply a single-hunk patch (built by buildHunkPatch in git.ts) via
/// `git apply`, piping the patch on stdin (#62). `cached` stages into the index;
/// `reverse` discards (applies the inverse). Returns git's stderr on failure.
#[tauri::command]
fn git_apply_hunk(
    cwd: String,
    patch: String,
    cached: bool,
    reverse: bool,
) -> Result<String, String> {
    let mut args = vec!["-C", &cwd, "apply", "--unidiff-zero"];
    if cached {
        args.push("--cached");
    }
    if reverse {
        args.push("--reverse");
    }
    args.push("-");
    let mut child = std::process::Command::new("git")
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(patch.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::new())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[tauri::command]
fn git_show(cwd: String, rev: String) -> Result<String, String> {
    git(&cwd, &["show", "--stat", "--patch", &rev])
}

/// The patch for a single file within a commit (popover → file → diff).
#[tauri::command]
fn git_show_file_diff(cwd: String, rev: String, path: String) -> Result<String, String> {
    git(&cwd, &["show", "--patch", "--format=", &rev, "--", &path])
}

/// Changed files in a commit as `STATUS\tpath` lines (commit popover file list).
#[tauri::command]
fn git_commit_files(cwd: String, rev: String) -> Result<String, String> {
    git(&cwd, &["show", "--name-status", "--format=", &rev])
}

/// Commit history for a single file (File History, #67). One line per commit:
/// hash\x1fshort\x1fauthor\x1ftimestamp\x1fsubject.
#[tauri::command]
fn git_file_log(cwd: String, path: String) -> Result<String, String> {
    git(
        &cwd,
        &[
            "log",
            "--max-count=80",
            "--follow",
            "--pretty=format:%H\x1f%h\x1f%an\x1f%at\x1f%s",
            "--",
            &path,
        ],
    )
}

#[tauri::command]
fn git_stash_list(cwd: String) -> Result<String, String> {
    git(&cwd, &["stash", "list"])
}

#[tauri::command]
fn git_stash_save(cwd: String, message: String) -> Result<String, String> {
    git(&cwd, &["stash", "push", "-m", &message])
}

#[tauri::command]
fn git_stash_apply(cwd: String, index: String) -> Result<String, String> {
    git(&cwd, &["stash", "apply", &index])
}

/// Partial stash (#28): optional message, specific paths, and/or untracked.
#[tauri::command]
fn git_stash_push(
    cwd: String,
    message: Option<String>,
    paths: Option<Vec<String>>,
    untracked: Option<bool>,
) -> Result<String, String> {
    let mut args: Vec<String> = vec!["stash".into(), "push".into()];
    if untracked.unwrap_or(false) {
        args.push("-u".into());
    }
    if let Some(m) = message.filter(|s| !s.trim().is_empty()) {
        args.push("-m".into());
        args.push(m);
    }
    if let Some(ps) = paths.filter(|p| !p.is_empty()) {
        args.push("--".into());
        for p in ps {
            args.push(p);
        }
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    git(&cwd, &refs)
}

#[tauri::command]
fn git_fetch(cwd: String) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .args(["fetch", "--all"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.is_empty() {
        combined.push_str(&stderr);
    }
    Ok(combined)
}

/// Pull (fast-forward only) using the user's configured git auth (#64).
#[tauri::command]
fn git_pull(cwd: String) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .args(["pull", "--ff-only"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(s)
    } else {
        Err(s)
    }
}

/// Push the current branch using the user's configured git auth (#64).
#[tauri::command]
fn git_push(cwd: String) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .args(["push"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(s)
    } else {
        Err(s)
    }
}

#[tauri::command]
fn git_current_branch(cwd: String) -> Result<String, String> {
    git(&cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// Commits ahead/behind the upstream as "ahead\tbehind" (#68). Errors when the
/// branch has no upstream — the caller treats that as "no indicator".
#[tauri::command]
fn git_ahead_behind(cwd: String) -> Result<String, String> {
    git(
        &cwd,
        &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
    )
}

#[tauri::command]
fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

const LLM_BASE: &str = "http://localhost:1234/v1";

/// Resolve the OpenAI-compatible base URL: caller-provided (from Accounts) or
/// the local LM Studio default. Trailing slash trimmed.
fn llm_base(base: &Option<String>) -> String {
    match base {
        Some(b) if !b.trim().is_empty() => b.trim().trim_end_matches('/').to_string(),
        _ => LLM_BASE.to_string(),
    }
}

/// reqwest client that never routes loopback through a system/corporate proxy.
fn llm_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())
}

/// List models from an OpenAI-compatible server. `base`/`api_key` come from the
/// Accounts settings; both optional (local LM Studio needs neither). Proxied
/// through Rust to dodge the webview's http/CORS/ATS restrictions.
#[tauri::command]
async fn llm_models(base: Option<String>, api_key: Option<String>) -> Result<Vec<String>, String> {
    let mut req = llm_client()?.get(format!("{}/models", llm_base(&base)));
    if let Some(k) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let r = req.send().await.map_err(|e| e.to_string())?;
    let j: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    Ok(j["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

/// One-shot chat completion. `messages` is the OpenAI-format array
/// `[{role, content}, …]`. `base`/`api_key` from Accounts (both optional).
#[tauri::command]
async fn llm_chat(
    model: String,
    messages: serde_json::Value,
    base: Option<String>,
    api_key: Option<String>,
) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model, "messages": messages, "temperature": 0.4, "stream": false
    });
    let mut req = llm_client()?
        .post(format!("{}/chat/completions", llm_base(&base)))
        .json(&body);
    if let Some(k) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let r = req.send().await.map_err(|e| e.to_string())?;
    let j: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    Ok(j["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

/// Streaming chat (#52): emits each content delta over `on_token` as it arrives
/// (OpenAI-compatible SSE). Goes through the Rust client so `.no_proxy()` still
/// applies (a frontend fetch would hit the corporate proxy on localhost).
#[tauri::command]
async fn llm_chat_stream(
    model: String,
    messages: serde_json::Value,
    base: Option<String>,
    api_key: Option<String>,
    on_token: Channel<String>,
) -> Result<(), String> {
    use futures_util::StreamExt;
    let body = serde_json::json!({
        "model": model, "messages": messages, "temperature": 0.4, "stream": true
    });
    let mut req = llm_client()?
        .post(format!("{}/chat/completions", llm_base(&base)))
        .json(&body);
    if let Some(k) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim().to_string();
            buf.drain(..=nl);
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    return Ok(());
                }
                if let Ok(j) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(tok) = j["choices"][0]["delta"]["content"].as_str() {
                        if !tok.is_empty() {
                            let _ = on_token.send(tok.to_string());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// `terraform plan` for a workspace dir (#78), no-color so the UI can colorize.
#[tauri::command]
fn terraform_plan(cwd: String) -> Result<String, String> {
    let out = std::process::Command::new("terraform")
        .current_dir(&cwd)
        .args(["plan", "-no-color", "-input=false"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(s)
}

/// `terraform state list` (#52) — managed resources in the current state.
#[tauri::command]
fn terraform_state(cwd: String) -> Result<String, String> {
    let out = std::process::Command::new("terraform")
        .current_dir(&cwd)
        .args(["state", "list"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(s)
}

/// `terraform apply -auto-approve` (#52). The approval gate is the in-app
/// confirm before this is invoked — never call it without explicit user consent.
#[tauri::command]
fn terraform_apply(cwd: String) -> Result<String, String> {
    let out = std::process::Command::new("terraform")
        .current_dir(&cwd)
        .args(["apply", "-no-color", "-input=false", "-auto-approve"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(s)
}

// --- Terraform / Terragrunt / OpenTofu (generic binary) -------------------
// The `bin` arg selects "terraform", "terragrunt", or "tofu". Only these three
// are accepted so a caller can't shell out to an arbitrary program.
fn tf_bin(bin: &str) -> Result<&'static str, String> {
    match bin {
        "terraform" => Ok("terraform"),
        "terragrunt" => Ok("terragrunt"),
        "tofu" => Ok("tofu"),
        _ => Err(format!("unsupported binary: {bin}")),
    }
}

fn tf_exec(bin: &str, cwd: &str, args: &[&str]) -> Result<String, String> {
    let prog = tf_bin(bin)?;
    let out = std::process::Command::new(prog)
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{prog} not found in PATH")
            } else {
                e.to_string()
            }
        })?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(s)
}

/// Detect which IaC tooling fits this dir: presence of terragrunt.hcl picks
/// terragrunt, otherwise terraform. Also reports which binaries are on PATH.
#[tauri::command]
fn tf_detect(cwd: String) -> Result<String, String> {
    let has_tg = std::path::Path::new(&cwd).join("terragrunt.hcl").exists()
        || std::path::Path::new(&cwd).join("root.hcl").exists();
    let on_path = |p: &str| {
        std::process::Command::new(p)
            .arg("version")
            .output()
            .map(|o| o.status.success() || !o.stdout.is_empty())
            .unwrap_or(false)
    };
    let prefer = if has_tg { "terragrunt" } else { "terraform" };
    // JSON so the frontend can pick a default and gray out missing tools.
    Ok(format!(
        "{{\"prefer\":\"{}\",\"terraform\":{},\"terragrunt\":{},\"tofu\":{}}}",
        prefer,
        on_path("terraform"),
        on_path("terragrunt"),
        on_path("tofu"),
    ))
}

/// `<bin> init -input=false -no-color` — downloads providers / modules.
#[tauri::command]
fn tf_init(cwd: String, bin: String) -> Result<String, String> {
    tf_exec(&bin, &cwd, &["init", "-input=false", "-no-color"])
}

/// `<bin> validate -no-color` — config validity, no remote state needed.
#[tauri::command]
fn tf_validate(cwd: String, bin: String) -> Result<String, String> {
    tf_exec(&bin, &cwd, &["validate", "-no-color"])
}

/// `<bin> plan -no-color -input=false` — preview changes, never mutates infra.
#[tauri::command]
fn tf_plan(cwd: String, bin: String) -> Result<String, String> {
    tf_exec(&bin, &cwd, &["plan", "-no-color", "-input=false"])
}

/// `<bin> state list` — managed resource addresses in current state.
#[tauri::command]
fn tf_state_list(cwd: String, bin: String) -> Result<String, String> {
    tf_exec(&bin, &cwd, &["state", "list"])
}

/// `<bin> output -json` — current root output values.
#[tauri::command]
fn tf_output(cwd: String, bin: String) -> Result<String, String> {
    tf_exec(&bin, &cwd, &["output", "-json", "-no-color"])
}

/// Instant Prometheus query (#77) — native HTTP, not an iframe. Returns the raw
/// JSON from `/api/v1/query`. no_proxy so it works behind the corporate proxy.
#[tauri::command]
async fn prom_query(base: String, query: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/api/v1/query", base.trim_end_matches('/'));
    let r = client
        .get(url)
        .query(&[("query", query.as_str())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    r.text().await.map_err(|e| e.to_string())
}

/// Range Prometheus query (#55) for sparklines — `/api/v1/query_range` over the
/// last `minutes`, with a step sized to ~60 points. Returns raw JSON. no_proxy.
#[tauri::command]
async fn prom_query_range(base: String, query: String, minutes: u64) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let span = minutes.max(1) * 60;
    let start = now.saturating_sub(span);
    let step = (span / 60).max(15);
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/api/v1/query_range", base.trim_end_matches('/'));
    let r = client
        .get(url)
        .query(&[
            ("query", query.as_str()),
            ("start", start.to_string().as_str()),
            ("end", now.to_string().as_str()),
            ("step", step.to_string().as_str()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    r.text().await.map_err(|e| e.to_string())
}

/// Loki instant LogQL query (#56). Native HTTP, no_proxy.
#[tauri::command]
async fn loki_query(base: String, query: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/loki/api/v1/query", base.trim_end_matches('/'));
    let r = client
        .get(url)
        .query(&[("query", query.as_str()), ("limit", "200")])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    r.text().await.map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
struct Entry {
    name: String,
    path: String,
    is_dir: bool,
}

/// List a directory: directories first, then files, both alphabetical.
/// Hidden entries (dot-prefixed) are skipped.
#[tauri::command]
fn list_dir(path: String) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for ent in std::fs::read_dir(&path)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        out.push(Entry {
            path: ent.path().to_string_lossy().into_owned(),
            name,
            is_dir,
        });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(out)
}

#[tauri::command]
fn read_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// Run a shell command in `cwd` and capture combined stdout+stderr for the agent
/// tool-use loop (#53). Always approval-gated in the UI. Output is truncated to
/// keep the captured text out of the model's way.
#[tauri::command]
fn run_capture(cwd: String, command: String) -> Result<String, String> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(&cwd)
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    const CAP: usize = 16_384;
    if s.len() > CAP {
        s.truncate(CAP);
        s.push_str("\n…(truncated)");
    }
    let code = out.status.code().unwrap_or(-1);
    Ok(format!("[exit {code}]\n{s}"))
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".svelte-kit",
    "build",
    "dist",
    ".cache",
    ".zig-cache",
    "zig-out",
    ".next",
    "vendor",
    "Pods",
    ".venv",
    "__pycache__",
];

fn walk(root: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>, cap: usize) {
    if out.len() >= cap {
        return;
    }
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for ent in rd.flatten() {
        if out.len() >= cap {
            return;
        }
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && name != ".env" {
            continue;
        }
        let p = ent.path();
        let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(&p, base, out, cap);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push(rel.to_string_lossy().into_owned());
        }
    }
}

/// Recursively list workspace files (relative paths) for the fuzzy finder,
/// skipping VCS/build dirs. Capped to keep it snappy on huge trees.
#[tauri::command]
fn walk_dir(root: String) -> Vec<String> {
    let base = std::path::PathBuf::from(&root);
    let mut out = Vec::new();
    walk(&base, &base, &mut out, 20000);
    out
}

/// #33 Lightweight import graph — greps import/from/require/use/include lines
/// across the workspace (via ripgrep) so the agent's repo-map carries module
/// edges, not just a file list. Returns `path: imported` lines (capped).
#[tauri::command]
fn repo_import_graph(root: String) -> Result<String, String> {
    let out = std::process::Command::new("rg")
        .args([
            "--no-heading",
            "--color=never",
            "--max-count=8",
            "-N",
            "-o",
            r#"^\s*(?:import .*|from \S+ import.*|.*require\(['"][^'"]+['"]\)|use [\w:]+;|#include [<"][^>"]+[>"])"#,
            "-g",
            "*.{ts,tsx,js,jsx,py,rs,go,c,cc,cpp,h,hpp,java,rb,svelte}",
            "--with-filename",
        ])
        .arg(&root)
        .output()
        .map_err(|_| "ripgrep (rg) not found".to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Trim absolute prefix to keep edges relative + cap size.
    let rel = text.replace(&format!("{}/", root.trim_end_matches('/')), "");
    Ok(rel.lines().take(2000).collect::<Vec<_>>().join("\n"))
}

/// Content search across the workspace via ripgrep (falls back to an error if
/// `rg` is missing). Returns raw `path:line:col:text` lines.
#[tauri::command]
fn grep(root: String, query: String) -> Result<String, String> {
    if query.trim().is_empty() {
        return Ok(String::new());
    }
    let out = std::process::Command::new("rg")
        .args([
            "--line-number",
            "--column",
            "--no-heading",
            "--color=never",
            "--max-count=200",
            "-S",
            &query,
        ])
        .arg(&root)
        .output()
        .map_err(|_| "ripgrep (rg) not found".to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[tauri::command]
fn write_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

fn state_path() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
        .join(".config")
        .join("anvil");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("state.json")
}

#[tauri::command]
fn read_state() -> String {
    std::fs::read_to_string(state_path()).unwrap_or_else(|_| "{}".into())
}

#[tauri::command]
fn write_state(contents: String) -> Result<(), String> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

// ── Secrets via the macOS Keychain (shell `security`, no extra crate) ──
// Stored under service "anvil:<key>" so they never touch localStorage/disk in
// plaintext. The frontend keeps only a list of WHICH keys are set.
const SECRET_SERVICE_PREFIX: &str = "anvil:";

/// #61 Unified read-only secret fetch from SSM / Vault / macOS Keychain.
/// Returns the value (never persisted) so the UI can mask/reveal it.
#[tauri::command]
fn secret_read(source: String, key: String) -> Result<String, String> {
    let out = match source.as_str() {
        "ssm" => {
            let mut cmd = std::process::Command::new("aws");
            cmd.args([
                "ssm",
                "get-parameter",
                "--with-decryption",
                "--name",
                &key,
                "--query",
                "Parameter.Value",
                "--output",
                "text",
            ]);
            let profile = aws_profile().lock().unwrap().clone();
            if !profile.is_empty() {
                cmd.env("AWS_PROFILE", &profile);
            }
            cmd.output()
        }
        "vault" => std::process::Command::new("vault")
            .args(["kv", "get", &key])
            .output(),
        "keychain" => std::process::Command::new("security")
            .args(["find-generic-password", "-s", &key, "-w"])
            .output(),
        other => return Err(format!("unknown secret source: {other}")),
    }
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[tauri::command]
fn secret_set(key: String, value: String) -> Result<(), String> {
    let service = format!("{SECRET_SERVICE_PREFIX}{key}");
    let out = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-a",
            "anvil",
            "-s",
            &service,
            "-w",
            &value,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[tauri::command]
fn secret_get(key: String) -> Result<String, String> {
    let service = format!("{SECRET_SERVICE_PREFIX}{key}");
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-a", "anvil", "-s", &service, "-w"])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
    } else {
        Ok(String::new()) // not found → empty, not an error
    }
}

/// #59 In-pane AWS resource listing — formatted text per service. AWS_PROFILE-aware.
#[tauri::command]
fn aws_list(service: String) -> Result<String, String> {
    let args: Vec<&str> = match service.as_str() {
        "ec2" => vec!["ec2", "describe-instances", "--query", "Reservations[].Instances[].{ID:InstanceId,Type:InstanceType,State:State.Name,Name:Tags[?Key==`Name`]|[0].Value}", "--output", "table"],
        "s3" => vec!["s3", "ls"],
        "lambda" => vec!["lambda", "list-functions", "--query", "Functions[].{Name:FunctionName,Runtime:Runtime,Mem:MemorySize}", "--output", "table"],
        "rds" => vec!["rds", "describe-db-instances", "--query", "DBInstances[].{ID:DBInstanceIdentifier,Engine:Engine,Class:DBInstanceClass,Status:DBInstanceStatus}", "--output", "table"],
        other => return Err(format!("unknown aws service: {other}")),
    };
    let mut cmd = std::process::Command::new("aws");
    cmd.args(&args);
    let profile = aws_profile().lock().unwrap().clone();
    if !profile.is_empty() {
        cmd.env("AWS_PROFILE", &profile);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && !stderr.is_empty() {
        s.push_str(&stderr);
    }
    Ok(s)
}

#[tauri::command]
fn secret_delete(key: String) -> Result<(), String> {
    let service = format!("{SECRET_SERVICE_PREFIX}{key}");
    let _ = std::process::Command::new("security")
        .args(["delete-generic-password", "-a", "anvil", "-s", &service])
        .output();
    Ok(())
}

#[tauri::command]
fn secret_has(key: String) -> Result<bool, String> {
    let service = format!("{SECRET_SERVICE_PREFIX}{key}");
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-a", "anvil", "-s", &service])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(out.status.success())
}

// Selected AWS named profile (from Accounts), applied as AWS_PROFILE to kubectl
// so EKS auth uses the right credentials.
static AWS_PROFILE: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new();
fn aws_profile() -> &'static Mutex<String> {
    AWS_PROFILE.get_or_init(|| Mutex::new(String::new()))
}

// #48 Managed kubectl port-forwards, keyed by child PID so they can be listed
// and stopped from the UI (a real managed list, not a fire-and-forget terminal).
static PORT_FORWARDS: std::sync::OnceLock<Mutex<HashMap<u32, (std::process::Child, String)>>> =
    std::sync::OnceLock::new();
fn port_forwards() -> &'static Mutex<HashMap<u32, (std::process::Child, String)>> {
    PORT_FORWARDS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[tauri::command]
fn kube_pf_start(
    context: String,
    namespace: String,
    pod: String,
    ports: String,
) -> Result<String, String> {
    let mut cmd = std::process::Command::new("kubectl");
    if !context.is_empty() {
        cmd.args(["--context", &context]);
    }
    cmd.args(["port-forward", "-n", &namespace, &pod, &ports]);
    let profile = aws_profile().lock().unwrap().clone();
    if !profile.is_empty() {
        cmd.env("AWS_PROFILE", &profile);
    }
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    let desc = format!("{namespace}/{pod} {ports}");
    port_forwards().lock().unwrap().insert(pid, (child, desc));
    Ok(pid.to_string())
}

#[tauri::command]
fn kube_pf_list() -> Result<String, String> {
    let mut map = port_forwards().lock().unwrap();
    let dead: Vec<u32> = map
        .iter_mut()
        .filter_map(|(pid, (c, _))| matches!(c.try_wait(), Ok(Some(_))).then_some(*pid))
        .collect();
    for d in dead {
        map.remove(&d);
    }
    Ok(map
        .iter()
        .map(|(pid, (_, desc))| format!("{pid}\t{desc}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

#[tauri::command]
fn kube_pf_stop(pid: u32) -> Result<(), String> {
    if let Some((mut c, _)) = port_forwards().lock().unwrap().remove(&pid) {
        let _ = c.kill();
        let _ = c.wait();
    }
    Ok(())
}

#[tauri::command]
fn set_aws_profile(profile: String) {
    *aws_profile().lock().unwrap() = profile;
}

/// Named profiles from ~/.aws/config (#58). One per line.
#[tauri::command]
fn aws_profiles() -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(format!("{home}/.aws/config")).unwrap_or_default();
    let mut names = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l
            .strip_prefix("[profile ")
            .and_then(|s| s.strip_suffix(']'))
        {
            names.push(rest.to_string());
        } else if l == "[default]" {
            names.push("default".to_string());
        }
    }
    Ok(names.join("\n"))
}

/// Host aliases from ~/.ssh/config (#17). One per line, wildcards skipped.
#[tauri::command]
fn ssh_hosts() -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(format!("{home}/.ssh/config")).unwrap_or_default();
    let mut hosts = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Host ").or_else(|| l.strip_prefix("host ")) {
            for h in rest.split_whitespace() {
                if !h.contains('*') && !h.contains('?') && !hosts.contains(&h.to_string()) {
                    hosts.push(h.to_string());
                }
            }
        }
    }
    Ok(hosts.join("\n"))
}

fn kubectl(args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("kubectl");
    cmd.args(args);
    let profile = aws_profile().lock().unwrap().clone();
    if !profile.is_empty() {
        cmd.env("AWS_PROFILE", &profile);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && !stderr.is_empty() {
        combined.push_str(&stderr);
    }
    Ok(combined)
}

#[tauri::command]
fn kube_contexts() -> Result<String, String> {
    kubectl(&["config", "get-contexts", "-o", "name"])
}

/// #50 Server-side diff of a manifest (`kubectl diff -f <path>`). Exit code 1
/// just means "differences found", so we return the combined output regardless.
#[tauri::command]
fn kube_diff(path: String) -> Result<String, String> {
    let out = kubectl(&["diff", "-f", &path])?;
    Ok(if out.trim().is_empty() {
        "(no differences)".into()
    } else {
        out
    })
}

/// #50 Apply a manifest after the user has approved the diff.
#[tauri::command]
fn kube_apply(path: String) -> Result<String, String> {
    kubectl(&["apply", "-f", &path])
}

#[tauri::command]
fn kube_current_context() -> Result<String, String> {
    kubectl(&["config", "current-context"])
}

#[tauri::command]
fn kube_use_context(name: String) -> Result<String, String> {
    kubectl(&["config", "use-context", &name])
}

/// Namespaces in the current context (#49).
#[tauri::command]
fn kube_namespaces() -> Result<String, String> {
    kubectl(&["get", "ns", "-o", "name"]).map(|s| {
        s.lines()
            .map(|l| l.trim_start_matches("namespace/"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// The namespace bound to the current context (defaults to "default").
#[tauri::command]
fn kube_current_namespace() -> Result<String, String> {
    let ns = kubectl(&["config", "view", "--minify", "-o", "jsonpath={..namespace}"])?;
    Ok(if ns.trim().is_empty() {
        "default".into()
    } else {
        ns.trim().to_string()
    })
}

/// Pin the namespace on the current context (#49).
#[tauri::command]
fn kube_set_namespace(namespace: String) -> Result<String, String> {
    kubectl(&[
        "config",
        "set-context",
        "--current",
        "--namespace",
        &namespace,
    ])
}

fn helm(args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("helm");
    cmd.args(args);
    let profile = aws_profile().lock().unwrap().clone();
    if !profile.is_empty() {
        cmd.env("AWS_PROFILE", &profile);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// All Helm releases across namespaces as JSON (#51).
#[tauri::command]
fn helm_list() -> Result<String, String> {
    helm(&["list", "-A", "-o", "json"])
}

/// Resolved values for one release (#51).
#[tauri::command]
fn helm_values(name: String, namespace: String) -> Result<String, String> {
    helm(&["get", "values", &name, "-n", &namespace])
}

/// #51 All computed values incl. chart defaults (`helm get values -a`), so the
/// UI can show user overrides vs the full merged set (defaults).
#[tauri::command]
fn helm_values_all(name: String, namespace: String) -> Result<String, String> {
    helm(&["get", "values", &name, "-n", &namespace, "-a"])
}

#[tauri::command]
fn kube_pods(context: String) -> Result<String, String> {
    if context.is_empty() {
        kubectl(&["get", "pods", "-A"])
    } else {
        kubectl(&["--context", &context, "get", "pods", "-A"])
    }
}

#[tauri::command]
fn kube_logs(context: String, namespace: String, pod: String) -> Result<String, String> {
    let mut args: Vec<&str> = Vec::new();
    if !context.is_empty() {
        args.push("--context");
        args.push(&context);
    }
    args.push("logs");
    args.push("-n");
    args.push(&namespace);
    args.push("--tail=300");
    args.push(&pod);
    kubectl(&args)
}

/// #46 In-pane multiplexed logs across pods matching a label selector.
#[tauri::command]
fn kube_logs_selector(
    context: String,
    namespace: String,
    selector: String,
) -> Result<String, String> {
    let mut args: Vec<&str> = Vec::new();
    if !context.is_empty() {
        args.push("--context");
        args.push(&context);
    }
    args.extend_from_slice(&[
        "logs",
        "-n",
        &namespace,
        "-l",
        &selector,
        "--all-containers",
        "--prefix",
        "--tail=200",
    ]);
    kubectl(&args)
}

/// `kubectl describe pod` (#74).
#[tauri::command]
fn kube_describe(context: String, namespace: String, pod: String) -> Result<String, String> {
    let mut args: Vec<&str> = Vec::new();
    if !context.is_empty() {
        args.push("--context");
        args.push(&context);
    }
    args.extend(["describe", "pod", "-n", &namespace, &pod]);
    kubectl(&args)
}

/// `kubectl delete pod` (#74). The controller recreates it — a quick restart.
#[tauri::command]
fn kube_delete_pod(context: String, namespace: String, pod: String) -> Result<String, String> {
    let mut args: Vec<&str> = Vec::new();
    if !context.is_empty() {
        args.push("--context");
        args.push(&context);
    }
    args.extend(["delete", "pod", "-n", &namespace, &pod]);
    kubectl(&args)
}

/// `kubectl rollout restart` for a pod's owning deployment, best-effort (#74).
#[tauri::command]
fn kube_restart(context: String, namespace: String, deployment: String) -> Result<String, String> {
    let dep = format!("deployment/{deployment}");
    let mut args: Vec<&str> = Vec::new();
    if !context.is_empty() {
        args.push("--context");
        args.push(&context);
    }
    args.extend(["rollout", "restart", "-n", &namespace, &dep]);
    kubectl(&args)
}

// GitHub token (from Accounts), passed to gh as GH_TOKEN.
static GITHUB_TOKEN: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new();
fn github_token() -> &'static Mutex<String> {
    GITHUB_TOKEN.get_or_init(|| Mutex::new(String::new()))
}

#[tauri::command]
fn set_github_token(token: String) {
    *github_token().lock().unwrap() = token;
}

fn gh_cmd(cwd: &str) -> std::process::Command {
    let mut c = std::process::Command::new("gh");
    c.current_dir(cwd);
    let t = github_token().lock().unwrap().clone();
    if !t.is_empty() {
        c.env("GH_TOKEN", &t);
    }
    c
}

#[tauri::command]
fn gh_runs(cwd: String) -> Result<String, String> {
    let out = gh_cmd(&cwd)
        .args(["run", "list", "-L", "20"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && !stderr.is_empty() {
        combined.push_str(&stderr);
    }
    Ok(combined)
}

#[tauri::command]
fn gh_runs_json(cwd: String) -> Result<String, String> {
    let out = gh_cmd(&cwd)
        .args([
            "run",
            "list",
            "-L",
            "20",
            "--json",
            "databaseId,status,conclusion,displayTitle,workflowName,headBranch,event",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[tauri::command]
fn gh_rerun(cwd: String, id: String) -> Result<String, String> {
    let out = gh_cmd(&cwd)
        .args(["run", "rerun", &id])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok("re-run queued".into())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// GitLab CI pipelines for the repo at `cwd` via the authed `glab` CLI (#54).
#[tauri::command]
fn glab_pipelines(cwd: String) -> Result<String, String> {
    let out = std::process::Command::new("glab")
        .current_dir(&cwd)
        .args(["ci", "list"])
        .output()
        .map_err(|e| format!("glab not found: {e}"))?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        s.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    Ok(s)
}

/// Status/details for one pipeline (#54). `glab ci get -p <id>`.
#[tauri::command]
fn glab_pipeline_get(cwd: String, id: String) -> Result<String, String> {
    let out = std::process::Command::new("glab")
        .current_dir(&cwd)
        .args(["ci", "get", "-p", &id])
        .output()
        .map_err(|e| format!("glab not found: {e}"))?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        s.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    Ok(s)
}

/// GitLab pipelines as JSON via `glab api` (25 most recent, sorted by updated_at desc).
#[tauri::command]
fn glab_pipelines_json(cwd: String) -> Result<String, String> {
    let out = std::process::Command::new("glab")
        .current_dir(&cwd)
        .args([
            "api",
            "projects/:id/pipelines?per_page=25&order_by=updated_at&sort=desc",
        ])
        .output()
        .map_err(|e| format!("glab not found: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Err(s)
    }
}

/// Jobs for one pipeline as JSON via `glab api`.
#[tauri::command]
fn glab_pipeline_jobs(cwd: String, pipeline: String) -> Result<String, String> {
    let path = format!("projects/:id/pipelines/{pipeline}/jobs?per_page=100");
    let out = std::process::Command::new("glab")
        .current_dir(&cwd)
        .args(["api", &path])
        .output()
        .map_err(|e| format!("glab not found: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Err(s)
    }
}

/// Raw log trace for one job. Returns partial content if the job is still running.
#[tauri::command]
fn glab_job_trace(cwd: String, job: String) -> Result<String, String> {
    let path = format!("projects/:id/jobs/{job}/trace");
    let out = std::process::Command::new("glab")
        .current_dir(&cwd)
        .args(["api", &path])
        .output()
        .map_err(|e| format!("glab not found: {e}"))?;
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() || !s.is_empty() {
        Ok(s)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Retry a pipeline via `glab api -X POST`.
#[tauri::command]
fn glab_pipeline_retry(cwd: String, pipeline: String) -> Result<String, String> {
    let path = format!("projects/:id/pipelines/{pipeline}/retry");
    let out = std::process::Command::new("glab")
        .current_dir(&cwd)
        .args(["api", "-X", "POST", &path])
        .output()
        .map_err(|e| format!("glab not found: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Err(s)
    }
}

/// Cancel a pipeline via `glab api -X POST`.
#[tauri::command]
fn glab_pipeline_cancel(cwd: String, pipeline: String) -> Result<String, String> {
    let path = format!("projects/:id/pipelines/{pipeline}/cancel");
    let out = std::process::Command::new("glab")
        .current_dir(&cwd)
        .args(["api", "-X", "POST", &path])
        .output()
        .map_err(|e| format!("glab not found: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Err(s)
    }
}

/// Full log for one Actions run (#53). `gh run view <id> --log`.
#[tauri::command]
fn gh_run_log(cwd: String, id: String) -> Result<String, String> {
    let out = gh_cmd(&cwd)
        .args(["run", "view", &id, "--log"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        // Logs may be unavailable mid-run — fall back to the job summary.
        if let Ok(v) = gh_cmd(&cwd).args(["run", "view", &id]).output() {
            s = String::from_utf8_lossy(&v.stdout).into_owned();
        }
    }
    Ok(s)
}

#[tauri::command]
fn git_blame(cwd: String, path: String) -> Result<String, String> {
    git(&cwd, &["blame", "--line-porcelain", "--", &path])
}

#[tauri::command]
fn gh_prs(cwd: String) -> Result<String, String> {
    let out = gh_cmd(&cwd)
        .args(["pr", "list", "-L", "20"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && !stderr.is_empty() {
        combined.push_str(&stderr);
    }
    Ok(combined)
}

/// #27 PR review: body + conversation comments for a PR number, as plain text.
#[tauri::command]
fn gh_pr_view(cwd: String, num: String) -> Result<String, String> {
    let out = gh_cmd(&cwd)
        .args(["pr", "view", &num, "--comments"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() && !stderr.is_empty() {
        s.push_str(&stderr);
    }
    Ok(s)
}

/// #27 Add a review comment to a PR via `gh pr comment <num> --body`.
#[tauri::command]
fn gh_pr_comment(cwd: String, num: String, body: String) -> Result<String, String> {
    let out = gh_cmd(&cwd)
        .args(["pr", "comment", &num, "--body", &body])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(s)
    } else {
        Err(s)
    }
}

/// Open a PR for the current branch via `gh pr create --fill` (#66).
#[tauri::command]
fn gh_pr_create(cwd: String) -> Result<String, String> {
    let out = gh_cmd(&cwd)
        .args(["pr", "create", "--fill"])
        .output()
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    if out.status.success() {
        Ok(s)
    } else {
        Err(s)
    }
}

/// View the current branch's PR in the browser via `gh pr view --web` (#66).
#[tauri::command]
fn gh_pr_web(cwd: String) -> Result<String, String> {
    gh_cmd(&cwd)
        .args(["pr", "view", "--web"])
        .output()
        .map_err(|e| e.to_string())
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
}

#[tauri::command]
fn git_tags(cwd: String) -> Result<String, String> {
    git(&cwd, &["tag", "--sort=-creatordate"])
}

#[tauri::command]
fn git_show_file(cwd: String, rev: String, path: String) -> Result<String, String> {
    let refpath = format!("{rev}:{path}");
    git(&cwd, &["show", &refpath])
}

/// Open a new top-level app window (⌘N). An optional `seed` (URL-encoded JSON,
/// built by the frontend) detaches a pane into the new window via a `?detach=`
/// query param (#17); the detached window seeds from it and skips state restore.
#[tauri::command]
fn new_window(app: tauri::AppHandle, seed: Option<String>) -> Result<(), String> {
    let label = format!("w{}", app.webview_windows().len() + 1);
    let path = match seed {
        Some(s) if !s.is_empty() => format!("index.html?detach={s}"),
        _ => "index.html".to_string(),
    };
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::App(path.into()))
        .title("Anvil")
        .inner_size(1280.0, 820.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Open an external URL (e.g. a Grafana dashboard) in a native webview window.
/// X-Frame-Options only blocks *framing*, so a top-level webview loads fine —
/// this is the iframe-free Grafana fix (#73, option a).
#[tauri::command]
fn open_url_window(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let u = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    let label = format!("ext{}", app.webview_windows().len() + 1);
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::External(u))
        .title("Anvil — Dashboard")
        .inner_size(1280.0, 860.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Native folder picker (File ▸ Open Folder…). Returns the chosen path or null.
#[tauri::command]
fn pick_folder(start: Option<String>) -> Option<String> {
    let mut d = rfd::FileDialog::new();
    if let Some(s) = start.filter(|s| !s.is_empty()) {
        d = d.set_directory(s);
    }
    d.pick_folder().map(|p| p.to_string_lossy().into_owned())
}

/// Native file picker (File ▸ Open File…). Returns the chosen path or null.
#[tauri::command]
fn pick_file(start: Option<String>) -> Option<String> {
    let mut d = rfd::FileDialog::new();
    if let Some(s) = start.filter(|s| !s.is_empty()) {
        d = d.set_directory(s);
    }
    d.pick_file().map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn create_path(path: String, is_dir: bool) -> Result<(), String> {
    if is_dir {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())
    } else {
        if let Some(p) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(p);
        }
        std::fs::write(&path, "").map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn rename_path(from: String, to: String) -> Result<(), String> {
    std::fs::rename(&from, &to).map_err(|e| e.to_string())
}

/// Last-modified time (unix seconds) for an open editor file's external-change
/// polling. Errors (missing file) map to 0.
#[tauri::command]
fn file_mtime(path: String) -> u64 {
    std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[tauri::command]
fn delete_path(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if p.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
    .map_err(|e| e.to_string())
}

// ── Caldera bridge (#53) ──────────────────────────────────────────────────
// Polls the local Caldera control-plane daemon (127.0.0.1:4175) over its GET
// API and returns a neutral snapshot. Everything is best-effort: if the daemon
// is down, `online` is false and the rest is empty — the UI shows "offline"
// rather than erroring. API shape ported from the Zig `caldera.zig` poller.
// Daemon listens on IPv4 loopback only; use 127.0.0.1 explicitly so we don't
// resolve `localhost` to ::1 (IPv6) first and fail.
const CALDERA_BASE: &str = "http://127.0.0.1:4175";

#[derive(serde::Serialize, Default)]
struct CalderaRun {
    agent: String,
    step: String,
    status: String,
    summary: String,
}

#[derive(serde::Serialize, Default)]
struct CalderaSnapshot {
    online: bool,
    project: String,
    branch: String,
    runs: Vec<CalderaRun>,
    attention: Vec<String>,
}

#[tauri::command]
async fn caldera_snapshot() -> CalderaSnapshot {
    let mut snap = CalderaSnapshot::default();
    // `.no_proxy()`: never route loopback through a system/corporate HTTP proxy
    // (e.g. FortiClient). reqwest honors system proxy env by default, which
    // breaks localhost connections inside the GUI app even though curl works.
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(_) => return snap,
    };
    let get = |path: &str| client.get(format!("{CALDERA_BASE}{path}")).send();

    // Health gate: daemon must answer with "ok".
    match get("/health").await {
        Ok(r) => match r.text().await {
            Ok(b) if b.contains("ok") => {}
            _ => return snap,
        },
        Err(_) => return snap,
    }
    snap.online = true;

    if let Ok(r) = get("/api/project").await {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            let p = &v["project"];
            snap.project = p["project_name"]
                .as_str()
                .or_else(|| p["name"].as_str())
                .unwrap_or("")
                .to_string();
            snap.branch = p["mode"].as_str().unwrap_or("").to_string();
        }
    }
    if let Ok(r) = get("/api/agent-runs").await {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(arr) = v["agent_runs"].as_array() {
                for run in arr {
                    let summary = run["events"]
                        .as_array()
                        .and_then(|e| e.last())
                        .and_then(|e| e["summary"].as_str())
                        .unwrap_or("")
                        .to_string();
                    snap.runs.push(CalderaRun {
                        agent: run["agent"].as_str().unwrap_or("").to_string(),
                        step: run["current_step"].as_str().unwrap_or("").to_string(),
                        status: run["backend_status"].as_str().unwrap_or("").to_string(),
                        summary,
                    });
                }
            }
        }
    }
    if let Ok(r) = get("/api/activity").await {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(arr) = v["attention"].as_array() {
                for a in arr {
                    if let Some(s) = a["summary"].as_str().or_else(|| a["title"].as_str()) {
                        snap.attention.push(s.to_string());
                    }
                }
            }
        }
    }
    snap
}

#[cfg(test)]
mod caldera_tests {
    use super::*;
    #[test]
    #[ignore = "requires a live Caldera daemon on localhost:4175"]
    fn reach() {
        let s = tauri::async_runtime::block_on(caldera_snapshot());
        eprintln!(
            "CALDERA online={} project='{}' runs={} attn={}",
            s.online,
            s.project,
            s.runs.len(),
            s.attention.len()
        );
    }
}

/// On-demand update check. Returns Some(version) if an update is available,
/// None if up to date, Err if the endpoint is unreachable/misconfigured. Never
/// runs at startup, so a missing release host degrades gracefully.
#[tauri::command]
async fn check_update(
    app: tauri::AppHandle,
    channel: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_updater::UpdaterExt;
    // #95 Release channel (stable/beta) sent as a header so the update server can
    // serve a different feed per channel. No-op until a release endpoint is live.
    let mut builder = app.updater_builder();
    if let Some(ch) = channel.filter(|c| !c.is_empty()) {
        builder = builder
            .header("X-Anvil-Channel", ch)
            .map_err(|e| e.to_string())?;
    }
    let updater = builder.build().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(update.version)),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Capture panics to a crash log so a hard failure leaves a trace.
    std::panic::set_hook(Box::new(|info| {
        if let Ok(home) = std::env::var("HOME") {
            let dir = std::path::Path::new(&home).join(".config/anvil");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("crash.log"), format!("{info}\n"));
        }
    }));

    let mut builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    // Desktop-only auto-update plugin (no-op until a release endpoint is live).
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }
    builder
        .manage(PtyState::default())
        .manage(lsp::LspState::default())
        .invoke_handler(tauri::generate_handler![
            pty_spawn,
            pty_set_active,
            pty_write,
            pty_resize,
            pty_kill,
            lsp::lsp_start,
            lsp::lsp_request,
            lsp::lsp_notify,
            check_update,
            caldera_snapshot,
            git_log,
            git_log_stats,
            git_status,
            git_repo_features,
            git_stage,
            git_unstage,
            git_discard,
            git_stage_all,
            git_commit,
            git_amend,
            git_last_message,
            git_branches,
            git_checkout,
            git_create_branch,
            git_diff,
            git_apply_hunk,
            git_show,
            git_show_file_diff,
            git_commit_files,
            git_file_log,
            git_stash_list,
            git_stash_save,
            git_stash_apply,
            git_stash_push,
            git_worktrees,
            git_worktree_add,
            git_log_range,
            git_rebase_run,
            git_checkout_side,
            git_submodule_update,
            git_lfs_pull,
            git_fetch,
            git_pull,
            git_push,
            git_current_branch,
            git_ahead_behind,
            home_dir,
            ssh_hosts,
            list_dir,
            read_file,
            run_capture,
            write_file,
            walk_dir,
            grep,
            repo_import_graph,
            read_state,
            write_state,
            secret_set,
            secret_read,
            aws_list,
            secret_get,
            secret_delete,
            secret_has,
            set_aws_profile,
            aws_profiles,
            set_github_token,
            llm_models,
            llm_chat,
            llm_chat_stream,
            prom_query,
            prom_query_range,
            loki_query,
            terraform_plan,
            terraform_state,
            tf_detect,
            tf_init,
            tf_validate,
            tf_plan,
            tf_state_list,
            tf_output,
            new_window,
            open_url_window,
            pick_folder,
            pick_file,
            create_path,
            rename_path,
            delete_path,
            file_mtime,
            kube_contexts,
            kube_diff,
            kube_apply,
            kube_current_context,
            kube_use_context,
            kube_namespaces,
            kube_current_namespace,
            kube_set_namespace,
            helm_list,
            helm_values,
            helm_values_all,
            kube_pods,
            kube_logs,
            kube_logs_selector,
            kube_pf_start,
            kube_pf_list,
            kube_pf_stop,
            kube_describe,
            kube_delete_pod,
            kube_restart,
            gh_runs,
            gh_runs_json,
            gh_rerun,
            gh_run_log,
            glab_pipelines,
            glab_pipeline_get,
            glab_pipelines_json,
            glab_pipeline_jobs,
            glab_job_trace,
            glab_pipeline_retry,
            glab_pipeline_cancel,
            terraform_apply,
            git_blame,
            gh_prs,
            gh_pr_view,
            gh_pr_comment,
            gh_pr_create,
            gh_pr_web,
            git_tags,
            git_show_file
        ])
        .setup(|app| {
            build_menu(app.handle())?;
            app.on_menu_event(|app, event| {
                let id = event.id().0.as_str();
                if let Some(action) = id.strip_prefix("menu:") {
                    let _ = app.emit("menu", action.to_string());
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Native macOS menu bar (File / Edit / View / Window). Custom items carry a
/// `menu:<action>` id and emit a `menu` event the frontend listens for; the
/// Edit/Window items are OS predefined (native cut/copy/paste/minimize).
fn build_menu(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};

    let mi =
        |id: &str, label: &str, accel: &str| -> Result<tauri::menu::MenuItem<_>, tauri::Error> {
            MenuItemBuilder::with_id(format!("menu:{id}"), label)
                .accelerator(accel)
                .build(app)
        };

    let app_menu = SubmenuBuilder::new(app, "Anvil")
        .about(None)
        .separator()
        .item(&mi("settings", "Settings…", "CmdOrCtrl+,")?)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&mi("new-term", "New Terminal", "CmdOrCtrl+T")?)
        .item(&mi("new-window", "New Window", "CmdOrCtrl+N")?)
        .separator()
        .item(&mi("open-file", "Open File…", "CmdOrCtrl+O")?)
        .item(&mi("open-folder", "Open Folder…", "CmdOrCtrl+Shift+O")?)
        .separator()
        .item(&mi("close-tab", "Close Tab", "CmdOrCtrl+W")?)
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&mi("palette", "Command Palette", "CmdOrCtrl+K")?)
        .item(&mi("goto-file", "Go to File…", "CmdOrCtrl+P")?)
        .separator()
        .item(&mi("toggle-sidebar", "Toggle Sidebar", "CmdOrCtrl+B")?)
        .item(&mi("zen", "Toggle Zen Mode", "CmdOrCtrl+.")?)
        .separator()
        .item(&mi("zoom-in", "Zoom In", "CmdOrCtrl+=")?)
        .item(&mi("zoom-out", "Zoom Out", "CmdOrCtrl+-")?)
        .item(&mi("zoom-reset", "Reset Zoom", "CmdOrCtrl+0")?)
        .build()?;

    let window_menu = SubmenuBuilder::new(app, "Window")
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::maximize(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::close_window(
            app,
            Some("Close Window"),
        )?)
        .build()?;

    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu])
        .build()?;
    app.set_menu(menu)?;
    Ok(())
}

#[cfg(test)]
mod git_integration_tests {
    use super::*;
    use std::path::Path;

    // Initialize a throwaway git repo in `dir` with local user config and no GPG signing.
    fn init_repo(dir: &Path) {
        let d = dir.to_str().unwrap();
        for args in &[
            vec!["-C", d, "init"],
            vec!["-C", d, "config", "init.defaultBranch", "main"],
            vec!["-C", d, "config", "user.email", "test@anvil.dev"],
            vec!["-C", d, "config", "user.name", "Anvil Test"],
            vec!["-C", d, "config", "commit.gpgsign", "false"],
        ] {
            let status = std::process::Command::new("git")
                .args(args)
                .status()
                .expect("git setup failed");
            assert!(status.success(), "git setup step failed: {args:?}");
        }
    }

    // Write a file and return its name as a String.
    fn write(dir: &Path, name: &str, content: &str) -> String {
        std::fs::write(dir.join(name), content).unwrap();
        name.to_string()
    }

    #[test]
    fn status_shows_untracked_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_repo(tmp.path());
        let cwd = tmp.path().to_str().unwrap().to_string();
        write(tmp.path(), "hello.txt", "hi");

        let out = git_status(cwd).unwrap();
        // Untracked files appear as "?? <name>" in porcelain output.
        assert!(
            out.contains("hello.txt"),
            "untracked file should appear in status: {out}"
        );
        assert!(
            out.contains("??"),
            "untracked marker '??' should be present: {out}"
        );
    }

    #[test]
    fn stage_and_commit_clears_untracked() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_repo(tmp.path());
        let cwd = tmp.path().to_str().unwrap().to_string();
        write(tmp.path(), "readme.md", "# Anvil");

        git_stage(cwd.clone(), "readme.md".into()).unwrap();
        git_commit(cwd.clone(), "initial commit".into(), None).unwrap();

        let status = git_status(cwd).unwrap();
        // After a clean commit the file must not appear as untracked or modified.
        assert!(
            !status.contains("readme.md"),
            "committed file should not appear in status: {status}"
        );
    }

    #[test]
    fn log_reflects_commit_message() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_repo(tmp.path());
        let cwd = tmp.path().to_str().unwrap().to_string();
        write(tmp.path(), "a.txt", "a");

        git_stage(cwd.clone(), "a.txt".into()).unwrap();
        git_commit(cwd.clone(), "feat: log round-trip".into(), None).unwrap();

        let log = git_log(cwd.clone(), None, None, None).unwrap();
        assert!(
            log.contains("feat: log round-trip"),
            "log should contain the commit message: {log}"
        );

        let last = git_last_message(cwd).unwrap();
        assert!(
            last.contains("feat: log round-trip"),
            "last message should match: {last}"
        );
    }

    #[test]
    fn branches_shows_new_branch_as_current() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_repo(tmp.path());
        let cwd = tmp.path().to_str().unwrap().to_string();

        // Need at least one commit before creating a branch.
        write(tmp.path(), "seed.txt", "seed");
        git_stage(cwd.clone(), "seed.txt".into()).unwrap();
        git_commit(cwd.clone(), "seed".into(), None).unwrap();

        git_create_branch(cwd.clone(), "feature".into()).unwrap();
        // create_branch uses `checkout -b` so we're already on it; no need to checkout again.

        let branches = git_branches(cwd).unwrap();
        // The current branch is prefixed with '*' in the format string %(HEAD).
        assert!(
            branches.contains("feature"),
            "feature branch should appear: {branches}"
        );
        let current_line = branches.lines().find(|l| l.starts_with('*'));
        assert!(
            current_line.map(|l| l.contains("feature")).unwrap_or(false),
            "feature should be the current branch (marked with *): {branches}"
        );
    }

    #[test]
    fn checkout_switches_branch() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_repo(tmp.path());
        let cwd = tmp.path().to_str().unwrap().to_string();

        write(tmp.path(), "seed.txt", "seed");
        git_stage(cwd.clone(), "seed.txt".into()).unwrap();
        git_commit(cwd.clone(), "seed".into(), None).unwrap();

        git_create_branch(cwd.clone(), "other".into()).unwrap();
        // Switch back to main to verify checkout works.
        git_checkout(cwd.clone(), "main".into()).unwrap();

        let branches = git_branches(cwd).unwrap();
        let current_line = branches.lines().find(|l| l.starts_with('*'));
        assert!(
            current_line.map(|l| l.contains("main")).unwrap_or(false),
            "main should be current after checkout: {branches}"
        );
    }

    #[test]
    fn diff_shows_modification() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_repo(tmp.path());
        let cwd = tmp.path().to_str().unwrap().to_string();

        write(tmp.path(), "file.txt", "original\n");
        git_stage(cwd.clone(), "file.txt".into()).unwrap();
        git_commit(cwd.clone(), "add file".into(), None).unwrap();

        // Modify the tracked file.
        write(tmp.path(), "file.txt", "original\nchanged\n");

        let diff = git_diff(cwd, "file.txt".into(), false).unwrap();
        // A hunk line added should appear as '+changed'.
        assert!(
            diff.contains("+changed"),
            "diff should show the added line: {diff}"
        );
    }

    #[test]
    fn error_on_non_repo_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        // No git init — plain directory. The git() helper captures only stdout,
        // so non-zero exit (stderr "fatal: not a git repository") maps to Ok("").
        // A commit attempt that uses combined stdout+stderr returns Err on failure.
        let cwd = tmp.path().to_str().unwrap().to_string();
        // git_status returns Ok("") — no useful output, proving no status data leaks.
        let status = git_status(cwd.clone()).unwrap_or_default();
        assert!(
            status.is_empty(),
            "non-repo status should produce no output: {status:?}"
        );
        // git_commit explicitly returns Err on non-zero exit.
        let commit = git_commit(cwd, "msg".into(), None);
        assert!(
            commit.is_err(),
            "commit on a non-repo should fail: {commit:?}"
        );
    }
}
