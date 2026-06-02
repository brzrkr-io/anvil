use std::io::Write;

fn git(cwd: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(cwd).args(args);
    let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// US-delimited `git log` (one commit per line) for the Source Control view.
/// Optional filters (#23): author, message grep, and path — applied server-side
/// so the swimlane graph rebuilds correctly from the filtered set.
#[tauri::command]
pub async fn git_log(
    cwd: String,
    author: Option<String>,
    grep: Option<String>,
    path: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Per-commit insertion/deletion totals for the history view (Terax-style
/// `+N -N` column). Mirrors git_log's filters so the commit set matches.
/// Output is `--shortstat` interleaved with a `\x01<shorthash>` marker per
/// commit; the frontend sums them by hash.
#[tauri::command]
pub async fn git_log_stats(
    cwd: String,
    author: Option<String>,
    grep: Option<String>,
    path: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_status(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["status", "--porcelain=v1", "-b"]))
        .await
        .map_err(|e| e.to_string())?
}

/// #21 One-line `git log` for a range (e.g. `origin/main..HEAD`) — rebase preview.
#[tauri::command]
pub async fn git_log_range(cwd: String, range: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        git(&cwd, &["log", "--oneline", "--no-decorate", &range])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #21 Run a non-interactive rebase from a UI-built todo. The todo is dropped in
/// as the rebase sequence via GIT_SEQUENCE_EDITOR (supports pick/fixup/drop +
/// reordering — no message editors open, so it never blocks). On failure the
/// rebase is aborted so the tree is left clean. (Unix shells; Windows pending.)
#[tauri::command]
pub async fn git_rebase_run(cwd: String, target: String, todo: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("anvil-rebase-{}.txt", std::process::id()));
        std::fs::write(&tmp, todo).map_err(|e| e.to_string())?;
        let editor = format!("cp '{}'", tmp.display());
        let mut rcmd = std::process::Command::new("git");
        rcmd.current_dir(&cwd)
            .env("GIT_SEQUENCE_EDITOR", &editor)
            .env("GIT_EDITOR", "true")
            .args(["rebase", "-i", &target]);
        let out = crate::shared::exec_capture(rcmd, 30).map_err(|e| e.to_string())?;
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
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #25 Resolve a merge conflict by taking one side wholesale, then stage it.
/// `side` is "ours" or "theirs".
#[tauri::command]
pub async fn git_checkout_side(cwd: String, path: String, side: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let flag = if side == "theirs" {
            "--theirs"
        } else {
            "--ours"
        };
        git(&cwd, &["checkout", flag, "--", &path])?;
        git(&cwd, &["add", "--", &path])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #29 Update submodules to their pinned commits.
#[tauri::command]
pub async fn git_submodule_update(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        git(&cwd, &["submodule", "update", "--init", "--recursive"])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #29 Pull Git LFS objects for the working tree.
#[tauri::command]
pub async fn git_lfs_pull(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("git");
        cmd.current_dir(&cwd).args(["lfs", "pull"]);
        let out = crate::shared::exec_capture(cmd, 180).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(s)
        } else {
            Err(s)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #24 Worktrees: tab-separated `path\tbranch` per line.
#[tauri::command]
pub async fn git_worktrees(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #24 Add a worktree for an existing branch at a sibling path.
#[tauri::command]
pub async fn git_worktree_add(cwd: String, path: String, branch: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["worktree", "add", &path, &branch]))
        .await
        .map_err(|e| e.to_string())?
}

/// Comma-separated repo features: "submodules" and/or "lfs" (#29).
#[tauri::command]
pub async fn git_repo_features(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_stage(cwd: String, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["add", "--", &path]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_unstage(cwd: String, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["restore", "--staged", "--", &path]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_discard(cwd: String, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["checkout", "--", &path]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_stage_all(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["add", "-A"]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_commit(
    cwd: String,
    message: String,
    amend: Option<bool>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args = vec!["-C", &cwd, "commit"];
        if amend.unwrap_or(false) {
            args.push("--amend");
        }
        args.extend(["-m", &message]);
        let mut cmd = std::process::Command::new("git");
        cmd.args(&args);
        let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
        let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(combined)
        } else {
            Err(combined.trim().to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Amend the last commit with currently staged changes, keeping its message (#63).
#[tauri::command]
pub async fn git_amend(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["commit", "--amend", "--no-edit"]))
        .await
        .map_err(|e| e.to_string())?
}

/// Full message (subject + body) of the last commit, for amend prefill.
#[tauri::command]
pub async fn git_last_message(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["log", "-1", "--pretty=%B"]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_branches(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        git(&cwd, &["branch", "--format=%(HEAD)\t%(refname:short)"])
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_checkout(cwd: String, branch: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["checkout", &branch]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_create_branch(cwd: String, name: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["checkout", "-b", &name]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_diff(cwd: String, path: String, staged: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if staged {
            git(&cwd, &["diff", "--cached", "--", &path])
        } else {
            git(&cwd, &["diff", "--", &path])
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Apply a single-hunk patch (built by buildHunkPatch in git.ts) via
/// `git apply`, piping the patch on stdin (#62). `cached` stages into the index;
/// `reverse` discards (applies the inverse). Returns git's stderr on failure.
#[tauri::command]
pub async fn git_apply_hunk(
    cwd: String,
    patch: String,
    cached: bool,
    reverse: bool,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_show(cwd: String, rev: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["show", "--stat", "--patch", &rev]))
        .await
        .map_err(|e| e.to_string())?
}

/// The patch for a single file within a commit (popover → file → diff).
#[tauri::command]
pub async fn git_show_file_diff(cwd: String, rev: String, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        git(&cwd, &["show", "--patch", "--format=", &rev, "--", &path])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Changed files in a commit as `STATUS\tpath` lines (commit popover file list).
#[tauri::command]
pub async fn git_commit_files(cwd: String, rev: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        git(&cwd, &["show", "--name-status", "--format=", &rev])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Commit history for a single file (File History, #67). One line per commit:
/// hash\x1fshort\x1fauthor\x1ftimestamp\x1fsubject.
#[tauri::command]
pub async fn git_file_log(cwd: String, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_stash_list(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["stash", "list"]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_stash_save(cwd: String, message: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["stash", "push", "-m", &message]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_stash_apply(cwd: String, index: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["stash", "apply", &index]))
        .await
        .map_err(|e| e.to_string())?
}

/// Partial stash (#28): optional message, specific paths, and/or untracked.
#[tauri::command]
pub async fn git_stash_push(
    cwd: String,
    message: Option<String>,
    paths: Option<Vec<String>>,
    untracked: Option<bool>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_fetch(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(&cwd).args(["fetch", "--all"]);
        let out = crate::shared::exec_capture(cmd, 180).map_err(|e| e.to_string())?;
        let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !stderr.is_empty() {
            combined.push_str(&stderr);
        }
        Ok(combined)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Pull (fast-forward only) using the user's configured git auth (#64).
#[tauri::command]
pub async fn git_pull(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(&cwd).args(["pull", "--ff-only"]);
        let out = crate::shared::exec_capture(cmd, 180).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(s)
        } else {
            Err(s)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Push the current branch using the user's configured git auth (#64).
#[tauri::command]
pub async fn git_push(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(&cwd).args(["push"]);
        let out = crate::shared::exec_capture(cmd, 180).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(s)
        } else {
            Err(s)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_current_branch(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["rev-parse", "--abbrev-ref", "HEAD"]))
        .await
        .map_err(|e| e.to_string())?
}

/// Commits ahead/behind the upstream as "ahead\tbehind" (#68). Errors when the
/// branch has no upstream — the caller treats that as "no indicator".
#[tauri::command]
pub async fn git_ahead_behind(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        git(
            &cwd,
            &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_blame(cwd: String, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        git(&cwd, &["blame", "--line-porcelain", "--", &path])
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_tags(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || git(&cwd, &["tag", "--sort=-creatordate"]))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_show_file(cwd: String, rev: String, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let refpath = format!("{rev}:{path}");
        git(&cwd, &["show", &refpath])
    })
    .await
    .map_err(|e| e.to_string())?
}
