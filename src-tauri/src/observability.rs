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

/// Loki instant LogQL query (#56). Native HTTP, no_proxy.
#[tauri::command]
pub async fn loki_query(base: String, query: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/loki/api/v1/query", base.trim_end_matches('/'));
    let r = client
        .get(url)
        .query(&[("query", query.as_str()), ("limit", "200")])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    r.text().await.map_err(|e| e.to_string())
}
