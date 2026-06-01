# Native Editor Roadmap (NE track)

Status: design handoff. Phase prefix: `NE`. Effective next session.
Supersedes the nvim-as-editor direction implied by BR3/BR4 — see §7 Migration.

---

## 1. Vision

Anvil owns the editor. A native text-editing surface, rendered with the same
Metal cell grid that already paints the terminal, eliminates the nvim RPC
roundtrip from every AI interaction. Caldera agents read and write buffers
through an in-process API; ghost-text completions paint in the same frame as
the user's cursor; diffs, blame, and code actions render in the same theme as
the terminal. The native editor is the surface that lets Anvil become the
single console for 100% of the work — terminal, browser, agents, *and* code,
without process boundaries.

## 2. Reuse Inventory

The editor is mostly assembly of existing parts. New code is the buffer, the
syntax/LSP layer, and the input mapping. Everything visual is already built.

| Existing component | File | How NE uses it |
| --- | --- | --- |
| Cell grid + font shaping (BlexMono, CoreText, ligatures) | `crates/anvil-platform/src/font.rs`, `crates/anvil-render/src/raster.rs` | Editor rows render as cell rows. Same atlas, same metrics. |
| Viewport draw loop (cells → GPU) | `crates/anvil-render/src/draw.rs::draw_viewport_into` | Adapt for editor: positioned char + fg/bg per cell, no PTY semantics. |
| Selection + clipboard | `crates/anvil-term/src/selection.rs`, `crates/anvil-platform` NSPasteboard | Same selection wash, same Cmd+C/V plumbing. |
| Pane tree (split / resize / focus / divider drag) | `crates/anvil-workspace/src/layout.rs` | Editor panes are leaves alongside terminal panes. |
| Theme | `crates/anvil-theme` | Syntax tokens map to existing palette roles. |
| Tabs, command palette, search bar, scrollback | `crates/anvil-workspace`, `ui/palette/` | All work without modification. |
| Searchbar (in-buffer find) | `crates/anvil-render/src/searchbar.rs`, `crates/anvil-term/src/search.rs` | Generalize the search target from `Grid` to a `Searchable` trait. |
| `anvil-editor` crate (nvim bridge) | `crates/anvil-editor/{bridge,codec,transport}.rs` | Repurpose: drop nvim modules, host the native editor model. (See §7.) |
| `Cmd+E` "New Editor Pane" + `Action::NewEditorPane` | `crates/anvil-workspace/src/palette.rs`, `main.rs` chord block | Re-point to native editor spawn instead of nvim. |

## 3. Phase List

Each phase: name, scope, deps, deliverable, verify, est. dispatches (1–3).

### NE1 — Buffer crate skeleton
- **Scope.** Replace `anvil-editor` contents with a new `buffer.rs` module
  built on `ropey`. Types: `Buffer`, `Cursor`, `Position { line, col }`,
  `Edit { range, replacement }`, `BufferId`. Operations: `insert_char`,
  `insert_str`, `delete_range`, `replace_range`, `line(n)`, `line_count`,
  `char_at`, `byte_len`, `char_to_line`, `line_to_char`. UTF-8 only;
  grapheme iteration via `unicode-segmentation`.
- **Deps.** None.
- **Deliverable.** `anvil-editor::Buffer` with 100% unit coverage on edit ops.
- **Verify.** `cargo test -p anvil-editor` — insert/delete/replace, line
  splitting, multi-byte chars (emoji, CJK), empty-buffer edge cases.
- **Dispatches.** 1.

### NE2 — File IO + encoding detection
- **Scope.** `Buffer::from_path(p)` and `Buffer::save(p)`. Read on the
  caller's thread for v1 (size guard: refuse files > 50 MB with explicit
  error — large-file streaming is a later phase). Encoding: detect UTF-8 vs
  UTF-16 LE/BE from BOM; everything else attempted as UTF-8 with an
  invalid-bytes error. Atomic save: write `path.tmp`, `rename` over `path`.
  Track on-disk `mtime` on the buffer; expose `is_externally_modified()`.
