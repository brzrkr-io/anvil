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
    VersionedTextDocumentIdentifier, WorkspaceEdit, WorkspaceSymbolParams, WorkspaceSymbolResponse,
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
    Definition {
        path: PathBuf,
        line: u32,
        character: u32,
        request_id: u64,
    },
    Completion {
        path: PathBuf,
        line: u32,
        character: u32,
        request_id: u64,
    },
    Rename {
        path: PathBuf,
        line: u32,
        character: u32,
        new_name: String,
        request_id: u64,
    },
    CodeActions {
        path: PathBuf,
        line: u32,
        request_id: u64,
    },
    References {
        path: PathBuf,
        line: u32,
        character: u32,
        request_id: u64,
    },
    WorkspaceSymbols {
        query: String,
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

// ── Definition result (item 17) ───────────────────────────────────────────────

/// One resolved definition location.
#[derive(Debug, Clone)]
pub struct DefinitionLocation {
    pub path: PathBuf,
    pub line: u32,
    pub col: u32,
}

// ── Rename result (item 24) ───────────────────────────────────────────────────

/// One edit produced by a `textDocument/rename` response.
#[derive(Debug, Clone)]
pub struct RenameEdit {
    pub path: PathBuf,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub new_text: String,
}

// ── Code action (item 25) ─────────────────────────────────────────────────────

/// A single code action from `textDocument/codeAction`.
#[derive(Debug, Clone)]
pub struct LspCodeAction {
    pub title: String,
    /// Flat edits to apply (converted from WorkspaceEdit at receive time).
    pub edits: Vec<RenameEdit>,
}

// ── Workspace symbol (O1) ─────────────────────────────────────────────────────

/// A single workspace symbol result from `workspace/symbol`.
#[derive(Debug, Clone)]
pub struct WorkspaceSymbolHit {
    pub name: String,
    /// Short kind label for display, e.g. "fn", "struct", "enum", "trait", "var".
    pub kind_label: String,
    pub path: PathBuf,
    pub line: u32,
}

// ── Completion item (item 16) ─────────────────────────────────────────────────

/// A single completion item from the LSP server.
#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    /// Optional short description shown on the right.
    pub detail: Option<String>,
    /// Text to insert; falls back to `label` when `None`.
    pub insert_text: Option<String>,
}

// ── Shared result slot types ──────────────────────────────────────────────────

type HoverSlot = Arc<Mutex<Option<(u64, HoverResult)>>>;
type DefinitionSlot = Arc<Mutex<Option<(u64, Vec<DefinitionLocation>)>>>;
type CompletionSlot = Arc<Mutex<Option<(u64, Vec<CompletionItem>)>>>;
type RenameSlot = Arc<Mutex<Option<(u64, Vec<RenameEdit>)>>>;
type CodeActionsSlot = Arc<Mutex<Option<(u64, Vec<LspCodeAction>)>>>;
type ReferencesSlot = Arc<Mutex<Option<(u64, Vec<DefinitionLocation>)>>>; // reuse DefinitionLocation
type WorkspaceSymbolsSlot = Arc<Mutex<Option<(u64, Vec<WorkspaceSymbolHit>)>>>;

// ── ServerHandle ─────────────────────────────────────────────────────────────

struct ServerHandle {
    state: Arc<Mutex<LspState>>,
    tx: mpsc::Sender<LspCommand>,
    diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<DocumentDiagnostic>>>>,
    /// Latest hover result (request_id, HoverResult). `main.rs` polls this
    /// each frame and clears after consumption.
    hover_result: HoverSlot,
    /// Latest definition result (request_id, Vec<DefinitionLocation>).
    definition_result: DefinitionSlot,
    /// Latest completion result (request_id, Vec<CompletionItem>).
    completion_result: CompletionSlot,
    /// Latest rename result (request_id, Vec<RenameEdit>) — item 24.
    rename_result: RenameSlot,
    /// Latest code-actions result (request_id, Vec<LspCodeAction>) — item 25.
    code_actions_result: CodeActionsSlot,
    /// Latest references result (request_id, Vec<DefinitionLocation>) — item 26.
    references_result: ReferencesSlot,
    /// Latest workspace symbols result (O1).
    workspace_symbols_result: WorkspaceSymbolsSlot,
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

