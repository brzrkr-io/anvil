//! LSP client core — NE9.
//!
//! `LspManager` owns a dedicated Tokio runtime and a map of running language
//! servers.  Each server is driven by a background task that speaks JSON-RPC
//! (LSP framing) over the server process's stdin/stdout.
//!
//! No UI is produced here.  Diagnostics are stored in a per-path map and
//! queried by the render layer (NE10).
//!
//! ## Transport
//!
//! We use `tokio::process::Child` + direct JSON-RPC framing instead of
//! `async-lsp` because async-lsp's main loop uses `futures::io` (AsyncRead),
//! not `tokio::io`, making the tokio feature a shim layer.  Direct framing is
//! ~120 lines and gives us full control.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lsp_types::notification::Notification as _;
use lsp_types::request::Request as _;
use lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, InitializeParams, InitializedParams, PublishDiagnosticsParams,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem, Url,
    VersionedTextDocumentIdentifier,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::mpsc;

// ── Public surface ────────────────────────────────────────────────────────────

/// Severity level for a published diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// One diagnostic entry for a path.
#[derive(Debug, Clone)]
pub struct DocumentDiagnostic {
    pub line: usize,
    pub col: usize,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

/// State of a language server connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspState {
    /// No server started (no `did_open` received yet for this server id).
    Down,
    /// Process is being spawned / initialize is in flight.
    Spawning,
    /// `initialize` response received; server is ready for requests.
    Live,
    /// Server could not start or crashed; message gives the reason.
    Failed(String),
}

// ── Commands sent to a server task ───────────────────────────────────────────

#[allow(dead_code)] // Shutdown is reserved; kill_on_drop handles app-exit cleanup
enum LspCommand {
    DidOpen {
        path: PathBuf,
        language_id: &'static str,
        version: i32,
        text: String,
    },
    DidChange {
        path: PathBuf,
        version: i32,
        text: String,
    },
    DidClose {
        path: PathBuf,
    },
    Hover {
        path: PathBuf,
        line: u32,
        character: u32,
        request_id: u64,
    },
    Shutdown,
}

// ── Hover result (NE10) ───────────────────────────────────────────────────────

/// The result of a hover request, ready to be shown in the UI.
#[derive(Debug, Clone)]
pub struct HoverResult {
    /// Markdown or plain text from the LSP hover response.
    pub text: String,
}

// ── ServerHandle ─────────────────────────────────────────────────────────────

struct ServerHandle {
    state: Arc<Mutex<LspState>>,
    tx: mpsc::Sender<LspCommand>,
    diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<DocumentDiagnostic>>>>,
    /// Latest hover result (request_id, HoverResult). `main.rs` polls this
    /// each frame and clears after consumption.
    hover_result: Arc<Mutex<Option<(u64, HoverResult)>>>,
}

// ── LspManager ────────────────────────────────────────────────────────────────

/// Owns the Tokio runtime and all language-server connections.
///
/// Constructed once on `App::new`; lives until the process exits.
pub struct LspManager {
    runtime: tokio::runtime::Runtime,
    servers: HashMap<&'static str, ServerHandle>,
}

impl LspManager {
    /// Create a new `LspManager` with a dedicated multi-thread Tokio runtime.
    ///
    /// Returns `None` if the Tokio runtime cannot be started (extremely rare;
    /// typically only on platforms with no async I/O support).
    pub fn new() -> Option<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("anvil-lsp")
            .enable_all()
            .build()
            .ok()?;
        Some(LspManager {
            runtime,
            servers: HashMap::new(),
        })
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Notify the server for `server_id` that `path` was opened with `text`.
    ///
    /// If the server has not been started yet, this spawns it (lazy start).
    pub fn did_open(
        &mut self,
        server_id: &'static str,
        path: PathBuf,
        lang_id: &'static str,
        text: String,
    ) {
        let handle = self.get_or_spawn(server_id, lang_id);
        let _ = handle.tx.blocking_send(LspCommand::DidOpen {
            path,
            language_id: lang_id,
            version: 1,
            text,
        });
    }

    /// Notify the server that `path` content changed.
    pub fn did_change(&mut self, server_id: &'static str, path: PathBuf, text: String) {
        if let Some(handle) = self.servers.get(server_id) {
            let version = next_version();
            let _ = handle.tx.blocking_send(LspCommand::DidChange {
                path,
                version,
                text,
            });
        }
    }

