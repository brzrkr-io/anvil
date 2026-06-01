---
date: 2026-05-25
kind: spec
status: live
target_agent: any (Claude, Codex, Cursor, etc.)
goal: turn the on-paper NE1-NE15 native-editor track into a real, usable, Zed-grade native IDE inside Anvil
---

# Native Editor Rescue Spec

## 0. Read this first

Anvil shipped NE1-NE15 as commits, but **the product is still a terminal
emulator with invisible editor stubs**. Cmd+E spawns a "native editor pane"
that is mechanically wired (rope buffer, undo/redo, LSP, syntax, etc.) but
visually presents as a black void with a faint "1" gutter and a 2px cursor
bar. The explorer renders a directory listing but **does nothing on click**.
There is **no file-open path in production code** — `Buffer::from_path` is
called only from tests. The user cannot actually open a file in the editor.

This spec is the work to make the native editor real.

The bar is **Zed**: instant file open from explorer, file picker, in-pane
buffer tabs, clear pane chrome, mouse-first everything, no terminal
appearing where an editor should be.

## 1. Required reading (in this order, before editing anything)

1. `AGENTS.md` — work rules, build/verify, brand gate.
2. `BRAND.md` — type system, palette, semantic colors.
3. `context/2026-05-25-native-editor-roadmap.md` — original NE1-NE15 plan.
4. `context/2026-05-24-ide-redesign.md` — earlier ID1-ID5 IDE layout intent.
5. `wiki/concepts/native-editor.md` — current concept page (NE2-NE5 summary).
6. `wiki/concepts/layout-modes.md` — Terminal vs Ide mode geometry.
7. This file in full before touching code.

Build/verify (every change):
- `scripts/run.sh` — builds **both** binaries and launches. NOT `cargo run -p anvil`.
- `cargo test --workspace --lib`
- `cargo clippy --workspace -- -D warnings`
- `cargo fmt --all`

## 2. Honest current state

What works mechanically:
- `Buffer` (rope, grapheme positions, file IO w/ BOM, undo/redo, mtime tracking).
- `EditorPane` + `EditorPaneRegistry` (`crates/anvil-workspace/src/editor_pane.rs`).
- `Cmd+E` chord at `crates/anvil/src/main.rs::handle_cmd_chord` → `new_native_editor_pane` at `main.rs:948` → `Tab::split_native_editor` at `crates/anvil-workspace/src/tab.rs:164`.
- `draw_editor_into` at `crates/anvil-render/src/editor.rs:68` paints bg + gutter + line numbers + cursor + selection + git gutter + diagnostics + ghost-text.
- LSP infra (NE9-NE12), tree-sitter syntax (NE8), git gutter (NE13), multi-cursor + AI edit + ghost-text (NE14).
- nvim path retired (NE15).

What is broken or missing:
- **Explorer is decorative.** `crates/anvil-render/src/left_dock.rs` walks the cwd via `fs_worker` and paints rows, but there is no mouse hit testing and no click handler. Clicking a file does nothing.
- **No file-open production path.** `Buffer::from_path` (`crates/anvil-editor/src/buffer.rs:293`) has zero production callers. No `Action::OpenFile`, no `Cmd+O`, no file picker.
- **Editor pane has no chrome.** No header bar showing filename, dirty dot, language. A blank editor pane is visually identical to a fresh terminal pane on the same dark background.
- **Cmd+E spawns blank scratch.** Splits the current tab, gives you an empty rope, no filename, no placeholder text, no affordance saying "this is an editor".
- **No in-pane buffer tabs.** `EditorPane` holds exactly one `BufferId`. Opening a second file replaces the pane or splits the tree.
- **Outline panel reads dead text.** `crates/anvil-render/src/left_dock.rs:299` had "Requires nvim bridge (BR5)" until 2026-05-25; now reads "LSP outline pending (NE10)". Outline is not actually populated from the LSP layer.
- **Right dock is HUD only.** No AGENT tab, no OUTLINE tab.
- **No left-dock scrolling, no expand/collapse, no folder tree.** Only cwd top-level entries.
- **Layout mode toggle.** `kb.layout_mode_toggle` defaults to `cmd+shift+e` and switches Terminal↔Ide. Not discoverable; user does not know it exists.
- **IDE mode default.** App boots into Terminal mode unless `ANVIL_LAYOUT_MODE=ide` is set. First-time user has no explorer, no dock — looks like a plain terminal.

