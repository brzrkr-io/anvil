# Anvil Direction Correction — Terminal-First Command Deck

Date: 2026-05-29
Status: active direction note

## Decision

Do not steer Anvil as a generic "better Zed" clone. Zed's editor-first surface
area, collaboration model, GPUI stack, and language tooling depth make a direct
general-editor race a poor bet for this project.

Anvil's viable wedge is a native, terminal-first command deck for terminal-heavy
engineering: shells, repos, agents, clusters, logs, and quick edits in one compact
workspace.

## Product Thesis

Anvil should feel like a fast native terminal and operational workbench that also
edits code, not like a code editor that happens to include a terminal.

The draw:
- Excellent PTY and terminal correctness, including scrollback, blocks, selection,
  mouse support, and fast clear/redraw behavior.
- Compact draggable panes and layout modes that support command work, editor work,
  and mixed workflows without jank.
- Native project context: explorer, git state, recent files, diagnostics, and
  outlines as supporting surfaces.
- Agent and Caldera integration as first-class workflow, not a bolted-on chat tab.
- Minimal, polished Mineral UI that stays dense and quiet under real work.

## Language Direction

Stay on the current Rust/AppKit/Metal implementation. A Zig rewrite is not a fix
for the current problems. The recent failures were caused by app-state coupling,
hidden surface repainting, terminal scroll-state drift, layout math, and an
oversized app controller. Those are architecture issues, not language issues.

Zig can remain a future exploration only if there is a specific technical reason
that Rust/AppKit/Metal cannot satisfy after the architecture is modularized.

## Near-Term Engineering Direction

1. Keep shrinking `crates/anvil/src/main.rs` into focused controller modules.
2. Make terminal mode the performance baseline and guard it with native smoke tests.
3. Treat the native editor as "good enough for command-deck editing" before chasing
   broad IDE parity.
4. Prioritize pane layout, terminal input/mouse correctness, and render invalidation
   over broad new features.
5. Move durable product and architecture decisions out of chat and into `context/`
   or `wiki/` before continuing large slices.