- **Deps.** NE1.
- **Deliverable.** Open/save round-trip for ASCII, UTF-8, and UTF-16 BOMed
  files. mtime change detection.
- **Verify.** Unit: round-trip every encoding through a `tempfile`. Unit:
  external touch flips `is_externally_modified` after a reload check.
- **Dispatches.** 1.

### NE3 — Undo/redo
- **Scope.** Inverse-edit stack on `Buffer`. `apply_edit` records the inverse
  on the undo stack. `undo()` / `redo()` apply the top and push to the
  opposite stack. Grouping: coalesce consecutive single-char inserts that are
  (a) adjacent in buffer and (b) within 500 ms, breaking on cursor jump,
  selection, deletion, or save. Cap: configurable, default 1000 groups.
- **Deps.** NE1.
- **Deliverable.** `Buffer::undo()` / `Buffer::redo()` with grouping.
- **Verify.** Unit: type "hello", one undo restores empty buffer (single
  group). Unit: type "hi", arrow-move, type "lo" — two groups. Unit: redo
  after undo restores. Unit: depth cap evicts oldest.
- **Dispatches.** 1.

### NE4 — EditorPane + PaneRegistry integration
- **Scope.** Introduce `PaneContent` enum (or sibling registry) so a pane can
  hold either a `Terminal` or an `EditorPane`. The minimal-blast-radius shape
  is a new `EditorPaneRegistry` in `anvil-workspace`, keyed by `PaneId`,
  parallel to the existing terminal `PaneRegistry`. `Pane`'s terminal field
  becomes `Option<Terminal>`, with a sibling `editor_id: Option<BufferId>`.
  Lookups branch on which is `Some`. The existing PaneTree, splits, divider
  drag, and focus model are untouched. `EditorPane` owns: `BufferId`, primary
  `Cursor`, `Selection`, `scroll_pos`, `scroll_target` (reusing the same
  smooth-scroll types). `App::new_editor_pane` allocates a `PaneId`, creates
  an empty `Buffer`, registers both.
- **Deps.** NE1.
- **Deliverable.** `Cmd+E` opens a native editor pane (empty buffer). Splits
  and focus nav work alongside terminal panes.
- **Verify.** Unit: split → close → split keeps tree invariants intact with
  mixed pane kinds. Manual: `Cmd+E`, `Cmd+D`, focus nav, divider drag.
- **Dispatches.** 2.

### NE5 — Editor render path
- **Scope.** `draw_editor_into(sink, editor_pane, buffer, theme, …)` mirroring
  `draw_viewport_into` but sourcing cells from buffer rows. Soft-wrap is
  **off** in v1 — long lines clip at the right edge with a faint glyph marker
  (matches terminal behavior the user already accepts). Gutter: left-anchored
  line numbers in `theme.text_muted`, right-aligned, monospace. Cursor:
  reuse `draw_cursor`. Selection wash: reuse `fill_pixel_rect` from terminal
  selection. Scroll: reuse the smooth-scroll easing types. No syntax color
  yet — everything paints `theme.fg`.
- **Deps.** NE4.
- **Deliverable.** Static text buffer renders correctly at all zoom levels
  with gutter, cursor, scroll.
- **Verify.** Snapshot test (cell grid → string) for a known buffer + cursor
  position. Manual: open a 2000-line file, scroll top-to-bottom, no tearing.
- **Dispatches.** 2.

### NE6 — Keyboard input (insert mode)
- **Scope.** No modal modes. Insert is the only mode. Keymap:
  printable chars → `insert_char`; Return → `\n`; Backspace/Delete; Arrows;
  Home/End (line); Cmd+Home/End (buffer); PgUp/PgDn; Tab → configurable
  (default: 4 spaces); Cmd+S save; Cmd+Z undo; Cmd+Shift+Z redo;
  Cmd+C/V/X clipboard; Cmd+A select all; Cmd+L go to line. Selection
  shift-extend on every cursor-move key. Decision in §6: defer modal vim
  emulation indefinitely; expose `anvil-control` actions so a future modal
  layer (or a vim plugin) can be a thin keymap on top.
