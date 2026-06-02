//! Minimal LSP bridge. Spawns a language server (rust-analyzer, gopls) per
//! language, speaks the JSON-RPC base protocol over stdio, forwards requests
//! from the webview, and streams diagnostics back as `lsp://diagnostics`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Resolve the user's real PATH once. A GUI app launched from Finder/Dock does
/// not inherit the login-shell PATH, so language servers in ~/.cargo/bin, Nix
/// profiles, Homebrew, or $GOPATH/bin would be invisible. Ask the login shell
/// for its PATH and union in the common locations as a fallback.
fn shell_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut dirs: Vec<String> = Vec::new();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        if let Ok(out) = Command::new(&shell).args(["-lic", "printf %s \"$PATH\""]).output() {
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
            if !d.contains("//") && !dirs.iter().any(|x| *x == d) {
                dirs.push(d);
            }
        }
        dirs.join(":")
    })
}

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

type Pending = Arc<Mutex<HashMap<i64, mpsc::Sender<Value>>>>;

struct LspHandle {
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: Arc<Mutex<i64>>,
    pending: Pending,
}

#[derive(Default)]
pub struct LspState(Mutex<HashMap<String, LspHandle>>);

/// Read one `Content-Length`-framed JSON-RPC message. Returns None on EOF.
fn read_message<R: BufRead>(reader: &mut R) -> Option<Value> {
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some(n) = t.strip_prefix("Content-Length:") {
            content_length = n.trim().parse().ok()?;
        }
    }
    if content_length == 0 {
        return None;
    }
    let mut buf = vec![0u8; content_length];
    reader.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn write_message(stdin: &mut ChildStdin, v: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(v)?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
    stdin.write_all(&body)?;
    stdin.flush()
}

fn server_command(lang: &str) -> Option<Command> {
    // Auto-detect: spawn the conventional language server for the file's language.
    // A missing binary just fails to spawn → the editor runs without LSP for it.
    match lang {
        "rust" => Some(Command::new("rust-analyzer")),
        "go" => Some(Command::new("gopls")),
        "typescript" => {
            let mut c = Command::new("typescript-language-server");
            c.arg("--stdio");
            Some(c)
        }
        "python" => {
            let mut c = Command::new("pyright-langserver");
            c.arg("--stdio");
            Some(c)
        }
        "cpp" => Some(Command::new("clangd")),
        _ => None,
    }
}

/// Send a request and block (up to `secs`) for the matching response.
fn request_blocking(
    h: &LspHandle,
    method: &str,
    params: Value,
    secs: u64,
) -> Result<Value, String> {
    let id = {
        let mut n = h.next_id.lock().unwrap();
        *n += 1;
        *n
    };
    let (tx, rx) = mpsc::channel();
    h.pending.lock().unwrap().insert(id, tx);
    let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    write_message(&mut h.stdin.lock().unwrap(), &msg).map_err(|e| e.to_string())?;
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(v) => Ok(v),
        Err(_) => {
            h.pending.lock().unwrap().remove(&id);
            Err(format!("lsp request '{method}' timed out"))
        }
    }
}

fn notify(h: &LspHandle, method: &str, params: Value) -> Result<(), String> {
    let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
    write_message(&mut h.stdin.lock().unwrap(), &msg).map_err(|e| e.to_string())
}

/// Start (or reuse) the server for `lang`, rooted at `root_uri`. Returns false
/// if no server is configured/installed for that language.
#[tauri::command]
pub fn lsp_start(
    app: AppHandle,
    state: State<LspState>,
    lang: String,
    root_uri: String,
) -> Result<bool, String> {
    if state.0.lock().unwrap().contains_key(&lang) {
        return Ok(true);
    }
    let mut cmd = match server_command(&lang) {
        Some(c) => c,
        None => return Ok(false),
    };
    cmd.env("PATH", shell_path());
    let mut child = match cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return Ok(false), // server binary not on PATH
    };

    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let stdout = child.stdout.take().unwrap();
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
    let next_id = Arc::new(Mutex::new(0i64));

    let rp = pending.clone();
    let rstdin = stdin.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Some(msg) = read_message(&mut reader) {
            if let Some(id) = msg.get("id").and_then(|v| v.as_i64()) {
                if msg.get("method").is_some() {
                    // Server→client request: answer the few that block startup.
                    let resp = json!({ "jsonrpc": "2.0", "id": id, "result": null });
                    let _ = write_message(&mut rstdin.lock().unwrap(), &resp);
                } else if let Some(tx) = rp.lock().unwrap().remove(&id) {
                    let _ = tx.send(msg);
                }
            } else if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                if method == "textDocument/publishDiagnostics" {
                    let _ = app.emit("lsp://diagnostics", msg.get("params").cloned());
                }
            }
        }
    });

    let handle = LspHandle {
        stdin,
        next_id,
        pending,
    };

    let init = json!({
        "processId": std::process::id(),
        "rootUri": root_uri,
        "capabilities": {
            "textDocument": {
                "hover": { "contentFormat": ["markdown", "plaintext"] },
                "completion": { "completionItem": { "snippetSupport": false } },
                "definition": {},
                "publishDiagnostics": {}
            }
        }
    });
    // A broken server (e.g. an uninstalled rustup proxy that exits at once)
    // never answers; treat that as "unavailable" rather than hanging.
    if request_blocking(&handle, "initialize", init, 6).is_err() {
        return Ok(false);
    }
    notify(&handle, "initialized", json!({}))?;

    state.0.lock().unwrap().insert(lang, handle);
    Ok(true)
}

#[tauri::command]
pub fn lsp_request(
    state: State<LspState>,
    lang: String,
    method: String,
    params: Value,
) -> Result<Value, String> {
    let map = state.0.lock().unwrap();
    let h = map.get(&lang).ok_or("lsp server not started")?;
    request_blocking(h, &method, params, 10)
}

#[tauri::command]
pub fn lsp_notify(
    state: State<LspState>,
    lang: String,
    method: String,
    params: Value,
) -> Result<(), String> {
    let map = state.0.lock().unwrap();
    let h = map.get(&lang).ok_or("lsp server not started")?;
    notify(h, &method, params)
}

/// Drop the handle for `lang`, closing its stdin so the server exits. Used by
/// the status indicator's restart action.
#[tauri::command]
pub fn lsp_stop(state: State<LspState>, lang: String) -> Result<(), String> {
    state.0.lock().unwrap().remove(&lang);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_framed_message() {
        let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":7}";
        let raw = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut r = Cursor::new(raw.into_bytes());
        let v = read_message(&mut r).expect("parse");
        assert_eq!(v["id"], 1);
        assert_eq!(v["result"], 7);
    }

    #[test]
    fn returns_none_on_eof() {
        let mut r = Cursor::new(b"" as &[u8]);
        assert!(read_message(&mut r).is_none());
    }
}