## 3. Vision: what "real native editor" means

A user launches Anvil and is in **IDE mode by default** when launched in a
project directory (i.e. the cwd contains a `.git/`, `Cargo.toml`,
`package.json`, or similar marker). The window shows:

```
┌──────────────────────────────────────────────────────────────────────┐
│  tabstrip                                                            │
├────────────┬───────────────────────────────────────┬─────────────────┤
│            │  ▢ src/main.rs  [•]                   │  CONTEXT        │
│  EXPLORER  ├───────────────────────────────────────┤                 │
│            │                                       │  REPO + GIT     │
│  ▾ src/    │  1  use anvil_workspace::…             │                 │
│    main.rs │  2                                    │  AGENTS         │
│    lib.rs  │  3  fn main() {                       │                 │
│  ▸ tests/  │  4      let app = App::new();         │  RECENT         │
│    Cargo…  │  5  }                                 │                 │
│            │                                       │  SYSTEM         │
│  OUTLINE   ├───────────────────────────────────────┤                 │
│            │   anvil@host  ~/projects/anvil  $     │  mem  load      │
│  fn main   │   → cargo test                         │                 │
│            │   …                                   │                 │
└────────────┴───────────────────────────────────────┴─────────────────┘
                  ↑ editor pane (top 70%)             
                  ↑ terminal pane (bottom 30%)        
```

The user can:
1. **Click a file in the explorer** → file opens in the active editor pane (or creates one if none).
2. **`Cmd+P`** → file picker over the project. Type to fuzzy match.
3. **`Cmd+O`** → native file open dialog.
4. **See the filename + dirty dot** in the pane chrome. Always know which file you're in.
5. **Tabs across the top of each editor pane** when multiple files are open in the same pane.
6. **Click the cursor** anywhere in a file. Drag to select. Mouse wheel scroll.
7. **Cmd+\\ / Cmd+Shift+\\** → split current editor pane vertical/horizontal.
8. **Cmd+W** → close active buffer tab (not pane). Close last buffer in pane → close pane.
9. **Outline panel** updates from the focused buffer's LSP symbols (NE10 infra already exists).
10. **`Cmd+Shift+E`** focuses explorer; arrow keys + Enter navigate it.

The editor is the **primary surface**; the terminal is a **bottom dock or sibling pane**, not the whole window.

## 4. Deliverables (priority order, top-down execution)

Each deliverable is shippable on its own and unblocks user value. Land one
at a time, commit, manually verify before moving on.

---

### D1 — Default to IDE mode when in a project directory

**Why first:** the user never sees the IDE chrome because it's off by default.

**Scope.**
- `crates/anvil/src/main.rs` startup: detect project markers (`.git/`,
  `Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`) in cwd or any
  ancestor up to `$HOME`. If found, default `layout_mode = Ide`. Else
  keep `Terminal`.
- Keep `ANVIL_LAYOUT_MODE` env override. Keep `kb.layout_mode_toggle`.
- Add `cmd+b` keybind to toggle the left dock (matches VSCode), alongside
  the existing `cmd+shift+e` mode toggle.

**Files.** `crates/anvil/src/main.rs` startup block around `App::new`; `crates/anvil-workspace/src/mode.rs`; `crates/anvil-config/src/lib.rs` for the new keybind default.

**Verify.** Launch `scripts/run.sh` from the anvil repo root. Window opens with explorer visible. Press `Cmd+B` → explorer hides/shows. Launch from `/tmp` → terminal mode.

