# BR3 — EditorBridge thread + EditorSnapshot

Background worker holding a `Transport` to a running `nvim --listen` socket;
surfaces an `EditorSnapshot` the main thread reads each frame.

## 1. Crate placement

Lives in `crates/anvil-editor/` as new module `bridge.rs`. No new crate.

`anvil-editor` already owns the wire protocol (`codec`, `transport`). The
bridge is the natural orchestrator over them and has no callers outside the
binary. Mirrors the caldera precedent (client + poller in one crate).

`lib.rs` adds: `pub mod bridge;` and re-exports
`EditorBridge`, `EditorSnapshot`, `ConnectionState`.

## 2. `EditorBridge` struct

```rust,ignore
pub struct EditorBridge {
    snapshot: Arc<Mutex<EditorSnapshot>>,
    tx: Option<SyncSender<Msg>>,
    handle: Option<JoinHandle<()>>,
}
enum Msg { Kick, Stop }
```

Line-for-line mirror of `anvil_caldera::Poller`. Constructor takes
`Option<PathBuf>` (the socket; `None` keeps the snapshot in `Disconnected`).
`snapshot()` returns a cheap clone. `kick()` triggers immediate re-poll
(used by BR4 after spawn). `stop()` joins.

## 3. `EditorSnapshot` fields (v1)

```rust,ignore
pub struct EditorSnapshot {
    pub socket_path: Option<PathBuf>,
    pub connection: ConnectionState,
    pub buffer_name: Option<String>,    // basename only
    pub cursor: Option<(usize, usize)>, // (row, col), 1-indexed (nvim native)
    pub modified: bool,
    pub polled_at_unix: i64,
}
```

Excluded from v1 (added by BR5 / later): diagnostics counts, buffer text,
mode (normal/insert).

## 4. Connection state machine

```rust,ignore
pub enum ConnectionState { Disconnected, Connecting, Live, Error }
```

- `Disconnected` — initial; or `socket_path == None`.
- `Connecting` — socket known, `Transport::connect` not yet succeeded.
- `Live` — `connect()` and latest poll both Ok.
- `Error` — last call failed (transport, RPC, codec). Data fields zeroed so
  UI never pairs stale data with an error. Bridge drops the transport, next
  tick re-enters `Connecting`.

Mirrors caldera's Offline / Live / ErrorState semantics.

## 5. Socket discovery

The bridge does not discover. Caller passes a path or `None`. Two sources
feed it from `main.rs`:

1. **Spawned-by-anvil (BR4)** — anvil picks `$TMPDIR/anvil-nvim-<pid>.sock`.
2. **Attach-to-existing** — read `$NVIM_LISTEN_ADDRESS` at startup if set.

Recommendation: ship env-var lookup in BR3 wiring (one line in `main.rs`),
defer a config key until a user asks. BR4 owns the spawn path.

## 6. Poll cadence

`POLL_INTERVAL = Duration::from_millis(250)` — 4 Hz. Below human perception
threshold for status changes; far under any nvim op cost. Re-uses caldera's
`recv_timeout` + `Msg::Kick` loop so events (mode change, BufEnter, once a
later ticket wires autocmds) can request an immediate refresh.

BR5 LSP outline will run a separate slower cadence (1–2 s with debounce).
Not this ticket.

## 7. Failure handling

- **Socket disappears / nvim quits** — `Transport::call` returns
  `Io(UnexpectedEof)` or `Timeout`. Bridge sets `connection = Error`, zeros
  data fields, drops the transport, sleeps `POLL_INTERVAL`, retries
  `Connecting`.
- **Slow response** — call timeout `Duration::from_millis(500)`. On
  `Timeout`, behave as above.
- **Codec / RpcError** — identical recovery path.

The bridge never panics on RPC failure. Same contract as the caldera poller.

## 8. Threading model

`Arc<Mutex<EditorSnapshot>>` (caldera pattern), not channel-drain.

Snapshot-pull lets the render thread read "current state" each frame in O(1)
without draining a queue. Channel-drain fits kube because data is sparse
(one update per 30 s); editor state changes every keystroke and only the
latest value matters. Mutex contention is negligible — worker holds the lock
only for `*guard = new_snap`, reader only for `clone`.

## 9. Snapshot read sites

BR3 ships data plumbing only. No UI code in this ticket. `main.rs` gains
`editor_bridge: Option<anvil_editor::EditorBridge>`. No `tick()` work
needed — the mutex auto-updates. Future consumers
(`anvil-render/agent_panel.rs` status row, `context_bar.rs`) call
`bridge.snapshot()` when BR4/BR5 wire them.

## 10. Out of scope for BR3

Spawning nvim (BR4); LSP outline / diagnostics pull (BR5); buffer text
mirroring; notification handling beyond the existing transport "discard";
multi-buffer (v1 reports current buffer only); any UI surface change.

## Open questions

1. **Reconnect backoff?** Caldera retries every 2 s flat. With our 250 ms
   tick, a closed editor spins fast. Recommend flat 250 ms for v1; add
   1 s backoff after first `Error` only if logs show churn.
2. **`buffer_name` — basename or full path?** Recommend basename for HUD;
   add full path only when a caller needs it.
3. **`Option<EditorBridge>` on `App`, or `EditorBridge` that no-ops when
   `socket_path == None`?** Recommend `Option<...>`, matching
   `caldera_poller: Option<...>`.

## Verification plan

- Unit: `bridge.rs` tests using a fake `UnixListener` (reuse the
  `transport.rs` harness) returning canned msgpack frames for
  `nvim_get_current_buf`, `nvim_buf_get_name`, `nvim_win_get_cursor`,
  `nvim_buf_get_option(modified)`. Assert snapshot fields populate.
- Unit: server drops connection mid-poll → `Error` with zeroed data; next
  cycle returns to `Connecting`.
- Unit: `start(None)` → `Disconnected`; `kick()` does not panic;
  `stop()` joins cleanly.
- Manual: `nvim --listen /tmp/anvil-test.sock` + a small harness binary
  printing `bridge.snapshot()` every 500 ms; move cursor, edit buffer,
  confirm fields update.
- `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` green.

Build order: 1. `ConnectionState` + `EditorSnapshot` types with tests
2. `EditorBridge::start/snapshot/kick/stop` skeleton (`Disconnected` only)
3. `poll_once` with the four nvim RPCs + `Connecting → Live` path, then
   the error / reconnect path — verify after each.
