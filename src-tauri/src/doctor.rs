// Connections doctor: probe each external integration so a new user sees, at a
// glance, what's installed and authenticated for their environment — and a
// one-click terminal command to fix what isn't. Read-only and cheap: each probe
// runs a quick CLI call (short timeout) through the shared PATH-aware spawner, so
// it works even from a Finder-launched app. No secrets are read or stored here.
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct Probe {
    id: String,
    label: String,
    installed: bool,
    version: String,
    detail: String,  // current context / account / login / daemon version
    ok: bool,        // green: installed AND (authed where applicable)
    note: String,    // short explanation when not ok
    fix_cmd: String, // terminal command that fixes it (empty = nothing to run)
    fix_label: String,
}

/// Run `prog args…`, capturing trimmed stdout (or stderr if stdout is empty).
/// Returns None when the binary isn't on PATH (spawn fails) — i.e. not installed.
fn run(prog: &str, args: &[&str], secs: u64) -> Option<(bool, String)> {
    let mut c = crate::shared::command(prog);
    c.args(args);
    let out = crate::shared::exec_capture(c, secs).ok()?;
    let so = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let se = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Some((out.status.success(), if so.is_empty() { se } else { so }))
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

fn probe_kubectl() -> Probe {
    let ver = run("kubectl", &["version", "--client", "-o", "json"], 5);
    let installed = ver.is_some();
    let version = ver
        .as_ref()
        .and_then(|(_, s)| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|j| j["clientVersion"]["gitVersion"].as_str().map(String::from))
        .unwrap_or_default();
    let ctx = run("kubectl", &["config", "current-context"], 4)
        .filter(|(ok, _)| *ok)
        .map(|(_, s)| first_line(&s))
        .unwrap_or_default();
    let ok = installed && !ctx.is_empty();
    Probe {
        id: "kubectl".into(),
        label: "Kubernetes".into(),
        installed,
        version,
        detail: if ctx.is_empty() {
            String::new()
        } else {
            format!("context: {ctx}")
        },
        ok,
        note: if !installed {
            "kubectl not found — install it (e.g. brew install kubernetes-cli)".into()
        } else if ctx.is_empty() {
            "no current context — point kubectl at a cluster".into()
        } else {
            String::new()
        },
        fix_cmd: if installed {
            "kubectl config get-contexts".into()
        } else {
            "brew install kubernetes-cli".into()
        },
        fix_label: if installed {
            "Choose context".into()
        } else {
            "Install".into()
        },
    }
}

fn probe_aws() -> Probe {
    let ver = run("aws", &["--version"], 5);
    let installed = ver.is_some();
    let version = ver.map(|(_, s)| first_line(&s)).unwrap_or_default();
    // sts get-caller-identity is the real auth test (valid creds / live SSO).
    let id = run(
        "aws",
        &["sts", "get-caller-identity", "--output", "json"],
        7,
    );
    let (authed, account) = match &id {
        Some((true, s)) => {
            let acct = serde_json::from_str::<serde_json::Value>(s)
                .ok()
                .and_then(|j| j["Account"].as_str().map(String::from))
                .unwrap_or_default();
            (true, acct)
        }
        _ => (false, String::new()),
    };
    let ok = installed && authed;
    Probe {
        id: "aws".into(),
        label: "AWS".into(),
        installed,
        version,
        detail: if account.is_empty() {
            String::new()
        } else {
            format!("account: {account}")
        },
        ok,
        note: if !installed {
            "aws CLI not found — brew install awscli".into()
        } else if !authed {
            "no valid credentials — log in to your SSO/profile".into()
        } else {
            String::new()
        },
        fix_cmd: if installed {
            "aws sso login".into()
        } else {
            "brew install awscli".into()
        },
        fix_label: if installed {
            "aws sso login".into()
        } else {
            "Install".into()
        },
    }
}

fn probe_gh() -> Probe {
    let installed = run("gh", &["--version"], 5).is_some();
    let status = run("gh", &["auth", "status"], 6);
    let authed = matches!(&status, Some((true, _)));
    let login = status
        .as_ref()
        .map(|(_, s)| s.clone())
        .unwrap_or_default()
        .lines()
        .find(|l| l.contains("Logged in"))
        .map(|l| l.trim().to_string())
        .unwrap_or_default();
    Probe {
        id: "gh".into(),
        label: "GitHub".into(),
        installed,
        version: String::new(),
        detail: login,
        ok: installed && authed,
        note: if !installed {
            "gh CLI not found — brew install gh".into()
        } else if !authed {
            "not logged in".into()
        } else {
            String::new()
        },
        fix_cmd: if installed {
            "gh auth login".into()
        } else {
            "brew install gh".into()
        },
        fix_label: if installed {
            "gh auth login".into()
        } else {
            "Install".into()
        },
    }
}

fn probe_glab() -> Probe {
    let installed = run("glab", &["--version"], 5).is_some();
    let authed = matches!(run("glab", &["auth", "status"], 6), Some((true, _)));
    Probe {
        id: "glab".into(),
        label: "GitLab".into(),
        installed,
        version: String::new(),
        detail: String::new(),
        ok: installed && authed,
        note: if !installed {
            "glab CLI not found — brew install glab".into()
        } else if !authed {
            "not logged in".into()
        } else {
            String::new()
        },
        fix_cmd: if installed {
            "glab auth login".into()
        } else {
            "brew install glab".into()
        },
        fix_label: if installed {
            "glab auth login".into()
        } else {
            "Install".into()
        },
    }
}

/// Installed-only probe for a tool whose auth flows through kube/cloud creds.
fn probe_tool(id: &str, label: &str, prog: &str, args: &[&str], brew: &str) -> Probe {
    let res = run(prog, args, 5);
    let installed = res.is_some();
    Probe {
        id: id.into(),
        label: label.into(),
        installed,
        version: res.map(|(_, s)| first_line(&s)).unwrap_or_default(),
        detail: String::new(),
        ok: installed,
        note: if installed {
            String::new()
        } else {
            format!("{prog} not found — {brew}")
        },
        fix_cmd: if installed {
            String::new()
        } else {
            brew.into()
        },
        fix_label: if installed {
            String::new()
        } else {
            "Install".into()
        },
    }
}

/// Presence-only probe for a language server. LSP servers are stdio programs
/// that would hang waiting on stdin if run directly, so we resolve the binary
/// with `command -v` (fast, no spawn of the server itself) and offer its install
/// command as the one-click fix.
fn probe_server(id: &str, label: &str, bin: &str, install: &str) -> Probe {
    let present = run("sh", &["-lc", &format!("command -v {bin}")], 4).is_some_and(|(ok, _)| ok);
    Probe {
        id: id.into(),
        label: label.into(),
        installed: present,
        version: String::new(),
        detail: if present {
            "installed".into()
        } else {
            String::new()
        },
        ok: present,
        note: if present {
            String::new()
        } else {
            format!("{bin} not found")
        },
        fix_cmd: if present {
            String::new()
        } else {
            install.into()
        },
        fix_label: if present {
            String::new()
        } else {
            "Install".into()
        },
    }
}

fn probe_docker() -> Probe {
    // Server version succeeds only when the daemon is running — the real signal.
    let server = run("docker", &["version", "--format", "{{.Server.Version}}"], 5);
    let client = run("docker", &["--version"], 5);
    let installed = client.is_some();
    let (up, ver) = match &server {
        Some((true, s)) if !s.trim().is_empty() => (true, first_line(s)),
        _ => (false, String::new()),
    };
    Probe {
        id: "docker".into(),
        label: "Docker".into(),
        installed,
        version: if ver.is_empty() {
            String::new()
        } else {
            format!("server {ver}")
        },
        detail: if up {
            "daemon running".into()
        } else {
            String::new()
        },
        ok: installed && up,
        note: if !installed {
            "docker not found — install Docker Desktop".into()
        } else if !up {
            "daemon not running — start Docker Desktop".into()
        } else {
            String::new()
        },
        fix_cmd: if installed {
            "open -a Docker".into()
        } else {
            String::new()
        },
        fix_label: if installed {
            "Start Docker".into()
        } else {
            String::new()
        },
    }
}

/// Probe every integration in parallel (each is an independent subprocess), so
/// the whole sweep takes the slowest single probe rather than the sum.
#[tauri::command]
pub async fn doctor_check() -> Result<Vec<Probe>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let builders: Vec<fn() -> Probe> = vec![
            probe_kubectl,
            probe_aws,
            probe_gh,
            probe_glab,
            || {
                probe_tool(
                    "flux",
                    "Flux",
                    "flux",
                    &["--version"],
                    "brew install fluxcd/tap/flux",
                )
            },
            || {
                probe_tool(
                    "helm",
                    "Helm",
                    "helm",
                    &["version", "--short"],
                    "brew install helm",
                )
            },
            || {
                probe_tool(
                    "terraform",
                    "Terraform",
                    "terraform",
                    &["-version"],
                    "brew install terraform",
                )
            },
            probe_docker,
            // Language servers (editor LSP). Presence-checked; one-click install.
            || {
                probe_server(
                    "lsp-go",
                    "LSP · Go (gopls)",
                    "gopls",
                    "go install golang.org/x/tools/gopls@latest",
                )
            },
            || {
                probe_server(
                    "lsp-python",
                    "LSP · Python (pyright)",
                    "pyright-langserver",
                    "npm i -g pyright",
                )
            },
            || {
                probe_server(
                    "lsp-ts",
                    "LSP · TypeScript",
                    "typescript-language-server",
                    "npm i -g typescript typescript-language-server",
                )
            },
            || {
                probe_server(
                    "lsp-rust",
                    "LSP · Rust (rust-analyzer)",
                    "rust-analyzer",
                    "rustup component add rust-analyzer",
                )
            },
            || {
                probe_server(
                    "lsp-terraform",
                    "LSP · Terraform (terraform-ls)",
                    "terraform-ls",
                    "brew install hashicorp/tap/terraform-ls",
                )
            },
            || {
                probe_server(
                    "lsp-yaml",
                    "LSP · YAML",
                    "yaml-language-server",
                    "npm i -g yaml-language-server",
                )
            },
            || {
                probe_server(
                    "lsp-json",
                    "LSP · JSON",
                    "vscode-json-language-server",
                    "npm i -g vscode-langservers-extracted",
                )
            },
            || {
                probe_server(
                    "lsp-bash",
                    "LSP · Shell (bash)",
                    "bash-language-server",
                    "npm i -g bash-language-server",
                )
            },
            || {
                probe_server(
                    "lsp-docker",
                    "LSP · Dockerfile",
                    "docker-langserver",
                    "npm i -g dockerfile-language-server-nodejs",
                )
            },
            || {
                probe_server(
                    "lsp-lua",
                    "LSP · Lua",
                    "lua-language-server",
                    "brew install lua-language-server",
                )
            },
        ];
        let handles: Vec<_> = builders.into_iter().map(std::thread::spawn).collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| Probe {
                    id: "error".into(),
                    label: "—".into(),
                    installed: false,
                    version: String::new(),
                    detail: String::new(),
                    ok: false,
                    note: "probe failed".into(),
                    fix_cmd: String::new(),
                    fix_label: String::new(),
                })
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}