---

### D2 — Explorer becomes interactive (click → open file)

**Why second:** this is the biggest visual lie — rendered rows that do nothing.

**Scope.**
- Add hit testing to `crates/anvil-render/src/left_dock.rs`. Build a
  `LeftDockHits` struct returned alongside the draw call: vector of
  `(Rect, ExplorerHit)` where `ExplorerHit` is `Row(usize)` or `Header` or
  `Outline(usize)`.
- Wire mouse-down in `crates/anvil/src/main.rs` to query
  `LeftDockHits::at(px, py)` *before* falling through to `PaneTree::hit_test`.
- On `ExplorerHit::Row(idx)`:
  - If entry is a directory: toggle expand/collapse (see D3).
  - If entry is a file: dispatch a new `Action::OpenFile(PathBuf)`.
- New `Action::OpenFile(PathBuf)` in `crates/anvil-workspace/src/palette.rs`
  variant. Handler in `main.rs::handle_palette_action`:
  - If there is a focused editor pane: load the file into a **new buffer
    tab** in that pane (see D5). If no editor pane in the current tab:
    create one via `new_native_editor_pane`, then load.
  - Use `Buffer::from_path(path)` from `anvil-editor`.
  - Handle `IoError::TooLarge` by surfacing a one-line status: `"file too
    large (max 50 MB)"` via existing toast/status mechanism (or eprintln
    plus a `dirty=true`).
- Cursor hover over explorer rows should set the system cursor to pointer
  (already wired for divider drag — same mechanism).

**Files.** `left_dock.rs`, `main.rs` mouse handler, `palette.rs` `Action`, `editor_pane.rs` (add `open_file` helper if not present).

**Verify.** IDE mode launched in anvil repo. Click `AGENTS.md` in the
explorer. Editor pane appears (or current one updates) showing the
contents. Click another file. It opens. Click a directory — expands.

---

### D3 — Tree explorer: expand/collapse + scroll

**Why third:** flat cwd listing is unusable for any real project.

**Scope.**
- Extend `DirSnapshot` (`left_dock.rs`) to a tree model: `ExplorerNode {
  name, kind, depth, expanded, children: Option<Vec<ExplorerNode>> }`.
- `fs_worker` lazy-loads child directories on first expand (don't recurse
  the whole tree on startup).
- Persist expand state on a `tab.explorer_state` field across redraws.
- Add vertical scroll: track `scroll_rows: usize`, clamp to visible window.
  Mouse wheel events when the cursor is over the dock advance/reverse
  `scroll_rows`. Render only rows in `[scroll_rows, scroll_rows + visible_rows)`.
- Indent visualization: 2 spaces per depth, with a faint vertical guide
  line at each depth using `theme.text_muted` at alpha 0.2 (brand check —
  do not invent a new color).
- File-type glyph: ▢ generic, ƒ rust/ts/py/go, ⚙ config, · other (mirror
  the outline glyph vocabulary that already exists in `left_dock.rs`).
- Respect `.gitignore` (call `git check-ignore` once per directory via
  the existing git query infra in `anvil-prompt-core`, or use the `ignore`
  crate — pick the lighter dep).

**Files.** `left_dock.rs`, `fs_worker.rs`, `main.rs` for mouse wheel routing.

**Verify.** Open in `~/projects/caldera/anvil`. Tree shows top-level. Click
`crates/` → expands. Click `anvil-editor/` inside → expands. Scroll wheel
inside the dock scrolls just the dock. `.git/` and `target/` hidden by
gitignore.

---

### D4 — Editor pane chrome (header bar w/ filename + dirty + close)

**Why fourth:** the user cannot tell what file they're looking at.

