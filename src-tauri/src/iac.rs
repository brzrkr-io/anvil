use crate::shared::aws_profile;

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

// Walk a repo for directories that contain IaC code so the UI can offer them as
// pickable stacks (TF code usually lives in subdirs, not the repo root).
fn scan_iac(
    dir: &std::path::Path,
    base: &std::path::Path,
    depth: usize,
    out: &mut Vec<(String, bool)>,
) {
    if depth > 6 || out.len() > 400 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut has_tf = false;
    let mut has_tg = false;
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            // Skip noise: VCS, caches, vendored modules, hidden dirs.
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "node_modules" | "vendor" | "target" | ".terraform"
                )
            {
                continue;
            }
            subdirs.push(path);
        } else if name == "terragrunt.hcl" {
            has_tg = true;
        } else if name.ends_with(".tf") || name.ends_with(".tf.json") {
            has_tf = true;
        }
    }
    if has_tf || has_tg {
        let rel = dir
            .strip_prefix(base)
            .unwrap_or(dir)
            .to_string_lossy()
            .into_owned();
        out.push((if rel.is_empty() { ".".into() } else { rel }, has_tg));
    }
    for sub in subdirs {
        scan_iac(&sub, base, depth + 1, out);
    }
}

/// `terraform plan` for a workspace dir (#78), no-color so the UI can colorize.
#[tauri::command]
pub async fn terraform_plan(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let out = std::process::Command::new("terraform")
            .current_dir(&cwd)
            .args(["plan", "-no-color", "-input=false"])
            .output()
            .map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `terraform state list` (#52) — managed resources in the current state.
#[tauri::command]
pub async fn terraform_state(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let out = std::process::Command::new("terraform")
            .current_dir(&cwd)
            .args(["state", "list"])
            .output()
            .map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `terraform apply -auto-approve` (#52). The approval gate is the in-app
/// confirm before this is invoked — never call it without explicit user consent.
#[tauri::command]
pub async fn terraform_apply(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let out = std::process::Command::new("terraform")
            .current_dir(&cwd)
            .args(["apply", "-no-color", "-input=false", "-auto-approve"])
            .output()
            .map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Find IaC stacks under `cwd` (#dirs holding *.tf or terragrunt.hcl). Returns
/// JSON `[{"path":"infra/prod","terragrunt":true}, ...]`, relative to cwd.
#[tauri::command]
pub async fn tf_discover(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base = std::path::Path::new(&cwd);
        let mut out: Vec<(String, bool)> = Vec::new();
        scan_iac(base, base, 0, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        let items: Vec<String> = out
            .iter()
            .map(|(p, tg)| format!("{{\"path\":{:?},\"terragrunt\":{}}}", p, tg))
            .collect();
        Ok(format!("[{}]", items.join(",")))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Detect which IaC tooling fits this dir: presence of terragrunt.hcl picks
/// terragrunt, otherwise terraform. Also reports which binaries are on PATH.
#[tauri::command]
pub async fn tf_detect(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `<bin> init -input=false -no-color` — downloads providers / modules.
#[tauri::command]
pub async fn tf_init(cwd: String, bin: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        tf_exec(&bin, &cwd, &["init", "-input=false", "-no-color"])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `<bin> validate -no-color` — config validity, no remote state needed.
#[tauri::command]
pub async fn tf_validate(cwd: String, bin: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || tf_exec(&bin, &cwd, &["validate", "-no-color"]))
        .await
        .map_err(|e| e.to_string())?
}

/// `<bin> plan -no-color -input=false` — preview changes, never mutates infra.
#[tauri::command]
pub async fn tf_plan(cwd: String, bin: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        tf_exec(&bin, &cwd, &["plan", "-no-color", "-input=false"])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `<bin> state list` — managed resource addresses in current state.
#[tauri::command]
pub async fn tf_state_list(cwd: String, bin: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || tf_exec(&bin, &cwd, &["state", "list"]))
        .await
        .map_err(|e| e.to_string())?
}

/// `<bin> output -json` — current root output values.
#[tauri::command]
pub async fn tf_output(cwd: String, bin: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        tf_exec(&bin, &cwd, &["output", "-json", "-no-color"])
    })
    .await
    .map_err(|e| e.to_string())?
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
pub async fn helm_list() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| helm(&["list", "-A", "-o", "json"]))
        .await
        .map_err(|e| e.to_string())?
}

/// Resolved values for one release (#51).
#[tauri::command]
pub async fn helm_values(name: String, namespace: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || helm(&["get", "values", &name, "-n", &namespace]))
        .await
        .map_err(|e| e.to_string())?
}

/// #51 All computed values incl. chart defaults (`helm get values -a`), so the
/// UI can show user overrides vs the full merged set (defaults).
#[tauri::command]
pub async fn helm_values_all(name: String, namespace: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        helm(&["get", "values", &name, "-n", &namespace, "-a"])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Revision history for one release as JSON.
#[tauri::command]
pub async fn helm_history(name: String, namespace: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        helm(&["history", &name, "-n", &namespace, "-o", "json"])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `helm status` summary (notes + resources) for one release.
#[tauri::command]
pub async fn helm_status(name: String, namespace: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || helm(&["status", &name, "-n", &namespace]))
        .await
        .map_err(|e| e.to_string())?
}

/// Rendered Kubernetes manifest for one release.
#[tauri::command]
pub async fn helm_manifest(name: String, namespace: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        helm(&["get", "manifest", &name, "-n", &namespace])
    })
    .await
    .map_err(|e| e.to_string())?
}
