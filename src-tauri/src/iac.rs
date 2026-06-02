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
    let mut cmd = std::process::Command::new(prog);
    cmd.current_dir(cwd).args(args);
    let out = crate::shared::exec_capture(cmd, 180).map_err(|e| {
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
// pickable stacks (TF code usually lives in subdirs, not the repo root). Each dir
// is classified by what it holds, because the right command differs per kind:
//   "terraform"  — *.tf / *.tf.json   → terraform|tofu plan/apply
//   "tg-unit"    — terragrunt.hcl      → terragrunt plan/apply (single unit)
//   "tg-stack"   — terragrunt.stack.hcl→ terragrunt stack run plan/apply
//   "tg-runall"  — root.hcl (a TG root)→ terragrunt run --all plan/apply
fn scan_iac(
    dir: &std::path::Path,
    base: &std::path::Path,
    depth: usize,
    out: &mut Vec<(String, String)>,
) {
    if depth > 6 || out.len() > 400 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let (mut has_tf, mut has_tg_unit, mut has_tg_stack, mut has_root) =
        (false, false, false, false);
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            // Skip noise: VCS, caches, vendored modules, generated stack output.
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "node_modules" | "vendor" | "target" | ".terraform" | ".terragrunt-stack"
                )
            {
                continue;
            }
            subdirs.push(path);
        } else if name == "terragrunt.stack.hcl" {
            has_tg_stack = true;
        } else if name == "terragrunt.hcl" {
            has_tg_unit = true;
        } else if name == "root.hcl" {
            has_root = true;
        } else if name.ends_with(".tf") || name.ends_with(".tf.json") {
            has_tf = true;
        }
    }
    // A dir holding terraform source AND a terragrunt.hcl is still a TG unit
    // (terragrunt wraps the local terraform); stacks win over plain units.
    let kind = if has_tg_stack {
        "tg-stack"
    } else if has_tg_unit {
        "tg-unit"
    } else if has_root {
        "tg-runall"
    } else if has_tf {
        "terraform"
    } else {
        ""
    };
    if !kind.is_empty() {
        let rel = dir
            .strip_prefix(base)
            .unwrap_or(dir)
            .to_string_lossy()
            .into_owned();
        out.push((if rel.is_empty() { ".".into() } else { rel }, kind.into()));
    }
    for sub in subdirs {
        scan_iac(&sub, base, depth + 1, out);
    }
}

/// `terraform plan` for a workspace dir (#78), no-color so the UI can colorize.
#[tauri::command]
pub async fn terraform_plan(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("terraform");
        cmd.current_dir(&cwd)
            .args(["plan", "-no-color", "-input=false"]);
        let out = crate::shared::exec_capture(cmd, 180).map_err(|e| e.to_string())?;
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
        let mut cmd = std::process::Command::new("terraform");
        cmd.current_dir(&cwd).args(["state", "list"]);
        let out = crate::shared::exec_capture(cmd, 25).map_err(|e| e.to_string())?;
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
        let mut cmd = std::process::Command::new("terraform");
        cmd.current_dir(&cwd)
            .args(["apply", "-no-color", "-input=false", "-auto-approve"]);
        let out = crate::shared::exec_capture(cmd, 180).map_err(|e| e.to_string())?;
        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(s)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Find IaC stacks under `cwd`, each classified by kind so the UI can pick the
/// right command. Returns JSON
/// `[{"path":"infra/prod","kind":"tg-unit","runall":true}, ...]`, relative to cwd.
/// `runall` = this terragrunt dir has descendant units, so `run --all` applies.
#[tauri::command]
pub async fn tf_discover(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let base = std::path::Path::new(&cwd);
        let mut out: Vec<(String, String)> = Vec::new();
        scan_iac(base, base, 0, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        // run --all only makes sense for terragrunt dirs that have descendant units.
        let runall: Vec<bool> = out
            .iter()
            .map(|(pi, ki)| {
                if ki == "terraform" || ki == "tg-stack" {
                    return false;
                }
                let prefix = if pi == "." {
                    String::new()
                } else {
                    format!("{pi}/")
                };
                out.iter().any(|(pj, _)| {
                    pj != pi && pj != "." && (prefix.is_empty() || pj.starts_with(&prefix))
                })
            })
            .collect();
        let items: Vec<String> = out
            .iter()
            .zip(runall)
            .map(|((p, k), ra)| format!("{{\"path\":{p:?},\"kind\":{k:?},\"runall\":{ra}}}"))
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
        let here = std::path::Path::new(&cwd);
        let has_tg = here.join("terragrunt.hcl").exists()
            || here.join("terragrunt.stack.hcl").exists()
            || here.join("root.hcl").exists();
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

/// `terragrunt stack output` — aggregated outputs across a stack's units. A stack
/// dir has no single root state, so the per-unit `output` above doesn't apply.
#[tauri::command]
pub async fn tg_stack_output(cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        tf_exec("terragrunt", &cwd, &["stack", "output", "--no-color"])
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
    let out = crate::shared::exec_capture(cmd, 60).map_err(|e| e.to_string())?;
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

/// `helm rollback <name> <revision> -n <ns>` — revert a release to a prior
/// revision. Mutating; the in-app confirm gates it.
#[tauri::command]
pub async fn helm_rollback(
    name: String,
    namespace: String,
    revision: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        helm(&["rollback", &name, &revision, "-n", &namespace, "--wait"])
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `helm diff revision <name> <revision> -n <ns>` — diff a past revision against
/// what's currently deployed (needs the helm-diff plugin; errors clearly if not).
#[tauri::command]
pub async fn helm_diff_revision(
    name: String,
    namespace: String,
    revision: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        helm(&[
            "diff",
            "revision",
            &name,
            &revision,
            "-n",
            &namespace,
            "--no-color",
        ])
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::scan_iac;
    use std::fs;

    // The classifier must distinguish plain Terraform, Terragrunt units, a
    // Terragrunt root (run --all), and Terragrunt stacks — the whole point of the
    // smart TF/TG page.
    #[test]
    fn classifies_iac_kinds() {
        let tmp = std::env::temp_dir().join(format!("anvil-iac-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("tf")).unwrap();
        fs::write(tmp.join("tf/main.tf"), "").unwrap();
        fs::create_dir_all(tmp.join("tg/app")).unwrap();
        fs::write(tmp.join("tg/app/terragrunt.hcl"), "").unwrap();
        fs::write(tmp.join("tg/root.hcl"), "").unwrap();
        fs::create_dir_all(tmp.join("stk")).unwrap();
        fs::write(tmp.join("stk/terragrunt.stack.hcl"), "").unwrap();

        let mut out: Vec<(String, String)> = Vec::new();
        scan_iac(&tmp, &tmp, 0, &mut out);
        let kind = |p: &str| out.iter().find(|(x, _)| x == p).map(|(_, k)| k.as_str());
        assert_eq!(kind("tf"), Some("terraform"));
        assert_eq!(kind("tg/app"), Some("tg-unit"));
        assert_eq!(kind("tg"), Some("tg-runall"));
        assert_eq!(kind("stk"), Some("tg-stack"));
        let _ = fs::remove_dir_all(&tmp);
    }
}