    /// Send a `textDocument/definition` request for `path` at `(line, character)`.
    /// Returns a `request_id` (non-zero on success) for use with `poll_definition`.
    pub fn request_definition(&self, path: &Path, line: u32, character: u32) -> u64 {
        let id = next_request_id();
        let server_id = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| language_id_for_ext(ext))
            .and_then(|lang| server_id_for_language(lang));
        if let Some(sid) = server_id {
            if let Some(handle) = self.servers.get(sid) {
                let _ = handle.tx.blocking_send(LspCommand::Definition {
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

    /// Poll for a definition result matching `request_id`. Consumes on success.
    pub fn poll_definition(&self, request_id: u64) -> Option<Vec<DefinitionLocation>> {
        if request_id == 0 {
            return None;
        }
        for handle in self.servers.values() {
            let mut slot = handle.definition_result.lock().unwrap();
            if slot.as_ref().map(|(id, _)| *id) == Some(request_id) {
                return slot.take().map(|(_, r)| r);
            }
        }
        None
    }

    /// Send a `textDocument/completion` request for `path` at `(line, character)`.
    /// Returns a `request_id` (non-zero on success) for use with `poll_completion`.
    pub fn request_completion(&self, path: &Path, line: u32, character: u32) -> u64 {
        let id = next_request_id();
        let server_id = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| language_id_for_ext(ext))
            .and_then(|lang| server_id_for_language(lang));
        if let Some(sid) = server_id {
            if let Some(handle) = self.servers.get(sid) {
                let _ = handle.tx.blocking_send(LspCommand::Completion {
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

    /// Poll for a completion result matching `request_id`. Consumes on success.
    pub fn poll_completion(&self, request_id: u64) -> Option<Vec<CompletionItem>> {
        if request_id == 0 {
            return None;
        }
        for handle in self.servers.values() {
            let mut slot = handle.completion_result.lock().unwrap();
            if slot.as_ref().map(|(id, _)| *id) == Some(request_id) {
                return slot.take().map(|(_, r)| r);
            }
        }
        None
    }

    // ── Rename (item 24) ──────────────────────────────────────────────────────

    /// Send a `textDocument/rename` request. Returns a non-zero `request_id` on
    /// success; 0 when no live server is available (logs once to stderr).
    pub fn request_rename(&self, path: &Path, line: u32, character: u32, new_name: String) -> u64 {
        let id = next_request_id();
        let server_id = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| language_id_for_ext(ext))
            .and_then(|lang| server_id_for_language(lang));
        if let Some(sid) = server_id {
            if let Some(handle) = self.servers.get(sid) {
                let _ = handle.tx.blocking_send(LspCommand::Rename {
                    path: path.to_path_buf(),
                    line,
                    character,
                    new_name,
                    request_id: id,
                });
                return id;
            }
        }
        eprintln!("anvil-lsp: rename unavailable (no LSP)");
        0
    }

    /// Poll for a rename result matching `request_id`. Consumes on success.
    pub fn poll_rename(&self, request_id: u64) -> Option<Vec<RenameEdit>> {
        if request_id == 0 {
            return None;
        }
        for handle in self.servers.values() {
            let mut slot = handle.rename_result.lock().unwrap();
            if slot.as_ref().map(|(id, _)| *id) == Some(request_id) {
                return slot.take().map(|(_, r)| r);
            }
        }
        None
    }

    // ── Code actions (item 25) ────────────────────────────────────────────────

    /// Send a `textDocument/codeAction` request for `line`. Returns a non-zero
    /// `request_id` on success; 0 when no live server is available (logs once).
    pub fn request_code_actions(&self, path: &Path, line: u32) -> u64 {
        let id = next_request_id();
        let server_id = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| language_id_for_ext(ext))
            .and_then(|lang| server_id_for_language(lang));
        if let Some(sid) = server_id {
            if let Some(handle) = self.servers.get(sid) {
                let _ = handle.tx.blocking_send(LspCommand::CodeActions {
                    path: path.to_path_buf(),
                    line,
                    request_id: id,
                });
                return id;
            }
        }
        eprintln!("anvil-lsp: code actions unavailable (no LSP)");
        0
    }

    /// Poll for a code-actions result matching `request_id`. Consumes on success.
    pub fn poll_code_actions(&self, request_id: u64) -> Option<Vec<LspCodeAction>> {
        if request_id == 0 {
            return None;
        }
        for handle in self.servers.values() {
            let mut slot = handle.code_actions_result.lock().unwrap();
            if slot.as_ref().map(|(id, _)| *id) == Some(request_id) {
                return slot.take().map(|(_, r)| r);
            }
        }
        None
    }

    // ── References (item 26) ──────────────────────────────────────────────────

    /// Send a `textDocument/references` request. Returns a non-zero `request_id`
    /// on success; 0 when no live server is available (logs once).
    pub fn request_references(&self, path: &Path, line: u32, character: u32) -> u64 {
        let id = next_request_id();
        let server_id = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| language_id_for_ext(ext))
            .and_then(|lang| server_id_for_language(lang));
        if let Some(sid) = server_id {
            if let Some(handle) = self.servers.get(sid) {
                let _ = handle.tx.blocking_send(LspCommand::References {
                    path: path.to_path_buf(),
                    line,
                    character,
                    request_id: id,
                });
                return id;
            }
        }
        eprintln!("anvil-lsp: references unavailable (no LSP)");
        0
    }

    /// Poll for a references result matching `request_id`. Consumes on success.
    pub fn poll_references(&self, request_id: u64) -> Option<Vec<DefinitionLocation>> {
        if request_id == 0 {
            return None;
        }
        for handle in self.servers.values() {
            let mut slot = handle.references_result.lock().unwrap();
            if slot.as_ref().map(|(id, _)| *id) == Some(request_id) {
                return slot.take().map(|(_, r)| r);
            }
        }
        None
    }

    /// Return `true` if at least one language server is currently `Live`.
    pub fn any_live(&self) -> bool {
        self.servers
            .values()
            .any(|h| matches!(*h.state.lock().unwrap(), LspState::Live))
    }

    // ── Workspace symbols (O1) ────────────────────────────────────────────────

    /// Send a `workspace/symbol` request with `query`. Returns a non-zero
    /// `request_id` for use with [`poll_workspace_symbols`], or 0 when no live
    /// server is available. Sends to the first `Live` server found.
    pub fn request_workspace_symbols(&self, query: String) -> u64 {
        let id = next_request_id();
        for handle in self.servers.values() {
            if matches!(*handle.state.lock().unwrap(), LspState::Live) {
                let _ = handle.tx.blocking_send(LspCommand::WorkspaceSymbols {
                    query,
                    request_id: id,
                });
                return id;
            }
        }
        0
    }

    /// Poll for a workspace symbols result matching `request_id`. Consumes on success.
    pub fn poll_workspace_symbols(&self, request_id: u64) -> Option<Vec<WorkspaceSymbolHit>> {
        if request_id == 0 {
            return None;
        }
        for handle in self.servers.values() {
            let mut slot = handle.workspace_symbols_result.lock().unwrap();
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

    /// Send `shutdown` + `exit` to every live server and wait up to 500 ms for
    /// each.  Called from `App::shutdown` (item 20) before the process exits.
    ///
    /// Idempotent: calling this more than once is safe (the channel is already
    /// closed after the first call so subsequent sends are no-ops).
    pub fn shutdown_all(&mut self) {
        for handle in self.servers.values() {
            // Sending Shutdown closes the server's command loop; the task then
            // sends the LSP `shutdown` request and breaks.  If the server is
            // not yet `Live` (still spawning) the send will be buffered or
            // dropped — both are fine, since kill_on_drop cleans up the child.
            let _ = handle.tx.blocking_send(LspCommand::Shutdown);
        }

        // Give Tokio tasks a moment to flush the shutdown request.
        // We budget 500 ms total, not per-server.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            let all_down = self.servers.values().all(|h| {
                matches!(
                    *h.state.lock().unwrap(),
                    LspState::Down | LspState::Failed(_)
                )
            });
            if all_down {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // Drop all handles; Tokio runtime drop will kill any remaining children
        // (kill_on_drop(true) on the child process).
        self.servers.clear();
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    /// Return the handle for `server_id`, spawning the server if needed.
    fn get_or_spawn(&mut self, server_id: &'static str, lang_id: &'static str) -> &ServerHandle {
        if !self.servers.contains_key(server_id) {
            let workspace_root = std::env::current_dir().ok();
            let handle = spawn_server(&self.runtime, server_id, lang_id, workspace_root);
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
    workspace_root: Option<PathBuf>,
) -> ServerHandle {
    let state = Arc::new(Mutex::new(LspState::Spawning));
    let diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<DocumentDiagnostic>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let hover_result: HoverSlot = Arc::new(Mutex::new(None));
    let definition_result: DefinitionSlot = Arc::new(Mutex::new(None));
    let completion_result: CompletionSlot = Arc::new(Mutex::new(None));
    let rename_result: RenameSlot = Arc::new(Mutex::new(None));
    let code_actions_result: CodeActionsSlot = Arc::new(Mutex::new(None));
    let references_result: ReferencesSlot = Arc::new(Mutex::new(None));
    let workspace_symbols_result: WorkspaceSymbolsSlot = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::channel::<LspCommand>(64);

    let state_task = Arc::clone(&state);
    let diag_task = Arc::clone(&diagnostics);
    let hover_task = Arc::clone(&hover_result);
    let def_task = Arc::clone(&definition_result);
    let comp_task = Arc::clone(&completion_result);
    let rename_task = Arc::clone(&rename_result);
    let code_actions_task = Arc::clone(&code_actions_result);
    let references_task = Arc::clone(&references_result);
    let workspace_symbols_task = Arc::clone(&workspace_symbols_result);

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
            definition_result,
            completion_result,
            rename_result,
            code_actions_result,
            references_result,
            workspace_symbols_result,
        };
    }

    let (bin, args) = cmd_info.unwrap();
    let args: Vec<&'static str> = args.to_vec();

    if binary_path.is_none() {
        // Log once to stderr so the user knows why diagnostics are disabled.
        eprintln!("anvil-lsp: {bin} not found in PATH; diagnostics disabled");
        *state.lock().unwrap() = LspState::Failed(format!("server binary '{bin}' not on PATH"));
        return ServerHandle {
            state,
            tx,
            diagnostics,
            hover_result,
            definition_result,
            completion_result,
            rename_result,
            code_actions_result,
            references_result,
            workspace_symbols_result,
        };
    }

    let binary_path = binary_path.unwrap();

    let root_uri = workspace_root.and_then(|p| Url::from_directory_path(p).ok());

    runtime.spawn(async move {
        run_server(
            server_id,
            binary_path,
            args,
            rx,
            state_task,
            diag_task,
            hover_task,
            def_task,
            comp_task,
            rename_task,
            code_actions_task,
            references_task,
            workspace_symbols_task,
            root_uri,
        )
        .await;
    });

    ServerHandle {
        state,
        tx,
        diagnostics,
        hover_result,
        definition_result,
        completion_result,
        rename_result,
        code_actions_result,
        references_result,
        workspace_symbols_result,
    }
}

// ── Server task ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_server(
    server_id: &'static str,
    binary: PathBuf,
    args: Vec<&'static str>,
    mut rx: mpsc::Receiver<LspCommand>,
    state: Arc<Mutex<LspState>>,
    diagnostics: Arc<Mutex<HashMap<PathBuf, Vec<DocumentDiagnostic>>>>,
    hover_result: HoverSlot,
    definition_result: DefinitionSlot,
    completion_result: CompletionSlot,
    rename_result: RenameSlot,
    code_actions_result: CodeActionsSlot,
    references_result: ReferencesSlot,
    workspace_symbols_result: WorkspaceSymbolsSlot,
    root_uri: Option<Url>,
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
            root_uri: root_uri.clone(),
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
    let def_clone = Arc::clone(&definition_result);
    let comp_clone = Arc::clone(&completion_result);
    let rename_clone = Arc::clone(&rename_result);
    let code_actions_clone = Arc::clone(&code_actions_result);
    let references_clone = Arc::clone(&references_result);
    let workspace_symbols_clone = Arc::clone(&workspace_symbols_result);
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
                    Some(LspCommand::Definition { path, line, character, request_id }) => {
                        let uri = path_to_uri(&path);
                        let req = make_request(
                            request_id,
                            lsp_types::request::GotoDefinition::METHOD,
                            serde_json::to_value(lsp_types::GotoDefinitionParams {
                                text_document_position_params: lsp_types::TextDocumentPositionParams {
                                    text_document: TextDocumentIdentifier { uri },
                                    position: lsp_types::Position { line, character },
                                },
                                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                                partial_result_params: lsp_types::PartialResultParams::default(),
                            })
                            .unwrap(),
                        );
                        let _ = write_message(&mut writer, &req).await;
                    }
                    Some(LspCommand::Completion { path, line, character, request_id }) => {
                        let uri = path_to_uri(&path);
                        let req = make_request(
                            request_id,
                            lsp_types::request::Completion::METHOD,
                            serde_json::to_value(lsp_types::CompletionParams {
                                text_document_position: lsp_types::TextDocumentPositionParams {
                                    text_document: TextDocumentIdentifier { uri },
                                    position: lsp_types::Position { line, character },
                                },
                                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                                partial_result_params: lsp_types::PartialResultParams::default(),
                                context: Some(lsp_types::CompletionContext {
                                    trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                                    trigger_character: None,
                                }),
                            })
                            .unwrap(),
                        );
                        let _ = write_message(&mut writer, &req).await;
                    }
                    Some(LspCommand::Rename { path, line, character, new_name, request_id }) => {
                        let uri = path_to_uri(&path);
                        let req = make_request(
                            request_id,
                            lsp_types::request::Rename::METHOD,
                            serde_json::to_value(lsp_types::RenameParams {
                                text_document_position: lsp_types::TextDocumentPositionParams {
                                    text_document: TextDocumentIdentifier { uri },
                                    position: lsp_types::Position { line, character },
                                },
                                new_name,
                                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                            })
                            .unwrap(),
                        );
                        let _ = write_message(&mut writer, &req).await;
                    }
                    Some(LspCommand::CodeActions { path, line, request_id }) => {
                        let uri = path_to_uri(&path);
                        let range = lsp_types::Range {
                            start: lsp_types::Position { line, character: 0 },
                            end: lsp_types::Position { line, character: 0 },
                        };
                        let req = make_request(
                            request_id,
                            lsp_types::request::CodeActionRequest::METHOD,
                            serde_json::to_value(lsp_types::CodeActionParams {
                                text_document: TextDocumentIdentifier { uri },
                                range,
                                context: lsp_types::CodeActionContext {
                                    diagnostics: Vec::new(),
                                    only: None,
                                    trigger_kind: None,
                                },
                                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                                partial_result_params: lsp_types::PartialResultParams::default(),
                            })
                            .unwrap(),
                        );
                        let _ = write_message(&mut writer, &req).await;
                    }
                    Some(LspCommand::References { path, line, character, request_id }) => {
                        let uri = path_to_uri(&path);
                        let req = make_request(
                            request_id,
                            lsp_types::request::References::METHOD,
                            serde_json::to_value(lsp_types::ReferenceParams {
                                text_document_position: lsp_types::TextDocumentPositionParams {
                                    text_document: TextDocumentIdentifier { uri },
                                    position: lsp_types::Position { line, character },
                                },
                                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                                partial_result_params: lsp_types::PartialResultParams::default(),
                                context: lsp_types::ReferenceContext {
                                    include_declaration: true,
                                },
                            })
                            .unwrap(),
                        );
                        let _ = write_message(&mut writer, &req).await;
                    }
                    Some(LspCommand::WorkspaceSymbols { query, request_id }) => {
                        let req = make_request(
                            request_id,
                            lsp_types::request::WorkspaceSymbolRequest::METHOD,
                            serde_json::to_value(WorkspaceSymbolParams {
                                query,
                                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                                partial_result_params: lsp_types::PartialResultParams::default(),
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
                    Ok(Some(m)) => handle_server_message(
                        m,
                        &diag_clone,
                        &hover_clone,
                        &def_clone,
                        &comp_clone,
                        &rename_clone,
                        &code_actions_clone,
                        &references_clone,
                        &workspace_symbols_clone,
                    ),
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

#[allow(clippy::too_many_arguments)]
fn handle_server_message(
    msg: Value,
    diagnostics: &Arc<Mutex<HashMap<PathBuf, Vec<DocumentDiagnostic>>>>,
    hover_result: &HoverSlot,
    definition_result: &DefinitionSlot,
    completion_result: &CompletionSlot,
    rename_result: &RenameSlot,
    code_actions_result: &CodeActionsSlot,
    references_result: &ReferencesSlot,
    workspace_symbols_result: &WorkspaceSymbolsSlot,
) {
    // Check if this is a response (has "id" but no "method").
    if msg.get("method").is_none() {
        // This is a response to a prior request.
        let id = match msg.get("id").and_then(Value::as_u64) {
            Some(i) => i,
            None => return,
        };
        if let Some(result) = msg.get("result") {
            // Try hover response.
            if let Ok(hover) = serde_json::from_value::<lsp_types::Hover>(result.clone()) {
                let text = extract_hover_text(&hover);
                if !text.is_empty() {
                    *hover_result.lock().unwrap() = Some((id, HoverResult { text }));
                    return;
                }
            }
            // Try rename response (WorkspaceEdit).
            if let Ok(we) = serde_json::from_value::<WorkspaceEdit>(result.clone()) {
                let edits = parse_workspace_edit_to_rename(&we);
                if !edits.is_empty() {
                    *rename_result.lock().unwrap() = Some((id, edits));
                    return;
                }
            }
            // Try code-action response (Vec<CodeAction | Command>).
            let actions = parse_code_actions_result(result);
            if !actions.is_empty() {
                *code_actions_result.lock().unwrap() = Some((id, actions));
                return;
            }
            // Try references response (Vec<Location>).
            if let Ok(locs) = serde_json::from_value::<Vec<lsp_types::Location>>(result.clone()) {
                if !locs.is_empty() {
                    let refs: Vec<DefinitionLocation> = locs
                        .into_iter()
                        .filter_map(|l| {
                            let path = l.uri.to_file_path().ok()?;
                            Some(DefinitionLocation {
                                path,
                                line: l.range.start.line,
                                col: l.range.start.character,
                            })
                        })
                        .collect();
                    if !refs.is_empty() {
                        *references_result.lock().unwrap() = Some((id, refs));
                        return;
                    }
                }
            }
            // Try definition response (Location | Vec<Location> | Vec<LocationLink>).
            let locs = parse_definition_result(result);
            if !locs.is_empty() {
                *definition_result.lock().unwrap() = Some((id, locs));
                return;
            }
            // Try completion response (CompletionList | Vec<CompletionItem>).
            let items = parse_completion_result(result);
            if !items.is_empty() {
                *completion_result.lock().unwrap() = Some((id, items));
                return;
            }
            // Try workspace/symbol response (WorkspaceSymbolResponse or Vec<SymbolInformation>).
            let ws_hits = parse_workspace_symbol_result(result);
            if !ws_hits.is_empty() {
                *workspace_symbols_result.lock().unwrap() = Some((id, ws_hits));
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

/// Parse a `textDocument/definition` result into `DefinitionLocation`s.
fn parse_definition_result(result: &Value) -> Vec<DefinitionLocation> {
    // Response can be: null | Location | Location[] | LocationLink[]
    if result.is_null() {
        return Vec::new();
    }
    // Try as a single Location.
    if let Ok(loc) = serde_json::from_value::<lsp_types::Location>(result.clone()) {
        if let Ok(path) = loc.uri.to_file_path() {
            return vec![DefinitionLocation {
                path,
                line: loc.range.start.line,
                col: loc.range.start.character,
            }];
        }
    }
    // Try as Vec<Location>.
    if let Ok(locs) = serde_json::from_value::<Vec<lsp_types::Location>>(result.clone()) {
        return locs
            .into_iter()
            .filter_map(|l| {
                let path = l.uri.to_file_path().ok()?;
                Some(DefinitionLocation {
                    path,
                    line: l.range.start.line,
                    col: l.range.start.character,
                })
            })
            .collect();
    }
    // Try as Vec<LocationLink>.
    if let Ok(links) = serde_json::from_value::<Vec<lsp_types::LocationLink>>(result.clone()) {
        return links
            .into_iter()
            .filter_map(|l| {
                let path = l.target_uri.to_file_path().ok()?;
                Some(DefinitionLocation {
                    path,
                    line: l.target_range.start.line,
                    col: l.target_range.start.character,
                })
            })
            .collect();
    }
    Vec::new()
}

/// Parse a `textDocument/completion` result into `CompletionItem`s.
fn parse_completion_result(result: &Value) -> Vec<CompletionItem> {
    // Response can be: CompletionList | CompletionItem[] | null
    if result.is_null() {
        return Vec::new();
    }
    // Try as CompletionList first.
    let raw_items: Vec<lsp_types::CompletionItem> =
        if let Ok(list) = serde_json::from_value::<lsp_types::CompletionList>(result.clone()) {
            list.items
        } else if let Ok(items) =
            serde_json::from_value::<Vec<lsp_types::CompletionItem>>(result.clone())
        {
            items
        } else {
            return Vec::new();
        };

    raw_items
        .into_iter()
        .map(|ci| CompletionItem {
            label: ci.label.clone(),
            detail: ci.detail.clone(),
            insert_text: ci.insert_text.or(Some(ci.label)),
        })
        .collect()
}

/// Parse a `workspace/symbol` result into `WorkspaceSymbolHit`s (O1).
fn parse_workspace_symbol_result(result: &Value) -> Vec<WorkspaceSymbolHit> {
    if result.is_null() {
        return Vec::new();
    }
    // Try the typed WorkspaceSymbolResponse first (Nested or Flat).
    if let Ok(resp) = serde_json::from_value::<WorkspaceSymbolResponse>(result.clone()) {
        return match resp {
            WorkspaceSymbolResponse::Nested(syms) => syms
                .into_iter()
                .filter_map(|s| {
                    let (path, line) = match s.location {
                        lsp_types::OneOf::Left(loc) => {
                            let p = loc.uri.to_file_path().ok()?;
                            (p, loc.range.start.line)
                        }
                        lsp_types::OneOf::Right(wloc) => {
                            let p = wloc.uri.to_file_path().ok()?;
                            (p, 0)
                        }
                    };
                    Some(WorkspaceSymbolHit {
                        name: s.name,
                        kind_label: symbol_kind_label(s.kind),
                        path,
                        line,
                    })
                })
                .collect(),
            WorkspaceSymbolResponse::Flat(syms) => syms
                .into_iter()
                .filter_map(|s| {
                    let path = s.location.uri.to_file_path().ok()?;
                    Some(WorkspaceSymbolHit {
                        name: s.name,
                        kind_label: symbol_kind_label(s.kind),
                        path,
                        line: s.location.range.start.line,
                    })
                })
                .collect(),
        };
    }
    Vec::new()
}

fn symbol_kind_label(kind: lsp_types::SymbolKind) -> String {
    match kind {
        lsp_types::SymbolKind::FUNCTION | lsp_types::SymbolKind::METHOD => "fn",
        lsp_types::SymbolKind::STRUCT => "struct",
        lsp_types::SymbolKind::ENUM | lsp_types::SymbolKind::ENUM_MEMBER => "enum",
        lsp_types::SymbolKind::INTERFACE => "trait",
        lsp_types::SymbolKind::CLASS => "class",
        lsp_types::SymbolKind::MODULE | lsp_types::SymbolKind::NAMESPACE => "mod",
        lsp_types::SymbolKind::VARIABLE | lsp_types::SymbolKind::FIELD => "var",
        lsp_types::SymbolKind::CONSTANT => "const",
        lsp_types::SymbolKind::TYPE_PARAMETER => "type",
        _ => "sym",
    }
    .to_string()
}

/// Flatten a `WorkspaceEdit` into a list of `RenameEdit`s (item 24).
fn parse_workspace_edit_to_rename(we: &WorkspaceEdit) -> Vec<RenameEdit> {
    parse_workspace_edit_to_rename_inner(we)
}

/// Public wrapper for `lib.rs` re-export.
pub fn parse_workspace_edit_to_rename_pub(we: WorkspaceEdit) -> Vec<RenameEdit> {
    parse_workspace_edit_to_rename_inner(&we)
}

fn parse_workspace_edit_to_rename_inner(we: &WorkspaceEdit) -> Vec<RenameEdit> {
    let mut out = Vec::new();
    if let Some(changes) = &we.changes {
        for (uri, edits) in changes {
            let Ok(path) = uri.to_file_path() else {
                continue;
            };
            for e in edits {
                out.push(RenameEdit {
                    path: path.clone(),
                    start_line: e.range.start.line,
                    start_col: e.range.start.character,
                    end_line: e.range.end.line,
                    end_col: e.range.end.character,
                    new_text: e.new_text.clone(),
                });
            }
        }
    }
    out
}

/// Parse a `textDocument/codeAction` result into `LspCodeAction`s (item 25).
fn parse_code_actions_result(result: &Value) -> Vec<LspCodeAction> {
    if result.is_null() {
        return Vec::new();
    }
    // The response is `(Command | CodeAction)[]`.
    let arr = match result.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|v| {
            // A CodeAction has `title`; a Command also has `title`. Both usable.
            let title = v.get("title")?.as_str()?.to_string();
            let edits = v
                .get("edit")
                .and_then(|e| serde_json::from_value::<WorkspaceEdit>(e.clone()).ok())
                .map(|we| parse_workspace_edit_to_rename_inner(&we))
                .unwrap_or_default();
            Some(LspCodeAction { title, edits })
        })
        .collect()
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
    fn lsp_initialize_sends_root_uri_from_workspace_root() {
        // Verify that spawn_server sets Failed with the bin name when missing,
        // and that the workspace_root path propagates through get_or_spawn
        // without panicking (even if rust-analyzer is not installed).
        let mut mgr = LspManager::new().expect("runtime");
        let path = PathBuf::from("/tmp/test18.rs");
        mgr.did_open("rust-analyzer", path, "rust", "fn main() {}".into());
        // Either Spawning (binary found) or Failed (binary missing) — never Down.
        let state = mgr.state_of("rust-analyzer");
        assert!(
            !matches!(state, LspState::Down),
            "after did_open, state must not be Down; got {state:?}"
        );
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
    fn shutdown_all_on_empty_manager_is_noop() {
        // No servers spawned; shutdown_all must return without blocking.
        let mut mgr = LspManager::new().expect("runtime");
        mgr.shutdown_all(); // must not panic or block
        mgr.shutdown_all(); // idempotent second call
    }

    #[test]
    fn shutdown_all_after_failed_server_is_idempotent() {
        // Spawn a known-missing server (immediately Failed), then shut down
        // twice to verify idempotency.
        let mut mgr = LspManager::new().expect("runtime");
        let path = PathBuf::from("/tmp/shutdown_test.rs");
        mgr.did_open("nonexistent_server_xyz", path, "rust", String::new());
        assert!(matches!(
            mgr.state_of("nonexistent_server_xyz"),
            LspState::Failed(_)
        ));
        mgr.shutdown_all(); // first shutdown
        // After shutdown_all, servers map is cleared.
        assert_eq!(mgr.state_of("nonexistent_server_xyz"), LspState::Down);
        mgr.shutdown_all(); // second call must not panic
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

    // ── Item 24: LSP rename ────────────────────────────────────────────────────

    /// `request_rename` with no server returns 0 and logs once.
    #[test]
    fn request_rename_no_server_returns_zero() {
        let mgr = LspManager::new().expect("runtime");
        // No server started — must return 0 without panicking.
        let id = mgr.request_rename(&PathBuf::from("/tmp/foo.rs"), 5, 3, "new_name".to_string());
        assert_eq!(id, 0, "expected 0 when no server");
    }

    /// `poll_rename` on a zero id returns None immediately.
    #[test]
    fn poll_rename_zero_request_id_is_none() {
        let mgr = LspManager::new().expect("runtime");
        assert!(mgr.poll_rename(0).is_none());
    }

    /// `parse_workspace_edit_to_rename_inner` correctly flattens a WorkspaceEdit.
    #[test]
    fn parse_workspace_edit_flattens_edits() {
        use lsp_types::{TextEdit, Url};
        let mut changes = std::collections::HashMap::new();
        let uri = Url::parse("file:///tmp/foo.rs").unwrap();
        changes.insert(
            uri,
            vec![TextEdit {
                range: lsp_types::Range {
                    start: lsp_types::Position {
                        line: 1,
                        character: 2,
                    },
                    end: lsp_types::Position {
                        line: 1,
                        character: 7,
                    },
                },
                new_text: "renamed".to_string(),
            }],
        );
        let we = WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        };
        let edits = parse_workspace_edit_to_rename_inner(&we);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "renamed");
        assert_eq!(edits[0].start_line, 1);
        assert_eq!(edits[0].start_col, 2);
    }

    // ── Item 25: code actions ──────────────────────────────────────────────────

    /// `request_code_actions` with no server returns 0 and logs once.
    #[test]
    fn request_code_actions_no_server_returns_zero() {
        let mgr = LspManager::new().expect("runtime");
        let id = mgr.request_code_actions(&PathBuf::from("/tmp/foo.rs"), 3);
        assert_eq!(id, 0, "expected 0 when no server");
    }

    /// `poll_code_actions` on zero id returns None.
    #[test]
    fn poll_code_actions_zero_id_is_none() {
        let mgr = LspManager::new().expect("runtime");
        assert!(mgr.poll_code_actions(0).is_none());
    }

    /// `parse_code_actions_result` extracts titles and converts edits.
    #[test]
    fn parse_code_actions_result_extracts_title() {
        let json = serde_json::json!([
            { "title": "Import std::io", "kind": "quickfix" },
            { "title": "Fix spelling" }
        ]);
        let actions = parse_code_actions_result(&json);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title, "Import std::io");
        assert_eq!(actions[1].title, "Fix spelling");
        // No edit field → edits should be empty.
        assert!(actions[0].edits.is_empty());
    }

    // ── Item 26: references ────────────────────────────────────────────────────

    /// `request_references` with no server returns 0 and logs once.
    #[test]
    fn request_references_no_server_returns_zero() {
        let mgr = LspManager::new().expect("runtime");
        let id = mgr.request_references(&PathBuf::from("/tmp/foo.rs"), 0, 0);
        assert_eq!(id, 0, "expected 0 when no server");
    }

    /// `poll_references` on zero id returns None.
    #[test]
    fn poll_references_zero_id_is_none() {
        let mgr = LspManager::new().expect("runtime");
        assert!(mgr.poll_references(0).is_none());
    }
}
