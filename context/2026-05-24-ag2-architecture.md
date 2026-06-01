---
title: AG2 — caldera-local HTTP client + poller
status: draft
date: 2026-05-24
role: systems-architect
---

# AG2 Architecture

## Scope correction (read first)

AG2 is **not** greenfield. `crates/anvil-caldera/` already ships a working
HTTP client, four endpoints, a background `Poller`, a five-state connection
machine, integration tests against a fake TCP server, and is wired into
`App` via `refresh_hud` (`crates/anvil/src/main.rs:1271-1284` — lazy spawn
on first cwd, snapshot drained each tick). The HUD already reads `Snapshot`
directly (`agent_panel.rs`).

What's left is small. This doc names that delta. Also: the prompt's 5454
placeholder is stale; codebase has committed to **4175**
(`anvil_caldera::DEFAULT_ENDPOINT`).

If the intent was a redesign, stop and reroute — no code or context handoff
supports that.

## 1. Module shape (already in place)

`crates/anvil-caldera/src/`:

- `lib.rs` — `CalderaClient` (raw `TcpStream` + hand-parsed HTTP/1.1, no
  external HTTP crate), `Endpoint`, `parse_response`, error enum. Methods:
  `health`, `current_session`, `start_session`, `activity`, `agent_runs`,
  `project`, `post_raw`.
- `client.rs` — `Raw*` serde types + `From` conversions into canonical
  `anvil_agent` types. Insulates the rest of the crate from caldera-local
  JSON drift (severity strings, ISO timestamps).
- `poller.rs` — `Poller { snapshot, tx, handle }`; `start`, `snapshot`,
  `kick`, `stop`. Five-state machine in `poll_once`.
- `actions.rs` — `approve`, `ack_finding`, `start_run`. AG3 surface; wire
  helpers ready, no caller yet.
- `detect.rs` — walks ancestors for `.caldera/project.json` with
  `enabled: true`. Pure FS gate; runs each cycle to handle `cd` across projects.

Layout maps 1:1 to the prompt's suggested split. No deviation.

## 2. Threading model

- One worker thread per `Poller`, spawned in `start`.
- Control channel: `std::sync::mpsc::sync_channel::<Msg>(1)`, `Msg = Kick | Stop`.
  Bounded to 1 so a flood of `kick()` coalesces to one wake.
- Data path: **not a channel.** Worker writes `Arc<Mutex<Snapshot>>`; render
  thread clones under lock each tick. Lock is held only for `Snapshot::clone`.
- Shutdown: `stop()` sends `Stop` then drops sender; worker exits on
  `Stop` or `Disconnected`. `Drop` is best-effort send-only — no join, to
  avoid drop-on-worker-thread deadlock.

**Decision: keep `std::sync::mpsc`.** SPSC, no select, no `crossbeam`
justification. Snapshot-pull (vs channel-drain in `tick()`) is intentional:
intermediate snapshots are lossy by design; the HUD only wants the latest.

## 3. Poll cadence

**Default: 2s** (`POLL_INTERVAL`, shipped). caldera-local is localhost; a
four-request cycle is <5ms. After explicit actions, callers `kick()` for
an immediate cycle.

**Backoff: none, intentional.** Failures collapse into `Offline` /
`NoProject` / `ErrorState` and the HUD renders honest empty state.
Connect to a closed port returns immediately — no battery cost. Revisit
only if measured to matter; cap at 30s in `Offline` if so.

**"caldera unreachable":** `Connection::Offline` is painted as
attention-amber bullet + dim no-data summary. No fabricated runs ever
appear because every error branch returns
`Snapshot { connection: X, ..Default::default() }` — connection and data
are zeroed together, structurally.

## 4. Endpoints

Critical v1 (all shipped):

| Path | Shape | Drives |
|------|-------|--------|
| `GET /health` | `{status, service}` | `Offline` gate |
| `GET /api/project` | `{project: {enabled, project_name, mode}?}` | `NoProject` / `Disabled` |
| `GET /api/activity` | `{pending_approvals[], attention[]}` | approvals, findings |
| `GET /api/agent-runs` | `{agent_runs[{run_id, agent, task, status, created_at}]}` | runs, `running_count` |