- **Deps.** NE3, NE5.
- **Deliverable.** Type, edit, save, undo, redo, copy, paste, cut.
- **Verify.** Unit per action (action → buffer state assertion). Manual:
  daily-driver smoke — open file, edit, save, reopen, verify.
- **Dispatches.** 2.

### NE7 — Mouse input
- **Scope.** Click → cursor at hit cell (reuse `hit_test` math, translated to
  buffer position). Drag → selection. Double-click → word select (reuse
  terminal word-boundary logic). Triple-click → line select. Shift+click →
  extend selection. Scroll wheel → existing smooth-scroll. Cmd+click
  multi-cursor is **deferred** to NE13.
- **Deps.** NE6.
- **Deliverable.** Full point-and-click editing.
- **Verify.** Unit: pixel-to-buffer-position round-trip across font sizes.
  Manual.
- **Dispatches.** 1.

### NE8 — Syntax highlighting (tree-sitter)
- **Scope.** Add `tree-sitter` (0.25+) and grammars: `tree-sitter-rust`,
  `tree-sitter-typescript`, `tree-sitter-python`, `tree-sitter-toml`,
  `tree-sitter-json`, `tree-sitter-md`. Filetype detection by extension only
  in v1. `SyntaxLayer` per buffer: holds the `Tree`, runs a Highlight query
  on the visible row range, caches `Vec<(byte_range, capture_name)>` per
  line. Edits: convert `Edit` → `InputEdit` and call `tree.edit` then
  `parser.parse(text, Some(&tree))`. Map capture names → theme roles via a
  small `SyntaxTheme` table (added to `anvil-theme`): `keyword`, `string`,
  `number`, `comment`, `function`, `type`, `variable`, `operator`,
  `punctuation`. Render path looks up per-cell fg via the cache.
- **Deps.** NE5.
- **Deliverable.** Rust / TS / Python files paint with semantic color.
- **Verify.** Unit: known Rust snippet → expected capture spans. Unit: edit
  invalidates only affected lines (cache hit on untouched rows). Manual:
  scroll a large file, no stutter.
- **Dispatches.** 2–3.

### NE9 — LSP client core
- **Scope.** Use `async-lsp` (preferred — lighter, no Tokio coupling forced
  on us; revisit if rust-analyzer needs more). Single `LspManager` owned by
  `App`. On buffer open of a recognized filetype, spawn the server
  (configurable path, default: rust-analyzer / typescript-language-server /
  pyright). Lifecycle: `initialize`, `initialized`, `didOpen`, `didChange`
  (full-doc sync v1; incremental in a follow-up), `didSave`, `didClose`,
  `shutdown` on app quit. Request handlers wired but not surfaced: `hover`,
  `completion`, `definition`, `references`, `codeAction`. Publish-diagnostic
  notifications stored on a per-buffer `Diagnostics` struct. **No UI** in
  this phase — verification is via a tracing log and a "show last
  diagnostics" debug command.
- **Deps.** NE2, NE6.
- **Deliverable.** rust-analyzer attaches to an open `.rs` file; diagnostics
  arrive and are queryable in the registry.
- **Verify.** Manual: open a `.rs` with a deliberate type error, watch the
  log show the published diagnostic.
- **Dispatches.** 2–3.

### NE10 — LSP UI surfaces
- **Scope.** Render diagnostics: gutter glyphs (•/!/?) colored by severity
  using `status.failure` / `status.attention` / `status.info`; underline
  wash on the offending range. Hover tooltip: Cmd+hover or Cmd+K K shows a
  Metal-rendered popover (new render helper, reuse atlas). Autocomplete
  popup: triggered on identifier chars, navigable with arrows, Tab/Return
  accepts. Code-action menu: Cmd+. opens a list. Definition jump: Cmd+click
  on identifier → open file + cursor. References: Cmd+Shift+F12 opens a
  project-search-style results pane (see NE12 for the results-pane chrome).
