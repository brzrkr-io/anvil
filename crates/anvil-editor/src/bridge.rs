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

/// LSP symbol kind — subset covering the most common cases.
/// Unknown LSP kind integers collapse to `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Interface,
    Module,
    Property,
    Constant,
    Variable,
    Other,
}

impl SymbolKind {
    /// Map an LSP `SymbolKind` integer to our subset.
    /// See LSP spec table (1-indexed): Module=2, Class=5, Method=6,
    /// Property=7, Enum=10, Interface=11, Function=12, Variable=13,
    /// Constant=14, Struct=23.
    fn from_lsp_int(n: i64) -> Self {
        match n {
            2 => SymbolKind::Module,
            5 => SymbolKind::Class,
            6 => SymbolKind::Method,
            7 => SymbolKind::Property,
            10 => SymbolKind::Enum,
            11 => SymbolKind::Interface,
            12 => SymbolKind::Function,
            13 => SymbolKind::Variable,
            14 => SymbolKind::Constant,
            23 => SymbolKind::Struct,
            _ => SymbolKind::Other,
        }
    }
}

/// A single document symbol flattened from the LSP hierarchy.
#[derive(Debug, Clone)]
pub struct OutlineSymbol {
    pub name: String,
    pub kind: SymbolKind,
    /// 0-indexed line number.
    pub line: u32,
    /// Nesting depth (0 = top-level).
    pub depth: u8,
}

