use crate::shared::aws_profile;

// ── Secrets via the macOS Keychain (shell `security`, no extra crate) ──
// Stored under service "anvil:<key>" so they never touch localStorage/disk in
// plaintext. The frontend keeps only a list of WHICH keys are set.
const SECRET_SERVICE_PREFIX: &str = "anvil:";

/// #61 Unified read-only secret fetch from SSM / Vault / macOS Keychain.
/// Returns the value (never persisted) so the UI can mask/reveal it.
#[tauri::command]
pub async fn secret_read(source: String, key: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn secret_set(key: String, value: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn secret_get(key: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

/// #59 In-pane AWS resource listing — formatted text per service. AWS_PROFILE-aware.
#[tauri::command]
pub async fn aws_list(service: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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

#[tauri::command]
pub async fn secret_delete(key: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let service = format!("{SECRET_SERVICE_PREFIX}{key}");
        let _ = std::process::Command::new("security")
            .args(["delete-generic-password", "-a", "anvil", "-s", &service])
            .output();
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn secret_has(key: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let service = format!("{SECRET_SERVICE_PREFIX}{key}");
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-a", "anvil", "-s", &service])
            .output()
            .map_err(|e| e.to_string())?;
        Ok(out.status.success())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn set_aws_profile(profile: String) {
    *aws_profile().lock().unwrap() = profile;
}

/// Named profiles from ~/.aws/config (#58). One per line.
#[tauri::command]
pub async fn aws_profiles() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

// GitHub token (from Accounts), passed to gh as GH_TOKEN.
static GITHUB_TOKEN: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();
fn github_token() -> &'static std::sync::Mutex<String> {
    GITHUB_TOKEN.get_or_init(|| std::sync::Mutex::new(String::new()))
}

#[tauri::command]
pub fn set_github_token(token: String) {
    *github_token().lock().unwrap() = token;
}

pub(crate) fn gh_cmd(cwd: &str) -> std::process::Command {
    let mut c = std::process::Command::new("gh");
    c.current_dir(cwd);
    let t = github_token().lock().unwrap().clone();
    if !t.is_empty() {
        c.env("GH_TOKEN", &t);
    }
    c
}