- **Deps.** NE9, NE12 (for references results pane).
- **Deliverable.** Daily-driver LSP UX.
- **Verify.** Manual: error → glyph + wash visible; hover → tooltip; type
  partial identifier → completions; Cmd+. → quickfix; Cmd+click jumps.
- **Dispatches.** 3.

### NE11 — In-buffer search
- **Scope.** Generalize `Search` to operate on either a `Grid` (terminal) or
  a `Buffer` (editor) via a `Searchable` trait with two methods: `line_str`
  and `line_count`. Cmd+F opens the existing search bar against the focused
  pane. Match highlight overlays via the existing wash.
- **Deps.** NE5.
- **Deliverable.** Cmd+F works in editor panes.
- **Verify.** Unit: trait impl for `Buffer` returns same match set as a
  reference regex pass. Manual.
- **Dispatches.** 1.

### NE12 — Project-wide search
- **Scope.** Use `ignore` + `grep-regex` + `grep-searcher` crates (the
  ripgrep building blocks — proper library API, no shelling out). New
  palette mode triggered by Cmd+Shift+F: input → live results list grouped
  by file. Selecting a result opens the file in a new editor pane (or
  focuses an existing one) at the line. Results pane is a Metal-rendered
  surface using the same row-style renderer as the agent panel.
- **Deps.** NE6.
- **Deliverable.** Project search palette.
- **Verify.** Unit: result formatter matches ripgrep CLI on a fixture repo.
  Manual.
- **Dispatches.** 2.

### NE13 — Git integration
- **Scope.** Use `gix` (gitoxide) — pure Rust, no libgit2 C dep. Per buffer:
  load the parent commit's blob, diff against the working buffer text on
  every save (and debounced 1s on edit). Gutter glyphs: `+` (verified
  green), `~` (attention amber), `-` (failure red), painted in the gutter
  column alongside line numbers. Blame: lazy per-line, fetched on first
  cursor-hover-pause; status-bar shows `<sha7> <author> · <relative date>`
  for the cursor line. Multi-cursor (deferred from NE7) belongs here as a
  small standalone step — gutter glyph for secondary cursors, Cmd+click adds.
- **Deps.** NE5, NE7.
- **Deliverable.** Live diff gutter; blame on status bar.
- **Verify.** Unit: diff against a known fixture commit. Manual.
- **Dispatches.** 2.

### NE14 — AI-native edit API
- **Scope.** Land the data model from §8. Wire `anvil-control` to expose
  `EditorAction::{ReadRange, ProposeEdit, AcceptProposal, RejectProposal,
  SetGhostText, ClearGhostText}` over the existing inbound bridge. Ghost
  text renders inline at the cursor in `theme.text_muted` italic; Tab
  accepts. Proposals render as a diff overlay (insertion = verified-green
  wash, deletion = failure-red strike) with an accept/reject affordance.
  Caldera agent integration is out of scope here — `anvil-caldera` will
  add a thin client in a follow-up — but the API surface is the deliverable.
- **Deps.** NE6, NE10.
- **Deliverable.** Inbound message → buffer mutation works end-to-end (test
  harness pumps messages, asserts buffer state).
- **Verify.** Unit: each EditorAction round-trips correctly. Unit: a
  pending proposal blocks a conflicting user edit until resolved.
- **Dispatches.** 2.

### NE15 — Nvim path removal
- **Scope.** Delete `anvil-editor::{bridge,codec,transport}` and the BR3
  nvim attach. Remove `which::which("nvim")`, the BR4 spawn, the socket
  bookkeeping, the `editor_pane_id` singleton, and the context-bar
  `EditorSnapshot` field (replaced by an `EditorPane`-aware variant that
  reads from the new buffer registry). Update wiki accordingly. This phase
  ships only after NE6 is stable enough to be a daily driver and the user
  signs off. (See §7 for the coexistence period.)
- **Deps.** NE6 (minimum), NE10 strongly recommended.
- **Deliverable.** No nvim dependency. `anvil-editor` is fully native.
- **Verify.** `cargo test --workspace`. Manual: no regressions across all
  existing flows. Grep for `nvim` returns zero hits in source.