    /// Notify the server that `path` was closed.
    pub fn did_close(&mut self, server_id: &'static str, path: PathBuf) {
        if let Some(handle) = self.servers.get(server_id) {
            let _ = handle.tx.blocking_send(LspCommand::DidClose { path });
        }
    }

    /// Return a snapshot of stored diagnostics for `path`.
    pub fn diagnostics_for(&self, path: &Path) -> Vec<DocumentDiagnostic> {
        for handle in self.servers.values() {
            let map = handle.diagnostics.lock().unwrap();
            if let Some(v) = map.get(path) {
                return v.clone();
            }
        }
        Vec::new()
    }

    /// Send a hover request for `path` at `(line, character)` to the server
    /// that manages the given path.  Returns the `request_id` assigned to this
    /// request; pass it to `poll_hover` to retrieve the result.
    ///
    /// A no-op (returns 0) when no live server is found for the path.
    pub fn request_hover(&self, path: &Path, line: u32, character: u32) -> u64 {
        let id = next_request_id();
        let server_id = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| language_id_for_ext(ext))
            .and_then(|lang| server_id_for_language(lang));
        if let Some(sid) = server_id {
            if let Some(handle) = self.servers.get(sid) {
                let _ = handle.tx.blocking_send(LspCommand::Hover {
                    path: path.to_path_buf(),
                    line,
                    character,
                    request_id: id,
                });
                return id;
            }
        }
        0
    }

    /// Poll for a hover result matching `request_id`.  Consumes the stored
    /// result on success (subsequent calls return `None` until a new request
    /// is issued).
    pub fn poll_hover(&self, request_id: u64) -> Option<HoverResult> {
        if request_id == 0 {
            return None;
        }
        for handle in self.servers.values() {
            let mut slot = handle.hover_result.lock().unwrap();
            if slot.as_ref().map(|(id, _)| *id) == Some(request_id) {
                return slot.take().map(|(_, r)| r);
            }
        }
        None
    }

    /// Return the current state of the named server.
    pub fn state_of(&self, server_id: &'static str) -> LspState {
        self.servers
            .get(server_id)
            .map(|h| h.state.lock().unwrap().clone())
            .unwrap_or(LspState::Down)
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    /// Return the handle for `server_id`, spawning the server if needed.
    fn get_or_spawn(&mut self, server_id: &'static str, lang_id: &'static str) -> &ServerHandle {
        if !self.servers.contains_key(server_id) {
            let handle = spawn_server(&self.runtime, server_id, lang_id);
            self.servers.insert(server_id, handle);
        }
        self.servers.get(server_id).unwrap()
    }
}

// ── Version counter ───────────────────────────────────────────────────────────

