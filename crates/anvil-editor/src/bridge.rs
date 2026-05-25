//! Background worker that polls a running `nvim --listen` socket and surfaces
//! an [`EditorSnapshot`] the main thread reads each frame.
//!
//! Threading model mirrors `anvil_caldera::Poller`:
//! - Worker thread holds the [`Transport`] and owns all IO.
//! - Main thread calls [`EditorBridge::snapshot()`] — a cheap mutex clone.
//! - [`EditorBridge::kick()`] / [`EditorBridge::set_socket()`] send messages
//!   over a bounded channel.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, SyncSender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::transport::{Endpoint, Transport, TransportError};
use crate::codec::Value;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// State of the bridge's connection to nvim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Live,
    Error,
}

/// A point-in-time snapshot of editor state from a connected nvim instance.
/// All fields are zeroed / `None` when the connection is in the `Error` state.
#[derive(Debug, Clone, Default)]
pub struct EditorSnapshot {
    /// Path of the nvim Unix socket.
    pub socket_path: Option<PathBuf>,
    /// Current connection state.
    pub connection: ConnectionState,
    /// Basename of the current buffer's file name (no directory part).
    pub buffer_name: Option<String>,
    /// Cursor position (row, col), 0-indexed.
    pub cursor: Option<(usize, usize)>,
    /// Whether the current buffer has unsaved changes.
    pub modified: bool,
    /// Unix timestamp (seconds) of the last successful poll. 0 when unset.
    pub polled_at_unix: i64,
}

// ---------------------------------------------------------------------------
// Internal message type
// ---------------------------------------------------------------------------

#[allow(dead_code)] // Shutdown is reserved for future stop() API
enum Msg {
    Kick,
    Shutdown,
    SetSocket(Option<PathBuf>),
}

// ---------------------------------------------------------------------------
// EditorBridge
// ---------------------------------------------------------------------------

/// Handle to the background nvim-polling thread.
pub struct EditorBridge {
    snapshot: Arc<Mutex<EditorSnapshot>>,
    tx: SyncSender<Msg>,
}

impl EditorBridge {
    /// Spawn the `anvil-editor-bridge` background thread.
    ///
    /// `initial_socket` is `None` → bridge starts `Disconnected`.
    /// Pass `Some(path)` to immediately attempt a connection.
    pub fn spawn(initial_socket: Option<PathBuf>) -> Self {
        let snapshot = Arc::new(Mutex::new(EditorSnapshot {
            socket_path: initial_socket.clone(),
            ..EditorSnapshot::default()
        }));
        let (tx, rx) = mpsc::sync_channel::<Msg>(16);

        let snap_clone = Arc::clone(&snapshot);
        thread::Builder::new()
            .name("anvil-editor-bridge".into())
            .spawn(move || {
                worker_loop(snap_clone, rx, initial_socket);
            })
            .expect("failed to spawn anvil-editor-bridge thread");

        Self { snapshot, tx }
    }

