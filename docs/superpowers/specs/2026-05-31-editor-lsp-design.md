# Editor LSP client arc

Date: 2026-05-31
Status: approved (design) — all forks resolved 2026-05-31

## Context

`src/editor.zig` is Anvil's native editable buffer. It owns lines as
`ArrayListUnmanaged(ArrayListUnmanaged(u8))`, tracks a cursor (`cur_row`,
`cur_col`), and renders into a Terminal grid that the existing GPU path draws.
`src/session.zig` wraps it as a `.editor` Session with `editorInput`,
`editorSave`, `editorClick`, `editorScroll`, `reloadEditor`. Syntax color today
comes from `src/syntax.zig` (`tokenizeLine`), a per-line regex-free tokenizer.

This is sub-project #3 of the native-IDE roadmap (gutter → buffer hardening →
**LSP client core → diagnostics → navigation → completion → edits**). The
gutter (#1) shipped. This spec covers the whole LSP arc as a dependency-ordered
sequence; each numbered slice below should ship as its own plan + commit. The
arc is large, so the spec front-loads the genuine design forks that need a human
decision before any slice starts.

The hard constraint is the **cell grid**. Anvil has no free-floating text layer:
everything the editor shows is cells in a `Terminal` grid (`cp`, `fg`, `bg`,
`attrs` with `bold/underline/reverse/dim/...`). Diagnostics, hover, and
completion all have to land either as grid cells or as a new Metal overlay.
That choice is the central fork.

## Resolved decisions (2026-05-31)

1. **JSON-RPC parsing:** `std.json.Value` (dynamic) in the client core, narrowing
   to typed reads at call sites. Allocation cost accepted — messages are small,
   parsed then dropped.

2. **Diagnostic rendering:** gutter severity marker **plus** inline cell styling
   (`underline` attr + severity `fg` on the offending range). Slice 3 must first
   `--dump`-verify that `attrs.underline` actually renders in the Metal glyph
   pipeline (today only VT output sets it); if it does not, that's a prerequisite
   fix inside slice 3.

3. **Hover / diagnostic detail:** a one-line **status strip row** at the bottom of
   the editor pane (modeline style), rendered into the grid. No floating panel in
   v1.

4. **Completion UI:** a **Metal overlay popup**, reusing the palette/overlay
   drawing path (lists routinely exceed remaining rows, must float above content).
   This is the largest slice; it is sequenced last among the feature slices.

5. **Edit application:** apply **synchronously** on the main thread when the
   response lands, guarded by a version check (reject stale edits). Format-on-
   demand only for v1 — no format-on-save.

6. **Threading:** LSP process I/O is fully **non-blocking, drained from the poll
   loop** (mirrors `pty.zig` / `session.poll`). No reader thread, no new threading
   model.

## Design

### Module boundary

New `src/lsp/` directory, three focused files plus a small Session hook:

- `src/lsp/transport.zig` — process lifecycle + JSON-RPC framing over stdio.
  Owns a child process (server binary) and its `stdin`/`stdout` fds, sets them
  non-blocking, and implements `Content-Length` header framing. Mirrors
  `pty.zig` exactly: a `spawn`/`read`/`write`/`deinit` struct, `ReadResult`
  union (`{ data, would_block, eof }`), libc `@cImport` for pipe + fork/exec.
  It does **not** know about JSON semantics — it hands complete message bytes up
  and frames outgoing bytes down. This is the only file that touches OS process
  state.

- `src/lsp/protocol.zig` — JSON-RPC encode/decode: build request/notification
  envelopes, parse incoming `{ jsonrpc, id?, method?, params?, result?, error? }`,
  and the LSP value types we use (Position, Range, Diagnostic, CompletionItem,
  TextEdit, Hover, Location). Pure data + (de)serialization, no I/O, no process.
  Heavily unit-testable from byte slices.

- `src/lsp/client.zig` — the stateful client per language server. Owns one
  `transport.zig` process and one `protocol.zig` codec, tracks: the next request
  id, an in-flight request table (id → pending kind), the server's
  initialize/capabilities state, and a small bounded queue of parsed results the
  Session drains. Exposes intent-level methods: `didOpen`, `didChange`,
  `definition`, `hover`, `completion`, `format`, and a `poll()` that drains the
  transport, parses messages, routes notifications (publishDiagnostics) into
  per-buffer state, and matches responses to in-flight ids. **`client.poll()` is
  the analogue of `session.poll()`** — non-blocking, drained from the app loop.

- Session hook: `Session` (editor kind) gains an optional `lsp: ?*lsp.Client`
  (owned by the session manager, not the editor — one server can back multiple
  editor sessions of the same language; see "Server registry"). The editor
  buffer stays LSP-agnostic except for the position/version contract below.

Dependency direction: `client → protocol → (std)`, `client → transport →
(libc)`. `editor.zig` depends on neither; `session.zig` depends on `client`.
No file gains a second responsibility.

### Slice 1 — Buffer hardening (prerequisite, no LSP yet)

LSP positions are `{ line, character }` where `character` is a UTF-16 code-unit
offset by default (servers may negotiate UTF-8 via `positionEncoding`; zls
supports UTF-8). The current editor uses **byte** offsets and is ASCII-correct
only. Gaps to close in `editor.zig`:

- **Encoding negotiation.** On initialize the client requests
  `positionEncoding: ["utf-8", "utf-16"]`. If the server grants UTF-8, editor
  byte offsets map directly (character == byte index within a line). If only
  UTF-16, we need a byte↔UTF-16 column converter per line. Recommendation: rely
  on UTF-8 encoding for zls and gate non-UTF-8 servers off (degrade: no LSP,
  editor still works). This keeps the editor on byte offsets.

- **Position mapping helpers.** Add `fn posToCursor(line, character)` and
  `fn cursorToPos()` to `editor.zig` (byte-based, valid under UTF-8 encoding),
  plus a `fn lineByteOffset(row)` for range math. Pure, unit-tested.

- **Version counter.** Add `version: i32 = 0` to `Editor`, bumped on every
  mutation (`insertByte`, `insertNewline`, `backspace`, `deleteForward`, edit
  application). `didChange` carries this version; edit application checks it to
  reject stale server edits (fork #5).

- **Change extent for didChange.** v1 sends **full-document** `didChange` (whole
  buffer text, `contentChanges: [{ text }]`) — simplest, correct, fine for files
  under the 2 MiB cap. Incremental ranged changes are deferred.

No new files. ~5 small methods + one field on `Editor`, each with a test.

### Slice 2 — LSP client core

- **Server registry** lives in `session_manager.zig` (it already owns the
  multi-session map): `lang → ?*lsp.Client`, lazily spawned the first time an
  editor of that language opens. zls is the only mapping for v1 (`.zig → "zls"`,
  found on `PATH`). Missing binary → registry stores a "no server" sentinel and
  the editor runs exactly as today.

- **Lifecycle:** spawn (`transport.spawn`) → send `initialize` → on `initialized`
  ack, replay `didOpen` for each open buffer of that language → steady state.
  On the editor opening a file: `didOpen`. On buffer mutation: `didChange`
  (debounced, see below). On pane close of the last buffer for a server:
  `shutdown` + `exit`, then reap.

- **Framing (`transport.zig`):** outgoing = `Content-Length: N\r\n\r\n` + body.
  Incoming = a small ring/accumulator buffer; the reader pulls bytes
  non-blocking, scans for the header, and yields a complete body slice when
  `N` bytes are present, else `would_block`. Partial messages are retained
  across polls (this is the one piece `pty.zig` doesn't have — a stateful
  message accumulator — and the main thing to unit-test).

- **Poll integration:** `anvil_poll` already loops `mgr.sessions` calling
  `s.poll()`. Add a sibling drain: after the session loop, iterate the registry's
  live clients and call `client.poll()`; if it produced new diagnostics or a
  response for the focused editor, `markDirty()`. This keeps LSP I/O on the same
  non-blocking cadence as the PTY and never blocks the UI.

- **Debounce:** `didChange` fires at most once per poll tick per buffer (coalesce
  rapid keystrokes), using full-document text. No timer needed — the poll loop is
  the clock.

Failure modes handled here: spawn fail / missing binary → no client, editor
unaffected; server EOF/crash → `client.poll()` returns dead, registry drops it,
editor keeps working, optional respawn-on-next-open; malformed message → log and
skip, don't crash the parser.

### Slice 3 — Diagnostics

- `client.zig` stores `publishDiagnostics` per document URI (a bounded list of
  `{ range, severity, message }`). The Session for that URI reads them at render
  time.
- Rendering (fork #2): in `editor.render`, after content cells are written, for
  each visible line that has diagnostics, set a **gutter marker** glyph in a
  reserved gutter column (severity → palette color: risk/red for error,
  attention/amber for warning — semantic colors per BRAND.md), and apply
  `attrs.underline` + severity `fg` to the cells of the diagnostic range on that
  line. The status strip (fork #3) shows the message for the diagnostic under the
  cursor.
- The diagnostic store is keyed by URI and survives buffer edits; stale
  diagnostics (older than the buffer version) are visually dimmed until the next
  `publishDiagnostics` arrives, so the gutter never goes blank mid-edit.

### Slice 4 — Navigation (definition + hover)

- `definition`: on a keybinding over the cursor, send `textDocument/definition`;
  the response is a `Location` (uri + range). If same file → move cursor + scroll
  (`ensureVisible`). If another file → `reloadEditor(path)` then position the
  cursor. Cross-file open reuses the existing pane-reuse path.
- `hover`: on a keybinding (or, deferred, on dwell), send `textDocument/hover`;
  render the result text into the status strip (fork #3). Hover/definition share
  the request/response plumbing in `client.zig`; only the result handler differs.
- Failure: no response within N polls → request is dropped from the in-flight
  table (bounded TTL), status strip shows nothing. Slow server never blocks.

### Slice 5 — Completion

- Trigger: explicit keybinding (Ctrl-Space) for v1; trigger-characters deferred.
  Send `textDocument/completion`; on response, populate a completion model
  (label + insertText + range) and show the overlay (fork #4: Metal popup).
- Selection inserts the item's text at its range (a single `TextEdit` applied via
  the slice-6 edit path) and dismisses the popup. Esc/cursor-move dismisses.
- The popup is new chrome; it is the largest slice and depends on fork #4 being
  resolved toward a Metal overlay.

### Slice 6 — Edits (formatting, code action, rename)

- A single `applyTextEdits(edits)` on `Editor`: sort edits by range descending so
  earlier offsets stay valid, then splice each into the line buffers. This is the
  shared primitive behind completion insert, formatting, and rename.
- `format`: `textDocument/formatting` → `applyTextEdits` (fork #5: synchronous,
  version-checked). Rename and code actions reuse the same apply, scoped to the
  current buffer in v1 (multi-file `WorkspaceEdit` deferred).
- Version skew guard: if `editor.version` changed between request and response,
  drop the edits (don't corrupt the buffer) and surface a quiet status note.

## Failure modes (cross-cutting)

| Failure | Behavior |
| --- | --- |
| Server binary missing / spawn fails | No client; editor identical to today. |
| Server crashes / closes stdout | `client.poll()` → dead; registry drops it; editor keeps working; respawn on next open of that lang. |
| Server slow / no response | Requests have a bounded in-flight TTL; UI never blocks (non-blocking I/O); stale diagnostics dimmed, not blank. |
| Version skew (buffer edited mid-request) | Edits rejected via version check; navigation result ignored if out of range. |
| Non-UTF-8 server | LSP disabled for that buffer (no position-encoding path); editor unaffected. |
| Malformed JSON-RPC message | Logged and skipped; framing resyncs on next `Content-Length`. |
| File over 2 MiB / binary | Already refused by `fileview.load`; never reaches LSP. |

The invariant across every failure: **the editor remains fully usable with no
language server.** LSP is strictly additive.

## Verification plan

- **Unit (`.zig/zig build test`):**
  - `editor.zig`: position↔cursor round-trip; version bumps on each mutation;
    `applyTextEdits` splices correctly (overlapping/descending order); stale
    edit rejected on version mismatch.
  - `protocol.zig`: encode a request → exact `Content-Length` + body bytes;
    decode a `publishDiagnostics` notification, a `definition` response, a
    `completion` response from fixture byte slices.
  - `transport.zig`: the message accumulator yields a complete body across a
    split read (feed the header and body in two chunks → one message);
    `would_block` when bytes are incomplete; `eof` on closed pipe. Use a pipe
    pair or an echo child, mirroring `pty.zig`'s round-trip test.
  - `client.zig`: in-flight id matching; diagnostics stored per URI; dead poll
    after child exit.
- **Integration (live, if zls present):** open a `.zig` file, confirm
  diagnostics appear in the gutter, go-to-definition jumps, hover shows text,
  completion lists, format applies. Skipped cleanly when zls is absent.
- **Render:** `./zig-out/bin/anvil --dump /tmp/x.png` rc 0 after each slice that
  touches rendering (especially confirm `attrs.underline` actually draws via the
  Metal path before relying on it for diagnostics — fork #2).
- **Done gate per slice:** `.zig/zig build test` green + `--dump` rc 0 + the
  failure-mode for that slice exercised (e.g. rename `zls` off `PATH`, confirm
  the editor still opens and edits).

## Proposed wiki decision entry

`wiki/decisions/` should record, once the forks are resolved: (1) LSP client
lives in `src/lsp/` as transport/protocol/client with the server registry in
`session_manager.zig`; (2) non-blocking, poll-loop-drained I/O modeled on
`pty.zig`, no new threads; (3) editor stays on byte offsets, requiring UTF-8
position encoding and disabling LSP for non-UTF-8 servers; (4) the chosen
diagnostic and completion surfaces (gutter+underline / Metal overlay). Defer
writing the page until the user answers the open questions, so it records real
choices rather than recommendations.

## Out of scope

Incremental `didChange`, multi-file `WorkspaceEdit`, format-on-save,
trigger-character completion, hover-on-dwell, signature help, semantic tokens
(syntax stays on `syntax.zig` until a dedicated slice), workspace symbols,
multiple servers per language, and any non-zls server configuration.
