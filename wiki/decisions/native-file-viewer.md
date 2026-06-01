---
status: active
type: decision
created: 2026-05-30
updated: 2026-05-30
sources:
  - ../../src/syntax.zig
  - ../../src/fileview.zig
  - ../../src/session.zig
  - ../../src/session_manager.zig
  - ../../src/app.zig
  - ../../src/ipc.zig
  - ../../src/main.zig
  - ../../docs/product/console-rebuild-plan.md
confidence: high
---

# Native Read-Only File Viewer

## Decision

Anvil's file-open surface is a native read-only viewer rendered into the
existing Metal terminal grid. It is NOT a webview editor and NOT an embedded
`$EDITOR` invocation. The `$EDITOR` escape hatch is preserved as the `edit`
CLI verb for cases where the user wants to mutate a file.

## Mechanism

A `Session.Kind = .viewer` pane carries no PTY (`src/pty.zig` `initNull`).
File bytes are loaded via `src/fileview.zig`:

- 2 MiB hard cap (`size_cap`); `LoadResult.truncated` is set when the file
  exceeds the cap.
- Binary detection: scans the first 1024 bytes for a NUL byte; sets
  `LoadResult.is_binary = true` when found.
- UTF-8 bytes are read as-is; no encoding conversion.

Lines are split via `fileview.splitLines` (no allocation; writes slices into a
caller-supplied buffer; trailing newline does not produce an empty line).

Tokenization is performed by a hand-written syntax module (`src/syntax.zig`):

| Type | Description |
|------|-------------|
| `Role` | `text`, `keyword`, `string`, `number`, `comment`, `type`, `punct` |
| `Token` | `start`, `len`, `role` — indexes into the source line |
| `Lang` | `zig`, `toml`, `markdown`, `sh`, `lua`, `generic`, `unknown` |

`detect(path)` selects `Lang` by file extension. `generic` covers `.c`, `.h`,
`.cpp`, `.rs`, `.go`, `.js`, `.ts`, `.py`. `unknown` returns the whole line as
a single `.text` token. Markdown headings (lines starting with `#`) are
highlighted as `.keyword`. The tokenizer is single-pass and line-scoped; there
is no multi-line string or block-comment handling.

`src/session.zig` `fillGrid` iterates the loaded lines and writes colored cells
into the `Terminal` grid using `roleColor`, which maps `Role` to an ANSI color
index:

| Role | ANSI index (semantic) |
|------|-----------------------|
| keyword | 4 (blue) |
| string | 2 (green) |
| number | 3 (yellow) |
| comment | 8 (bright black) |
| type | 5 (magenta) |
| punct | 8 (bright black) |
| text | default fg |

Because the viewer writes into the same `Terminal` grid, the existing renderer,
scrollback, copy-mode, and search all work without modification.

## Entry Points

- **Explorer file-click**: clicking a file in the explorer panel calls
  `SessionManager.addViewer` (`src/session_manager.zig`).
- **IPC verb `view`**: `src/app.zig` `drainIpc` dispatches `.view` path
  argument → `mgr.addViewer`. Exposed via `src/ipc.zig` and the `anvil view
  <path>` CLI verb in `src/main.zig`.
- **Close**: `q` or `Escape` closes the viewer pane; all other input is
  swallowed (`src/app.zig` key handler, `s.kind == .viewer` branch).

## Rationale

- Upholds the "native Metal only" constraint (no WebKit).
- Provides a first-class AI-native read surface for agents inspecting files.
- Minimal new surface area: reuses the grid, renderer, scroll, copy, and
  search unchanged.

## Known Limits

- Read-only. The `edit` CLI verb (`$EDITOR` in a pane) remains the mutation
  path.
- `fillGrid` writes at most 8192 lines (terminal grid scrollback cap ~5000);
  files beyond that are truncated silently at the grid boundary.
- Viewer panes are not persisted across restarts (restored as shell panes).
- No multi-line syntax constructs (block strings, block comments).
- Binary files are detected but not rendered usefully (binary flag is set;
  display behavior is not specified beyond that).

## Relationship to Other Pages

- [[concepts/native-editor|Native Editor]]: that page describes the Rust-port
  rope/LSP editor (NE1–NE15, `rust-port-archive`). The viewer is a separate,
  simpler Zig concept: read-only, grid-backed, no rope. They are not the same
  surface.
- [[decisions/0005-render-host]]: "no webview in the Zig tree" — the viewer
  upholds this constraint.
- `docs/product/console-rebuild-plan.md` M5 milestone now reads "native
  read-only file viewer" (updated 2026-05-30).