fn next_version() -> i32 {
    use std::sync::atomic::{AtomicI32, Ordering};
    static COUNTER: AtomicI32 = AtomicI32::new(2); // 1 is used by did_open
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn next_request_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(100); // start above initialize id=1
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ── Server binary resolution ──────────────────────────────────────────────────

/// Map a server id to `(binary, args)`.
fn server_command(server_id: &str) -> Option<(&'static str, &'static [&'static str])> {
    match server_id {
        "rust-analyzer" => Some(("rust-analyzer", &[])),
        "typescript-language-server" => Some(("typescript-language-server", &["--stdio"])),
        "pyright-langserver" => Some(("pyright-langserver", &["--stdio"])),
        "vscode-json-language-server" => Some(("vscode-json-language-server", &["--stdio"])),
        "taplo" => Some(("taplo", &["lsp", "stdio"])),
        _ => None,
    }
}

// ── Spawn a server task ───────────────────────────────────────────────────────

fn spawn_server(
    runtime: &tokio::runtime::Runtime,
    server_id: &'static str,
    _lang_id: &'static str,
) -> ServerHandle {
    let state = Arc::new(Mutex::new(LspState::Spawning));
    let diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<DocumentDiagnostic>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let hover_result: Arc<Mutex<Option<(u64, HoverResult)>>> = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::channel::<LspCommand>(64);

    let state_task = Arc::clone(&state);
    let diag_task = Arc::clone(&diagnostics);
    let hover_task = Arc::clone(&hover_result);

    // Resolve the binary before entering the async task so we can report the
    // failure synchronously-ish via the shared state.
    let cmd_info = server_command(server_id);
    let binary_path = cmd_info.and_then(|(bin, _)| which::which(bin).ok());

    if cmd_info.is_none() {
        *state.lock().unwrap() = LspState::Failed(format!("unknown server id '{server_id}'"));
        return ServerHandle {
            state,
            tx,
            diagnostics,
            hover_result,
        };
    }

    let (bin, args) = cmd_info.unwrap();
    let args: Vec<&'static str> = args.to_vec();

    if binary_path.is_none() {
        *state.lock().unwrap() = LspState::Failed(format!("server binary '{bin}' not on PATH"));
        return ServerHandle {
            state,
            tx,
            diagnostics,
            hover_result,
        };
    }

    let binary_path = binary_path.unwrap();

    runtime.spawn(async move {
        run_server(
            server_id,
            binary_path,
            args,
            rx,
            state_task,
            diag_task,
            hover_task,
        )
        .await;
    });

    ServerHandle {
        state,
        tx,
        diagnostics,
        hover_result,
    }
}

// ── Server task ───────────────────────────────────────────────────────────────

async fn run_server(
    server_id: &'static str,
    binary: PathBuf,
    args: Vec<&'static str>,
    mut rx: mpsc::Receiver<LspCommand>,
    state: Arc<Mutex<LspState>>,
    diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<DocumentDiagnostic>>>>,
    hover_result: Arc<Mutex<Option<(u64, HoverResult)>>>,
) {
    let mut child: Child = match tokio::process::Command::new(&binary)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            *state.lock().unwrap() =
                LspState::Failed(format!("failed to spawn '{server_id}': {e}"));
            return;
        }
    };

    let stdin: ChildStdin = child.stdin.take().unwrap();
    let stdout: ChildStdout = child.stdout.take().unwrap();

    let mut writer = tokio::io::BufWriter::new(stdin);
    let mut reader = BufReader::new(stdout);

    // Send initialize.
    let init_id: u64 = 1;
    let init_req = make_request(
        init_id,
        lsp_types::request::Initialize::METHOD,
        #[allow(deprecated)]
        serde_json::to_value(InitializeParams {
            process_id: Some(std::process::id()),
            capabilities: ClientCapabilities::default(),
            workspace_folders: None,
            initialization_options: None,
            client_info: None,
            locale: None,
            root_uri: None,
            root_path: None,
            trace: None,
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
        })
        .unwrap(),
    );

    if write_message(&mut writer, &init_req).await.is_err() {
        *state.lock().unwrap() = LspState::Failed("write error during initialize".into());
        return;
    }

    // Wait for initialize response.
    loop {
        let msg = match read_message(&mut reader).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                *state.lock().unwrap() =
                    LspState::Failed("server closed stdout before initialize".into());
                return;
            }
            Err(e) => {
                *state.lock().unwrap() = LspState::Failed(format!("read error: {e}"));
                return;
            }
        };

        if msg.get("id") == Some(&Value::Number(init_id.into())) {
            break; // initialize response received
        }
        // Ignore pre-initialize notifications (e.g. window/logMessage).
    }

    // Send initialized notification.
    let initialized_notif = make_notification(
        lsp_types::notification::Initialized::METHOD,
        serde_json::to_value(InitializedParams {}).unwrap(),
    );
    if write_message(&mut writer, &initialized_notif)
        .await
        .is_err()
    {
        *state.lock().unwrap() = LspState::Failed("write error during initialized".into());
        return;
    }

    *state.lock().unwrap() = LspState::Live;

    // Main loop: interleave incoming commands with incoming server messages.
    let diag_clone = Arc::clone(&diagnostics);
    let hover_clone = Arc::clone(&hover_result);
    loop {
        tokio::select! {
            biased;
            // Outbound: command from the main thread.
            cmd = rx.recv() => {
                match cmd {
                    None => break,
                    Some(LspCommand::Shutdown) => {
                        let req = make_request(
                            2,
                            lsp_types::request::Shutdown::METHOD,
                            Value::Null,
                        );
                        let _ = write_message(&mut writer, &req).await;
                        break;
                    }
                    Some(LspCommand::DidOpen { path, language_id, version, text }) => {
                        let uri = path_to_uri(&path);
                        let notif = make_notification(
                            lsp_types::notification::DidOpenTextDocument::METHOD,
                            serde_json::to_value(DidOpenTextDocumentParams {
                                text_document: TextDocumentItem {
                                    uri,
                                    language_id: language_id.to_string(),
                                    version,
                                    text,
                                },
                            })
                            .unwrap(),
                        );
                        let _ = write_message(&mut writer, &notif).await;
                    }
                    Some(LspCommand::DidChange { path, version, text }) => {
                        let uri = path_to_uri(&path);
                        let notif = make_notification(
                            lsp_types::notification::DidChangeTextDocument::METHOD,
                            serde_json::to_value(DidChangeTextDocumentParams {
                                text_document: VersionedTextDocumentIdentifier { uri, version },
                                content_changes: vec![TextDocumentContentChangeEvent {
                                    range: None,
                                    range_length: None,
                                    text,
                                }],
                            })
                            .unwrap(),
                        );
                        let _ = write_message(&mut writer, &notif).await;
                    }
                    Some(LspCommand::DidClose { path }) => {
                        let uri = path_to_uri(&path);
                        let notif = make_notification(
                            lsp_types::notification::DidCloseTextDocument::METHOD,
                            serde_json::to_value(DidCloseTextDocumentParams {
                                text_document: TextDocumentIdentifier { uri },
                            })
                            .unwrap(),
                        );
                        let _ = write_message(&mut writer, &notif).await;
                    }
                    Some(LspCommand::Hover { path, line, character, request_id }) => {
                        let uri = path_to_uri(&path);
                        let req = make_request(
                            request_id,
                            lsp_types::request::HoverRequest::METHOD,
                            serde_json::to_value(lsp_types::HoverParams {
                                text_document_position_params: lsp_types::TextDocumentPositionParams {
                                    text_document: TextDocumentIdentifier { uri },
                                    position: lsp_types::Position { line, character },
                                },
                                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                            })
                            .unwrap(),
                        );
                        let _ = write_message(&mut writer, &req).await;
                    }
                }
            }
            // Inbound: message from the server.
            msg = read_message(&mut reader) => {
                match msg {
                    Ok(Some(m)) => handle_server_message(m, &diag_clone, &hover_clone),
                    _ => break,
                }
            }
        }
    }

    *state.lock().unwrap() = LspState::Down;
}

