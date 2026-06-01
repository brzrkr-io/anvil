# BR4 — "New Editor Pane" action + chrome wiring

Spawn `nvim --listen <socket>` in a fresh pane, hand the socket to
`EditorBridge`, surface buffer name in context bar. v1 = one editor pane.

## 1. Action surface

- **Palette.** New `CATALOG` entry in `anvil-workspace/src/palette.rs`:
  id `"editor.new"`, title `"New Editor Pane"`, `Action::NewEditorPane`.
- **Keybind.** `Cmd+E`. Add `editor_new: Option<Chord>` to
  `anvil-config::Keybindings` (default `"cmd+e"`) and to runtime
  `Keybindings` in `main.rs` (~351). Dispatch in the chord block near
  `main.rs:2045` alongside `kb.split_right`.
- **Handler.** Extend `handle_palette_action` (`main.rs:1915`) with
  `Action::NewEditorPane => self.new_editor_pane()`, then dismiss.

Both entry points call one host method, `App::new_editor_pane`.

## 2. Socket path derivation

Anvil derives; not env-driven. `$TMPDIR/anvil-nvim-<pid>-<counter>.sock`.

- `pid = std::process::id()` — distinguishes multiple anvil instances.
- `counter` = monotonic `u64` on `App`, bumped per spawn (stale-file
  insurance even though v1 is singleton).
- `$TMPDIR` (macOS per-user) avoids `/tmp` perm corners; well under the
  104-byte sun_path cap.
- `remove_file().ok()` on pane close.

BR3's `NVIM_LISTEN_ADDRESS` startup attach stays the separate path.

## 3. Spawning nvim

Reuse `Pty::spawn_exec(path, argv, cols, rows)` — already exists. In
`App::new_editor_pane`:

1. Resolve nvim via `which::which("nvim")` (new dep), cache in
   `App::nvim_path: Option<PathBuf>`.
2. Compute `cols, rows` exactly as `split_focused_pane` does.
3. Allocate `PaneId` through the same tree-split helper.
4. `Pty::spawn_exec(nvim, &["nvim", "--listen", socket_str], cols, rows)`.
5. `self.ptys.insert(new_id, pty)`.
6. Tag pane (§4), update bridge (§5).

Copy `split_focused_pane`; don't parameterise it. The argv + kind tagging
+ bridge handoff make a parallel helper cleaner than flag-soup unification.

## 4. Pane bookkeeping

**`editor_pane_id: Option<PaneId>` on `App`.** Not `Pane::kind` enum.

- v1 invariant is "at most one editor pane" — one nullable field matches.
- `Pane` is *pure* today (no PTY, no I/O — see module comment); editor
  metadata belongs on the singleton.
- Socket already lives on `EditorBridge`; don't duplicate.
- Multi-editor promotion path: `HashMap<PaneId, PathBuf>` — mechanical.

## 5. EditorBridge handoff

Bridge today is built once from `NVIM_LISTEN_ADDRESS` (`main.rs:4021`).
BR4 makes it runtime-mutable.

Add `EditorBridge::set_socket(&mut self, path: Option<PathBuf>)`. Sends
`Msg::Resocket`, worker swaps path, drops `Transport`, re-enters
`Connecting` (or `Disconnected`). `kick()` after.

On spawn success: if `editor_bridge.is_none()` →
`Some(EditorBridge::spawn(Some(path)))`; else `bridge.set_socket(Some)`.

On editor-pane close: `set_socket(None)`, `editor_pane_id = None`,
`remove_file().ok()`. Bridge stays alive, idle in `Disconnected`.

## 6. Single-editor invariant

**Focus existing pane**, do not spawn a second. When
`editor_pane_id == Some(id)`: linear-scan `tabs` for owner (< 10), switch
to it, set `tab.tree.focused = id`. No spawn, no bridge change. Same
behaviour even if bridge is in `Error` — user sees failure in-pane and
decides.

## 7. Chrome wiring

**Context bar (ID2).** Extend `draw_context_bar` (`context_bar.rs:19`)
with `editor: Option<&EditorSnapshot>`. Render right-anchored between
kube and head_short, lowest priority:

```
edit: <buffer_name>[•]
```

- name: `buffer_name.as_deref().unwrap_or("[no name]")`.
- `•` suffix when `modified == true`, colour `theme.accent`.
- run colour `theme.text_muted`.
- whole field omitted when `connection != Live`.

**Tab title (optional).** If tab contains `editor_pane_id` and snapshot is `Live`, prefix label with basename. Defer if tabbar needs more than a string concat.

## 8. Failure modes

- **nvim not installed.** `which` returns `Err`. `eprintln!` matching
  `split_focused_pane` failure-log style; no pane created.
- **Socket bind / RPC mismatch.** nvim's error lands in the PTY (user
  sees it). Bridge → `Error`, context bar omits buffer field; pane
  remains useable as a terminal.
- **Pane closed, socket dangling.** `close_focused_pane` checks
  `editor_pane_id == Some(focused_id)` and runs the cleanup. Tab-close
  routes through the same helper. A crash leaves the file in `$TMPDIR`,
  reaped on reboot.
- **nvim `:q`.** PTY EOF → existing close-on-EOF removes the pane; same
  cleanup fires.
- **App quit.** `Pty::Drop` SIGHUPs nvim. Add a `remove_file` line on
  the editor socket in `terminate_app`; optional polish.

## 9. Out of scope for BR4

LSP outline / diagnostics (BR5); multi-editor mode; spawn-on-startup;
editor keybind pass-through (pane is a normal PTY); native nvim-UI
rendering; config knobs for nvim path / args; session restore.

## 10. Open questions

1. `which` crate vs inline PATH walk — recommend `which`.
2. Tab-title prefix in BR4 or follow-up — recommend follow-up if tabbar
   change isn't a one-line concat.
3. User runs `nvim --listen /tmp/other.sock` in a shell pane: anvil's
   bridge tracks only the anvil-chosen socket. Document as expected.
4. Configurable `Cmd+E` — yes, wire through `anvil-config`. Default
   `"cmd+e"`.

## Verification plan

- Unit: catalog maps `editor.new` → `Action::NewEditorPane`.
- Unit: `set_socket(Some)` from `Disconnected` → `Connecting`;
  `set_socket(None)` → `Disconnected`.
- Unit (host helper): `new_editor_pane` twice does not double-spawn.
- Manual: `Cmd+E` opens nvim pane; context bar progresses through
  `edit: [no name]` → `edit: foo.rs` → `edit: foo.rs •`. `:q` clears.
- Manual: `Cmd+E` twice — second focuses existing pane.
- Manual: `kill <nvim-pid>` — slot clears within ~250 ms; pane stays.
- `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`.

## Proposed wiki/decisions entry

`wiki/decisions/editor-pane-is-a-singleton.md` — v1 single-editor rule,
`App::editor_pane_id` over `Pane::kind`, multi-editor promotion path.

Build order: 1. `EditorBridge::set_socket` + `Msg::Resocket` with unit
tests 2. `Action::NewEditorPane` + palette entry + keybind field +
`App::new_editor_pane` (spawn + bookkeeping + handoff) 3. context-bar
`editor` param + render slot + close-path cleanup — verify after each.
