---
title: Plugin API Spec (v1)
date: 2026-05-26
status: proposal
owner: systems-architect
---

# Plugin API Spec (v1)

## Goal

Let third-party authors extend Anvil without recompiling the binary.
v1 scope: palette commands, keybindings, status-bar chips, themes,
snippets, simple editor/file hooks. Language servers and WASM modules
out of scope for v1.

Source of truth lives at `~/.config/anvil/plugins/<name>/`.

## Architecture Choice

**Lua via `mlua` (Lua 5.4, vendored).** Rationale:

- Small, fast embed; familiar to LazyVim/Neovim users.
- `mlua` enforces a sandboxed `Lua` state.
- Lua state cheap enough to give each plugin its own VM.
- WASM (wasmtime) deferred to v2 when LSP/heavy-compute appears.
- Shell-out / JSON-RPC rejected: too slow + no sandbox.

Tradeoff: Lua not strongly typed at the FFI boundary. Mitigate by validating every value crossing FFI + isolating each plugin in its own state + thread.

## New Crate: `crates/anvil-plugin/`

- `host.rs` — `PluginHost`: registry, worker pool, event dispatch.
- `plugin.rs` — `Plugin`: manifest, Lua state, registered hooks.
- `manifest.rs` — TOML parse + validation.
- `api/` — Lua API surface (one file per namespace).
- `sandbox.rs` — Lua globals scrubbing + `require` shim.
- `reload.rs` — file-watch + atomic reload.
- `bridge.rs` — `HostRequest`/`HostResponse` mpsc between worker + main.

Public types: `PluginHost`, `PluginId`, `PluginEvent`, `PluginChip`, `PluginCommand`, `PluginError`.

## Manifest

`~/.config/anvil/plugins/<name>/plugin.toml`:

```toml
[plugin]
name = "my-plugin"
version = "0.1.0"
description = "..."
api = "1.0"

[entry]
lua = "init.lua"
```

Validation: `name` matches `^[a-z0-9-]{1,40}$` and equals dir name; `api` major must equal host major; `entry.lua` must exist inside plugin dir.

## Lifecycle

```
discover → parse manifest → spawn worker thread → load Lua state →
  install sandbox → run init.lua → register hooks → idle
  ─(file change)→ unload → reload
  ─(host shutdown)→ unload
```

Unload is atomic: host removes commands/chips/subscriptions before dropping worker. Mid-flight events dropped.

## Lua API Surface (v1)

All functions under global `anvil` table.

- `anvil.command(name, handler)` — register palette entry.
- `anvil.keymap(chord, command_name)` — bind chord.
- `anvil.statusbar.add(text, position)` → `chip_id`; `position` ∈ `"left"|"right"`. Plus `update(id, text)` / `remove(id)`.
- `anvil.editor.current_buffer()` → `{ path, language, line, col }` (nil if no focus).
- `anvil.editor.insert(text)` — insert at cursor.
- `anvil.notify(level, msg)` — toast; `level` ∈ `"info"|"warn"|"error"`.
- `anvil.fs.read(path)` → string; `anvil.fs.write(path, content)`.
- `anvil.hooks.on(event, fn)` — events: `"buffer.opened"`, `"buffer.saved"`, `"cursor.moved"`.

Each `editor.*` and `fs.*` call posts `HostRequest` via mpsc and blocks the plugin worker until response. App thread handles requests between frames.

## Sandbox

Removed Lua globals: `io`, `os.execute`, `os.exit`, `os.remove`, `os.rename`, `os.getenv`, `package.loadlib`, `dofile`, `loadfile`, `debug`. `require` is replaced with a shim that only resolves modules under the plugin's own directory.

`os.date` kept (pure, sample plugin needs it). Allowlist documented in `sandbox.rs`.

`anvil.fs` enforces:
- Read/write only under workspace root or plugin dir.
- Max file size 8 MiB per call.
- No symlink traversal outside allowed roots.

## Resource Limits

- 32 MiB Lua heap per plugin via `mlua`'s memory limit hook.
- 200 ms wall-clock per hook invocation via instruction-count hook + watchdog thread.
- Synchronous host calls count against same 200 ms budget.
- 3 consecutive timeouts → plugin disabled until reload.

