use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Emitter, State};

/// Bounded FIFO of the PTY's most recent raw output. On (re)attach we replay
/// these bytes — including escape sequences — so a remounted terminal redraws
/// its screen + as much scrollback as the buffer holds (#99).
pub struct RingBuffer {
    buf: std::collections::VecDeque<u8>,
    cap: usize,
}

impl RingBuffer {
    fn new(cap: usize) -> Self {
        Self {
            buf: std::collections::VecDeque::new(),
            cap,
        }
    }

    /// Append bytes, evicting oldest data so the buffer never exceeds `cap`.
    fn push(&mut self, data: &[u8]) {
        if data.len() >= self.cap {
            // The incoming chunk alone fills/overflows the ring: keep only its tail.
            self.buf.clear();
            self.buf.extend(&data[data.len() - self.cap..]);
            return;
        }
        let overflow = (self.buf.len() + data.len()).saturating_sub(self.cap);
        for _ in 0..overflow {
            self.buf.pop_front();
        }
        self.buf.extend(data);
    }

    fn snapshot(&self) -> Vec<u8> {
        self.buf.iter().copied().collect()
    }
}

pub struct Pty {
    pub writer: Box<dyn Write + Send>,
    pub master: Box<dyn MasterPty + Send>,
    /// Current frontend sink. Swapped on re-attach so live output follows the
    /// remounted terminal; the coalescer reads this slot each flush.
    pub on_data: Arc<Mutex<Channel<InvokeResponseBody>>>,
    /// Recent raw output, replayed on (re)attach to restore the screen.
    pub ring: Arc<Mutex<RingBuffer>>,
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

// 256 KiB of recent output is replayed on re-attach. Large enough to redraw a
// full screen plus a useful slice of scrollback; bounded so memory per PTY stays
// flat regardless of how much has scrolled by.
const RING_CAP: usize = 262_144;

/// Spawn a login shell in a new PTY under `id`, OR re-attach if one is already
/// live for `id` (#99). Output streams to the webview as raw bytes over the
/// per-terminal `on_data` channel (no base64); process exit is signalled via the
/// `pty://exit` event tagged with the same id.
///
/// Returns `true` when an existing PTY was re-attached (the running shell was
/// preserved and its recent output replayed), `false` when a fresh shell was
/// spawned. Moving/re-docking a pane remounts `<Terminal>`, which calls this
/// again with the same `id`; re-attach keeps the live shell instead of killing
/// it and showing `[process exited]`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn pty_spawn(
    app: tauri::AppHandle,
    state: State<PtyState>,
    id: String,
    cols: u16,
    rows: u16,
    cwd: Option<String>,
    shell: Option<String>,
    on_data: Channel<InvokeResponseBody>,
) -> Result<bool, String> {
    // Re-attach path: a live PTY already owns this id. Rebind its sink to the new
    // (remounted) channel and replay recent output so the screen is restored. Do
    // NOT spawn — that would orphan the running shell.
    {
        let map = state.0.lock().unwrap();
        if let Some(p) = map.get(&id) {
            let replay = p.ring.lock().unwrap().snapshot();
            {
                let mut sink = p.on_data.lock().unwrap();
                *sink = on_data;
                if !replay.is_empty() {
                    let _ = sink.send(InvokeResponseBody::Raw(replay));
                }
            }
            // Keep the slave sized to the remounted terminal.
            let _ = p.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
            return Ok(true);
        }
    }

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

    // Shared sink (rebindable on re-attach) and replay ring, both owned by the
    // registry entry and read by the coalescer thread below.
    let sink = Arc::new(Mutex::new(on_data));
    let ring = Arc::new(Mutex::new(RingBuffer::new(RING_CAP)));

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

    let sink_rx = sink.clone();
    let ring_rx = ring.clone();
    std::thread::spawn(move || {
        // Active terminals flush every 4 ms for snappy echo; backgrounded ones
        // coalesce over a much wider window to cut CPU/IPC for off-screen floods (#78).
        const WINDOW_ACTIVE: std::time::Duration = std::time::Duration::from_millis(4);
        const WINDOW_BG: std::time::Duration = std::time::Duration::from_millis(200);
        const FLUSH_CAP: usize = 262_144; // 256 KiB: bound latency/memory under flood.
                                          // Record into the ring first, then emit to the CURRENT sink (a re-attach
                                          // may have swapped it). Block for the first chunk; Err = reader gone.
        let flush = |bytes: Vec<u8>| {
            ring_rx.lock().unwrap().push(&bytes);
            let _ = sink_rx.lock().unwrap().send(InvokeResponseBody::Raw(bytes));
        };
        while let Ok(mut pending) = rx.recv() {
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
                        flush(pending);
                        let _ = app.emit("pty://exit", PtyExit { id: rid.clone() });
                        return;
                    }
                }
            }
            flush(pending);
        }
        let _ = app.emit("pty://exit", PtyExit { id: rid.clone() });
    });

    state.0.lock().unwrap().insert(
        id,
        Pty {
            writer,
            master: pair.master,
            on_data: sink,
            ring,
        },
    );
    Ok(false)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_short_output_intact() {
        let mut ring = RingBuffer::new(16);
        ring.push(b"abc");
        ring.push(b"def");
        assert_eq!(ring.snapshot(), b"abcdef");
    }

    #[test]
    fn ring_evicts_oldest_when_over_cap() {
        let mut ring = RingBuffer::new(4);
        ring.push(b"ab");
        ring.push(b"cd");
        ring.push(b"ef"); // pushes "ab" out
        assert_eq!(ring.snapshot(), b"cdef");
    }

    #[test]
    fn ring_truncates_oversized_single_push_to_tail() {
        let mut ring = RingBuffer::new(4);
        ring.push(b"0123456789"); // only the last 4 bytes survive
        assert_eq!(ring.snapshot(), b"6789");
    }

    #[test]
    fn ring_never_exceeds_cap_under_flood() {
        let mut ring = RingBuffer::new(8);
        for _ in 0..100 {
            ring.push(b"xyz");
        }
        assert!(ring.snapshot().len() <= 8);
    }

    // The replay-on-reattach contract: whatever the ring holds when a terminal
    // remounts is exactly what gets re-sent to redraw the screen. This pins the
    // behavior #99 depends on — a re-attach replays recent output, not nothing.
    #[test]
    fn snapshot_is_what_reattach_replays() {
        let ring = Arc::new(Mutex::new(RingBuffer::new(64)));
        ring.lock().unwrap().push(b"\x1b[2J");
        ring.lock().unwrap().push(b"user@host:~$ ls\r\n");
        let replay = ring.lock().unwrap().snapshot();
        assert_eq!(replay, b"\x1b[2Juser@host:~$ ls\r\n");
        assert!(!replay.is_empty(), "reattach must replay buffered output");
    }
}
