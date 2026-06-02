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

/// Build a `Command` for an external CLI with the login-shell PATH injected, so a
/// freshly-downloaded app finds the user's tools regardless of how it was
/// launched. Use this instead of `Command::new` for any third-party binary.
pub(crate) fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.env("PATH", shell_path());
    c
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
