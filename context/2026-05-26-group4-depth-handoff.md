---
date: 2026-05-26
kind: handoff
status: live
goal: editor depth — LSP wiring, Outline data flow, multi-window detach (group 4 of 20-task list)
---

# Handoff — Group 4: Editor Depth (#18, #19, #20)

These three items each warrant their own focused slice. They are not part
of the current 20-task burn-down's group 1-3 — they are the depth work
that turns Anvil from "edits files" into "edits projects."

## Branch state at handoff time

`rust-port`, ahead of origin by ≥ 20 commits (depends on when group 2
lands). Working tree clean assumed. Gates green expected at the time
group 4 starts.

## #18 — Real LSP wiring

**Today.** `crates/anvil-editor/src/lsp.rs` exists (search the file).
`LspManager` provides diagnostics already (see `RenderDiagnostic` in
`crates/anvil-render/src/editor.rs`). What's NOT wired:
- A real LSP server process per language.
- Symbol queries (workspace/document symbols).
- Completion (textDocument/completion).
- Hover (textDocument/hover).
- Go-to-definition.

**Scope for #18.** Just **rust-analyzer for `.rs` files**, hover and
diagnostics only. Completion + goto come later.

**Plan.**
1. Spawn `rust-analyzer` lazily on first `.rs` file open. One process per
   workspace root.
2. Send `initialize` + `initialized`.
3. Send `textDocument/didOpen` on file open, `didChange` on edit,
   `didClose` on tab close.
4. Receive `publishDiagnostics`; map to `RenderDiagnostic`; route to
   render.
5. `textDocument/hover` on Cmd+hover or sustained hover; render in the
   existing hover popup path in `editor.rs`.

**Constraints.** No new crate dep beyond what's already in
`anvil-editor`. Use `tokio` or stdlib spawn — match what `LspManager`
already does. Don't touch other panes or chrome.

**Tests.** Mock LSP server in `tests/` that replies to `initialize`
with empty capabilities; assert the lifecycle does not deadlock.

## #19 — Outline panel populates

**Today.** Outline section in `crates/anvil-render/src/left_dock.rs` is
header-only (text_subtle, "OUTLINE", no body). The `outline: Option<&[OutlineRow]>`
parameter exists; main.rs passes `None`.

**Two options.**

A. **LSP-driven.** Issue `textDocument/documentSymbol` when active
   buffer changes; map response to `Vec<OutlineRow>`; thread through
   `draw_left_dock_with_scroll(... outline: Some(&rows) ...)`. Depends
   on #18 (LSP wiring).

B. **Syntax-tree driven.** Walk the existing `SyntaxLayer` (see
   `anvil-editor::syntax`) for fn / struct / impl heads; produce a
   flat `Vec<OutlineRow>`. No LSP needed. Faster to ship, less
   accurate, language-by-language tree-sitter rules required.

**Recommended.** Ship B first as a parity feature; promote to A once
   #18 lands.

**Click behavior.** Click an outline row → jump editor cursor to
that line. Reuse the `EditorPane.scroll_pos` / `cursors` mutation
path.

## #20 — Multi-window / detach buffer

**Today.** Anvil opens exactly one window. The whole `AppShell` is
single-window.

**Scope.** Drag a buffer tab out of its strip → spawn a new native
window hosting just that buffer.

**Plan.**
1. `crates/anvil-platform/src/appkit.rs::AppKitApp::new` becomes
   `spawn_window(title, frame) -> AppKitApp`; the existing entry
   path becomes the first call to `spawn_window`.
2. `App` becomes `Vec<App>` or a workspace + per-window state. Each
   window has its own `tabs`, `tab_bar_hits`, `editor_tab_hits`,
   etc.
3. Drag-to-detach gesture: detect via `mouse_dragged` on an editor
   tab that exceeds a vertical-drag threshold. On release outside
   the source window's tab strip, call `EditorPaneRegistry::
   take_buffer(pane_id, buffer_id)` and pass to a new window's
   pane.
4. The shared filesystem worker, LSP processes, theme, font atlas
   live in a process-global state (the `App` becomes the
   `WindowState` and a `ProcessState` holds shared services).

**This is the largest item in the 20-task list.** Estimate: 1-2 full
sessions for a builder, plus a design-lead review for the detach
gesture. Don't bundle it with anything else.

## Routing for group 4

1. **systems-architect** — design the LSP threading model (#18) and
   the process-global vs window-local state split (#20). Output: a
   short arch doc in `context/`.
2. **builder** — execute #19B (syntax-tree outline) — smallest of the
   three, doesn't depend on the architect's #18 output.
3. **builder (separate dispatch)** — execute #18 against the
   architect's threading doc.
4. **design-lead** — drag-to-detach interaction model for #20.
5. **builder** — execute #20.
6. **reviewer** — closeout per item before handoff to user.

## Open questions for the user

- LSP per-language config: Anvil decides automatically? Or user can
  override in `anvil.toml`?
- Multi-window: same workspace tab structure cloned, or genuinely
  independent? (Workspace-level tabs replicated vs per-window.)
- #20 cost is high. Worth the cycles, or is the user fine with a
  single-window editor for v1?
