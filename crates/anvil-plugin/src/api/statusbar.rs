use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_channel::Sender;
use mlua::{Lua, Table};

use crate::bridge::HostRequest;
use crate::{ChipPosition, PluginError, PluginId};

static CHIP_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_chip_id() -> u64 {
    CHIP_COUNTER.fetch_add(1, Ordering::Relaxed)
}

const MAX_TEXT_LEN: usize = 64;

/// Install `anvil.statusbar.add / update / remove`.
pub fn install(
    lua: &Lua,
    anvil: &Table,
    plugin_id: PluginId,
    tx: Sender<HostRequest>,
) -> Result<(), PluginError> {
    let bar = lua.create_table()?;

    // anvil.statusbar.add(text, position) -> chip_id
    {
        let tx2 = tx.clone();
        let add = lua.create_function(move |_lua, (text, position): (String, String)| {
            let text = if text.chars().count() > MAX_TEXT_LEN {
                text.chars().take(MAX_TEXT_LEN).collect()
            } else {
                text
            };
            let pos = parse_position(&position)?;
            let chip_id = next_chip_id();
            super::send(
                &tx2,
                HostRequest::StatusbarAdd {
                    plugin_id,
                    chip_id,
                    text,
                    position: pos,
                },
            )?;
            Ok(chip_id)
        })?;
        bar.set("add", add)?;
    }

    // anvil.statusbar.update(chip_id, text)
    {
        let tx2 = tx.clone();
        let update = lua.create_function(move |_lua, (chip_id, text): (u64, String)| {
            let text = if text.chars().count() > MAX_TEXT_LEN {
                text.chars().take(MAX_TEXT_LEN).collect()
            } else {
                text
            };
            super::send(&tx2, HostRequest::StatusbarUpdate { chip_id, text })?;
            Ok(())
        })?;
        bar.set("update", update)?;
    }

    // anvil.statusbar.remove(chip_id)
    {
        let tx2 = tx;
        let remove = lua.create_function(move |_lua, chip_id: u64| {
            super::send(&tx2, HostRequest::StatusbarRemove { chip_id })?;
            Ok(())
        })?;
        bar.set("remove", remove)?;
    }

    anvil.set("statusbar", bar)?;
    Ok(())
}

fn parse_position(s: &str) -> mlua::Result<ChipPosition> {
    match s {
        "left" => Ok(ChipPosition::Left),
        "right" => Ok(ChipPosition::Right),
        other => Err(mlua::Error::RuntimeError(format!(
            "statusbar.add: position must be 'left' or 'right', got '{other}'"
        ))),
    }
}
