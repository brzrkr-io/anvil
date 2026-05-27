use crossbeam_channel::Sender;
use mlua::{Lua, Table};

use crate::bridge::HostRequest;
use crate::{PluginError, PluginId};

/// Install `anvil.command(name, handler)`.
///
/// On call: stores the handler in `anvil._callbacks[name]` (a Lua table on
/// the anvil global) and sends `RegisterCommand` to the host. When the host
/// wants to fire the command it sends `InvokeCommand`; the worker thread
/// retrieves the callback from `_callbacks` and calls it.
pub fn install(
    lua: &Lua,
    anvil: &Table,
    plugin_id: PluginId,
    tx: Sender<HostRequest>,
) -> Result<(), PluginError> {
    // `_callbacks` table: name → Lua function.
    let callbacks: Table = lua.create_table()?;
    anvil.set("_callbacks", callbacks)?;

    let f = lua.create_function(move |lua, (name, handler): (String, mlua::Function)| {
        // Store the callback in anvil._callbacks[name].
        let globals = lua.globals();
        let anvil_tbl: Table = globals.get("anvil")?;
        let cbs: Table = anvil_tbl.get("_callbacks")?;
        cbs.set(name.clone(), handler)?;
        super::send(&tx, HostRequest::RegisterCommand { plugin_id, name })?;
        Ok(())
    })?;
    anvil.set("command", f)?;
    Ok(())
}
