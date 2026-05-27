pub mod bridge;
pub mod host;
pub mod manifest;
pub mod plugin;
pub mod sandbox;

pub mod api;

pub use host::PluginHost;
pub use manifest::Manifest;
pub use plugin::{Plugin, PluginState};

use std::sync::atomic::{AtomicU64, Ordering};

/// Opaque identifier for a loaded plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PluginId(pub u64);

impl PluginId {
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        PluginId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

/// A registered palette command from a plugin.
#[derive(Debug, Clone)]
pub struct PluginCommand {
    pub plugin_id: PluginId,
    pub name: String,
}

/// A registered keymap binding from a plugin.
#[derive(Debug, Clone)]
pub struct PluginKeymap {
    pub plugin_id: PluginId,
    pub chord: String,
    pub command_name: String,
}

/// A status-bar chip from a plugin.
#[derive(Debug, Clone)]
pub struct PluginChip {
    pub plugin_id: PluginId,
    pub chip_id: u64,
    pub text: String,
    pub position: ChipPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipPosition {
    Left,
    Right,
}

/// Events the host fans out to plugin worker threads.
#[derive(Debug, Clone)]
pub enum PluginEvent {
    Shutdown,
    InvokeCommand { name: String },
}

/// Errors produced by the plugin subsystem.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("manifest error: {0}")]
    Manifest(String),
    #[error("lua error: {0}")]
    Lua(#[from] mlua::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml error: {0}")]
    Toml(String),
}
