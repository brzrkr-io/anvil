//! PTY layer: spawns a real shell in a pseudo-terminal and lets the caller
//! read its output, write input, and resize it. macOS/POSIX only.

use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

use nix::pty::openpty;
use nix::sys::wait::waitpid;
use nix::unistd::{ForkResult, Pid, fork};
use thiserror::Error;

/// The shell used when `$SHELL` is unset.
const DEFAULT_SHELL: &str = "/bin/zsh";

/// The terminal type advertised to the child process.
const TERM_VALUE: &str = "xterm-256color";

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("openpty failed: {0}")]
    OpenPty(#[from] nix::Error),
    #[error("fork failed")]
    Fork,
    #[error("write failed")]
    Write,
    #[error("EOF")]
    Eof,
    #[error("NUL in argument: {0}")]
    NulArg(#[from] std::ffi::NulError),
}

/// A running pseudo-terminal with a child process.
///
/// Dropping a `Pty` closes the master fd and reaps the child (equivalent to
/// the Zig `deinit`).
pub struct Pty {
    master: OwnedFd,
    child: Pid,
}

impl Pty {
    /// Spawn the program at `path` with `argv` in a new PTY sized `cols` x
    /// `rows`. `argv[0]` is the name passed to the process (may differ from
    /// `path`, e.g. `-zsh` for a login shell).
    pub fn spawn_exec(path: &str, argv: &[&str], cols: u16, rows: u16) -> Result<Self, PtyError> {
        assert!(!argv.is_empty(), "argv must not be empty");

        let winsize = libc::winsize {
            ws_col: cols,
            ws_row: rows,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let result = openpty(Some(&winsize), None)?;
        let master_fd = result.master.into_raw_fd();
        let slave_fd = result.slave.into_raw_fd();

        let path_c = CString::new(path)?;
        let argv_c: Vec<CString> = argv
            .iter()
            .map(|s| CString::new(*s))
            .collect::<Result<_, _>>()?;
        let env_c = build_child_env()?;

        // SAFETY: fork() is unsafe because in a multithreaded program only
        // async-signal-safe functions may be called in the child. We call only
        // libc login_tty, execve, and _exit — all async-signal-safe.
        let fork_result = unsafe { fork() }.map_err(|_| PtyError::Fork)?;

        if fork_result.is_child() {
            // SAFETY: we are in the child process. login_tty makes `slave_fd`
            // the controlling tty and dups it to stdin/stdout/stderr, then
            // closes it. execve replaces the process image. _exit is called on
            // any failure so we never return into the parent's memory space.
            unsafe {
                child_exec(slave_fd, &path_c, &argv_c, &env_c);
            }
        }

        // Parent path.
        let child = match fork_result {
            ForkResult::Parent { child } => child,
            ForkResult::Child => unreachable!("child called _exit"),
        };
        // SAFETY: we own slave_fd; closing it here is correct.
        unsafe { libc::close(slave_fd) };
        // Make the master non-blocking so `read` returns immediately when no
        // shell output is pending. The main run loop polls per-tick — a
        // blocking read would freeze the whole UI between bytes of output.
        // SAFETY: master_fd is a valid fd we own from openpty.
        unsafe {
            let flags = libc::fcntl(master_fd, libc::F_GETFL, 0);
            if flags >= 0 {
                libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
        // SAFETY: master_fd is a valid fd we own from openpty.
        let master = unsafe { OwnedFd::from_raw_fd(master_fd) };
        Ok(Pty { master, child })
    }

    /// Spawn `argv[0]` as both the executable and the first argument.
    pub fn spawn(argv: &[&str], cols: u16, rows: u16) -> Result<Self, PtyError> {
        assert!(!argv.is_empty());
        Self::spawn_exec(argv[0], argv, cols, rows)
    }

    /// Spawn the user's login shell (`$SHELL`, fallback `/bin/zsh`). A login
    /// shell is signalled by a `-` prefix on `argv[0]`, e.g. `-zsh`.
    pub fn spawn_shell(cols: u16, rows: u16) -> Result<Self, PtyError> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| DEFAULT_SHELL.to_owned());
        let basename = shell
            .rsplit('/')
            .next()
            .unwrap_or(shell.as_str())
            .to_owned();
        let arg0 = format!("-{basename}");
        Self::spawn_exec(&shell, &[arg0.as_str()], cols, rows)
    }

    /// Read available output from the child. Non-blocking (the master fd has
    /// `O_NONBLOCK` set in `spawn_exec`). Returns `Ok(0)` when no data is
    /// currently available (EAGAIN/EWOULDBLOCK); returns `PtyError::Eof` when
    /// the child has exited (macOS delivers EIO on a read from a master whose
    /// slave has been closed).
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, PtyError> {
        let fd = self.master.as_raw_fd();
        // SAFETY: fd is valid and buf is a valid writable slice.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if errno == libc::EAGAIN || errno == libc::EWOULDBLOCK {
                return Ok(0); // no data available right now
            }
            // EIO and any other read error: treat as EOF.
            return Err(PtyError::Eof);
        }
        if n == 0 {
            return Err(PtyError::Eof);
        }
        Ok(n as usize)
    }

    /// Write input bytes to the child.
    pub fn write(&self, bytes: &[u8]) -> Result<usize, PtyError> {
        let fd = self.master.as_raw_fd();
        // SAFETY: fd is valid and bytes is a valid readable slice.
        let n = unsafe { libc::write(fd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
        if n < 0 {
            return Err(PtyError::Write);
        }
        Ok(n as usize)
    }

    /// Return the child process PID. Used by the panic hook to clean up PTYs.
    pub fn child_pid(&self) -> libc::pid_t {
        self.child.as_raw()
    }

    /// Resize the PTY (TIOCSWINSZ) so the child sees SIGWINCH.
    pub fn resize(&self, cols: u16, rows: u16) {
        let ws = libc::winsize {
            ws_col: cols,
            ws_row: rows,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: fd is valid; TIOCSWINSZ expects a *const winsize.
        unsafe {
            libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws);
        }
    }
}

impl Drop for Pty {
    /// Send SIGHUP to the child, close the master fd, and reap the zombie.
    /// Equivalent to the Zig `deinit`.
    fn drop(&mut self) {
        // SAFETY: SIGHUP is a valid signal; child is a valid pid.
        unsafe { libc::kill(self.child.as_raw(), libc::SIGHUP) };
        // master is closed by OwnedFd drop.
        let _ = waitpid(self.child, None);
    }
}

/// Runs only in the forked child. Makes the slave the controlling tty,
/// then replaces the process image with execve. Never returns on success;
/// calls _exit on any failure.
///
/// # Safety
/// Must only be called in a freshly-forked child process. All functions
/// called here are async-signal-safe.
unsafe fn child_exec(slave_fd: RawFd, path: &CStr, argv: &[CString], envp: &[CString]) -> ! {
    // login_tty: sets the slave as the controlling tty, dups it to 0/1/2,
    // then closes it. This is the BSD/macOS equivalent of a manual
    // setsid+dup2+close sequence.
    // SAFETY: slave_fd is a valid tty fd in the child.
    if unsafe { libc::login_tty(slave_fd) } != 0 {
        unsafe { libc::_exit(127) };
    }

    // Build null-terminated pointer arrays for execve.
    let argv_ptrs: Vec<*const libc::c_char> = argv
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let env_ptrs: Vec<*const libc::c_char> = envp
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    // SAFETY: path, argv_ptrs, and env_ptrs are all valid C strings/arrays.
    unsafe {
        libc::execve(path.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr());
    }
    // execve only returns on failure.
    unsafe { libc::_exit(127) };
}

/// Copy the parent environment, replacing/adding `TERM=xterm-256color`.
fn build_child_env() -> Result<Vec<CString>, PtyError> {
    let mut env: Vec<CString> = std::env::vars_os()
        .filter(|(k, _)| k != "TERM")
        .map(|(k, v)| {
            let mut pair = k.into_encoded_bytes();
            pair.push(b'=');
            pair.extend(v.into_encoded_bytes());
            CString::new(pair)
        })
        .collect::<Result<_, _>>()?;
    env.push(CString::new(format!("TERM={TERM_VALUE}"))?);
    Ok(env)
}

// -- helper ------------------------------------------------------------------

/// Drain a pty into `out` until EOF or a 2s timeout, capped at `out.len`.
/// Under non-blocking reads `Ok(0)` means "no data right now"; the helper
/// briefly sleeps and retries until real EOF or the deadline elapses.
#[cfg(test)]
fn drain_to_eof(pty: &Pty, out: &mut [u8]) -> usize {
    use std::thread;
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut total = 0;
    while total < out.len() {
        match pty.read(&mut out[total..]) {
            Err(_) => break, // real EOF
            Ok(0) => {
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Ok(n) => total += n,
        }
    }
    total
}

// -- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_echo_and_read_its_output() {
        let pty = Pty::spawn(&["/bin/echo", "hello"], 80, 24).unwrap();
        let mut buf = [0u8; 256];
        let total = drain_to_eof(&pty, &mut buf);
        assert!(
            buf[..total].windows(5).any(|w| w == b"hello"),
            "expected 'hello' in output"
        );
    }

    #[test]
    fn write_input_is_echoed_back_through_the_pty() {
        let pty = Pty::spawn(&["/bin/cat"], 80, 24).unwrap();
        pty.write(b"ping\n").unwrap();

        let mut buf = [0u8; 256];
        let mut total = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while total < buf.len() {
            match pty.read(&mut buf[total..]) {
                Err(_) => break,
                Ok(0) => {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(n) => {
                    total += n;
                    if buf[..total].windows(4).any(|w| w == b"ping") {
                        break;
                    }
                }
            }
        }
        assert!(
            buf[..total].windows(4).any(|w| w == b"ping"),
            "expected 'ping' in echoed output"
        );
    }

    #[test]
    fn resize_does_not_crash() {
        let pty = Pty::spawn(&["/bin/cat"], 80, 24).unwrap();
        pty.resize(120, 40);
        pty.resize(80, 24);
    }

    #[test]
    fn spawn_shell_starts_an_interactive_login_shell() {
        let pty = Pty::spawn_shell(80, 24).unwrap();
        pty.write(b"printf ANVIL_PTY_OK\n").unwrap();
        pty.write(b"exit\n").unwrap();

        let mut buf = [0u8; 16384];
        let total = drain_to_eof(&pty, &mut buf);
        assert!(
            buf[..total].windows(12).any(|w| w == b"ANVIL_PTY_OK"),
            "expected 'ANVIL_PTY_OK' in shell output"
        );
    }

    #[test]
    fn lookup_env_finds_present_and_misses_absent() {
        assert!(
            std::env::var("PATH").is_ok(),
            "PATH should be set in test env"
        );
        assert!(
            std::env::var("ANVIL_DEFINITELY_UNSET_VAR_XYZ").is_err(),
            "phantom var should be absent"
        );
    }

    #[test]
    fn child_environment_advertises_xterm_256color() {
        let pty = Pty::spawn(&["/bin/sh", "-c", "printf %s \"$TERM\""], 80, 24).unwrap();
        let mut buf = [0u8; 256];
        let total = drain_to_eof(&pty, &mut buf);
        assert!(
            buf[..total]
                .windows(TERM_VALUE.len())
                .any(|w| w == TERM_VALUE.as_bytes()),
            "expected TERM=xterm-256color in child env"
        );
    }
}
