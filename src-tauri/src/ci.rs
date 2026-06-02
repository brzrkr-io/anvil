use crate::cloud::gh_cmd;

#[tauri::command]
pub fn gh_runs(cwd: String) -> Result<String, String> {
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
pub fn gh_runs_json(cwd: String) -> Result<String, String> {
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
pub fn gh_rerun(cwd: String, id: String) -> Result<String, String> {
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

/// Full log for one Actions run (#53). `gh run view <id> --log`.
#[tauri::command]
pub fn gh_run_log(cwd: String, id: String) -> Result<String, String> {
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
pub fn gh_prs(cwd: String) -> Result<String, String> {
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
pub fn gh_pr_view(cwd: String, num: String) -> Result<String, String> {
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
pub fn gh_pr_comment(cwd: String, num: String, body: String) -> Result<String, String> {
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
pub fn gh_pr_create(cwd: String) -> Result<String, String> {
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
pub fn gh_pr_web(cwd: String) -> Result<String, String> {
    gh_cmd(&cwd)
        .args(["pr", "view", "--web"])
        .output()
        .map_err(|e| e.to_string())
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
}

/// GitLab CI pipelines for the repo at `cwd` via the authed `glab` CLI (#54).
#[tauri::command]
pub fn glab_pipelines(cwd: String) -> Result<String, String> {
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
pub fn glab_pipeline_get(cwd: String, id: String) -> Result<String, String> {
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
pub fn glab_pipelines_json(cwd: String) -> Result<String, String> {
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
pub fn glab_pipeline_jobs(cwd: String, pipeline: String) -> Result<String, String> {
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
pub fn glab_job_trace(cwd: String, job: String) -> Result<String, String> {
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
pub fn glab_pipeline_retry(cwd: String, pipeline: String) -> Result<String, String> {
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

/// Retry one job via `glab api -X POST projects/:id/jobs/<id>/retry`.
#[tauri::command]
pub fn glab_job_retry(cwd: String, job: String) -> Result<String, String> {
    let path = format!("projects/:id/jobs/{job}/retry");
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

/// Play (start) a manual job via `glab api -X POST projects/:id/jobs/<id>/play`.
#[tauri::command]
pub fn glab_job_play(cwd: String, job: String) -> Result<String, String> {
    let path = format!("projects/:id/jobs/{job}/play");
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
pub fn glab_pipeline_cancel(cwd: String, pipeline: String) -> Result<String, String> {
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
