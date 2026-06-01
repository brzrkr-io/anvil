---
date: 2026-05-25
kind: handoff
status: live
goal: real per-file tab pipeline (#2 in user's prioritized slice)
---

# Handoff — Real Tab Pipeline

## Where the branch is

- Branch: `rust-port`, ahead of `origin/rust-port` by **10 commits** (unpushed).
- Working tree: clean.
- Gates: green at HEAD.
  - `cargo fmt --all`
  - `cargo clippy --workspace -- -D warnings`
  - `cargo test --workspace` — 921 pass, 0 fail.

```
c97cff4 docs(wiki): log Explorer nesting and file-open pipeline (item 7)
4d16a94 feat(ide): nested Explorer expansion and file-open pipeline   ← #1
3399fb4 feat(ide): items 11/13/14 P0 — strip raw-hex chrome + 2px accent rule
527ef35 docs(context): handoff for items 11-20 IDE polish (render A gap)
94bb6f3 fix(ide): chevron uses text_subtle per spec, not text_muted
2c342d2 docs(context): design spec for IDE polish items 7, 8, 10
b8b0188 feat(ide): items 7-10 expand-in-place + scroll affordance + outline empty-state
e36b327 feat(ide): passive Explorer hover via NSTrackingArea + mouse_moved
c40b7ef docs(context): handoff for items 7-20 IDE polish work
ed290d6 feat(ide): polish Explorer rows + bottom drawer for operator-console feel
```

## What just landed (#1 — Explorer functional)

Commits `4d16a94` + `c97cff4`:

- **Nested directory expansion.** Click a dir chevron → child entries
  load via `spawn_child_fs_worker` (new) → render indented under parent
  with `▾`. Click again → collapse (entries hide; cached snapshot stays
  in `child_snapshots` for re-expand).
- **File-open pipeline confirmed working.** Click a file row →
  `open_path_in_native_editor(path)` loads the file into the focused
  pane's editor, updates tab title chip, ember left-rail selection on
  the explorer row.
- **State.** `App.child_snapshots: HashMap<PathBuf, DirSnapshot>` (lazy
  cache). `App.expanded_dirs` keyed by entry index in flat snapshot
  walk (still works because expansion is index-stable per snapshot).
- **Files changed.**
  - `crates/anvil/src/fs_worker.rs` — new `spawn_child_fs_worker`.
  - `crates/anvil/src/main.rs` — `child_snapshots`, `child_fs_tx/rx`,
    dispatch in `mouse_down`, drain in tick.
  - `crates/anvil-render/src/left_dock.rs` — `collect_visible_rows`
    walks the tree; `LeftDockHits.visible_rows` for hit-test parity.

**Manual smoke (screenshot evidence at `/tmp/anvil-explorer-functional.png`
and `/tmp/anvil-after-click-crates.png`):** Click on CLAUDE.md loaded
its full content into the editor, tab title updated to "CLAUDE.md",
right chip showed "markdown · native". File-open pipeline works.

## What #2 actually is

**User's complaint:** "Real tabs missing." In the Option A sketch
(`sketches/native-editor-directions/index.html`) the editor area
contains a tab strip listing open files (`editor.rs / left_dock.rs /
workspace.rs`). Today, opening a file in Anvil replaces the focused
pane's buffer — there is no per-file tab UI.

**Fork in the road — needs decision before builder dispatch:**

Three architectures, pick one:

1. **Top-level tabs (one tab per file).** Each file gets its own
   workspace tab. Closest to Sublime Text. Conflict: today's top tabs
   are workspace-level ("shell" tab = entire pane tree). Mixing
   files and shells in the same tab strip is messy.
2. **Sub-tab strip per editor pane (VS Code / Zed model).** Below the
   chrome row, inside the editor pane region, render a buffer tab
   strip. Buffers live on the pane (`EditorPanes` already tracks
   `Vec<Buffer>` per pane via `buffer_id`s). This matches the Option A
   sketch exactly. **Recommended.**
3. **Single editor, no tabs.** Forget tabs entirely; opening a file
   replaces the current buffer (today's behavior). User explicitly
   said this is wrong.

**Default recommendation: option 2.** It matches the sketch, the
existing data model (`EditorPanes` already has multi-buffer support),
and modern IDE conventions.

## Architectural notes for the builder (option 2 assumed)

- **Data model.** `EditorPanes` per pane already tracks open buffers
  (grep `BufferId`, `open_path` returns one). What's missing: a
  current/active buffer per pane, and a way to switch between them.
  Add `EditorPane.active_buffer: BufferId` if not already present.
- **Render.** Add a new render call between the existing chrome strip
  and the editor pane body. Looks like the existing tabbar but
  per-pane, narrower height (~28px), and listing buffer filenames
  (basenames of buffer paths) instead of workspace tabs.
- **Click routing.** Hit-test the buffer tab strip in `mouse_down`
  before falling through to the editor body. Click on a tab →
  `pane.active_buffer = buffer_id`, redraw.
- **Close `×` per tab.** Closes that buffer in the pane. If only one
  buffer remains and user closes it, fall back to the scratch buffer
  (same UX as today's empty editor).
- **Open-from-Explorer wiring.** `open_path_in_native_editor` already
  calls `tab.editor_panes.open_path(pane_id, path)` and gets a
  `BufferId`. After that call, set `pane.active_buffer = buffer_id`.
  If the buffer was already open (open_path returned an existing id),
  just switch the active buffer — don't re-read the file.
- **Dirty indicator.** Buffer-level. Show a dot on tabs with unsaved
  edits (today buffers are read-only AFAICT — flag for future).
- **Tab width / overflow.** Mirror the existing top-tab logic: clamp
  per-tab width, allow horizontal scroll OR truncate with `…` if too
  many open.

## Files the builder will touch

- `crates/anvil/src/main.rs` — wire buffer-tab click in `mouse_down`,
  update `open_path_in_native_editor` to set active buffer, plumb
  buffer-tab geometry into the editor pane draw call.
- `crates/anvil-render/src/` — new module `editor_tabbar.rs` (or
  inline into `editor.rs`) for the per-pane buffer tab strip.
- `crates/anvil-workspace/` — add `EditorPane.active_buffer` if not
  present; expose `open_buffer_ids()` iterator on `EditorPanes`.
- `crates/anvil-render/src/editor.rs` — draw call signature gains a
  `top_inset_px` for the buffer tab strip OR returns a reduced rect
  for the body.

## Visual target

`sketches/native-editor-directions/index.html` Option A tab strip
(lines 150 of that file). Match:
- 34px row height
- Inactive tabs: muted text, no background
- Active tab: lighter background (the gradient is fine to approximate
  with solid `theme.charcoal` or `theme.surface`), inset 2px ember
  top accent rule
- Dirty dot: 7px ember (`accent_primary`) with subtle glow (skip the
  glow for first cut — single pixel rect)
- `×` close glyph: visible on active and hover (hover needs the
  passive `mouse_moved` wired in `e36b327` — already done)

## Success criteria for #2

1. Open multiple files via Explorer click. Each appears as a tab in
   the editor pane's buffer tab strip.
2. Click a buffer tab → editor body switches to that buffer's content.
3. Click `×` on a buffer tab → buffer closes, tab disappears, next
   buffer becomes active (or scratch placeholder if last).
4. Gates green: fmt + clippy + test workspace.
5. Manual smoke screenshot showing 2-3 files open, active tab clear,
   editor body matches active buffer.

## Backlog after #2

- **#3 Terminal command blocks (Option D).** Refactor pty stream into
  collapsible cmd / dur / exit blocks per
  `docs/design/layout-mockups.html` Option D. This is the next big
  visual delta after #1 + #2.
- **Top context bar.** `top_h` is still 0 in IDE mode. Restore with
  the cleaned-up `context_bar.rs` to show IDE chip + path + ctx/tok
  chips. Smaller than #3.
- **Manual UI verification of dir expansion** — builder smoke
  reported it works; I couldn't reliably click a dir via `cliclick`
  to confirm. Click an explorer dir in a real session as the first
  sanity check next time.
- **Push `rust-port` upstream** — 10 commits ahead, all green.
  Recommend pushing.

## Routing recommendation

1. **design-lead** (5 min) — confirm option 2 vs option 1 vs other.
   This is a UX/info-arch decision that should not be made by builder.
2. **builder** — execute #2 per the decision in (1) and the
   architectural notes above.
3. **reviewer** — closeout + gate check + next handoff.

## Useful pointers

- Live target sketch: `sketches/native-editor-directions/index.html`
  (Option A is the relevant variant).
- Mineral chrome details for chrome strip semantics:
  `docs/design/layout-mockups.html` Option D.
- Items 1-6 visual spec:
  `context/2026-05-25-ide-polish-slice-decisions.md`.
- Items 7-8-10 spec: `context/2026-05-25-ide-polish-items-7-8-10-spec.md`.
- Item 19 visual diff: `context/2026-05-25-ide-polish-item-19-visual-diff.md`.
- This slice's predecessor handoff:
  `context/2026-05-25-ide-polish-items-11-20-handoff.md`.

## Open questions for the user

- **Confirm option 2 (per-pane buffer tab strip).** Default if
  silent.
- **Buffer save / dirty model.** Today buffers appear to be read-only
  view. Is editing in scope soon? If yes, dirty-dot UX matters more.
  If no, skip the dirty dot for now.
- **Push `rust-port` upstream after #2?** 10 commits ahead; user
  hasn't authorized a push yet this session.