**Scope.**
- New `draw_editor_header` function in `crates/anvil-render/src/editor.rs`
  (or a sibling `editor_chrome.rs`). 22 px tall. Contents:
  - Left: file-type glyph + buffer filename. If `tracked_path.is_none()`,
    show `[scratch]`. If `buffer.is_dirty()`, append a `•` in `theme.attention`.
  - Center: empty (reserved for breadcrumb / outline path later).
  - Right: small `×` close-button hit area for the focused buffer tab.
- Carve 22 px off the top of the pane rect in `workspace.rs::draw_workspace`
  before calling `draw_editor_into`.
- Header background: `theme.surface_raised` (or whatever the brand panel
  shade is — read `BRAND.md`).
- Header is hit-test target for buffer-tab switching (D5).
- Add `Buffer::is_dirty()` if absent: track a `dirty: bool` flag on
  `Buffer`, set on every `apply_edit`, cleared on `save`.

**Files.** `crates/anvil-render/src/editor.rs` (or new file), `crates/anvil-editor/src/buffer.rs`, `crates/anvil-render/src/workspace.rs`.

**Verify.** Open a file via D2. Header shows filename + glyph. Type one
character → `•` appears. `Cmd+S` → `•` clears.

---

### D5 — In-pane buffer tabs

**Why fifth:** opening a second file currently replaces the buffer or
forces a pane split. Both are wrong.

**Scope.**
- `EditorPane` (`crates/anvil-workspace/src/editor_pane.rs`) gains:
  - `buffers: Vec<BufferId>` (was: single `buffer_id`).
  - `active: usize` index into `buffers`.
  - `peek_buffer_id() -> BufferId` returns `buffers[active]`.
- `EditorPaneRegistry::open_in_pane(pane_id, buffer_id)` appends or
  focuses if already open.
- D4 header gains a tab strip across the top showing one chip per open
  buffer. Active tab highlighted. Click switches active. `×` button on
  each closes that buffer (not pane). Closing last buffer in pane →
  removes the pane via `close_focused_pane`.
- `Cmd+1` … `Cmd+9` switch to nth buffer tab in active pane.
- `Cmd+W` semantics:
  - If active pane has >1 buffer: close active buffer tab.
  - Else: close pane (existing behavior).
- `Cmd+Shift+]` / `Cmd+Shift+[` cycle buffer tabs forward/backward.

**Files.** `editor_pane.rs`, the new `editor_chrome.rs` from D4, `main.rs` chord handlers, `palette.rs` for new actions.

**Verify.** Open three files via the explorer. All three show as chips in
the pane header. Click middle chip → switches. `Cmd+2` → switches. `×`
on a chip → closes. Buffer state preserved across switches (cursor pos,
scroll, selection, undo stack).

---

### D6 — File picker (Cmd+P fuzzy match)

**Why sixth:** explorer click works, but the keyboard-driven workflow
needs a picker for files not in the current view.

**Scope.**
- Reuse the palette webview (`ui/palette/`) — add a `file-picker` mode.
- New keybind `kb.file_picker = "cmd+p"`. Chord opens palette with a fuzzy
  index of all files in the project tree (gitignore-respecting). Type to
  filter. Enter to open in active pane (same path as D2/D5).
- File index built by `fs_worker` on startup + refreshed on git-watcher
  signal (the watcher already exists for prompt updates).
- Fuzzy match: use the existing `nucleo-matcher` if already in workspace,
  else add it. Score by Sublime/VSCode rules (camelCase boundary,
  consecutive char bonus).

**Files.** `ui/palette/` (HTML/JS for the picker mode), `crates/anvil-control/` (new bridge messages: `ShowFilePicker`, `FilePickerPick`), `main.rs` chord wiring, `fs_worker.rs` for the project file index.

**Verify.** `Cmd+P`, type "main", `main.rs` is the top match. Enter
opens it.

---

### D7 — Outline panel wires to LSP symbols (NE10 infra)

**Why seventh:** the outline panel is currently dead. NE10 added LSP
hover/diagnostics but not document symbols.

**Scope.**
- Add `documentSymbol` request to the existing `LspManager`. On each
  buffer load + on `apply_edit` debounced 500 ms, fire the request for
  the active buffer's language server.
