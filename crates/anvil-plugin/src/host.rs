use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread;

use crossbeam_channel::{Receiver, Sender, bounded};
use log::{info, warn};

use crate::bridge::{HostRequest, NotifyLevel};
use crate::plugin::{Plugin, PluginState};
use crate::{
    ChipPosition, Manifest, PluginChip, PluginCommand, PluginError, PluginEvent, PluginId,
    PluginKeymap,
};

/// Per-plugin worker handle kept by the host.
struct WorkerHandle {
    id: PluginId,
    #[allow(dead_code)]
    manifest: Manifest,
    event_tx: Sender<PluginEvent>,
    #[allow(dead_code)]
    state: PluginState,
}

/// Manages all loaded plugins. Lives on the app (main) thread.
pub struct PluginHost {
    workers: Vec<WorkerHandle>,
    /// Commands registered by all plugins. Snapshot updated on registration.
    commands: Vec<PluginCommand>,
    /// Keymaps registered by all plugins.
    keymaps: Vec<PluginKeymap>,
    /// Status-bar chips indexed by chip_id.
    chips: HashMap<u64, PluginChip>,
    /// Inbox: worker threads post `HostRequest`s here; host drains in `tick`.
    request_rx: Receiver<HostRequest>,
    request_tx: Sender<HostRequest>,
    /// Pending toasts from notify calls, drained by the app each tick.
    pub pending_toasts: Vec<(NotifyLevel, String)>,
}

impl PluginHost {
    pub fn new() -> Self {
        let (request_tx, request_rx) = bounded(256);
        PluginHost {
            workers: Vec::new(),
            commands: Vec::new(),
            keymaps: Vec::new(),
            chips: HashMap::new(),
            request_rx,
            request_tx,
            pending_toasts: Vec::new(),
        }
    }

    /// Discover and load plugins from `plugins_dir` (e.g. `~/.config/anvil/plugins`).
    /// Returns the number of successfully loaded plugins.
    pub fn discover_and_load(&mut self, plugins_dir: impl AsRef<Path>) -> usize {
        let dir = plugins_dir.as_ref();
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                info!("plugin discover: cannot read {}: {e}", dir.display());
                return 0;
            }
        };

        let mut loaded = 0;
        for entry in entries.flatten() {
            let plugin_dir = entry.path();
            if !plugin_dir.is_dir() {
                continue;
            }
            match self.load_plugin(plugin_dir) {
                Ok(()) => loaded += 1,
                Err(e) => {
                    warn!("plugin load failed: {e}");
                    self.pending_toasts
                        .push((NotifyLevel::Warn, format!("Plugin load failed: {e}")));
                }
            }
        }
        loaded
    }

    fn load_plugin(&mut self, plugin_dir: PathBuf) -> Result<(), PluginError> {
        let manifest = crate::manifest::load(&plugin_dir)?;
        let id = PluginId::next();
        let tx = self.request_tx.clone();

        let (event_tx, event_rx) = bounded::<PluginEvent>(64);

        let manifest_clone = manifest.clone();
        let dir_clone = plugin_dir.clone();

        thread::Builder::new()
            .name(format!("plugin-{}", manifest.name))
            .spawn(move || {
                run_plugin_worker(id, manifest_clone, dir_clone, tx, event_rx);
            })
            .map_err(PluginError::Io)?;

        self.workers.push(WorkerHandle {
            id,
            manifest,
            event_tx,
            state: PluginState::Loaded,
        });
        Ok(())
    }

    /// Drain pending `HostRequest`s from worker threads. Call once per frame.
    pub fn tick(&mut self) {
        while let Ok(req) = self.request_rx.try_recv() {
            self.handle_request(req);
        }
    }

    fn handle_request(&mut self, req: HostRequest) {
        match req {
            HostRequest::RegisterCommand { plugin_id, name } => {
                // Deduplicate: replace if same plugin+name.
                self.commands
                    .retain(|c| !(c.plugin_id == plugin_id && c.name == name));
                self.commands.push(PluginCommand { plugin_id, name });
            }
            HostRequest::RegisterKeymap {
                plugin_id,
                chord,
                command_name,
            } => {
                // Built-in wins: warn if the chord is already used by another plugin.
                let shadowed = self
                    .keymaps
                    .iter()
                    .any(|k| k.chord == chord && k.plugin_id != plugin_id);
                if shadowed {
                    warn!(
                        "plugin keymap '{chord}' shadows another plugin's binding; built-in wins"
                    );
                }
                self.keymaps
                    .retain(|k| !(k.plugin_id == plugin_id && k.chord == chord));
                self.keymaps.push(PluginKeymap {
                    plugin_id,
                    chord,
                    command_name,
                });
            }
            HostRequest::StatusbarAdd {
                plugin_id,
                chip_id,
                text,
                position,
            } => {
                self.chips.insert(
                    chip_id,
                    PluginChip {
                        plugin_id,
                        chip_id,
                        text,
                        position,
                    },
                );
            }
            HostRequest::StatusbarUpdate { chip_id, text } => {
                if let Some(chip) = self.chips.get_mut(&chip_id) {
                    chip.text = text;
                }
            }
            HostRequest::StatusbarRemove { chip_id } => {
                self.chips.remove(&chip_id);
            }
            HostRequest::Notify { level, msg } => {
                self.pending_toasts.push((level, msg));
            }
        }
    }

    /// Snapshot of all registered commands (for the command palette).
    pub fn commands(&self) -> Vec<PluginCommand> {
        self.commands.clone()
    }

    /// Snapshot of all status-bar chips (for the status bar renderer).
    pub fn statusbar_chips(&self) -> Vec<PluginChip> {
        let mut chips: Vec<PluginChip> = self.chips.values().cloned().collect();
        // Stable order: left chips first, then right, by chip_id within each group.
        chips.sort_by_key(|c| (c.position == ChipPosition::Right, c.chip_id));
        chips
    }

    /// Invoke a registered command by name. Sends `InvokeCommand` to the
    /// appropriate worker thread.
    pub fn invoke_command(&self, name: &str) {
        for cmd in &self.commands {
            if cmd.name == name {
                if let Some(w) = self.workers.iter().find(|w| w.id == cmd.plugin_id) {
                    let _ = w.event_tx.try_send(PluginEvent::InvokeCommand {
                        name: name.to_string(),
                    });
                }
                return;
            }
        }
    }

    /// Drain and return pending toasts, then clear the buffer.
    pub fn drain_toasts(&mut self) -> Vec<(NotifyLevel, String)> {
        std::mem::take(&mut self.pending_toasts)
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}

