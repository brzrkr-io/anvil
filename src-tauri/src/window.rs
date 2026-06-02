use tauri::Manager;

/// Open a new top-level app window (⌘N). An optional `seed` (URL-encoded JSON,
/// built by the frontend) detaches a pane into the new window via a `?detach=`
/// query param (#17); the detached window seeds from it and skips state restore.
#[tauri::command]
pub fn new_window(app: tauri::AppHandle, seed: Option<String>) -> Result<(), String> {
    let label = format!("w{}", app.webview_windows().len() + 1);
    let path = match seed {
        Some(s) if !s.is_empty() => format!("index.html?detach={s}"),
        _ => "index.html".to_string(),
    };
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::App(path.into()))
        .title("Anvil")
        .inner_size(1280.0, 820.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Open an external URL (e.g. a Grafana dashboard) in a native webview window.
/// X-Frame-Options only blocks *framing*, so a top-level webview loads fine —
/// this is the iframe-free Grafana fix (#73, option a).
#[tauri::command]
pub fn open_url_window(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let u = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    let label = format!("ext{}", app.webview_windows().len() + 1);
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::External(u))
        .title("Anvil — Dashboard")
        .inner_size(1280.0, 860.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}