- Pipe the response into `tab.outline_symbols: Vec<OutlineRow>`.
- `left_dock.rs::draw_outline_section` already supports `Some(&[OutlineRow])`
  — feed it the real data instead of `None`.
- Click an outline row → jump cursor to that symbol's line in the focused
  buffer, scroll into view.

**Files.** `crates/anvil-editor/src/lsp.rs` (or wherever NE10 LSP lives), `crates/anvil/src/main.rs`, `left_dock.rs` outline hit testing (mirror D2 explorer hit pattern).

**Verify.** Open `main.rs`. Outline shows `fn main`, `impl App`, etc.
Click `App::new` → jumps to that function.

---

### D8 — Empty editor pane placeholder

**Why eighth:** when a pane has no buffer loaded (e.g. closed last tab),
it should not be a black void.

**Scope.**
- In `draw_editor_into`, if `buffer.is_scratch_empty()`, paint centered
  placeholder text in `theme.text_muted`:
  ```
  Anvil

  Cmd+P  open file
  Cmd+O  open from disk
  ```
- "Scratch empty" = no tracked path AND buffer is empty rope AND no edits
  on the undo stack.

**Files.** `crates/anvil-render/src/editor.rs`, `crates/anvil-editor/src/buffer.rs` (the helper).

**Verify.** `Cmd+E` w/ no file → placeholder visible.

---

### D9 — Mouse cursor handling on the editor surface

**Why ninth:** mouse already works for selection (NE7), but several gaps remain.

**Scope audit.**
- Click → place cursor (works, NE7).
- Drag → select (works, NE7).
- Double-click → select word (works, NE7).
- Triple-click → select line. **MISSING** — add.
- Mouse wheel → scroll buffer. **VERIFY** — may already work via `editor_pane.rs::apply` `Scroll(delta)`.
- Cursor shape: I-beam over editor content area, pointer over header/tabs/explorer. **VERIFY** — likely missing.
- Middle-click paste from primary selection: **defer, not part of D9**.

**Files.** `editor_pane.rs` apply path, `main.rs` mouse handler, `anvil-platform` cursor-shape API.

**Verify.** Triple-click on a line in an opened file → entire line is
selected. Wheel scrolls buffer. I-beam shows over text, arrow over chrome.

---

### D10 — Terminal becomes a bottom dock in IDE mode

**Why last:** by D1-D9 the IDE is real; this finishes the layout so the
terminal is a *companion*, not the main surface.

**Scope.**
- New layout rule in `Docks::for_mode(Ide)`: the *first* pane in the tab
  tree is the editor area (top 70%); any terminal panes go to a bottom
  drawer (30%) that is collapsible via `Cmd+J` (matches VSCode).
