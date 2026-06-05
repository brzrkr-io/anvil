use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// Selected AWS named profile (from Accounts), applied as AWS_PROFILE to kubectl
// so EKS auth uses the right credentials.
static AWS_PROFILE: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new();
pub(crate) fn aws_profile() -> &'static Mutex<String> {
    AWS_PROFILE.get_or_init(|| Mutex::new(String::new()))
}

/// Resolve the user's real login-shell PATH once. A GUI app launched from
/// Finder/Dock inherits a stripped PATH (`/usr/bin:/bin`), so Homebrew / Nix /
/// cargo tools (kubectl, aws, glab, flux, helm, terraform, docker, …) are
/// invisible and every integration fails with "not found in PATH". Ask the login
/// shell for its PATH and union the common locations as a fallback.
pub(crate) fn shell_path() -> &'static str {
    static PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let mut dirs: Vec<String> = Vec::new();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        if let Ok(out) = Command::new(&shell)
            .args(["-lic", "printf %s \"$PATH\""])
            .output()
        {
            let p = String::from_utf8_lossy(&out.stdout);
            for d in p.trim().split(':').filter(|s| !s.is_empty()) {
                dirs.push(d.to_string());
            }
        }
        if let Ok(cur) = std::env::var("PATH") {
            for d in cur.split(':').filter(|s| !s.is_empty()) {
                if !dirs.iter().any(|x| x == d) {
                    dirs.push(d.to_string());
                }
            }
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let user = std::env::var("USER").unwrap_or_default();
        for d in [
            format!("{home}/.cargo/bin"),
            format!("{home}/go/bin"),
            format!("{home}/.local/bin"),
            "/opt/homebrew/bin".into(),
            "/usr/local/bin".into(),
            format!("/etc/profiles/per-user/{user}/bin"),
            "/run/current-system/sw/bin".into(),
        ] {
            if !d.contains("//") && !dirs.contains(&d) {
                dirs.push(d);
            }
        }
        dirs.join(":")
    })
}

/// Resolve the login shell's environment once. A Finder/Dock-launched GUI app
/// gets a minimal env, so rc-defined vars the CLIs rely on (KUBECONFIG,
/// `AWS_REGION`, `DOCKER_HOST`, `HTTPS_PROXY`, …) are missing. Capture `env` from a
/// login shell and keep every var EXCEPT PATH (owned by `shell_path`) and a few
/// volatile ones that shouldn't be carried into child processes.
fn shell_env() -> &'static Vec<(String, String)> {
    static ENV: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    ENV.get_or_init(|| {
        let mut vars = Vec::new();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        if let Ok(out) = Command::new(&shell).args(["-lic", "env"]).output() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Some((k, v)) = line.split_once('=') {
                    if k.is_empty() || matches!(k, "PATH" | "PWD" | "OLDPWD" | "SHLVL" | "_") {
                        continue;
                    }
                    vars.push((k.to_string(), v.to_string()));
                }
            }
        }
        vars
    })
}

/// Build a `Command` for an external CLI with the login-shell PATH + environment
/// injected, so a freshly-downloaded app finds the user's tools AND their
/// rc-defined config (KUBECONFIG, regions, proxies) regardless of how it was
/// launched. rc vars are only filled when not already present in the process
/// env, so an explicitly-set var (or a later caller `.env`) always wins.
pub(crate) fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    for (k, v) in shell_env() {
        if std::env::var_os(k).is_none() {
            c.env(k, v);
        }
    }
    c.env("PATH", shell_path());
    c
}

/// Promote the login-shell PATH + rc env into THIS process, so tools spawned via
/// the process environment — e.g. kube-rs's EKS exec plugin (`aws eks get-token`)
/// — resolve like the user's terminal instead of a Finder-stripped PATH. Call
/// once at startup. Existing process vars win (never clobbered).
pub(crate) fn promote_login_env() {
    std::env::set_var("PATH", shell_path());
    for (k, v) in shell_env() {
        if std::env::var_os(k).is_none() {
            std::env::set_var(k, v);
        }
    }
}

/// Shared pooled HTTP client. Built once so TCP/TLS connections are kept alive
/// and reused across every integration request (Prometheus, LLM, Sentry, …)
/// instead of a cold handshake per call. `.no_proxy()` matches the rest of the
/// app — direct egress, since the corporate proxy breaks these connections from
/// inside the GUI. Per-request timeouts are applied by callers (they vary: short
/// for a metrics query, none for a streaming LLM response).
pub(crate) fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .no_proxy()
            .user_agent("Anvil")
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .expect("build shared reqwest client")
    })
}

/// Run a command capturing stdout+stderr, killing it after `secs` so a hung
/// external CLI (unreachable cluster, VPN down, auth prompt) can't block forever.
/// Reader threads drain the pipes so a chatty child can't deadlock on a full pipe.
pub(crate) fn exec_capture(mut cmd: Command, secs: u64) -> std::io::Result<Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let to = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.read_to_end(&mut b);
        b
    });
    let te = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = se.read_to_end(&mut b);
        b
    });
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(status) = child.try_wait()? {
            let stdout = to.join().unwrap_or_default();
            let stderr = te.join().unwrap_or_default();
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("command timed out after {secs}s"),
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