Deferred (AG3 wire helpers exist, no HUD path):

- `POST /api/approvals`, `POST /api/findings/ack`, `POST /api/task-handoff`.

Auth: none. caldera-local binds 127.0.0.1.

## 5. HUD wire-up

`Snapshot` is read directly by `agent_panel.rs`. Done.

`LocalContext` is a **separate** struct for non-caldera HUD inputs (cwd,
git, last-run, ports, recent files, recent prompts, kube). The prompt
conflated the two — they're independent inputs to `draw_agent_panel(snap,
local, ...)`.

**No new `LocalContext` fields needed for AG2.** `last_run` is computed
from terminal prompt-marks (main.rs:1297-1311), not caldera. `agent` is
a `Snapshot` field, not a `LocalContext` one.

**Kubectl worker conflict: none.** Kubectl uses one-shot `Sender<KubeCtx>`
drained in `refresh_hud`; caldera uses `Arc<Mutex<Snapshot>>` pull. They
coexist on different fields. Consistency port (kubectl → snapshot-pull)
is a future cleanup, not blocking.

## 6. Failure modes & invariants

- Daemon down → `Offline`, data zeroed.
- Not opted in → `NoProject`, data zeroed.
- `enabled: false` → `Disabled`, data zeroed.
- Slow response → worker blocks up to 4s `timeout`; render thread never
  blocks (only clones under brief lock).
- Partial response → `serde(default)` on every `Raw*` field; missing keys
  become defaults. Schema drift silent, not fatal.
- Unknown enum strings → `Info` / `Unknown` via the converters.
- Non-2xx HTTP → `ErrorState` from activity/runs, `Offline` from health.

**Invariants:**

1. HUD never renders run/approval rows when `connection != Live`.
   Enforced structurally by zeroing on every error branch.
2. `snapshot()` never blocks the worker for IO — only Snapshot clone.
3. `drop()` never joins; `stop()` is the join path.

## 7. Tests

Already covered:

- Parsers/converters: ~20 unit tests in `lib.rs` + `tests/integration.rs`
  fixture round-trip.
- State machine: `poll_once_*` covers all five Connection branches via
  a multi-response fake TCP server.
- Lifecycle: `poller_start_stop_cleanly`, `poller_kick_does_not_panic`,
  `poller_transitions_through_live_state`.

The fake-TCP-server harness is sufficient. **Skip `mockito`** — it
imports reqwest assumptions and adds a dep we don't need.

**Gap to fill:** one test for `stop()` arriving mid-cycle during a slow
response. Today the worker may wait the full timeout before noticing
shutdown.

## 8. Out of scope for v1

Confirmed:

- AG3 (interactive approve / ack / start). Wire helpers in `actions.rs`
  exist but no dispatcher, no HUD affordance.
- Auth, TLS, non-localhost endpoints, per-session daemons.
- Backoff. Add only if measured cost justifies.

## 9. Open questions

1. **Port: 4175 confirmed?** Code committed; prompt says 5454. Please
   confirm 4175 stands.
2. **`adapter_preflight`** in `.caldera/project.json` — present in test
   fixtures, unused in code. Surface in UI?
3. **AG3 trigger surface** — keystroke / command palette / row click?
   Needed before any action dispatcher.
4. **Schema versions** — caldera-local emits `caldera.activity.v0` etc.
   Surface "incompatible caldera-local" state, or keep silent
   forward-compat?

## Build order

1. **Confirm the scope correction.** If the intent was a fresh design,
   reroute. Verify: user acknowledges this doc.
2. **Add the `stop()`-mid-poll test.** Verify: new test fails on
   today's worker, passes after a `select`-style fix (or shorter
   timeout). `cargo test -p anvil-caldera` green.
3. **Pin port in `wiki/decisions/`.** Single ADR: `127.0.0.1:4175` as
   default; override via `CalderaClient::new`. Verify: doc exists,
   `wiki/log.md` appended, librarian lint clean.

Build order: 1. Confirm scope 2. Add stop-mid-poll test 3. Pin port ADR — verify after each.
