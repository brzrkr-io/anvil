---
status: active
type: concept
created: 2026-05-25
updated: 2026-05-29
sources:
  - ../../context/2026-05-25-native-editor-roadmap.md
confidence: low
---

# Native Editor

> **Status (2026-05-29):** This page describes the native editor roadmap as
> designed for the archived Rust port (`rust-port-archive` tag). The source
> paths below (`crates/anvil-editor/`, etc.) do not exist in the active `zig`
> branch. The design intent and phase descriptions remain valid as a roadmap
> artifact. Confidence is low until the editor is ported or re-specified for Zig.

Anvil's native editor replaces the nvim RPC path with an in-process text
editing surface rendered by the same Metal cell grid as the terminal. The NE
track (NE1–NE15) is the build sequence. Phases NE1–NE5 had landed on
`rust-port` as of 2026-05-25. See
`context/2026-05-25-native-editor-roadmap.md` for the full phase list,
critical path, and decisions.

## Buffer Model (NE1)

`Buffer` in `crates/anvil-editor/src/buffer.rs` is rope-backed via `ropey
1.6`. The rope is treated as an implementation detail — all positions cross
the public API as `Position { line, col }` (grapheme column), never as raw
rope char or byte indices. This facade is the primary mitigation for ropey's
byte-vs-char-vs-line index foot-gun (see §5, Risk 5 of the roadmap).

Key types exported from `crates/anvil-editor`:

| Type | Role |
|------|------|
| `Buffer` | Rope text store + undo stack + AI-native placeholders |
| `BufferId` | Opaque `u32` handle used by `EditorPaneRegistry` |
| `Position { line, col }` | Grapheme-aware cursor address |
| `Range` | `start..end` pair of `Position` |
| `Edit { range, replacement }` | Single atomic edit applied via `apply_edit` |
| `Cursor` | Buffer-level cursor position (wraps `Position`) |
| `EditProposal`, `GhostTextSpan`, `RevisionTag` | AI-native stubs reserved for NE14 |

`pos_to_char_idx` converts a `Position` to a ropey char index using
`unicode-segmentation` grapheme iteration. Grapheme columns, not scalar
values, are the unit of col measurement throughout the codebase.

`revisions: u64` is bumped on every `apply_edit`. This counter is the
staleness anchor for the NE14 agent-edit proposal model: a `Proposal` whose
`base_revision` is behind the current `revisions` is auto-rejected.

## File IO and Encoding (NE2)

`Buffer::from_path(p)` and `Buffer::save(p)` provide the file surface.

- Size guard: files > 50 MB are refused with an explicit error. No streaming
  in v1.
- Encoding detection: UTF-8 BOM (`EF BB BF`), UTF-16 LE BOM (`FF FE`), and
  UTF-16 BE BOM (`FE FF`) are detected and converted on read. Everything else
  is attempted as UTF-8; invalid bytes return an error.
- Atomic save: `Buffer::save` writes `path.tmp` then `rename`s over `path`,
  preventing partial writes.
- `mtime` is recorded on load; `is_externally_modified()` compares the
  current on-disk mtime against the stored value for external-change
  detection.
- `save` calls `flush_undo_group()` so the undo history boundary aligns with
  each save.

## Undo/Redo (NE3)

The undo model lives entirely in `buffer.rs` alongside the rope. It is not a
separate crate.

`EditRecord { edit, inverse, at: Instant }` is built by `apply_edit_at`
(internal): the pre-edit text is captured, the rope is mutated, and the
inverse `Edit` is derived. Records are grouped into undo groups on
`UndoStack`.

**Coalescing rule:** consecutive single-character inserts that are (a) buffer-
adjacent and (b) within 500 ms of each other merge into one group. The group
breaks on:
- `flush_undo_group()` — the explicit break hook called by callers on cursor
  jump, selection change, or save.
- A deletion or multi-char insert.
- A 500 ms elapsed gap between keystrokes.

`UndoStack` holds two `VecDeque<Vec<EditRecord>>` (`undo` and `redo`). The
cap defaults to 1000 groups; the oldest group is evicted when the cap is
exceeded. `undo()` applies inverses in reverse order and pushes the group to
redo. `redo()` re-applies forward edits and pushes back to undo. Both bump
`revisions`.

## EditorPane and Registry (NE4)

NE4 introduced two structures that allow editor panes to coexist with terminal
panes in the existing `PaneTree`.

### Pane shape change

