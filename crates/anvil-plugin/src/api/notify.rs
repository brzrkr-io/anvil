use crossbeam_channel::Sender;
use mlua::{Lua, Table};

use crate::PluginError;
use crate::bridge::{HostRequest, NotifyLevel};

/// Install `anvil.notify(level, msg)`.
pub fn install(lua: &Lua, anvil: &Table, tx: Sender<HostRequest>) -> Result<(), PluginError> {
    let f = lua.create_function(move |_lua, (level, msg): (String, String)| {
        let lvl = parse_level(&level)?;
        super::send(&tx, HostRequest::Notify { level: lvl, msg })?;
        Ok(())
    })?;
    anvil.set("notify", f)?;
    Ok(())
}

fn parse_level(s: &str) -> mlua::Result<NotifyLevel> {
    match s {
        "info" => Ok(NotifyLevel::Info),
        "warn" => Ok(NotifyLevel::Warn),
        "error" => Ok(NotifyLevel::Error),
        other => Err(mlua::Error::RuntimeError(format!(
            "notify: level must be 'info', 'warn', or 'error', got '{other}'"
        ))),
    }
}