/// Worker thread: loads the plugin, then spins on the event channel.
fn run_plugin_worker(
    id: PluginId,
    manifest: Manifest,
    dir: PathBuf,
    host_tx: Sender<HostRequest>,
    event_rx: Receiver<PluginEvent>,
) {
    let mut plugin = match Plugin::load(manifest, dir, id, host_tx) {
        Ok(p) => p,
        Err(e) => {
            warn!("plugin {id:?} failed to load: {e}");
            return;
        }
    };

    // Event loop.
    while let Ok(event) = event_rx.recv() {
        match event {
            PluginEvent::Shutdown => break,
            PluginEvent::InvokeCommand { name } => {
                invoke_command_lua(&mut plugin, &name);
            }
        }
    }
}

/// Call the Lua callback registered for `name`. Tracks timeout strikes.
fn invoke_command_lua(plugin: &mut Plugin, name: &str) {
    // Retrieve the callback from the `anvil._callbacks` table by name.
    let result: mlua::Result<()> = (|| {
        let globals = plugin.lua.globals();
        let anvil: mlua::Table = globals.get("anvil")?;
        let callbacks: mlua::Table = anvil.get("_callbacks")?;
        let cb: mlua::Function = callbacks.get(name.to_string())?;
        cb.call::<(), ()>(())?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            plugin.timeout_strikes = 0;
        }
        Err(mlua::Error::RuntimeError(ref msg)) if msg.contains("budget exceeded") => {
            plugin.timeout_strikes += 1;
            warn!(
                "plugin {:?} command '{name}' timed out (strike {})",
                plugin.id, plugin.timeout_strikes
            );
            if plugin.timeout_strikes >= 3 {
                plugin.state = PluginState::Disabled("3 consecutive hook timeouts".to_string());
                warn!("plugin {:?} disabled after 3 timeouts", plugin.id);
            }
        }
        Err(e) => {
            warn!("plugin {:?} command '{name}' error: {e}", plugin.id);
        }
    }
}