`Pane.terminal: Terminal` became `Pane.terminal: Option<Terminal>`. A sibling
`Pane.editor_id: Option<BufferId>` was added. Exactly one of the two is
`Some` for any live pane. All ~60 call sites in `main.rs` that accessed
`pane.terminal.*` directly are now guarded with `if let Some(terminal) =
&pane.terminal` or equivalent. The `PaneTree`, split/close/focus model, and
divider drag are untouched — editor panes are leaves in the same tree.

### EditorPane

`EditorPane` in `crates/anvil-workspace/src/editor_pane.rs` holds the pane-
level view state for a native editor:

| Field | Role |
|-------|------|
| `buffer_id` | Links to the `Buffer` in `EditorPaneRegistry` |
| `cursor` | Primary `Cursor` (multi-cursor deferred to NE13) |
| `selection` | Current selection range |
| `scroll_pos / scroll_target / scroll_vel` | Smooth-scroll reusing existing `Pane` easing types |

### EditorPaneRegistry

`EditorPaneRegistry` in the same file is owned by `Tab`. It holds two maps:
- `HashMap<PaneId, EditorPane>` — view state per pane.
- `HashMap<BufferId, Buffer>` — one buffer per pane (single-buffer-per-pane
  is a locked decision; see roadmap §6).

`next_buffer_id` is a monotonic counter. `Tab::split_native_editor(dir)`
calls `PaneRegistry::peek_next_id()` → `Pane::new_editor(id, buffer_id)` →
`create_and_register_editor(buffer_id)` → `tree.split`.

### Keybinding

`Cmd+E` triggers `new_native_editor_pane()` in `main.rs`, which opens an
empty buffer in a split. (NE15 retired the nvim path; `Cmd+E` is now the
sole editor pane keybind. The `Cmd+Opt+E` side-channel and the
`editor.backend` config key were removed in the same dispatch.)

## Editor Render Path (NE5)

`draw_editor_into` in `crates/anvil-render/src/editor.rs` is the NE5
deliverable, mirroring `draw_viewport_into` from the terminal render path.

Rendering steps in order:

1. **Background fill** — charcoal (`theme.graphite`) across the full rect.
2. **Gutter** — width is `buffer.line_count().to_string().len() + 2` columns.
   Line numbers are right-aligned in `theme.text_muted`. Lines outside the
   buffer are left blank.
3. **Text rows** — graphemes are iterated per visible row via
   `unicode-segmentation`. Each grapheme paints in `theme.foreground` using
   the shared cell-grid atlas and metrics. Long lines that exceed the pane
   width are clipped with a `▸` overflow marker.
4. **Cursor** — a 2 px vertical bar in `theme.accent` at the cursor's column.
5. **Selection wash** — `fill_pixel_rect_alpha` at α=0.18 using
   `theme.accent_ember` over the selection range.
6. **Scroll** — integer-row-aligned (`floor(scroll_pos)`). The smooth-scroll
   easing values from `EditorPane` drive the offset.

`draw_workspace` in `crates/anvil-render/src/workspace.rs` gains an
`editor_panes: &EditorPaneRegistry` parameter. For editor-pane leaves it
calls `draw_editor_into` (looking up both the `EditorPane` and its `Buffer`
from the registry) in place of the previous stub. Terminal-pane leaves are
unchanged.

## Relationship to Existing Concepts

- **Block model** (`concepts/block-model.md`): the native editor has no block
  structure. Blocks are an OSC 133 terminal concept and are not part of editor
  panes. The two surface types coexist in the `PaneTree` as siblings but do
  not interact.
- **Layout modes** (`concepts/layout-modes.md`): editor panes participate in
  both `Terminal` and `Ide` layout modes. The `Ide` context bar now reads
  the focused native editor pane's `Buffer::tracked_path` directly (NE15);
  the old `EditorSnapshot`-based nvim bridge segment was retired in the
  same dispatch.
- **Agent actions** (`concepts/agent-actions.md`): the AI-native data model
  (NE14) will expose `EditorAction` variants over `anvil-control` for agent
  read/propose/accept/reject. The field stubs (`EditProposal`, `GhostTextSpan`,
  `RevisionTag`) on `Buffer` are reserved for that phase. Nothing in NE1–NE5
  wires agent actions to the editor.
- **Workspace panes** (`concepts/workspace-panes.md`): editor panes are leaves
  in the existing `PaneTree`. The split, close, focus, and divider-drag model
  is unchanged.

## Syntax Highlighting (NE8)

