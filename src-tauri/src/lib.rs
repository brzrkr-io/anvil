//! Anvil core — Tauri backend.
//! Owns the PTY (cross-platform via portable-pty) and thin git helpers.
//! The webview frontend (Svelte + xterm.js) drives everything over IPC.

use tauri::Manager;

mod ci;
mod cloud;
mod docker;
mod doctor;
mod flux;
mod fs;
mod git;
mod iac;
mod kube;
mod llm;
mod lsp;
mod observability;
mod pty;
mod shared;
mod window;

use tauri::Emitter;

use pty::PtyState;

// Per-window state file so multiple windows don't clobber each other's session.
// The primary window ("main" or unset) keeps the legacy `state.json`; others get
// `state-<label>.json`. The label is sanitized to a safe filename fragment.
fn state_path(label: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
        .join(".config")
        .join("anvil");
    let _ = std::fs::create_dir_all(&dir);
    let file = if label.is_empty() || label == "main" {
        "state.json".to_string()
    } else {
        let safe: String = label
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        format!("state-{safe}.json")
    };
    dir.join(file)
}

#[tauri::command]
async fn read_state(label: Option<String>) -> String {
    let label = label.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read_to_string(state_path(&label)).unwrap_or_else(|_| "{}".into())
    })
    .await
    .unwrap_or_else(|_| "{}".into())
}

#[tauri::command]
async fn write_state(contents: String, label: Option<String>) -> Result<(), String> {
    let label = label.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        let path = state_path(&label);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, contents).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Run a shell command in `cwd` and capture combined stdout+stderr for the agent