- **Dispatches.** 1.

## 4. Critical Path

```
NE1 ── NE2 ──────────────────────────────── NE9 ── NE10 ─┐
  └─── NE3 ── NE6 ── NE7 ── NE13            │           │
              │       │                      │           │
              │       └──────── NE14 ────────┘           │
              │                                          │
NE4 ── NE5 ── NE8                                        │
       ├──── NE11                                        │
       └──── NE12 ──────────────────────────────────────┘
                                                         │
                                NE15 (after NE6 stable) ─┘
```

**Sequential spine:** NE1 → NE3 → NE4 → NE5 → NE6 unblocks everything.
**Parallelizable after NE6:**
- NE7 (mouse) and NE8 (syntax) and NE11 (search) are independent.
- NE9 (LSP core) is independent of NE7/NE8/NE11.
- NE10 needs NE9 *and* NE12 (only because references reuses the results pane).
- NE12 (project search) and NE13 (git) are independent of LSP.
- NE14 (AI hooks) needs NE6 minimum; the diff-overlay polish wants NE10.

**Practical execute order:** NE1, NE2, NE3, NE4, NE5, NE6 in strict
sequence. Then NE7 + NE8 + NE11 can be three back-to-back small dispatches.
NE9 + NE12 + NE13 next. NE10, NE14, NE15 last.

## 5. Risks

1. **Tree-sitter highlight quality.** Off-the-shelf highlight queries vary
   in quality per grammar. Mitigation: ship our own minimal `highlights.scm`
   files under `assets/tree-sitter/` rather than depending on upstream.
2. **LSP server install burden.** Users don't have rust-analyzer / pyright
   on PATH by default. Mitigation: NE9 ships with a config `editor.lsp.<lang>
   = { command, args }` and a clear status-bar indicator when the server is
   missing. No auto-install — that path leads to bundling pain.
3. **Large-file perf.** Ropey handles 100 MB fine for ops, but syntax +
   diagnostics + diff + blame across that range becomes O(file). Mitigation:
   NE2 caps reads at 50 MB; NE8/NE9/NE13 each restrict to viewport ± 500
   lines for the active recompute; user opt-in for "no syntax / no LSP /
   no git" per buffer when working with logs or generated code.
4. **Undo correctness under concurrent agent edits.** Agent-applied edits
   (NE14) and user undo can interleave pathologically. Mitigation: agent
   edits land in their own undo group with a `source: Agent` tag; undo
   prompts before crossing an agent boundary (HUD-style toast).
5. **Ropey API quirks.** Byte vs char vs line index space is a known
   foot-gun. Mitigation: NE1 wraps ropey behind a `Position { line, col }`
   facade; raw ropey indices never leak past `Buffer`.
6. **async-lsp Tokio assumption.** async-lsp does require a Tokio runtime.
   We don't have one yet. Mitigation: spawn a dedicated `tokio::Runtime` in
   `LspManager` (one thread is fine for v1 since LSP is IO-bound). Decision
   locked in §6.
7. **Tree-sitter incremental reparse on every keystroke.** Naive impl will
   reparse the full visible region. Mitigation: NE8 caches per-line capture
   spans keyed by `(line_text_hash, tree_version)`; only invalidate lines
   touched by the edit's `InputEdit` span.
8. **Coexistence period drift.** While both nvim and native exist (between
   NE6 and NE15), it's easy for one path to regress. Mitigation: BR3/BR4 go
   behind a `editor.backend = "nvim" | "native"` config (default `nvim`
   until NE6 ships, flipped to `native` at NE6, code removed at NE15).
9. **Brand drift.** Nothing in `BRAND.md` is editor-specific yet. Risk that
   syntax colors fight the Mineral palette. Mitigation: NE8's `SyntaxTheme`
   table is authored by design-lead, not picked ad hoc. Status colors
   (diagnostics) reuse the locked palette roles in §3.

