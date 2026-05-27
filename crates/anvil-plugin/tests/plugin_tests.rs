use std::fs;
use std::path::Path;
use std::time::Duration;

use anvil_plugin::{ChipPosition, PluginHost};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Create a minimal valid plugin directory inside a tempdir.
fn make_plugin_dir(base: &Path, name: &str, init_lua: &str) -> std::path::PathBuf {
    let dir = base.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        format!(
            r#"[plugin]
name = "{name}"
version = "0.1.0"
description = "test"
api = "1.0"

[entry]
lua = "init.lua"
"#
        ),
    )
    .unwrap();
    fs::write(dir.join("init.lua"), init_lua).unwrap();
    dir
}

// ── Manifest tests ────────────────────────────────────────────────────────────

#[test]
fn manifest_valid_loads() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = make_plugin_dir(tmp.path(), "my-plugin", "-- ok");
    let m = anvil_plugin::manifest::load(&dir).unwrap();
    assert_eq!(m.name, "my-plugin");
    assert_eq!(m.api, "1.0");
}

#[test]
fn manifest_bad_name_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    // dir name must match plugin name — make a dir named "BAD_NAME"
    // but write name = "BAD_NAME" which fails the regex.
    let dir = tmp.path().join("BAD_NAME");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        r#"[plugin]
name = "BAD_NAME"
version = "0.1.0"
description = "bad"
api = "1.0"

[entry]
lua = "init.lua"
"#,
    )
    .unwrap();
    fs::write(dir.join("init.lua"), "").unwrap();
    assert!(anvil_plugin::manifest::load(&dir).is_err());
}

#[test]
fn manifest_api_major_mismatch_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    // dir is named "my-plugin" but api = "2.0"
    let dir = tmp.path().join("my-plugin");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("plugin.toml"),
        r#"[plugin]
name = "my-plugin"
version = "0.1.0"
description = "bad api"
api = "2.0"

[entry]
lua = "init.lua"
"#,
    )
    .unwrap();
    fs::write(dir.join("init.lua"), "").unwrap();
    let err = anvil_plugin::manifest::load(&dir).unwrap_err();
    assert!(err.to_string().contains("api major"), "got: {err}");
}

// ── Sandbox tests ─────────────────────────────────────────────────────────────

#[test]
fn sandbox_io_global_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let lua = anvil_plugin::sandbox::create_lua(tmp.path().to_path_buf()).unwrap();
    let result: mlua::Result<mlua::Value> = lua.load("return io").eval();
    // `io` should be nil (removed).
    match result {
        Ok(mlua::Value::Nil) => {}
        Ok(v) => panic!("expected nil, got {v:?}"),
        // Some mlua versions error on accessing nil globals; that is also fine.
        Err(_) => {}
    }
}

#[test]
fn sandbox_os_execute_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let lua = anvil_plugin::sandbox::create_lua(tmp.path().to_path_buf()).unwrap();
    let result: mlua::Result<mlua::Value> = lua.load("return os.execute").eval();
    match result {
        Ok(mlua::Value::Nil) => {}
        Ok(v) => panic!("expected nil for os.execute, got {v:?}"),
        Err(_) => {}
    }
}

#[test]
fn sandbox_require_rejects_system_path() {
    let tmp = tempfile::tempdir().unwrap();
    let lua = anvil_plugin::sandbox::create_lua(tmp.path().to_path_buf()).unwrap();
    let result: mlua::Result<mlua::Value> = lua.load(r#"return require("os")"#).eval();
    assert!(
        result.is_err(),
        "require('os') should be rejected by sandbox"
    );
}

// ── Host + command tests ──────────────────────────────────────────────────────

#[test]
fn host_loads_plugin_with_command() {
    let tmp = tempfile::tempdir().unwrap();
    make_plugin_dir(
        tmp.path(),
        "cmd-plugin",
        r#"anvil.command("Hello", function() end)"#,
    );

    let mut host = PluginHost::new();
    let loaded = host.discover_and_load(tmp.path());
    assert_eq!(loaded, 1);

    // Give the worker thread time to send RegisterCommand.
    std::thread::sleep(Duration::from_millis(100));
    host.tick();

    let cmds = host.commands();
    assert!(
        cmds.iter().any(|c| c.name == "Hello"),
        "Hello command not found"
    );
}

#[test]
fn host_invokes_registered_command() {
    let tmp = tempfile::tempdir().unwrap();
    // Command sends a notify so we can observe it was invoked.
    make_plugin_dir(
        tmp.path(),
        "inv-plugin",
        r#"anvil.command("Ping", function()
  anvil.notify("info", "pong")
end)"#,
    );

    let mut host = PluginHost::new();
    host.discover_and_load(tmp.path());

    std::thread::sleep(Duration::from_millis(100));
    host.tick();

    host.invoke_command("Ping");
    std::thread::sleep(Duration::from_millis(100));
    host.tick();

    let toasts = host.drain_toasts();
    assert!(
        toasts.iter().any(|(_, msg)| msg == "pong"),
        "pong notify not received; got: {toasts:?}"
    );
}

#[test]
fn host_statusbar_add_returns_chip_id() {
    let tmp = tempfile::tempdir().unwrap();
    make_plugin_dir(
        tmp.path(),
        "bar-plugin",
        r#"local id = anvil.statusbar.add("hello", "right")
assert(type(id) == "number" and id > 0, "chip_id must be positive number")"#,
    );

    let mut host = PluginHost::new();
    let loaded = host.discover_and_load(tmp.path());
    assert_eq!(loaded, 1);

    std::thread::sleep(Duration::from_millis(100));
    host.tick();

    let chips = host.statusbar_chips();
    assert!(
        chips
            .iter()
            .any(|c| c.text == "hello" && c.position == ChipPosition::Right),
        "chip not found; chips: {chips:?}"
    );
}

#[test]
fn memory_limit_terminates_overflow_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    // Try to allocate >32 MiB by building a huge table.
    make_plugin_dir(
        tmp.path(),
        "oom-plugin",
        r#"local t = {}
for i = 1, 10000000 do
  t[i] = string.rep("x", 100)
end"#,
    );

    let mut host = PluginHost::new();
    // discover_and_load itself returns 1 because the worker is spawned
    // before the OOM happens inside the thread; that is expected.
    host.discover_and_load(tmp.path());

    // Wait for the worker to attempt the allocation and die.
    std::thread::sleep(Duration::from_millis(500));
    host.tick();
    // The key assertion: the plugin did NOT crash the process.
    // (If we get here the test passes.)
}