// ── JSON-RPC framing ─────────────────────────────────────────────────────────

fn make_request(id: u64, method: &str, params: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

fn make_notification(method: &str, params: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

async fn write_message(
    writer: &mut tokio::io::BufWriter<ChildStdin>,
    msg: &Value,
) -> std::io::Result<()> {
    let body = serde_json::to_string(msg).unwrap();
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    writer.write_all(frame.as_bytes()).await?;
    writer.flush().await
}

async fn read_message(reader: &mut BufReader<ChildStdout>) -> std::io::Result<Option<Value>> {
    // Read headers until blank line.
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None); // EOF
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("Content-Length: ") {
            content_length = rest.trim().parse().ok();
        }
    }
    let len = match content_length {
        Some(n) => n,
        None => return Ok(None),
    };
    let mut body = vec![0u8; len];
    tokio::io::AsyncReadExt::read_exact(reader, &mut body).await?;
    let val: Value = serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(val))
}

// ── Inbound message handler ──────────────────────────────────────────────────

fn handle_server_message(
    msg: Value,
    diagnostics: &Arc<Mutex<HashMap<PathBuf, Vec<DocumentDiagnostic>>>>,
    hover_result: &Arc<Mutex<Option<(u64, HoverResult)>>>,
) {
    // Check if this is a response (has "id" but no "method").
    if msg.get("method").is_none() {
        // This is a response to a prior request.
        let id = match msg.get("id").and_then(Value::as_u64) {
            Some(i) => i,
            None => return,
        };
        // Handle hover response.
        if let Some(result) = msg.get("result") {
            if let Ok(hover) = serde_json::from_value::<lsp_types::Hover>(result.clone()) {
                let text = extract_hover_text(&hover);
                if !text.is_empty() {
                    *hover_result.lock().unwrap() = Some((id, HoverResult { text }));
                }
            }
        }
        return;
    }

    let method = match msg.get("method").and_then(Value::as_str) {
        Some(m) => m,
        None => return,
    };

    if method == lsp_types::notification::PublishDiagnostics::METHOD {
        if let Some(params) = msg.get("params") {
            if let Ok(p) = serde_json::from_value::<PublishDiagnosticsParams>(params.clone()) {
                let path = match p.uri.to_file_path() {
                    Ok(p) => p,
                    Err(_) => return,
                };
                let entries: Vec<DocumentDiagnostic> = p
                    .diagnostics
                    .iter()
                    .map(|d| DocumentDiagnostic {
                        line: d.range.start.line as usize,
                        col: d.range.start.character as usize,
                        severity: match d.severity {
                            Some(lsp_types::DiagnosticSeverity::WARNING) => {
                                DiagnosticSeverity::Warning
                            }
                            Some(lsp_types::DiagnosticSeverity::INFORMATION) => {
                                DiagnosticSeverity::Info
                            }
                            Some(lsp_types::DiagnosticSeverity::HINT) => DiagnosticSeverity::Hint,
                            _ => DiagnosticSeverity::Error,
                        },
                        message: d.message.clone(),
                    })
                    .collect();
                diagnostics.lock().unwrap().insert(path, entries);
            }
        }
    }
    // Future: handle window/logMessage, $/progress, etc.
}