## 6. Decisions To Lock Before NE1

1. **Rope crate: `ropey` 1.6+.** Mature, used by Helix and Lapce. Faster
   than xi-rope for our edit patterns. Decision: locked.
2. **LSP client: `async-lsp`.** Lighter than `tower-lsp` for client use
   (`tower-lsp` is server-oriented). Pulls in Tokio; acceptable cost. We
   own the runtime construction in `LspManager`. Decision: locked, with
   `tower-lsp` as the fallback if async-lsp has a blocker.
3. **Single buffer per pane vs tabs-of-buffers.** *Single buffer per pane*.
   Anvil already has tabs at the window level; adding a second tab tier
   inside the editor pane fights the existing model. To "switch buffer"
   the user opens a new editor pane (Cmd+E) and uses the file palette
   (Cmd+P, see NE12 follow-up). Decision: locked.
4. **Modal modes (vim).** *No*. Insert mode only. The `anvil-control`
   action surface is structured so a future modal layer is a keymap-on-top,
   not a rewrite. Decision: locked.
5. **Git library: `gix` (gitoxide).** Pure Rust, no libgit2 build cost.
   Decision: locked.
6. **Project search: ripgrep crates (`ignore` + `grep-*`).** Not shell-out.
   Decision: locked.
7. **Encoding scope.** UTF-8 native; UTF-16 read+write only when BOM
   present. Latin-1 and friends are explicit out-of-scope for v1.
   Decision: locked.
8. **File size limit (v1).** 50 MB. Larger files refused with explicit
   error. Decision: locked, revisit when a real user hits it.

## 7. Migration Plan (nvim → native)

The transition is staged so the user always has a working editor.

| Stage | nvim path | native path | User experience |
| --- | --- | --- | --- |
| Today | BR3/BR4: `Cmd+E` spawns nvim. | None. | Status quo. |
| NE1–NE5 | Unchanged. | New crate work, no user-visible surface. | Status quo. |
| NE6 ships | Still default; `editor.backend = "native"` opt-in. | `Cmd+E` opens native when opt-in. | Dogfood the native editor on the user's terms. |
| NE6 stable + user sign-off | Default flips to `"native"`. nvim retained behind explicit opt-in. | Default. | Native is the daily driver. |
| NE15 | Removed. | Sole path. | Codebase clean. |

Concretely: NE4 introduces the `editor.backend` config key. Both paths read
`Cmd+E` via `Action::NewEditorPane` and branch on `cfg.editor.backend`. The
nvim path is the BR3/BR4 code untouched.

**BR3 EditorBridge fate.** Retired at NE15. Until then it stays in
`crates/anvil-editor/`, but the crate's *primary* identity becomes the
native buffer model (NE1). The bridge moves into a `nvim/` submodule for
the coexistence period.

## 8. AI-Native Data Model (lands at NE14, designed now)

Designing this up front so NE1's `Buffer` and NE4's `EditorPane` don't need
breaking changes when agents arrive.

### Buffer-level

```rust,ignore
pub struct Buffer {
    rope: Rope,
    undo: UndoStack,
    proposals: Vec<Proposal>,        // pending agent edits, not yet applied
    ghost_text: Option<GhostText>,   // single inline suggestion at cursor
    revisions: u64,                  // monotonic counter, bumped on any apply
}

pub struct Proposal {
    pub id: ProposalId,
    pub source: EditSource,          // User | Agent { agent_id, run_id }
    pub edits: Vec<Edit>,            // multi-edit proposals (e.g. rename)
    pub rationale: Option<String>,   // shown in the accept/reject UI
    pub base_revision: u64,          // proposal rejected if buffer moved past
}

pub struct GhostText {
    pub at: Position,
    pub text: String,
    pub source: AgentId,
}

pub enum EditSource { User, Agent { agent_id: AgentId, run_id: RunId } }
```

### Pane-level

