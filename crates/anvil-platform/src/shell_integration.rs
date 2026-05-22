//! Shell integration setup: write the embedded zsh/bash scripts to a runtime
//! directory and export the env vars that wire spawned shells to them.
//!
//! Port of `src/app/shell_integration.zig`. The `setup` function must be
//! called once at startup, before any PTY is spawned, so that the env vars
//! are inherited by every shell child process.

use std::env;
use std::fs;
use std::path::PathBuf;

const INTEGRATION_ZSH: &str = include_str!("../../../shell/anvil-integration.zsh");
const INTEGRATION_BASH: &str = include_str!("../../../shell/anvil-integration.bash");
const ZDOTDIR_ZSHENV: &str = include_str!("../../../shell/zdotdir-zshenv.zsh");

/// Absolute path to the `anvil-prompt` binary, resolved next to this
/// executable. Returns `None` if the executable path cannot be determined.
fn prompt_binary_path() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(dir.join("anvil-prompt"))
}

/// Resolve `~/.cache/anvil/shell`. Returns `None` when `$HOME` is unset.
fn runtime_dir() -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("anvil")
            .join("shell"),
    )
}

/// Write `content` to `dir/name`. Returns `false` on any failure (best-effort,
/// never fatal — mirrors the Zig behaviour).
fn write_file(dir: &std::path::Path, name: &str, content: &str) -> bool {
    let path = dir.join(name);
    fs::write(&path, content.as_bytes()).is_ok()
}

/// Set up shell integration. Writes the scripts and exports the wiring env
/// vars. When `enabled` is false, exports only the harmless markers and skips
/// the `ZDOTDIR` injection. Any filesystem failure is logged and degrades to
/// "no integration" — never fatal. Call once at startup, before any tab spawns.
pub fn setup(enabled: bool) {
    let dir = match runtime_dir() {
        Some(d) => d,
        None => {
            eprintln!("anvil: shell integration: $HOME unset, skipped");
            return;
        }
    };

    if fs::create_dir_all(&dir).is_err() {
        eprintln!("anvil: shell integration: mkdir failed, skipped");
        return;
    }

    let ok_zsh = write_file(&dir, "anvil-integration.zsh", INTEGRATION_ZSH);
    let ok_bash = write_file(&dir, "anvil-integration.bash", INTEGRATION_BASH);
    let ok_env = write_file(&dir, ".zshenv", ZDOTDIR_ZSHENV);
    if !(ok_zsh && ok_bash && ok_env) {
        eprintln!("anvil: shell integration: write failed, skipped");
        return;
    }

    // SAFETY: setup() is documented as single-threaded startup only. All
    // set_var/remove_var calls below happen before any PTY threads are spawned,
    // so there is no concurrent env access that could trigger data races.

    // Markers — always exported; harmless to any shell.
    unsafe { env::set_var("ANVIL", "1") };

    let bash_path = dir.join("anvil-integration.bash");
    if let Some(s) = bash_path.to_str() {
        unsafe { env::set_var("ANVIL_SHELL_INTEGRATION", s) };
    }

    if let Some(pp) = prompt_binary_path() {
        if let Some(s) = pp.to_str() {
            unsafe { env::set_var("ANVIL_PROMPT", s) };
        }
    }

    if !enabled {
        return;
    }

    // zsh auto-injection: stash the real ZDOTDIR, then point ZDOTDIR at our
    // runtime dir so our .zshenv shim is sourced first.
    if let Ok(real) = env::var("ZDOTDIR") {
        unsafe { env::set_var("ANVIL_REAL_ZDOTDIR", &real) };
    }

    let zsh_path = dir.join("anvil-integration.zsh");
    if let Some(s) = zsh_path.to_str() {
        unsafe { env::set_var("ANVIL_SHELL_INTEGRATION_ZSH", s) };
    }

    if let Some(s) = dir.to_str() {
        unsafe { env::set_var("ZDOTDIR", s) };
    }
}

// -- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Serialise all tests that mutate process env to prevent races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Helper: override HOME for the duration of a test, restore on drop.
    struct HomeGuard {
        saved: Option<String>,
    }
    impl HomeGuard {
        fn set(home: &str) -> Self {
            let saved = env::var("HOME").ok();
            // SAFETY: ENV_LOCK is held by the caller, ensuring no concurrent
            // env access from other test threads.
            unsafe { env::set_var("HOME", home) };
            HomeGuard { saved }
        }
    }
    impl Drop for HomeGuard {
        fn drop(&mut self) {
            // SAFETY: ENV_LOCK is held by the caller.
            match &self.saved {
                Some(s) => unsafe { env::set_var("HOME", s) },
                None => unsafe { env::remove_var("HOME") },
            }
        }
    }

    #[test]
    fn runtime_dir_resolves_under_home() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = HomeGuard::set("/tmp/anvil-shell-test");
        let dir = runtime_dir().expect("runtime_dir should return Some");
        assert_eq!(
            dir,
            PathBuf::from("/tmp/anvil-shell-test/.cache/anvil/shell")
        );
    }

    #[test]
    fn setup_writes_scripts_and_exports_markers() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = HomeGuard::set("/tmp/anvil-rust-shell-test");
        // Remove any stale ZDOTDIR so this test is clean.
        // SAFETY: ENV_LOCK serialises env access across test threads.
        unsafe { env::remove_var("ZDOTDIR") };

        setup(true);

        let dir = runtime_dir().unwrap();
        for name in &["anvil-integration.zsh", "anvil-integration.bash", ".zshenv"] {
            let path = dir.join(name);
            assert!(path.exists(), "{name} should exist");
            let content = fs::read(&path).unwrap();
            assert!(!content.is_empty(), "{name} should be non-empty");
        }

        assert!(env::var("ANVIL").is_ok(), "ANVIL marker should be set");
        assert!(
            env::var("ZDOTDIR").is_ok(),
            "ZDOTDIR should be injected when enabled=true"
        );
        assert!(
            env::var("ANVIL_SHELL_INTEGRATION").is_ok(),
            "ANVIL_SHELL_INTEGRATION should be set"
        );
        assert!(
            env::var("ANVIL_SHELL_INTEGRATION_ZSH").is_ok(),
            "ANVIL_SHELL_INTEGRATION_ZSH should be set"
        );
    }

    #[test]
    fn setup_preserves_a_pre_existing_zdotdir() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = HomeGuard::set("/tmp/anvil-rust-zdotdir-test");
        // SAFETY: ENV_LOCK serialises env access across test threads.
        unsafe { env::remove_var("ANVIL_REAL_ZDOTDIR") };
        unsafe { env::set_var("ZDOTDIR", "/tmp/my-zdotdir") };

        setup(true);

        let real = env::var("ANVIL_REAL_ZDOTDIR")
            .expect("ANVIL_REAL_ZDOTDIR should be set when ZDOTDIR was present");
        assert_eq!(real, "/tmp/my-zdotdir");
    }

    #[test]
    fn setup_false_does_not_export_zdotdir() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = HomeGuard::set("/tmp/anvil-rust-shell-test-2");
        // SAFETY: ENV_LOCK serialises env access across test threads.
        unsafe { env::remove_var("ZDOTDIR") };

        setup(false);

        assert!(env::var("ANVIL").is_ok(), "ANVIL marker should be set");
        assert!(
            env::var("ZDOTDIR").is_err(),
            "ZDOTDIR must not be injected when enabled=false"
        );
    }
}