`SyntaxLayer` in `crates/anvil-editor/src/syntax.rs` provides per-buffer
tree-sitter parsing and highlight query results.

### Grammars wired

| Extension | Grammar crate | Crate version |
|-----------|---------------|---------------|
| `.rs` | `tree-sitter-rust` | 0.24.2 |
| `.ts`, `.tsx` | `tree-sitter-typescript` | 0.23.2 |
| `.py` | `tree-sitter-python` | 0.25.0 |
| `.toml` | `tree-sitter-toml-ng` | 0.7.0 |
| `.json` | `tree-sitter-json` | 0.24.8 |
| `.md`, `.markdown` | `tree-sitter-md` (block grammar) | 0.5.3 |

All grammars use `tree-sitter` core 0.25.x. Highlight queries come from each
grammar's bundled `HIGHLIGHTS_QUERY` constant (except `tree-sitter-md` which
uses `HIGHLIGHT_QUERY_BLOCK`).

### SyntaxRole and capture name mapping

```rust
pub enum SyntaxRole {
    Plain, Keyword, String, Number, Comment,
    Function, Type, Variable, Operator, Punctuation,
}
```

Capture names are matched on their first dot-segment so sub-captures like
`keyword.control` or `string.special` resolve correctly. Roles that map to
`Plain` are dropped from the span list to keep the cache lean.

### Cache strategy

`SyntaxLayer::visible_cache` is a `Option<((usize, usize), Vec<...>)>` keyed
by `(start_byte, end_byte)`. It is:
- Populated on the first `highlights_for_range` call for a given range.
- Returned unchanged on a subsequent call with the same key (cache hit).
- Cleared by `invalidate()`, which is called from `Buffer::apply_edit_at` on
  every edit. This means edits always invalidate, but unchanged frames are
  free after the first query.

This one-slot cache avoids re-querying tree-sitter every frame for the same
visible window while keeping correctness trivial (no partial invalidation).

### SyntaxTheme

`SyntaxTheme` is added to `anvil-theme::theme` and embedded as `Theme::syntax`:

```rust
pub struct SyntaxTheme {
    pub keyword, string, number, comment, function,
    pub type_, variable, operator, punctuation: [u8; 3],
}
```

All four built-in themes (`EMBER_DARK`, `EMBER_LIGHT`, `MINERAL_DARK`,
`MINERAL_LIGHT`) populate the `syntax` field using existing palette roles:
`accent_bright` for keywords, `verified` for strings, `attention` for numbers,
`text_subtle` for comments, `info` for functions, `agent` for types.

### Buffer integration

`Buffer` gains `pub syntax: SyntaxLayer`. Initialised as `SyntaxLayer::new()`
in `Buffer::new` and `Buffer::from_text`. `Buffer::from_path` calls
`syntax.set_language_from_path(path)` then `syntax.parse(&text)` after
loading. `Buffer::apply_edit_at` calls `syntax.invalidate()` after every rope
mutation.

The incremental `SyntaxLayer::edit(InputEdit, text)` path (tree-sitter's
`tree.edit` + re-parse from the old tree) is available for callers that
construct an `InputEdit` from the edit coordinates; `Buffer::apply_edit` uses
`invalidate()` only (v1 simplicity — full incremental wire-up is a follow-up
in the render path, NE5 integration).

### Release binary size

`target/release/anvil` after adding all six grammar crates: **5.9 MB**
(baseline before NE8 was not measured separately, but the 20 MB ceiling is
comfortably cleared). All grammar C parsers are compiled into the binary.

## In-Buffer Search (NE11)

`EditorSearch` in `crates/anvil-workspace/src/editor_search.rs` holds the
in-pane search state for native editor panes.

```rust
pub struct EditorSearch {
    pub query: String,
    pub is_regex: bool,
    pub hits: Vec<Range>,
    pub current: usize,
}
```

`rescan(buffer)` rebuilds `hits` from `buffer.to_text()`. Literal mode uses
`str::match_indices`; regex mode uses the `regex` crate (already in workspace
via root `Cargo.toml`). Byte offsets from match results are converted to
`Position` via `buffer.char_to_line` + `buffer.line_to_char`.

`EditorPane.search: Option<EditorSearch>` is `None` when search is closed.
Six new `EditorAction` variants drive it: `SearchOpen`, `SearchClose`,
`SearchSetQuery(String)`, `SearchNext`, `SearchPrev`, `SearchToggleRegex`.
`SearchNext`/`SearchPrev` set `cursor.anchor = hit.start`, `cursor.pos =
hit.end`, and `scroll_target = hit.start.line as f32` so the match is
visible and selected.

