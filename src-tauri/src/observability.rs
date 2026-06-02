/// Instant Prometheus query (#77) — native HTTP, not an iframe. Returns the raw
/// JSON from `/api/v1/query`. no_proxy so it works behind the corporate proxy.
#[tauri::command]
pub async fn prom_query(base: String, query: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/api/v1/query", base.trim_end_matches('/'));
    let r = client
        .get(url)
        .query(&[("query", query.as_str())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    r.text().await.map_err(|e| e.to_string())
}

/// Range Prometheus query (#55) for sparklines — `/api/v1/query_range` over the
/// last `minutes`, with a step sized to ~60 points. Returns raw JSON. no_proxy.
#[tauri::command]
pub async fn prom_query_range(base: String, query: String, minutes: u64) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let span = minutes.max(1) * 60;
    let start = now.saturating_sub(span);
    let step = (span / 60).max(15);
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/api/v1/query_range", base.trim_end_matches('/'));
    let r = client
        .get(url)
        .query(&[
            ("query", query.as_str()),
            ("start", start.to_string().as_str()),
            ("end", now.to_string().as_str()),
            ("step", step.to_string().as_str()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    r.text().await.map_err(|e| e.to_string())
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// List Grafana dashboards via the search API. `token` is a Grafana API token /
/// service-account token (Bearer). Returns the raw JSON array of dashboards
/// (title, uid, url, folderTitle, tags…). no_proxy.
#[tauri::command]
pub async fn grafana_dashboards(base: String, token: String) -> Result<String, String> {
    let url = format!(
        "{}/api/search?type=dash-db&limit=1000",
        base.trim_end_matches('/')
    );
    let mut req = http_client()?.get(url);
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let r = req.send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!(
            "grafana {}: {}",
            r.status(),
            r.text().await.unwrap_or_default()
        ));
    }
    r.text().await.map_err(|e| e.to_string())
}

/// SigNoz query — POST a builder query to `/api/v3/query_range`. `body` is the
/// JSON request the frontend builds (logs/traces/metrics); `api_key` is a SigNoz
/// API key (SIGNOZ-API-KEY header). Returns raw JSON. no_proxy.
#[tauri::command]
pub async fn signoz_query(base: String, api_key: String, body: String) -> Result<String, String> {
    let url = format!("{}/api/v3/query_range", base.trim_end_matches('/'));
    let mut req = http_client()?
        .post(url)
        .header("Content-Type", "application/json")
        .body(body);
    if !api_key.is_empty() {
        req = req.header("SIGNOZ-API-KEY", api_key);
    }
    let r = req.send().await.map_err(|e| e.to_string())?;
    let status = r.status();
    let text = r.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "signoz {status}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(text)
}

/// SigNoz services overview — GET `/api/v1/services` (last 5m). Simple, robust
/// across SigNoz versions; good for a default landing view. no_proxy.
#[tauri::command]
pub async fn signoz_services(base: String, api_key: String) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let url = format!(
        "{}/api/v1/services?start={}&end={}",
        base.trim_end_matches('/'),
        (now.saturating_sub(300)) * 1_000_000_000,
        now * 1_000_000_000,
    );
    let mut req = http_client()?.get(url);
    if !api_key.is_empty() {
        req = req.header("SIGNOZ-API-KEY", api_key);
    }
    let r = req.send().await.map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!(
            "signoz {}: {}",
            r.status(),
            r.text().await.unwrap_or_default()
        ));
    }
    r.text().await.map_err(|e| e.to_string())
}
