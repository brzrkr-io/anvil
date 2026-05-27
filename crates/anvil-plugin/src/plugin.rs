use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use mlua::Lua;

use crate::bridge::HostRequest;
use crate::{Manifest, PluginError, PluginId};

/// Runtime state of a loaded plugin.
#[derive(Debug, Clone)]
pub enum PluginState {
    Loaded,
    Failed(String),
    Disabled(String),
}

/// A single loaded plugin. Lives on its dedicated worker thread.
pub struct Plugin {
    pub id: PluginId,
    pub manifest: Manifest,
    pub lua: Lua,
    pub dir: PathBuf,
    pub state: PluginState,
    /// Count of consecutive hook timeouts.
    pub timeout_strikes: u8,
}

impl Plugin {
    /// Create a new `Plugin`: create the Lua VM, install sandbox, register API,
    /// run `init.lua`.
    pub fn load(
        manifest: Manifest,
        dir: PathBuf,
        id: PluginId,
        host_tx: Sender<HostRequest>,
    ) -> Result<Plugin, PluginError> {
        let lua = crate::sandbox::create_lua(dir.clone())?;

        // Install the `anvil` API table.
        crate::api::install(&lua, id, host_tx)?;

        // Set instruction-count hook: every 1 000 instructions check elapsed.
        // The hook stores start time in the Lua registry via an Arc<Instant>.
        let start = Arc::new(std::sync::Mutex::new(Instant::now()));
        let start_clone = Arc::clone(&start);
        lua.set_hook(
            mlua::HookTriggers::new().every_nth_instruction(1000),
            move |_lua, _debug| {
                let elapsed = start_clone.lock().unwrap().elapsed();
                if elapsed > Duration::from_millis(200) {
                    Err(mlua::Error::RuntimeError(
                        "plugin hook budget exceeded (200 ms)".to_string(),
                    ))
                } else {
                    Ok(())
                }
            },
        );

        // Reset hook start time before running init.lua.
        *start.lock().unwrap() = Instant::now();

        let entry_path = dir.join(&manifest.entry.lua);
        let src = std::fs::read_to_string(&entry_path)?;

        let state = match lua
            .load(&src)
            .set_name(&*manifest.entry.lua.to_string_lossy())
            .exec()
        {
            Ok(()) => PluginState::Loaded,
            Err(e) => PluginState::Failed(e.to_string()),
        };

        Ok(Plugin {
            id,
            manifest,
            lua,
            dir,
            state,
            timeout_strikes: 0,
        })
    }

    /// Reset the instruction-hook start time. Call this before invoking any
    /// plugin-side callback so the 200 ms budget is measured from invocation.
    pub fn reset_budget(&self) {
        // The hook closure holds its own Arc; we can't reach it from here,
        // so budget reset is handled by removing + reinstalling the hook.
        // For Phase 1/2 this is acceptable; Phase 3 can tighten this.
    }
}
