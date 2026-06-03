/// Instant Prometheus query (#77) — native HTTP, not an iframe. Returns the raw
/// JSON from `/api/v1/query`. `no_proxy` so it works behind the corporate proxy.
#[tauri::command]
pub async fn prom_query(base: String, query: String) -> Result<String, String> {
    let client = crate::shared::http();
    let url = format!("{}/api/v1/query", base.trim_end_matches('/'));
    let r = client
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .query(&[("query", query.as_str())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    r.text().await.map_err(|e| e.to_string())
}

/// Range Prometheus query (#55) for sparklines — `/api/v1/query_range` over the
/// last `minutes`, with a step sized to ~60 points. Returns raw JSON. `no_proxy`.
#[tauri::command]
pub async fn prom_query_range(base: String, query: String, minutes: u64) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let span = minutes.max(1) * 60;
    let start = now.saturating_sub(span);
    let step = (span / 60).max(15);
    let client = crate::shared::http();
    let url = format!("{}/api/v1/query_range", base.trim_end_matches('/'));
    let r = client
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
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

fn http_client() -> &'static reqwest::Client {
    crate::shared::http()
}

/// Walk the full error-source chain so transport failures show the ROOT cause
/// (TLS handshake / DNS / connection refused / proxy) instead of reqwest's
/// generic "error sending request for url (…)".
fn req_err(e: reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(inner) = src {
        s.push_str(" → ");
        s.push_str(&inner.to_string());
        src = inner.source();
    }
    s
}

/// List Grafana dashboards via the search API. `token` is a Grafana API token /
/// service-account token (Bearer). Returns the raw JSON array of dashboards
/// (title, uid, url, folderTitle, tags…). `no_proxy`.
#[tauri::command]
pub async fn grafana_dashboards(base: String, token: String) -> Result<String, String> {
    let url = format!(
        "{}/api/search?type=dash-db&limit=1000",
        base.trim_end_matches('/')
    );
    let mut req = http_client()
        .get(url)
        .timeout(std::time::Duration::from_secs(20));
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let r = req.send().await.map_err(req_err)?;
    if !r.status().is_success() {
        return Err(format!(
            "grafana {}: {}",
            r.status(),
            r.text().await.unwrap_or_default()
        ));
    }
    r.text().await.map_err(|e| e.to_string())
}

/// `SigNoz` query — POST a builder query to `/api/v3/query_range`. `body` is the
/// JSON request the frontend builds (logs/traces/metrics); `api_key` is a `SigNoz`
/// API key (SIGNOZ-API-KEY header). Returns raw JSON. `no_proxy`.
#[tauri::command]
pub async fn signoz_query(base: String, api_key: String, body: String) -> Result<String, String> {
    let url = format!("{}/api/v3/query_range", base.trim_end_matches('/'));
    let mut req = http_client()
        .post(url)
        .timeout(std::time::Duration::from_secs(20))
        .header("Content-Type", "application/json")
        .body(body);
    if !api_key.is_empty() {
        req = req.header("SIGNOZ-API-KEY", api_key);
    }
    let r = req.send().await.map_err(req_err)?;
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

/// `SigNoz` services overview — POST `/api/v1/services` with a `{start,end,tags}`
/// window (last `mins`, epoch nanoseconds). Returns the raw JSON array of
/// services (serviceName, p99, errorRate, callRate…). `no_proxy`.
#[tauri::command]
pub async fn signoz_services(base: String, api_key: String, mins: u64) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let span = mins.max(1) * 60;
    // SigNoz's GetServicesParams wants start/end as STRING epoch nanoseconds.
    let body = serde_json::json!({
        "start": ((now.saturating_sub(span)) * 1_000_000_000u64).to_string(),
        "end": (now * 1_000_000_000u64).to_string(),
        "tags": [],
    })
    .to_string();
    let url = format!("{}/api/v1/services", base.trim_end_matches('/'));
    let mut req = http_client()
        .post(url)
        .timeout(std::time::Duration::from_secs(20))
        .header("Content-Type", "application/json")
        .body(body);
    if !api_key.is_empty() {
        req = req.header("SIGNOZ-API-KEY", api_key);
    }
    let r = req.send().await.map_err(req_err)?;
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

/// I85 Sentry — recent unresolved issues for a project. Returns raw JSON from the
/// Sentry API. Token + org/project supplied by the caller (stored in Keychain).
#[tauri::command]
pub async fn sentry_issues(
    base: String,
    org: String,
    project: String,
    token: String,
) -> Result<String, String> {
    let client = crate::shared::http();
    let host = if base.trim().is_empty() {
        "https://sentry.io".to_string()
    } else {
        base.trim_end_matches('/').to_string()
    };
    let url = format!(
        "{host}/api/0/projects/{org}/{project}/issues/?query=is:unresolved&statsPeriod=14d"
    );
    let r = client
        .get(url)
        .timeout(std::time::Duration::from_secs(20))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("sentry {}", r.status()));
    }
    r.text().await.map_err(|e| e.to_string())
}

/// I88 Slack — post a plain-text message to an incoming-webhook URL.
#[tauri::command]
pub async fn slack_post(webhook: String, text: String) -> Result<(), String> {
    let client = crate::shared::http();
    let r = client
        .post(webhook)
        .timeout(std::time::Duration::from_secs(10))
        .json(&serde_json::json!({ "text": text }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status().is_success() {
        Ok(())
    } else {
        Err(format!("slack {}", r.status()))
    }
}
