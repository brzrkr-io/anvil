pub mod command;
pub mod keymap;
pub mod notify;
pub mod statusbar;

use crossbeam_channel::Sender;
use mlua::Lua;

use crate::bridge::HostRequest;
use crate::{PluginError, PluginId};

/// Install the top-level `anvil` table and all sub-namespaces into `lua`.
pub fn install(lua: &Lua, plugin_id: PluginId, tx: Sender<HostRequest>) -> Result<(), PluginError> {
    let anvil = lua.create_table()?;

    // anvil.command(name, handler)
    command::install(lua, &anvil, plugin_id, tx.clone())?;

    // anvil.keymap(chord, command_name)
    keymap::install(lua, &anvil, plugin_id, tx.clone())?;

    // anvil.statusbar.*
    statusbar::install(lua, &anvil, plugin_id, tx.clone())?;

    // anvil.notify(level, msg)
    notify::install(lua, &anvil, tx)?;

    lua.globals().set("anvil", anvil)?;
    Ok(())
}

/// Helper: send a `HostRequest` from inside a Lua function, converting channel
/// errors to `mlua::Error`.
pub(crate) fn send(tx: &Sender<HostRequest>, req: HostRequest) -> mlua::Result<()> {
    tx.send(req)
        .map_err(|e| mlua::Error::RuntimeError(format!("host channel closed: {e}")))
}