```rust,ignore
pub struct EditorPane {
    pub buffer_id: BufferId,
    pub cursors: Vec<Cursor>,        // primary at [0]; multi-cursor at NE13
    pub selection: Selection,
    pub scroll_pos: f32,
    pub scroll_target: f32,
    pub agent_cursors: Vec<AgentCursor>, // remote presence (multi-agent)
}

pub struct AgentCursor {
    pub agent_id: AgentId,
    pub position: Position,
    pub color: StatusRole,           // status.agent or per-agent assignment
}
```

### Control-surface actions

Added to `anvil_control::Inbound`:

```rust,ignore
EditorAction::ReadRange       { buffer: BufferId, range: Range<Position> }
EditorAction::ProposeEdit     { buffer: BufferId, edits: Vec<Edit>, rationale }
EditorAction::AcceptProposal  { buffer: BufferId, proposal: ProposalId }
EditorAction::RejectProposal  { buffer: BufferId, proposal: ProposalId }
EditorAction::SetGhostText    { buffer: BufferId, at: Position, text: String }
EditorAction::ClearGhostText  { buffer: BufferId }
```

**Invariants enforced at the Buffer boundary, not in callers:**
- A `Proposal` whose `base_revision < buffer.revisions` is auto-rejected on
  accept-attempt (returns `EditOutcome::Stale`).
- A direct user edit while proposals exist does *not* invalidate them;
  proposals carry their own base_revision and are revalidated lazily.
- Ghost text clears on any buffer mutation that isn't its own acceptance.
- Multi-agent: `agent_cursors` is a free-form `Vec`; no presence protocol in
  v1 — the field exists so the render path can paint them when wired.

Why this matters now: NE1's `Buffer` constructor must allocate
`proposals: Vec::new()` and `revisions: 0`. NE3's undo must bump
`revisions`. NE4's `EditorPane` must own `Vec<AgentCursor>` even if it
stays empty. Adding any of these later is a breaking change to call sites.

## 9. Out Of Scope For V1

- Real-time collaboration over network (CRDT, OT). Multi-agent local
  presence yes; cross-machine collaboration no.
- Plugin / extension API. The editor is configurable, not extensible.
- Vim / emacs modal parity. Insert mode only.
- Full LSP semantic tokens. Tree-sitter is the source of color; LSP
  semantic tokens land if and when they materially improve a specific
  language.
- Snippets, refactor previews beyond what `codeAction` provides.
- Soft-wrap (long lines clip in v1).
- Folding (code folds, not the existing terminal block folds).
- Macros, multiple-clipboards, register-style yank.
- Built-in file tree pane. File operations route through the command
  palette and project search.
- Side-by-side diff view. NE13 ships gutter diff only; full diff view is a
  follow-up.
- Format-on-save. Wired through LSP `textDocument/formatting` in a
  follow-up after NE10 stabilizes.
- Per-language config beyond the `editor.lsp.<lang>` block. Indent size,
  tab style, ruler columns are global v1.

## 10. Open Questions

1. **Config key shape.** `editor.backend = "native" | "nvim"` (recommend)
   vs separate keys per pane action. Recommend backend key — single
   migration knob.
2. **Default LSP autostart.** Spawn rust-analyzer on first `.rs` open
   automatically, or require explicit `editor.lsp.rust.enabled = true`?
   Recommend autostart with a config kill switch.
3. **File palette (Cmd+P).** Reuse the existing command palette with a
   `file:` prefix, or a dedicated mode? Recommend prefix mode — one
   palette, one keybind to learn. Implementation lives alongside NE12.
4. **Agent edit approval UX.** Per-proposal accept/reject prompt vs an
   approval queue panel? Recommend per-proposal inline overlay for v1;
   queue panel is a NE14 follow-up if proposals stack.
5. **Buffer-with-no-file-yet.** New editor pane buffer name defaults to
   `[scratch]` or prompts for path on first save? Recommend `[scratch]`
   then save-as on Cmd+S if no path.

---

Build order: NE1 → NE2 → NE3 → NE4 → NE5 → NE6 → NE7 → NE8 → NE9 → NE10
→ NE11 → NE12 → NE13 → NE14 → NE15 — verify after each.
