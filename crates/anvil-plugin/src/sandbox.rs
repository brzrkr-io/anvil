use std::path::PathBuf;

use mlua::{Lua, LuaOptions, StdLib};

use crate::PluginError;

/// Create a new sandboxed `Lua` state with a 32 MiB memory limit and
/// dangerous globals stripped. The plugin `dir` is used to constrain
/// the `require` shim.
pub fn create_lua(dir: PathBuf) -> Result<Lua, PluginError> {
    // Open only safe standard libraries; skip io, os, package, debug, ffi.
    let libs = StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8;
    let lua = Lua::new_with(libs, LuaOptions::default())?;

    // 32 MiB memory limit.
    lua.set_memory_limit(32 << 20)?;

    install(&lua, dir)?;
    Ok(lua)
}

/// Strip dangerous globals from an already-created `Lua` state and install
/// the safe subset of `os` plus a constrained `require` shim.
pub fn install(lua: &Lua, plugin_dir: PathBuf) -> Result<(), PluginError> {
    let globals = lua.globals();

    // Remove dangerous top-level globals.
    for name in &["io", "dofile", "loadfile", "debug"] {
        globals.set(*name, mlua::Value::Nil)?;
    }

    // Rebuild `os`: keep only date / time / clock.
    {
        let os_table = lua.create_table()?;
        let base_os: mlua::Table = globals
            .get("os")
            .unwrap_or_else(|_| lua.create_table().unwrap());
        for keep in &["date", "time", "clock"] {
            let v: mlua::Value = base_os.get(*keep).unwrap_or(mlua::Value::Nil);
            os_table.set(*keep, v)?;
        }
        globals.set("os", os_table)?;
    }

    // Remove package.loadlib if package exists.
    let pkg_result: mlua::Result<mlua::Table> = globals.get("package");
    if let Ok(pkg) = pkg_result {
        pkg.set("loadlib", mlua::Value::Nil)?;
    }

    // Replace `require` with a shim that only allows modules under plugin_dir.
    let dir_clone = plugin_dir.clone();
    let require_shim = lua.create_function(move |lua, module: String| {
        // Reject anything that looks like a system module (no '.' prefix needed).
        // Allow only relative-style names that resolve to a .lua file in plugin_dir.
        let file_name = module.replace('.', "/");
        let candidate = dir_clone.join(format!("{file_name}.lua"));
        if !candidate.exists() {
            return Err(mlua::Error::RuntimeError(format!(
                "require: module '{module}' not found in plugin directory"
            )));
        }
        // Safety: we already checked it exists; if canonicalize fails treat as reject.
        let canonical = candidate.canonicalize().map_err(|_| {
            mlua::Error::RuntimeError(format!("require: cannot resolve '{module}'"))
        })?;
        let dir_canonical = dir_clone.canonicalize().map_err(|_| {
            mlua::Error::RuntimeError("require: cannot resolve plugin dir".to_string())
        })?;
        if !canonical.starts_with(&dir_canonical) {
            return Err(mlua::Error::RuntimeError(format!(
                "require: module '{module}' escapes plugin directory"
            )));
        }
        let src = std::fs::read_to_string(&canonical).map_err(|e| {
            mlua::Error::RuntimeError(format!("require: cannot read '{module}': {e}"))
        })?;
        let chunk = lua.load(&src);
        chunk.eval::<mlua::MultiValue>()
    })?;
    globals.set("require", require_shim)?;

    Ok(())
}