/// tool-use loop (#53). Always approval-gated in the UI. Output is truncated to
/// keep the captured text out of the model's way.
#[tauri::command]
async fn run_capture(cwd: String, command: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(&command).current_dir(&cwd);
        let out = shared::exec_capture(cmd, 120).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        const CAP: usize = 16_384;
        if s.len() > CAP {
            s.truncate(CAP);
            s.push_str("\n…(truncated)");
        }
        let code = out.status.code().unwrap_or(-1);
        Ok(format!("[exit {code}]\n{s}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #33 Lightweight import graph — greps import/from/require/use/include lines
/// across the workspace (via ripgrep) so the agent's repo-map carries module
/// edges, not just a file list. Returns `path: imported` lines (capped).
#[tauri::command]
async fn repo_import_graph(root: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("rg");
        cmd.args([
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
        .arg(&root);
        let out = shared::exec_capture(cmd, 25)
            .map_err(|_| "ripgrep (rg) not found".to_string())?;
        let text = String::from_utf8_lossy(&out.stdout);
        // Trim absolute prefix to keep edges relative + cap size.
        let rel = text.replace(&format!("{}/", root.trim_end_matches('/')), "");
        Ok(rel.lines().take(2000).collect::<Vec<_>>().join("\n"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Content search across the workspace via ripgrep (falls back to an error if
/// `rg` is missing). Returns raw `path:line:col:text` lines.
#[tauri::command]
async fn grep(root: String, query: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if query.trim().is_empty() {
            return Ok(String::new());
        }
        let mut cmd = std::process::Command::new("rg");
        cmd.args([
            "--line-number",
            "--column",
            "--no-heading",
            "--color=never",
            "--max-count=200",
            "-S",
            &query,
        ])
        .arg(&root);
        let out =
            shared::exec_capture(cmd, 25).map_err(|_| "ripgrep (rg) not found".to_string())?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Host aliases from ~/.ssh/config (#17). One per line, wildcards skipped.
#[tauri::command]
async fn ssh_hosts() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn rename_path(from: String, to: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::rename(&from, &to).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn delete_path(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = std::path::Path::new(&path);
        if p.is_dir() {
            std::fs::remove_dir_all(p)
        } else {
            std::fs::remove_file(p)
        }
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
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

/// Download + install the pending update (signature-verified by the updater),
/// then relaunch into the new version. User-initiated from the update prompt.
/// Returns Err if no update is available or the install fails; on success the
/// app restarts and this never returns.
#[tauri::command]
async fn install_update(app: tauri::AppHandle, channel: Option<String>) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let mut builder = app.updater_builder();
    if let Some(ch) = channel.filter(|c| !c.is_empty()) {
        builder = builder
            .header("X-Anvil-Channel", ch)
            .map_err(|e| e.to_string())?;
    }
    let updater = builder.build().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No update available".to_string())?;
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
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
        // Persist window size/position across relaunches. Only size/position/
        // maximized — leave decorations/visibility/fullscreen to the config so the
        // custom overlay titlebar + transparency aren't disturbed.
        use tauri_plugin_window_state::StateFlags;
        builder = builder.plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        );
    }
    builder
        .manage(PtyState::default())
        .manage(lsp::LspState::default())
        .invoke_handler(tauri::generate_handler![
            pty::pty_spawn,
            pty::pty_set_active,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_kill,
            lsp::lsp_start,
            lsp::lsp_stop,
            lsp::lsp_request,
            lsp::lsp_notify,
            check_update,
            install_update,
            caldera_snapshot,
            git::git_log,
            git::git_log_stats,
            git::git_status,
            git::git_repo_features,
            git::git_stage,
            git::git_unstage,
            git::git_discard,
            git::git_stage_all,
            git::git_commit,
            git::git_amend,
            git::git_last_message,
            git::git_branches,
            git::git_reflog,
            git::git_branch_compare,
            git::git_checkout,
            git::git_create_branch,
            git::git_diff,
            git::git_apply_hunk,
            git::git_show,
            git::git_show_file_diff,
            git::git_commit_files,
            git::git_file_log,
            git::git_stash_list,
            git::git_stash_save,
            git::git_stash_apply,
            git::git_stash_push,
            git::git_worktrees,
            git::git_worktree_add,
            git::git_log_range,
            git::git_rebase_run,
            git::git_checkout_side,
            git::git_op_state,
            git::git_op_abort,
            git::git_op_continue,
            git::git_submodule_update,
            git::git_lfs_pull,
            git::git_fetch,
            git::git_pull,
            git::git_push,
            git::git_current_branch,
            git::git_ahead_behind,
            fs::home_dir,
            ssh_hosts,
            fs::list_dir,
            fs::read_file,
            run_capture,
            fs::write_file,
            fs::walk_dir,
            grep,
            repo_import_graph,
            read_state,
            write_state,
            doctor::doctor_check,
            cloud::secret_set,
            cloud::secret_read,
            cloud::aws_list,
            cloud::secret_get,
            cloud::secret_delete,
            cloud::secret_has,
            cloud::set_aws_profile,
            cloud::aws_profiles,
            cloud::set_github_token,
            llm::llm_models,
            llm::llm_chat,
            llm::llm_chat_stream,
            observability::prom_query,
            observability::sentry_issues,
            observability::slack_post,
            observability::prom_query_range,
            observability::grafana_dashboards,
            observability::signoz_query,
            observability::signoz_services,
            iac::terraform_plan,
            iac::terraform_state,
            iac::tf_detect,
            iac::tf_discover,
            iac::tf_init,
            iac::tf_validate,
            iac::tf_plan,
            iac::tf_state_list,
            iac::tf_output,
            iac::tg_stack_output,
            flux::flux_get,
            flux::flux_check,
            flux::flux_reconcile,
            flux::flux_suspend,
            flux::flux_resume,
            window::new_window,
            window::open_url_window,
            window::open_named_window,
            window::set_vibrancy,
            fs::pick_folder,
            fs::pick_file,
            fs::create_path,
            rename_path,
            delete_path,
            fs::file_mtime,
            kube::kube_contexts,
            kube::kube_diff,
            kube::kube_current_context,
            kube::kube_use_context,
            kube::kube_namespaces,
            kube::kube_current_namespace,
            kube::kube_set_namespace,
            iac::helm_list,
            iac::helm_values,
            iac::helm_values_all,
            iac::helm_history,
            iac::helm_status,
            iac::helm_manifest,
            iac::helm_rollback,
            iac::helm_diff_revision,
            kube::kube_pods,
            kube::kube_nodes,
            kube::kube_deployments,
            kube::kube_logs,
            kube::kube_logs_selector,
            kube::kube_pf_start,
            kube::kube_pf_list,
            kube::kube_pf_stop,
            kube::kube_describe,
            kube::kube_delete_pod,
            kube::kube_restart,
            ci::gh_runs,
            ci::gh_runs_json,
            ci::gh_rerun,
            ci::gh_run_log,
            ci::glab_pipelines,
            ci::glab_pipeline_get,
            ci::glab_pipelines_json,
            ci::glab_pipeline_jobs,
            ci::glab_job_trace,
            ci::glab_pipeline_retry,
            ci::glab_pipeline_cancel,
            ci::glab_job_retry,
            ci::glab_job_play,
            iac::terraform_apply,
            git::git_blame,
            ci::gh_prs,
            ci::gh_prs_json,
            ci::gh_pr_view,
            ci::gh_pr_comment,
            ci::gh_pr_review,
            ci::gh_pr_diff,
            ci::gh_pr_create,
            ci::gh_pr_merge,
            ci::gh_workflow_list,
            ci::gh_workflow_run,
            docker::docker_ps,
            docker::docker_action,
            ci::gh_pr_web,
            git::git_tags,
            git::git_show_file
        ])
        .setup(|app| {
            build_menu(app.handle())?;
            app.on_menu_event(|app, event| {
                let id = event.id().0.as_str();
                if let Some(action) = id.strip_prefix("menu:") {
                    let _ = app.emit("menu", action.to_string());
                }
            });
            // Multi-monitor safety: a restored position can land off-screen if the
            // display it was on is gone. Recenter any window whose center isn't on
            // a connected monitor.
            for win in app.handle().webview_windows().values() {
                ensure_on_screen(win);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Recenter a window if its center point lies on no connected monitor (e.g. a
/// saved position from an external display that's since been unplugged).
fn ensure_on_screen(win: &tauri::WebviewWindow) {
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else {
        return;
    };
    let cx = pos.x + size.width as i32 / 2;
    let cy = pos.y + size.height as i32 / 2;
    let monitors = win.available_monitors().unwrap_or_default();
    if monitors.is_empty() {
        return;
    }
    let on_screen = monitors.iter().any(|m| {
        let mp = m.position();
        let ms = m.size();
        cx >= mp.x && cx < mp.x + ms.width as i32 && cy >= mp.y && cy < mp.y + ms.height as i32
    });
    if !on_screen {
        let _ = win.center();
    }
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

    fn block<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn status_shows_untracked_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        init_repo(tmp.path());
        let cwd = tmp.path().to_str().unwrap().to_string();
        write(tmp.path(), "hello.txt", "hi");

        let out = block(git::git_status(cwd)).unwrap();
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

        block(git::git_stage(cwd.clone(), "readme.md".into())).unwrap();
        block(git::git_commit(cwd.clone(), "initial commit".into(), None)).unwrap();

        let status = block(git::git_status(cwd)).unwrap();
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

        block(git::git_stage(cwd.clone(), "a.txt".into())).unwrap();
        block(git::git_commit(
            cwd.clone(),
            "feat: log round-trip".into(),
            None,
        ))
        .unwrap();

        let log = block(git::git_log(cwd.clone(), None, None, None)).unwrap();
        assert!(
            log.contains("feat: log round-trip"),
            "log should contain the commit message: {log}"
        );

        let last = block(git::git_last_message(cwd)).unwrap();
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
        block(git::git_stage(cwd.clone(), "seed.txt".into())).unwrap();
        block(git::git_commit(cwd.clone(), "seed".into(), None)).unwrap();

        block(git::git_create_branch(cwd.clone(), "feature".into())).unwrap();
        // create_branch uses `checkout -b` so we're already on it; no need to checkout again.

        let branches = block(git::git_branches(cwd)).unwrap();
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
        block(git::git_stage(cwd.clone(), "seed.txt".into())).unwrap();
        block(git::git_commit(cwd.clone(), "seed".into(), None)).unwrap();

        block(git::git_create_branch(cwd.clone(), "other".into())).unwrap();
        // Switch back to main to verify checkout works.
        block(git::git_checkout(cwd.clone(), "main".into())).unwrap();

        let branches = block(git::git_branches(cwd)).unwrap();
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
        block(git::git_stage(cwd.clone(), "file.txt".into())).unwrap();
        block(git::git_commit(cwd.clone(), "add file".into(), None)).unwrap();

        // Modify the tracked file.
        write(tmp.path(), "file.txt", "original\nchanged\n");

        let diff = block(git::git_diff(cwd, "file.txt".into(), false)).unwrap();
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
        let status = block(git::git_status(cwd.clone())).unwrap_or_default();
        assert!(
            status.is_empty(),
            "non-repo status should produce no output: {status:?}"
        );
        // git_commit explicitly returns Err on non-zero exit.
        let commit = block(git::git_commit(cwd, "msg".into(), None));
        assert!(
            commit.is_err(),
            "commit on a non-repo should fail: {commit:?}"
        );
    }
}
