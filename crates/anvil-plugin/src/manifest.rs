use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;

use crate::PluginError;

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestPlugin {
    pub name: String,
    pub version: String,
    pub description: String,
    pub api: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestEntry {
    pub lua: PathBuf,
}

/// Parsed and validated `plugin.toml`.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub api: String,
    pub entry: ManifestEntry,
}

/// Raw TOML shape for deserialization.
#[derive(Deserialize)]
struct RawManifest {
    plugin: ManifestPlugin,
    entry: ManifestEntry,
}

/// Load and validate a `plugin.toml` from `plugin_dir`.
pub fn load(plugin_dir: &Path) -> Result<Manifest, PluginError> {
    let toml_path = plugin_dir.join("plugin.toml");
    let raw = std::fs::read_to_string(&toml_path)?;
    let parsed: RawManifest =
        toml::from_str(&raw).map_err(|e| PluginError::Manifest(format!("toml parse: {e}")))?;

    let p = parsed.plugin;

    // Validate name regex.
    let name_re = Regex::new(r"^[a-z0-9-]{1,40}$").unwrap();
    if !name_re.is_match(&p.name) {
        return Err(PluginError::Manifest(format!(
            "name {:?} must match ^[a-z0-9-]{{1,40}}$",
            p.name
        )));
    }

    // Validate dir name == plugin name.
    if let Some(dir_name) = plugin_dir.file_name().and_then(|n| n.to_str()) {
        if dir_name != p.name {
            return Err(PluginError::Manifest(format!(
                "plugin name {:?} must match directory name {:?}",
                p.name, dir_name
            )));
        }
    }

    // Validate api major == 1.
    let major: u32 = p
        .api
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| PluginError::Manifest(format!("api {:?} is not a valid version", p.api)))?;
    if major != 1 {
        return Err(PluginError::Manifest(format!(
            "api major {major} != host major 1"
        )));
    }

    // Validate entry.lua exists inside the plugin dir.
    let entry_path = plugin_dir.join(&parsed.entry.lua);
    if !entry_path.exists() {
        return Err(PluginError::Manifest(format!(
            "entry.lua {:?} does not exist",
            entry_path
        )));
    }
    // Ensure it doesn't escape the plugin dir.
    let canonical_entry = entry_path.canonicalize()?;
    let canonical_dir = plugin_dir.canonicalize()?;
    if !canonical_entry.starts_with(&canonical_dir) {
        return Err(PluginError::Manifest(
            "entry.lua must be inside the plugin directory".to_string(),
        ));
    }

    Ok(Manifest {
        name: p.name,
        version: p.version,
        description: p.description,
        api: p.api,
        entry: parsed.entry,
    })
}
