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
