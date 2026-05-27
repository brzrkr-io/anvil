/// Requests posted by plugin worker threads to the app thread.
#[derive(Debug)]
pub enum HostRequest {
    /// Register a palette command.
    RegisterCommand {
        plugin_id: crate::PluginId,
        name: String,
    },
    /// Register a keymap binding.
    RegisterKeymap {
        plugin_id: crate::PluginId,
        chord: String,
        command_name: String,
    },
    /// Add a status-bar chip; the u64 is the chip_id.
    StatusbarAdd {
        plugin_id: crate::PluginId,
        chip_id: u64,
        text: String,
        position: crate::ChipPosition,
    },
    /// Update a status-bar chip text.
    StatusbarUpdate { chip_id: u64, text: String },
    /// Remove a status-bar chip.
    StatusbarRemove { chip_id: u64 },
    /// Show a toast notification.
    Notify { level: NotifyLevel, msg: String },
}

#[derive(Debug, Clone, Copy)]
pub enum NotifyLevel {
    Info,
    Warn,
    Error,
}

/// Responses from the app thread back to a plugin worker (currently only
/// needed for blocking calls like `editor.*` and `fs.*` — Phase 3+4).
#[derive(Debug)]
pub enum HostResponse {
    Ok,
}
