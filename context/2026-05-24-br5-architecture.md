# BR5 — Outline LSP pull + debounce

Pull document symbols from nvim's attached LSP client; surface in the
left-dock outline. Path B: anvil never speaks LSP directly, only msgpack-RPC.

## 1. RPC entry point

**`nvim_exec_lua` with a sync helper.** `nvim_buf_request_sync` is not on
the public RPC surface; `nvim_exec_lua` is the load-bearing escape hatch.
One round-trip, server-side fan-out.

```lua
local clients = vim.lsp.get_active_clients({ bufnr = 0 })
if #clients == 0 then return { attached = false } end
local res = vim.lsp.buf_request_sync(0, "textDocument/documentSymbol",
  { textDocument = vim.lsp.util.make_text_document_params(0) }, 1500)
return { attached = true, symbols = flatten(res) }  -- nil if timeout
```

Return shape encodes four states: `attached=false` (no server);
`attached=true, symbols=nil` (timeout); `symbols=[]` (server returned
nothing); `symbols=[...]` (done). Decoded via the existing `Value::Map`
path. Beats `luaeval` (no string-escaping of buffer paths).

## 2. `OutlineSymbol`

Lives in `crates/anvil-editor/src/bridge.rs`.

```rust,ignore
pub enum SymbolKind { File, Module, Class, Method, Function, Variable,
    Constant, Struct, Trait, Interface, Enum, Field, Other }
pub struct OutlineSymbol { name: String, kind: SymbolKind,
    line: u32 /*0-idx*/, depth: u8 /*0=top*/ }
```

Added to `EditorSnapshot`: `outline: Vec<OutlineSymbol>` and
`outline_state: OutlineState` (`Idle | Pending | Ready | NoServer`).
Unknown LSP kinds collapse to `Other`.

## 3. Debounce

**Piggyback on the 250ms tick with a 1500ms accumulator + one-shot kick
on buffer change.** Worker holds `last_outline_pull_ms`. Each tick after
the cheap poll: fire if `buffer_name` changed, else fire if
`now - last >= 1500ms`. Cursor moves and `:w` do not trigger (no autocmd
RPC yet; 1.5s ceiling is acceptable).

Net: cheap poll 4Hz, outline pull ~0.66Hz worst-case, instant on buffer
switch.

## 4. Failure modes

| Case | Behavior |
| --- | --- |
| `attached=false` | `outline.clear()`, state `NoServer` |
| Timeout (1.5s) | **Retain last outline**, state `Pending` |
| `symbols=nil`/`[]` | `outline.clear()`, state `Ready` |
| RPC error on `exec_lua` | `ConnectionState::Error`, all data zeroed (BR3 contract) |

Stale policy: keep last outline through Pending; clear on buffer switch
and NoServer. Transient slow LSP shouldn't blank mid-scroll; buffer
change must blank to avoid foo.rs symbols while viewing bar.py.

## 5. Cadence vs BR3's 250ms

Separate timer, **same worker**. The existing `recv_timeout(POLL_INTERVAL)`
loop ticks at 250ms; a `last_outline_pull` epoch gate sits in front of the
heavy RPC. One worker, two cadences, zero extra synchronization. A
separate thread doubles mutex traffic and needs its own transport (the
nvim socket serializes server-side anyway).

## 6. Left dock render integration

Extend `draw_left_dock` in `crates/anvil-render/src/left_dock.rs`:

```rust,ignore
pub fn draw_left_dock(..., snapshot: Option<&DirSnapshot>,
    outline: Option<&[OutlineRow]>, rect: Rect)
```

`draw_outline_section` rules:
- `None` → today's "Outline unavailable / Requires nvim bridge" state
  (bridge not Live, or first pull pending).
- `Some(&[])` → single line "No symbols" in `text_muted`.
- `Some(syms)` → rows: `<indent><kind-tag> <name>`. Indent = `depth*2`
  chars. Truncate via `truncate_name`. Color `text_muted` for
  fn/method/struct/trait/class, `text_subtle` otherwise.

`main.rs` maps snapshot → param: `Live+Ready → Some(&outline)`;
`Live+NoServer → Some(&[])`; else `None`.

`anvil-render` does not depend on `anvil-editor`. Mirror BR3's `DirEntry`
trick: render-side `OutlineRow { name, kind, depth }` + duplicated
`SymbolKind`; `main.rs` maps by value.

## 7. Scroll / selection

v1: none. Truncate to visible rows (`(content_h / ROW_H).floor()`), drop
the rest. Same shape as today's explorer section. Scroll arrives only on
demand.

## 8. Out of scope

Click-to-jump (needs `nvim_win_set_cursor` + hit-testing); filter / search
inside outline; diagnostics on symbols; symbol ranges (col); multi-buffer
outline; live updates faster than 1.5s.

## 9. Open questions

1. **`NoServer` copy.** "No language server" vs "LSP not attached"?
   Recommend "No language server" — user-language, not LSP jargon.
2. **Kind glyphs.** Nerd-font icons vs ASCII tags (`fn`, `cl`, `st`)?
   Recommend ASCII — chrome font is bundled IBM Plex; no nerd-font fallback.
3. **Pull on `:w`?** Needs autocmd RPC. Defer.

## 10. Verification plan

- Unit (`bridge.rs`, fake transport): `{attached=false}` → `NoServer`.
- Unit: flat DocumentSymbol array → `outline` populated, `Ready`.
- Unit: nested `DocumentSymbol[].children` → flattened with depth.
- Unit: timeout → previous outline retained, `Pending`.
- Unit: buffer switch clears outline before the next pull lands.
- Unit (`left_dock.rs`): `None` keeps current empty state; `Some(&[])`
  paints "No symbols"; `Some([...])` paints first row.
- Manual: open Rust file → outline within ~2s; rename a fn, wait 2s,
  outline updates. Open `.txt` → "No language server".
- `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`.

## Proposed wiki/decisions entry

`wiki/decisions/lsp-via-nvim-bridge.md` — anvil does not speak LSP. All
language-server data flows through nvim's attached LSP client via
`nvim_exec_lua`. Pre-empts a future "embed tower-lsp" detour.

Build order: 1. `OutlineSymbol`/`SymbolKind`/outline fields on
`EditorSnapshot`; type tests. 2. `nvim_exec_lua` snippet + flatten + state
transitions in the worker; stub-transport tests. 3. `outline` param on
`draw_left_dock`, render paths for None / empty / non-empty; `main.rs`
wiring — verify after each.