/// Extract plain text from a `Hover` response.
fn extract_hover_text(hover: &lsp_types::Hover) -> String {
    match &hover.contents {
        lsp_types::HoverContents::Scalar(m) => marked_string_text(m),
        lsp_types::HoverContents::Array(arr) => arr
            .iter()
            .map(marked_string_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        lsp_types::HoverContents::Markup(mu) => mu.value.clone(),
    }
}

fn marked_string_text(ms: &lsp_types::MarkedString) -> String {
    match ms {
        lsp_types::MarkedString::String(s) => s.clone(),
        lsp_types::MarkedString::LanguageString(ls) => ls.value.clone(),
    }
}

// ── Utility ───────────────────────────────────────────────────────────────────

fn path_to_uri(path: &Path) -> Url {
    Url::from_file_path(path).unwrap_or_else(|_| Url::parse("file:///dev/null").unwrap())
}

// ── Language-id map (used by Buffer::language_id) ────────────────────────────

/// Return the LSP language-id string for a file extension.
pub fn language_id_for_ext(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "py" => Some("python"),
        "toml" => Some("toml"),
        "json" => Some("json"),
        "md" | "markdown" => Some("markdown"),
        _ => None,
    }
}

/// Return the default server id for a language-id.
pub fn server_id_for_language(lang_id: &str) -> Option<&'static str> {
    match lang_id {
        "rust" => Some("rust-analyzer"),
        "typescript" => Some("typescript-language-server"),
        "python" => Some("pyright-langserver"),
        "json" => Some("vscode-json-language-server"),
        "toml" => Some("taplo"),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_manager_new_does_not_panic() {
        let _mgr = LspManager::new();
    }

    #[test]
    fn lsp_manager_state_of_down_when_no_did_open() {
        let mgr = LspManager::new().expect("runtime");
        assert_eq!(mgr.state_of("rust-analyzer"), LspState::Down);
        assert_eq!(mgr.state_of("typescript-language-server"), LspState::Down);
    }

    #[test]
    fn lsp_manager_did_open_with_missing_binary_returns_failed_state() {
        let mut mgr = LspManager::new().expect("runtime");
        let path = PathBuf::from("/tmp/test.rs");
        mgr.did_open(
            "nonexistent_server_xyz",
            path,
            "rust",
            "fn main() {}".into(),
        );
        // The binary-not-found path sets Failed synchronously before the async
        // task runs, so no sleep is needed.
        let state = mgr.state_of("nonexistent_server_xyz");
        assert!(
            matches!(state, LspState::Failed(_)),
            "expected Failed, got {state:?}"
        );
    }

    #[test]
    fn language_id_for_ext_rs_is_rust() {
        assert_eq!(language_id_for_ext("rs"), Some("rust"));
        assert_eq!(language_id_for_ext("ts"), Some("typescript"));
        assert_eq!(language_id_for_ext("py"), Some("python"));
        assert_eq!(language_id_for_ext("toml"), Some("toml"));
        assert_eq!(language_id_for_ext("json"), Some("json"));
        assert_eq!(language_id_for_ext("md"), Some("markdown"));
        assert_eq!(language_id_for_ext("c"), None);
    }
}
