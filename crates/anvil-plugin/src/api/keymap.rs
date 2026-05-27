use crossbeam_channel::Sender;
use mlua::{Lua, Table};

use crate::bridge::HostRequest;
use crate::{PluginError, PluginId};

/// Install `anvil.keymap(chord, command_name)`.
pub fn install(
    lua: &Lua,
    anvil: &Table,
    plugin_id: PluginId,
    tx: Sender<HostRequest>,
) -> Result<(), PluginError> {
    let f = lua.create_function(move |_lua, (chord, command_name): (String, String)| {
        super::send(
            &tx,
            HostRequest::RegisterKeymap {
                plugin_id,
                chord,
                command_name,
            },
        )?;
        Ok(())
    })?;
    anvil.set("keymap", f)?;
    Ok(())
}
