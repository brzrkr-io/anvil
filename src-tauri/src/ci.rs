use crate::cloud::gh_cmd;

#[tauri::command]
pub async fn gh_runs(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["run", "list", "-L", "20"]);
        let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
        let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() && !stderr.is_empty() {
            combined.push_str(&stderr);
        }
        Ok(combined)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn gh_runs_json(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args([
            "run",
            "list",
            "-L",
            "20",
            "--json",
            "databaseId,status,conclusion,displayTitle,workflowName,headBranch,event",
        ]);
        let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn gh_rerun(cwd: String, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["run", "rerun", &id]);
        let out = crate::shared::exec_capture(cmd, 30).map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok("re-run queued".into())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Full log for one Actions run (#53). `gh run view <id> --log`.
#[tauri::command]
pub async fn gh_run_log(cwd: String, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["run", "view", &id, "--log"]);
        let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            s.push_str(&String::from_utf8_lossy(&out.stderr));
            // Logs may be unavailable mid-run — fall back to the job summary.
            let mut fallback = gh_cmd(&cwd);
            fallback.args(["run", "view", &id]);
            if let Ok(v) = crate::shared::exec_capture(fallback, 25) {
                s = String::from_utf8_lossy(&v.stdout).into_owned();
            }
        }
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn gh_prs(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["pr", "list", "-L", "20"]);
        let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
        let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() && !stderr.is_empty() {
            combined.push_str(&stderr);
        }
        Ok(combined)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// PR list with CI check status, so failing PRs surface without a browser trip.
/// Returns the raw `gh pr list --json …` array (number, title, branch, draft,
/// statusCheckRollup) for the frontend to roll up and sort failing-first.
#[tauri::command]
pub async fn gh_prs_json(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args([
            "pr",
            "list",
            "-L",
            "20",
            "--json",
            "number,title,headRefName,isDraft,statusCheckRollup",
        ]);
        let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #27 PR review: body + conversation comments for a PR number, as plain text.
#[tauri::command]
pub async fn gh_pr_view(cwd: String, num: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["pr", "view", &num, "--comments"]);
        let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() && !stderr.is_empty() {
            s.push_str(&stderr);
        }
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #27 Add a review comment to a PR via `gh pr comment <num> --body`.
#[tauri::command]
pub async fn gh_pr_comment(cwd: String, num: String, body: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["pr", "comment", &num, "--body", &body]);
        let out = crate::shared::exec_capture(cmd, 30).map_err(|e| e.to_string())?;
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

/// Submit a PR review: `gh pr review <num> --approve|--request-changes|--comment`
/// with an optional body. `action` is allow-listed so it can't inject flags.
#[tauri::command]
pub async fn gh_pr_review(
    cwd: String,
    num: String,
    action: String,
    body: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let flag = match action.as_str() {
            "approve" => "--approve",
            "request-changes" => "--request-changes",
            "comment" => "--comment",
            _ => return Err(format!("unsupported review action: {action}")),
        };
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["pr", "review", &num, flag]);
        if !body.trim().is_empty() {
            cmd.args(["--body", &body]);
        }
        let out = crate::shared::exec_capture(cmd, 30).map_err(|e| e.to_string())?;
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

/// The unified diff of a PR via `gh pr diff <num>` (no color so the UI colorizes).
#[tauri::command]
pub async fn gh_pr_diff(cwd: String, num: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["pr", "diff", &num]);
        let out = crate::shared::exec_capture(cmd, 30).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() && !stderr.is_empty() {
            s.push_str(&stderr);
        }
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open a PR for the current branch via `gh pr create --fill` (#66).
#[tauri::command]
pub async fn gh_pr_create(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["pr", "create", "--fill"]);
        let out = crate::shared::exec_capture(cmd, 30).map_err(|e| e.to_string())?;
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

/// View the current branch's PR in the browser via `gh pr view --web` (#66).
#[tauri::command]
pub async fn gh_pr_web(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = gh_cmd(&cwd);
        cmd.args(["pr", "view", "--web"]);
        crate::shared::exec_capture(cmd, 25)
            .map_err(|e| e.to_string())
            .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// GitLab CI pipelines for the repo at `cwd` via the authed `glab` CLI (#54).
#[tauri::command]
pub async fn glab_pipelines(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("glab");
        cmd.current_dir(&cwd).args(["ci", "list"]);
        let out =
            crate::shared::exec_capture(cmd, 25).map_err(|e| format!("glab not found: {e}"))?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            s.push_str(&String::from_utf8_lossy(&out.stderr));
        }
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Status/details for one pipeline (#54). `glab ci get -p <id>`.
#[tauri::command]
pub async fn glab_pipeline_get(cwd: String, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("glab");
        cmd.current_dir(&cwd).args(["ci", "get", "-p", &id]);
        let out =
            crate::shared::exec_capture(cmd, 25).map_err(|e| format!("glab not found: {e}"))?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            s.push_str(&String::from_utf8_lossy(&out.stderr));
        }
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// GitLab pipelines as JSON via `glab api` (25 most recent, sorted by updated_at desc).
#[tauri::command]
pub async fn glab_pipelines_json(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("glab");
        cmd.current_dir(&cwd).args([
            "api",
            "projects/:id/pipelines?per_page=25&order_by=updated_at&sort=desc",
        ]);
        let out =
            crate::shared::exec_capture(cmd, 25).map_err(|e| format!("glab not found: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&out.stderr));
            Err(s)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Jobs for one pipeline as JSON via `glab api`.
#[tauri::command]
pub async fn glab_pipeline_jobs(cwd: String, pipeline: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = format!("projects/:id/pipelines/{pipeline}/jobs?per_page=100");
        let mut cmd = std::process::Command::new("glab");
        cmd.current_dir(&cwd).args(["api", &path]);
        let out =
            crate::shared::exec_capture(cmd, 25).map_err(|e| format!("glab not found: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&out.stderr));
            Err(s)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Raw log trace for one job. Returns partial content if the job is still running.
#[tauri::command]
pub async fn glab_job_trace(cwd: String, job: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = format!("projects/:id/jobs/{job}/trace");
        let mut cmd = std::process::Command::new("glab");
        cmd.current_dir(&cwd).args(["api", &path]);
        let out =
            crate::shared::exec_capture(cmd, 25).map_err(|e| format!("glab not found: {e}"))?;
        let s = String::from_utf8_lossy(&out.stdout).into_owned();
        if out.status.success() || !s.is_empty() {
            Ok(s)
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Retry a pipeline via `glab api -X POST`.
#[tauri::command]
pub async fn glab_pipeline_retry(cwd: String, pipeline: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = format!("projects/:id/pipelines/{pipeline}/retry");
        let mut cmd = std::process::Command::new("glab");
        cmd.current_dir(&cwd).args(["api", "-X", "POST", &path]);
        let out =
            crate::shared::exec_capture(cmd, 30).map_err(|e| format!("glab not found: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&out.stderr));
            Err(s)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Retry one job via `glab api -X POST projects/:id/jobs/<id>/retry`.
#[tauri::command]
pub async fn glab_job_retry(cwd: String, job: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = format!("projects/:id/jobs/{job}/retry");
        let mut cmd = std::process::Command::new("glab");
        cmd.current_dir(&cwd).args(["api", "-X", "POST", &path]);
        let out =
            crate::shared::exec_capture(cmd, 30).map_err(|e| format!("glab not found: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&out.stderr));
            Err(s)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Play (start) a manual job via `glab api -X POST projects/:id/jobs/<id>/play`.
#[tauri::command]
pub async fn glab_job_play(cwd: String, job: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = format!("projects/:id/jobs/{job}/play");
        let mut cmd = std::process::Command::new("glab");
        cmd.current_dir(&cwd).args(["api", "-X", "POST", &path]);
        let out =
            crate::shared::exec_capture(cmd, 30).map_err(|e| format!("glab not found: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&out.stderr));
            Err(s)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Cancel a pipeline via `glab api -X POST`.
#[tauri::command]
pub async fn glab_pipeline_cancel(cwd: String, pipeline: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = format!("projects/:id/pipelines/{pipeline}/cancel");
        let mut cmd = std::process::Command::new("glab");
        cmd.current_dir(&cwd).args(["api", "-X", "POST", &path]);
        let out =
            crate::shared::exec_capture(cmd, 30).map_err(|e| format!("glab not found: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&out.stderr));
            Err(s)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
