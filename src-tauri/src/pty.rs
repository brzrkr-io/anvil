use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Emitter, State};

pub struct Pty {
    pub writer: Box<dyn Write + Send>,
    pub master: Box<dyn MasterPty + Send>,
}

/// Registry of live PTYs keyed by a frontend-chosen session id, so each
/// terminal tab/split owns an independent shell.
#[derive(Default)]
pub struct PtyState(pub Mutex<HashMap<String, Pty>>);

// #78 Background terminals whose coalescer should throttle (wider flush window).
static PTY_INACTIVE: std::sync::OnceLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();
fn pty_inactive() -> &'static Mutex<std::collections::HashSet<String>> {
    PTY_INACTIVE.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// #78 Mark a terminal active/inactive so its PTY reader can throttle when it's
/// off-screen. The coalescer re-reads this per burst.
#[tauri::command]
pub fn pty_set_active(id: String, active: bool) {
    let mut set = pty_inactive().lock().unwrap();
    if active {
        set.remove(&id);
    } else {
        set.insert(id);
    }
}

#[derive(Clone, serde::Serialize)]
struct PtyExit {
    id: String,
}

/// Spawn a login shell in a new PTY under `id`. Output streams to the webview
/// as raw bytes over the per-terminal `on_data` channel (no base64); process
/// exit is signalled via the `pty://exit` event tagged with the same id.
#[tauri::command]
pub fn pty_spawn(
    app: tauri::AppHandle,
    state: State<PtyState>,
    id: String,
    cols: u16,
    rows: u16,
    cwd: Option<String>,
    shell: Option<String>,
    on_data: Channel<InvokeResponseBody>,
) -> Result<(), String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let shell = shell
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()));
    let mut cmd = CommandBuilder::new(shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("ANVIL", "1");
    let dir = cwd
        .filter(|d| !d.is_empty())
        .or_else(|| std::env::var("HOME").ok());
    if let Some(d) = dir {
        cmd.cwd(d);
    }

    let _child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    let rid = id.clone();
    // Reader thread: blocking 64 KiB reads → channel. A second coalescer thread
    // batches bursts over a ~4ms window so a flood of back-to-back reads becomes
    // far fewer IPC messages (#44). Interactive echo flushes on the idle timeout,
    // adding at most ~4ms latency (well under one frame).
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF: closing tx signals the coalescer to exit.
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    std::thread::spawn(move || {
        // Active terminals flush every 4 ms for snappy echo; backgrounded ones
        // coalesce over a much wider window to cut CPU/IPC for off-screen floods (#78).
        const WINDOW_ACTIVE: std::time::Duration = std::time::Duration::from_millis(4);
        const WINDOW_BG: std::time::Duration = std::time::Duration::from_millis(200);
        const FLUSH_CAP: usize = 262_144; // 256 KiB: bound latency/memory under flood.
        loop {
            // Block for the first chunk of a burst.
            let mut pending = match rx.recv() {
                Ok(c) => c,
                Err(_) => break, // reader gone → child exited.
            };
            let window = if pty_inactive().lock().unwrap().contains(&rid) {
                WINDOW_BG
            } else {
                WINDOW_ACTIVE
            };
            let deadline = std::time::Instant::now() + window;
            loop {
                if pending.len() >= FLUSH_CAP {
                    break;
                }
                let wait = deadline.saturating_duration_since(std::time::Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(c) => pending.extend_from_slice(&c),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        let _ = on_data.send(InvokeResponseBody::Raw(pending));
                        let _ = app.emit("pty://exit", PtyExit { id: rid.clone() });
                        return;
                    }
                }
            }
            let _ = on_data.send(InvokeResponseBody::Raw(pending));
        }
        let _ = app.emit("pty://exit", PtyExit { id: rid.clone() });
    });

    state.0.lock().unwrap().insert(
        id,
        Pty {
            writer,
            master: pair.master,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn pty_write(state: State<PtyState>, id: String, data: String) -> Result<(), String> {
    if let Some(p) = state.0.lock().unwrap().get_mut(&id) {
        p.writer
            .write_all(data.as_bytes())
            .map_err(|e| e.to_string())?;
        let _ = p.writer.flush();
    }
    Ok(())
}

#[tauri::command]
pub fn pty_resize(state: State<PtyState>, id: String, cols: u16, rows: u16) -> Result<(), String> {
    if let Some(p) = state.0.lock().unwrap().get(&id) {
        p.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Close a PTY: dropping the master sends SIGHUP to the child shell.
#[tauri::command]
pub fn pty_kill(state: State<PtyState>, id: String) {
    state.0.lock().unwrap().remove(&id);
}