/// State of the outline pull.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutlineState {
    /// No pull attempted yet (bridge just connected / idle).
    #[default]
    Idle,
    /// A pull is in flight or the last pull timed out.
    Pending,
    /// Last pull completed successfully (may be empty).
    Ready,
    /// No LSP server attached to the current buffer.
    NoServer,
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
    /// Flattened document symbols from the attached LSP client.
    pub outline: Vec<OutlineSymbol>,
    /// State of the last outline pull.
    pub outline_state: OutlineState,
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
const OUTLINE_INTERVAL_MS: u64 = 1500;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn worker_loop(
    snap: Arc<Mutex<EditorSnapshot>>,
    rx: mpsc::Receiver<Msg>,
    initial_socket: Option<PathBuf>,
) {
    let mut socket_path: Option<PathBuf> = initial_socket;
    let mut transport: Option<Transport> = None;
    // Millisecond timestamp of the last outline pull (0 = never).
    let mut last_outline_pull_ms: u64 = 0;
    // Buffer name at the time of the last outline pull. Used to detect buffer
    // switches that require an immediate re-pull (and outline clear).
    let mut last_buf_name: Option<String> = None;

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
                last_outline_pull_ms = 0;
                last_buf_name = None;
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
                let current_buf = data.buffer_name.clone();
                set_snap(&snap, |s| {
                    s.connection = ConnectionState::Live;
                    s.buffer_name = data.buffer_name;
                    s.cursor = data.cursor;
                    s.modified = data.modified;
                    s.polled_at_unix = now_unix;
                });

                // Outline debounce: fire immediately on buffer switch, otherwise
                // fire when OUTLINE_INTERVAL_MS has elapsed since the last pull.
                let buf_changed = current_buf != last_buf_name;
                let elapsed = now_ms().saturating_sub(last_outline_pull_ms);
                if buf_changed || elapsed >= OUTLINE_INTERVAL_MS {
                    if buf_changed {
                        // Clear stale outline before the new pull lands.
                        set_snap(&snap, |s| {
                            s.outline.clear();
                            s.outline_state = OutlineState::Idle;
                        });
                        last_buf_name = current_buf;
                    }
                    pull_outline(t, &snap);
                    last_outline_pull_ms = now_ms();
                }
            }
            Err(_) => {
                transport = None;
                last_outline_pull_ms = 0;
                last_buf_name = None;
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
// Outline pull
// ---------------------------------------------------------------------------

/// Lua script sent to nvim via `nvim_exec_lua`.
///
/// Returns a msgpack map with shape:
///   `{ attached = false }`                   — no LSP client
///   `{ attached = true, symbols = nil }`     — LSP timed out (1.5 s)
///   `{ attached = true, symbols = [] }`      — empty
///   `{ attached = true, symbols = [{name,kind,line,depth}, ...] }`
const OUTLINE_LUA: &str = r#"
local clients = vim.lsp.get_clients({ bufnr = 0 })
if not clients or #clients == 0 then
  return { attached = false }
end
local params = { textDocument = vim.lsp.util.make_text_document_params(0) }
local res, err = vim.lsp.buf_request_sync(0, "textDocument/documentSymbol", params, 1500)
if err ~= nil or res == nil then
  return { attached = true, symbols = vim.NIL }
end
local out = {}
local function flatten(syms, depth)
  if not syms then return end
  for _, s in ipairs(syms) do
    local r = s.range or s.selectionRange
    local line = r and r.start and r.start.line or 0
    table.insert(out, { name = s.name, kind = s.kind, line = line, depth = depth })
    if s.children then flatten(s.children, depth + 1) end
  end
end
for _, client_res in pairs(res) do
  if client_res.result then flatten(client_res.result, 0) end
end
return { attached = true, symbols = out }
"#;

/// Call `nvim_exec_lua` with the outline script and update the snapshot.
///
/// Four-state decode:
/// - `attached=false`         → `NoServer`, clear outline.
/// - `attached=true, symbols=nil` → `Pending`, keep last outline.
/// - `attached=true, symbols=[]`  → `Ready`, clear outline.
/// - `attached=true, symbols=[..]`→ `Ready`, replace outline.
/// - malformed / RPC error        → `NoServer`, clear outline.
fn pull_outline(t: &mut Transport, snap: &Arc<Mutex<EditorSnapshot>>) {
    let script = Value::Str(OUTLINE_LUA.to_string());
    let args = Value::Array(vec![]);
    let result = t.call("nvim_exec_lua", &[script, args], Duration::from_millis(2000));

    let map = match result {
        Ok(Value::Map(m)) => m,
        _ => {
            // RPC error or unexpected shape — treat as NoServer.
            set_snap(snap, |s| {
                s.outline.clear();
                s.outline_state = OutlineState::NoServer;
            });
            return;
        }
    };

    // Extract `attached` boolean.
    let attached = map_bool(&map, "attached").unwrap_or(false);
    if !attached {
        set_snap(snap, |s| {
            s.outline.clear();
            s.outline_state = OutlineState::NoServer;
        });
        return;
    }

    // Extract `symbols`.
    let symbols_val = map_get(&map, "symbols");
    match symbols_val {
        None | Some(Value::Nil) => {
            // nil → LSP timeout; retain last outline, mark Pending.
            set_snap(snap, |s| {
                s.outline_state = OutlineState::Pending;
            });
        }
        Some(Value::Array(arr)) => {
            let symbols: Vec<OutlineSymbol> = arr
                .into_iter()
                .filter_map(decode_symbol)
                .collect();
            set_snap(snap, |s| {
                s.outline = symbols;
                s.outline_state = OutlineState::Ready;
            });
        }
        _ => {
            // Unexpected type — fall back to NoServer.
            set_snap(snap, |s| {
                s.outline.clear();
                s.outline_state = OutlineState::NoServer;
            });
        }
    }
}

/// Decode a single symbol entry from the Lua return array.
fn decode_symbol(v: Value) -> Option<OutlineSymbol> {
    let map = match v {
        Value::Map(m) => m,
        _ => return None,
    };
    let name = match map_get(&map, "name") {
        Some(Value::Str(s)) => s,
        _ => return None,
    };
    let kind_int = match map_get(&map, "kind") {
        Some(Value::Uint(n)) => n as i64,
        Some(Value::Int(n)) => n,
        _ => -1,
    };
    let line = match map_get(&map, "line") {
        Some(Value::Uint(n)) => n as u32,
        Some(Value::Int(n)) if n >= 0 => n as u32,
        _ => 0,
    };
    let depth = match map_get(&map, "depth") {
        Some(Value::Uint(n)) => n.min(255) as u8,
        Some(Value::Int(n)) if n >= 0 => (n as u64).min(255) as u8,
        _ => 0,
    };
    Some(OutlineSymbol {
        name,
        kind: SymbolKind::from_lsp_int(kind_int),
        line,
        depth,
    })
}

fn map_get(map: &[(Value, Value)], key: &str) -> Option<Value> {
    for (k, v) in map {
        if let Value::Str(s) = k {
            if s == key {
                return Some(v.clone());
            }
        }
    }
    None
}

fn map_bool(map: &[(Value, Value)], key: &str) -> Option<bool> {
    match map_get(map, key) {
        Some(Value::Bool(b)) => Some(b),
        _ => None,
    }
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
    s.outline.clear();
    s.outline_state = OutlineState::Idle;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_default_idle_and_empty() {
        let snap = EditorSnapshot::default();
        assert_eq!(snap.outline_state, OutlineState::Idle);
        assert!(snap.outline.is_empty());
    }

    #[test]
    fn outline_kind_serializes_known_values() {
        assert_eq!(SymbolKind::from_lsp_int(12), SymbolKind::Function);
        assert_eq!(SymbolKind::from_lsp_int(6), SymbolKind::Method);
        assert_eq!(SymbolKind::from_lsp_int(5), SymbolKind::Class);
        assert_eq!(SymbolKind::from_lsp_int(23), SymbolKind::Struct);
        assert_eq!(SymbolKind::from_lsp_int(10), SymbolKind::Enum);
        assert_eq!(SymbolKind::from_lsp_int(11), SymbolKind::Interface);
        assert_eq!(SymbolKind::from_lsp_int(2), SymbolKind::Module);
        assert_eq!(SymbolKind::from_lsp_int(7), SymbolKind::Property);
        assert_eq!(SymbolKind::from_lsp_int(14), SymbolKind::Constant);
        assert_eq!(SymbolKind::from_lsp_int(13), SymbolKind::Variable);
        // Unknown integer → Other.
        assert_eq!(SymbolKind::from_lsp_int(99), SymbolKind::Other);
        assert_eq!(SymbolKind::from_lsp_int(-1), SymbolKind::Other);
    }

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