    /// Clone the current snapshot. O(1) mutex lock + clone.
    pub fn snapshot(&self) -> EditorSnapshot {
        self.snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Update the socket path the bridge targets.
    /// Sends a best-effort message; silently drops if the channel is full.
    pub fn set_socket(&self, path: Option<PathBuf>) {
        let _ = self.tx.try_send(Msg::SetSocket(path));
    }

    /// Request an immediate poll on the next worker iteration.
    /// Silently drops if the channel is full.
    pub fn kick(&self) {
        let _ = self.tx.try_send(Msg::Kick);
    }
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const CALL_TIMEOUT: Duration = Duration::from_millis(500);

fn worker_loop(
    snap: Arc<Mutex<EditorSnapshot>>,
    rx: mpsc::Receiver<Msg>,
    initial_socket: Option<PathBuf>,
) {
    let mut socket_path: Option<PathBuf> = initial_socket;
    let mut transport: Option<Transport> = None;

    loop {
        // Drain pending messages (non-blocking), then wait up to POLL_INTERVAL.
        let msg = match rx.recv_timeout(POLL_INTERVAL) {
            Ok(m) => Some(m),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        match msg {
            Some(Msg::Shutdown) => break,
            Some(Msg::SetSocket(path)) => {
                // Drop any existing transport and reset to Disconnected so
                // the next iteration reconnects to the new path.
                transport = None;
                socket_path = path.clone();
                set_snap(&snap, |s| {
                    s.socket_path = path;
                    s.connection = ConnectionState::Disconnected;
                    zero_data(s);
                });
            }
            Some(Msg::Kick) | None => {
                // Fall through to poll logic below.
            }
        }

        // If we have no socket, stay Disconnected.
        let Some(ref path) = socket_path else {
            continue;
        };

        // If we don't have a live transport, attempt to connect.
        if transport.is_none() {
            set_snap(&snap, |s| {
                s.connection = ConnectionState::Connecting;
            });
            let ep = Endpoint { path: path.clone() };
            match Transport::connect(&ep) {
                Ok(t) => {
                    transport = Some(t);
                    set_snap(&snap, |s| {
                        s.connection = ConnectionState::Live;
                    });
                }
                Err(_) => {
                    set_snap(&snap, |s| {
                        s.connection = ConnectionState::Error;
                        zero_data(s);
                    });
                    continue;
                }
            }
        }

        // Transport is live — poll nvim.
        let t = transport.as_mut().unwrap();
        match poll_once(t) {
            Ok(data) => {
                let now_unix = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                set_snap(&snap, |s| {
                    s.connection = ConnectionState::Live;
                    s.buffer_name = data.buffer_name;
                    s.cursor = data.cursor;
                    s.modified = data.modified;
                    s.polled_at_unix = now_unix;
                });
            }
            Err(_) => {
                transport = None;
                set_snap(&snap, |s| {
                    s.connection = ConnectionState::Error;
                    zero_data(s);
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// nvim RPC helpers
// ---------------------------------------------------------------------------

struct PollData {
    buffer_name: Option<String>,
    cursor: Option<(usize, usize)>,
    modified: bool,
}

/// Execute the four nvim RPCs that make up one poll cycle.
fn poll_once(t: &mut Transport) -> Result<PollData, TransportError> {
    // 1. Get current buffer id.
    let buf_val = t.call("nvim_get_current_buf", &[], CALL_TIMEOUT)?;
    let buf_id = match buf_val {
        Value::Uint(n) => n,
        Value::Int(n) if n >= 0 => n as u64,
        _ => return Err(TransportError::BadFrame),
    };

    // 2. Get buffer file name → basename only.
    let name_val = t.call(
        "nvim_buf_get_name",
        &[Value::Uint(buf_id)],
        CALL_TIMEOUT,
    )?;
    let buffer_name = match name_val {
        Value::Str(s) if !s.is_empty() => {
            Path::new(&s)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_string())
        }
        _ => None,
    };

    // 3. Get cursor position from window 0 (current window).
    //    nvim_win_get_cursor(0) returns [row, col] (1-indexed row, 0-indexed col).
    let cursor_val = t.call(
        "nvim_win_get_cursor",
        &[Value::Uint(0)],
        CALL_TIMEOUT,
    )?;
    let cursor = parse_cursor(cursor_val);

    // 4. Get modified flag.
    let modified_val = t.call(
        "nvim_buf_get_option",
        &[Value::Uint(buf_id), Value::Str("modified".into())],
        CALL_TIMEOUT,
    )?;
    let modified = matches!(modified_val, Value::Bool(true));

    Ok(PollData { buffer_name, cursor, modified })
}

/// Parse `nvim_win_get_cursor` response `[row, col]` (row 1-indexed) into
/// a 0-indexed `(row, col)` tuple.
fn parse_cursor(v: Value) -> Option<(usize, usize)> {
    let arr = match v {
        Value::Array(a) if a.len() >= 2 => a,
        _ => return None,
    };
    let row = match &arr[0] {
        Value::Uint(n) => *n as usize,
        Value::Int(n) if *n > 0 => *n as usize,
        _ => return None,
    };
    let col = match &arr[1] {
        Value::Uint(n) => *n as usize,
        Value::Int(n) if *n >= 0 => *n as usize,
        _ => return None,
    };
    // nvim row is 1-indexed; convert to 0-indexed.
    Some((row.saturating_sub(1), col))
}

// ---------------------------------------------------------------------------
// Snapshot helpers
// ---------------------------------------------------------------------------

fn set_snap<F: FnOnce(&mut EditorSnapshot)>(snap: &Arc<Mutex<EditorSnapshot>>, f: F) {
    if let Ok(mut g) = snap.lock() {
        f(&mut g);
    }
}

/// Zero data fields (called on Error to prevent stale data being displayed).
fn zero_data(s: &mut EditorSnapshot) {
    s.buffer_name = None;
    s.cursor = None;
    s.modified = false;
    s.polled_at_unix = 0;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_bridge_default_snapshot_is_disconnected() {
        let bridge = EditorBridge::spawn(None);
        let snap = bridge.snapshot();
        assert_eq!(snap.connection, ConnectionState::Disconnected);
        assert!(snap.socket_path.is_none());
        assert!(snap.buffer_name.is_none());
        assert!(snap.cursor.is_none());
        assert!(!snap.modified);
    }

    #[test]
    fn editor_bridge_set_socket_does_not_panic() {
        let bridge = EditorBridge::spawn(None);
        // Must not panic; channel may or may not be consumed before we read.
        bridge.set_socket(Some(PathBuf::from("/tmp/nonexistent.sock")));
        bridge.kick();
        // Give the worker a tick to process.
        std::thread::sleep(Duration::from_millis(50));
        let snap = bridge.snapshot();
        // Should have tried to connect and landed on Error (no socket exists).
        assert!(
            matches!(
                snap.connection,
                ConnectionState::Disconnected
                    | ConnectionState::Connecting
                    | ConnectionState::Error
            ),
            "unexpected state: {:?}",
            snap.connection
        );
    }
}
