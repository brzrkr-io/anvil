use tauri::ipc::Channel;

const LLM_BASE: &str = "http://localhost:1234/v1";

/// Resolve the OpenAI-compatible base URL: caller-provided (from Accounts) or
/// the local LM Studio default. Trailing slash trimmed.
fn llm_base(base: &Option<String>) -> String {
    match base {
        Some(b) if !b.trim().is_empty() => b.trim().trim_end_matches('/').to_string(),
        _ => LLM_BASE.to_string(),
    }
}

/// reqwest client that never routes loopback through a system/corporate proxy.
fn llm_client() -> &'static reqwest::Client {
    crate::shared::http()
}

/// List models from an OpenAI-compatible server. `base`/`api_key` come from the
/// Accounts settings; both optional (local LM Studio needs neither). Proxied
/// through Rust to dodge the webview's http/CORS/ATS restrictions.
#[tauri::command]
pub async fn llm_models(
    base: Option<String>,
    api_key: Option<String>,
) -> Result<Vec<String>, String> {
    let mut req = llm_client().get(format!("{}/models", llm_base(&base)));
    if let Some(k) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let r = req.send().await.map_err(|e| e.to_string())?;
    let j: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    Ok(j["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

/// One-shot chat completion. `messages` is the OpenAI-format array
/// `[{role, content}, …]`. `base`/`api_key` from Accounts (both optional).
#[tauri::command]
pub async fn llm_chat(
    model: String,
    messages: serde_json::Value,
    base: Option<String>,
    api_key: Option<String>,
) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model, "messages": messages, "temperature": 0.4, "stream": false
    });
    let mut req = llm_client()
        .post(format!("{}/chat/completions", llm_base(&base)))
        .json(&body);
    if let Some(k) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let r = req.send().await.map_err(|e| e.to_string())?;
    let j: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    Ok(j["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

/// Streaming chat (#52): emits each content delta over `on_token` as it arrives
/// (OpenAI-compatible SSE). Goes through the Rust client so `.no_proxy()` still
/// applies (a frontend fetch would hit the corporate proxy on localhost).
#[tauri::command]
pub async fn llm_chat_stream(
    model: String,
    messages: serde_json::Value,
    base: Option<String>,
    api_key: Option<String>,
    on_token: Channel<String>,
) -> Result<(), String> {
    use futures_util::StreamExt;
    let body = serde_json::json!({
        "model": model, "messages": messages, "temperature": 0.4, "stream": true
    });
    let mut req = llm_client()
        .post(format!("{}/chat/completions", llm_base(&base)))
        .json(&body);
    if let Some(k) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(nl) = buf.find('\n') {
            let line = buf[..nl].trim().to_string();
            buf.drain(..=nl);
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    return Ok(());
                }
                if let Ok(j) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(tok) = j["choices"][0]["delta"]["content"].as_str() {
                        if !tok.is_empty() {
                            let _ = on_token.send(tok.to_string());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
