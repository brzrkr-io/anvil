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

/// Open `url` in a single reusable, persistent webview window keyed by `label`
/// (e.g. "grafana", "signoz"). If the window already exists, navigate it instead
/// of opening a new one — so the user logs in to Grafana/SSO ONCE and every
/// later dashboard reuses that authenticated session (cookies persist in the
/// shared WKWebsiteDataStore across windows + restarts).
#[tauri::command]
pub fn open_named_window(app: tauri::AppHandle, url: String, label: String) -> Result<(), String> {
    let u = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    if let Some(w) = app.get_webview_window(&label) {
        w.navigate(u).map_err(|e| e.to_string())?;
        let _ = w.set_focus();
        return Ok(());
    }
    let title = format!("Anvil — {label}");
    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(u))
        .title(title)
        .inner_size(1280.0, 860.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Swap the macOS window vibrancy material to match the active theme's light/dark
/// mode. Dark themes get a dark frosted material (HudWindow); light themes get
/// Sidebar. Without this, the fixed light "sidebar" material washes out dark
/// themes when window translucency is on. Called from the frontend on theme change.
#[tauri::command]
pub fn set_vibrancy(app: tauri::AppHandle, dark: bool) -> Result<(), String> {
    use tauri::window::{Effect, EffectsBuilder};
    let effect = if dark { Effect::HudWindow } else { Effect::Sidebar };
    for w in app.webview_windows().values() {
        let _ = w.set_effects(EffectsBuilder::new().effect(effect).build());
    }
    Ok(())
}