- When user creates a terminal pane in IDE mode (e.g. `Cmd+T` or
  `Cmd+\` for shell split), it lands in the bottom drawer, not as a
  sibling editor split.
- `Cmd+J` toggles the bottom drawer visible/hidden. Hidden state
  preserves terminal pane(s) in memory.
- Existing Terminal-mode behavior unchanged — splits are normal pane
  tree siblings.

**Files.** `crates/anvil-workspace/src/mode.rs`, `crates/anvil-workspace/src/tab.rs` split logic, `main.rs` chord wiring.

**Verify.** In IDE mode, open `main.rs`. Press `Cmd+T` → terminal opens
at the bottom, not beside. `Cmd+J` → drawer collapses; editor takes full
height. `Cmd+J` again → drawer back.

## 5. Cross-cutting requirements

- **No regressions.** Terminal mode must still behave exactly as before. All existing scrollback, prompt, palette, search, agent panel behavior preserved.
- **Brand.** Every visible surface must pass `BRAND.md`. No invented colors, no rounded corners that don't match the existing chrome shape, no decorative volcano imagery.
- **No GPU-default toggle.** `ANVIL_RENDER=gpu` remains opt-in until the existing selection-invisible + diff-tint-invisible bugs are fixed (CRAP audit). Do not address those bugs in this rescue spec.
- **Don't touch `crates/anvil-editor/src/nvim/`.** Already removed at NE15, but if any lingering reference shows up, do not revive it.
- **Test every phase.** Each deliverable lands with new tests for its own surface (hit testing, file-open routing, in-pane tab switching, etc.). `cargo test --workspace --lib` and `cargo clippy --workspace -- -D warnings` must pass at every commit.
- **Manual smoke is mandatory.** A deliverable is not done until the verify steps run green by hand in `scripts/run.sh`. Headless agent runs cannot certify visual surfaces — leave a note in the commit message if a step requires the user.
- **One deliverable per commit.** D1 commit, then D2 commit, etc. Atomic, reviewable, revertable.
- **Append `wiki/log.md`** with a one-line dated entry per deliverable, matching the style of recent NE entries.
- **Wiki pages.** Update `wiki/concepts/native-editor.md` after D5 lands (it currently documents only NE1-NE5). Author a new `wiki/concepts/explorer.md` after D3.

## 6. Open questions to resolve before executing

These need answers from the user (not assumed). Ask before D1.

1. **Project root detection.** Walk up from cwd to find the first marker, or only check cwd itself? (Recommend: walk up to `$HOME`.)
2. **Default editor pane content on first launch in IDE mode.** Empty `[scratch]` pane, or auto-open the project README if present? (Recommend: scratch w/ placeholder per D8.)
3. **Bottom terminal drawer height.** Fixed 30%, or remember the last user-resized value? (Recommend: remember, persist to config.)
4. **File picker scope.** Project tree only, or include `~` recents? (Recommend: project only for v1.)
5. **`Cmd+E` semantics in IDE mode after D10.** Open a new editor pane (split editor area) or open scratch in a new buffer tab? (Recommend: new buffer tab in active pane; `Cmd+\` for editor split.)

## 7. Out of scope

Things this spec does **not** cover (track as follow-ups):
- Workspaces / multi-root projects.
- Remote files / SSH.
- Settings UI.
- Themes-by-language.
- Snippets, code-folding, multi-window.
- Performance tuning beyond what NE5 already does (integer-row scroll).
- Sublime/VSCode keybinding compatibility presets.

## 8. Definition of done for the whole rescue

User launches `scripts/run.sh` from the anvil repo root. App opens in IDE
mode. Explorer is visible on the left with the project tree. Click
`crates/anvil-editor/src/buffer.rs` in the explorer. File opens in the
main pane with syntax highlighting, line numbers, cursor visible, header
showing `buffer.rs` and the file-type glyph. `Cmd+P`, type "main", press
Enter. `main.rs` opens as a second tab chip in the pane header. Click
the first chip to switch back. Type a character. Dirty dot appears in the
chip. `Cmd+S`. Dirty dot clears. `Cmd+T`. A terminal opens in the bottom
drawer. `Cmd+J`. Drawer collapses. Editor fills the whole pane area.

At this point Anvil is a real native IDE that can be the user's daily
driver for editing code, not a terminal emulator with editor stubs.

## 9. Author guidance for the executing AI

- Trust the audit in §2 — those file:line references are accurate as of
  rust-port HEAD at commit `16175e2` (NE15 retirement).
- **Do not** dispatch sub-agents for this work unless explicitly asked.
  The previous session burned hours on agent ping-pong and shipped
  hollow phases. This time: read the spec, edit the files, run the build,
  show the user. One deliverable, one commit, one verification.
- **Surface every assumption** before writing code. If a section of this
  spec is ambiguous, ask the user.
- **Never claim a deliverable is done without manual smoke.** The previous
  session shipped "NE4" that killed editor panes within one frame. Tests
  passed. The user found it in 5 seconds.
- The user wants Zed quality. Bias toward fewer features done well, not
  more features half-built.
