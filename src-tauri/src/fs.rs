#[derive(serde::Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// List a directory: directories first, then files, both alphabetical.
/// Hidden entries (dot-prefixed) are skipped.
#[tauri::command]
pub fn list_dir(path: String) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    for ent in std::fs::read_dir(&path)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        out.push(Entry {
            path: ent.path().to_string_lossy().into_owned(),
            name,
            is_dir,
        });
    }
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    Ok(out)
}

#[tauri::command]
pub fn read_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn write_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".svelte-kit",
    "build",
    "dist",
    ".cache",
    ".zig-cache",
    "zig-out",
    ".next",
    "vendor",
    "Pods",
    ".venv",
    "__pycache__",
];

fn walk(root: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>, cap: usize) {
    if out.len() >= cap {
        return;
    }
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for ent in rd.flatten() {
        if out.len() >= cap {
            return;
        }
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && name != ".env" {
            continue;
        }
        let p = ent.path();
        let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(&p, base, out, cap);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push(rel.to_string_lossy().into_owned());
        }
    }
}

/// Recursively list workspace files (relative paths) for the fuzzy finder,
/// skipping VCS/build dirs. Capped to keep it snappy on huge trees.
#[tauri::command]
pub fn walk_dir(root: String) -> Vec<String> {
    let base = std::path::PathBuf::from(&root);
    let mut out = Vec::new();
    walk(&base, &base, &mut out, 20000);
    out
}

#[tauri::command]
pub fn create_path(path: String, is_dir: bool) -> Result<(), String> {
    if is_dir {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())
    } else {
        if let Some(p) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(p);
        }
        std::fs::write(&path, "").map_err(|e| e.to_string())
    }
}

/// Last-modified time (unix seconds) for an open editor file's external-change
/// polling. Errors (missing file) map to 0.
#[tauri::command]
pub fn file_mtime(path: String) -> u64 {
    std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[tauri::command]
pub fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

/// Native folder picker (File ▸ Open Folder…). Returns the chosen path or null.
#[tauri::command]
pub fn pick_folder(start: Option<String>) -> Option<String> {
    let mut d = rfd::FileDialog::new();
    if let Some(s) = start.filter(|s| !s.is_empty()) {
        d = d.set_directory(s);
    }
    d.pick_folder().map(|p| p.to_string_lossy().into_owned())
}

/// Native file picker (File ▸ Open File…). Returns the chosen path or null.
#[tauri::command]
pub fn pick_file(start: Option<String>) -> Option<String> {
    let mut d = rfd::FileDialog::new();
    if let Some(s) = start.filter(|s| !s.is_empty()) {
        d = d.set_directory(s);
    }
    d.pick_file().map(|p| p.to_string_lossy().into_owned())
}