## Threading

Each plugin runs on dedicated `std::thread` owning its Lua state. Events fan out from `PluginHost::dispatch_event` via per-plugin `crossbeam_channel::Sender<PluginEvent>` (bounded, capacity 64; full drops oldest with `warn!`). Host responses use oneshot per request.

App-thread API: `PluginHost::tick()` drains pending `HostRequest`s between frames; `PluginHost::commands()` and `statusbar_chips()` return cached snapshots updated on registration.

## Hot Reload

Add `notify = "6"` as workspace dep (NEW — not currently used). Watch `~/.config/anvil/plugins/`; on any change inside `<name>/`, debounce 150 ms, then unload + reload that plugin only.

Manifest parse errors → toast, previous version stays running. `init.lua` errors → `failed` state visible in `Plugins` palette command.

## Integration Points

- `crates/anvil-render/src/overlay/mod.rs` — palette returns `Submission::PickerRow { id, index }`. App layer (which owns row list) gains `plugin_commands` source. Selection → `PluginHost::invoke_command(id)`. No new render-layer variant needed.
- `crates/anvil/src/main.rs` — `App` gains `plugin_host: PluginHost`. Boot order: config → fs → git → plugins (after editor surface exists).
- Editor save path → `plugin_host.dispatch_event(BufferSaved { path })`.
- Status bar renderer reads `plugin_host.statusbar_chips()` each frame.

Themes and snippets v1: static files under plugin dir (`themes/*.toml`, `snippets/*.toml`) merged into existing registries at load. No Lua API for these.

## Failure Modes

- Manifest invalid → skipped, toast on startup.
- `init.lua` error → `failed`, no hooks fire.
- Hook timeout → invocation aborted; plugin keeps running.
- 3 consecutive timeouts → plugin disabled.
- Memory limit → allocator returns nil; host catches panic + disables.
- Host channel full → oldest event dropped + `warn!`.
- Symlink escape → `anvil.fs.*` returns error.

## Verification Plan

Unit tests in `crates/anvil-plugin/`:
- Manifest: valid, missing field, bad `name`, `api` major mismatch.
- Sandbox: `io`, `os.execute`, `require "ffi"` all error.
- Path guard: `anvil.fs.read("/etc/passwd")` rejected; workspace path accepted.
- Memory cap: 64 MiB Lua table → clean error.
- Hook timeout: `while true do end` aborts within 250 ms.

Integration test:
- Load `examples/plugins/hello-anvil/`.
- Assert one command + one chip registered.
- Fire `BufferSaved`, assert handler ran (via `cfg(test)` `anvil.test.signal`).
- Touch `init.lua`, assert reload within 500 ms.

Smoke: symlink `examples/plugins/hello-anvil/` into `~/.config/anvil/plugins/`; verify `Hello` appears in Cmd+P + clock chip ticks.

## Sample Plugin

`examples/plugins/hello-anvil/`:

```
plugin.toml
init.lua
```

`init.lua`:

```lua
anvil.command("Hello", function()
  anvil.notify("info", "Hello from Anvil")
end)

local chip = anvil.statusbar.add(os.date("%H:%M"), "right")
anvil.hooks.on("cursor.moved", function()
  anvil.statusbar.update(chip, os.date("%H:%M"))
end)
```

## Phases

1. Crate skeleton, `PluginHost`, manifest loader, sandboxed Lua VM, memory + time limits. (~2 days)
2. API surface: `command`, `keymap`, `statusbar`, `editor`, `fs`, `notify`. Palette + status-bar integration. (~2 days)
3. Hooks + hot reload via `notify`. (~1 day)
4. Sample plugin + `wiki/concepts/plugin-api.md` + decision entry. (~0.5 day)

## Open Questions

- Keymap conflicts (plugin vs built-in): suggest built-in wins, plugin logged as shadowed. Confirm with design-lead.
- Snippet/theme schemas: reuse existing formats or define plugin-side? Defer to Phase 4.