`draw_search_bar` in `anvil-render/src/searchbar.rs` now accepts an
`editor_search: Option<&EditorSearch>` parameter. When `Some`, it reads
`query`/`count`/`current` from the editor search instead of the terminal
`Search`. The "find: " prefix is used in both cases (no scope tag for editor
search).

`main.rs` routing: `open_search` branches on `focused_is_native_editor()` —
if true, dispatches `SearchOpen` to the registry instead of scanning the
terminal. The `search_next`/`search_prev`/`search_regex_toggle` keybind
blocks branch similarly. The `key_down` search-open handler branches on the
native editor case to route Backspace and Char keystrokes to `SearchSetQuery`.

## LSP UI Surfaces (NE10, partial — 2026-05-25)

NE10 landed diagnostics gutter rendering and the hover popup (Cmd+K). Deferred to a follow-up dispatch: autocomplete popup, code action menu, definition jump.

### Diagnostics gutter (NE10)

`RenderDiagnostic { line, severity }` and `RenderSeverity` live in `crates/anvil-render/src/editor.rs`. This keeps the render crate cycle-safe (no dependency on `LspManager`). `main.rs` translates `DocumentDiagnostic → RenderDiagnostic` each frame and passes a `HashMap<PaneId, Vec<RenderDiagnostic>>` to `draw_workspace`, which forwards a per-pane slice to `draw_editor_into`.

Per visible row that has a diagnostic: a 4 px wide colored stripe at the left edge of the gutter and a row tint at α=0.06. Color mapping: Error → `theme.failure`, Warning → `theme.attention`, Info → `theme.info`, Hint → `theme.alloy`. Worst-severity wins when multiple diagnostics share a line.

### Hover popup (NE10)

`HoverPopup { text, anchor }` on `EditorPane`. Cmd+K in a native editor pane calls `trigger_hover_request()` → `LspManager::request_hover(path, line, character)` → stores `(pane_id, request_id)` in `App::pending_hover`. Each tick, `poll_hover_result()` checks `LspManager::poll_hover(request_id)`. On arrival, `EditorPane::hover_popup` is set. The render path in `draw_editor_into` paints a floating panel (surface background, border outline, glyph text) anchored one row below the cursor.

`EditorAction::HoverRequest` clears the popup (stale state while request is in-flight). `EditorAction::HoverDismiss` clears it on Esc.

`LspManager` changes: `LspCommand::Hover { path, line, character, request_id }`, `ServerHandle::hover_result: Arc<Mutex<Option<(u64, HoverResult)>>>`, `extract_hover_text` parses `HoverContents` (Scalar/Array/Markup). `HoverResult { text }` is re-exported from `anvil-editor::lsp`.

### Deferred (NE10 follow-up)

- Autocomplete popup (triggered on insert_char, Up/Down navigate, Tab/Enter accept)
- Code action menu (Cmd+.)
- Definition jump (Cmd+click)

## NE15 — Nvim Path Retired

The BR3/BR4 nvim RPC bridge was deleted in NE15:

- `crates/anvil-editor/src/nvim/` submodule (bridge, codec, transport)
  removed; `anvil-editor` re-exports trimmed to native types only.
- `App` fields removed: `editor_bridge`, `editor_snapshot`, `editor_pane_id`,
  `editor_socket_counter`, `nvim_path`.
- `App::new_editor_pane` (nvim PTY spawner) and `clear_editor_pane_if`
  removed. `Cmd+E` and `Action::NewEditorPane` now invoke
  `new_native_editor_pane` unconditionally.
- `editor.backend = "nvim" | "native"` config key removed; `EditorCfg`
  dropped from `anvil-config`.
- Context-bar editor segment now reads the focused native editor pane's
  `Buffer::tracked_path` (basename, or `[scratch]`). No modified flag yet —
  `Buffer` does not track dirty-since-save state.
- Left-dock outline section now passes `None` (the nvim BR5 LSP outline
  pull is gone). Native LSP (NE9) does not yet feed the outline; a
  follow-up will wire `editor_pane.outline` to the native LSP layer.

## Open / Low-confidence

- Soft-wrap is off in v1 (NE5 clips long lines). The roadmap lists it as out
  of scope. The `▸` marker matches terminal behavior.
- Native LSP outline does not yet drive the left-dock outline section; that
  panel renders blank in Ide mode until a follow-up wires it.
