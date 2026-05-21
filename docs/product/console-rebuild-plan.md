# Caldera Console — Rebuild Plan & Handoff (Zig)

Status: handoff doc — start a fresh session and point it here.
Updated: 2026-05-21

Single source of truth for rebuilding Caldera Console as a Zig application.

---

## 1. Product vision

Caldera Console is a single, unified **control plane for everything a software /
devops engineer does** — not a project-management tool. It is "more than a terminal":
a developer workspace / emulator app.

Core surfaces:

- **Terminal** — first-class, multi-tab, GPU-rendered. Quality bar: Ghostty / Alacritty.
- **Built-in browser** — a browsing surface inside the app.
- **Editor + file explorer** — lightweight code editing and navigation.
- **Agents** — orchestrate coding agents (Claude Code, Codex, etc.).
- **Overlays** — summonable, pinnable context panels (repo, checks, AI, integrations).
- **Plugins** — the app is plugin-extensible; it behaves as a backplane / control plane.

## 2. Language decision — Zig (DECIDED)

The implementation language is **Zig**. Decided by the owner on 2026-05-21 after
reviewing the Rust+Tauri vs Zig tradeoff. Do not relitigate it — build the best Zig
version.

One clear-eyed scoping note (not an objection): this is a from-scratch, multi-month
build. A great terminal core, a GPU renderer, and font shaping are each real
engineering. Scope hard, ship the terminal first, layer surfaces after. **Keep the
existing Tauri app (`anvil-console`) running as the interim daily driver** — the Zig
app is a new repo built in parallel; do not delete the working app.

## 3. Reference projects (ideas only — copy no code)

- **Ghostty** (Mitchell Hashimoto, MIT) — the reference for a Zig terminal: clean split
  of terminal core / GPU renderer / platform layer. Watch `libghostty` (its core as a
  reusable library) — if usable, building on it saves enormous effort.
- **terax-ai** (`crynta/terax-ai`, Apache-2.0) — reference for feature scope and UX of
  the non-terminal surfaces.

Rule: architecture, patterns, and feature ideas are free to adopt (not copyrightable);
code is not. No copied files, no foreign LICENSE/NOTICE text in this repo.

## 4. UI strategy — the key architectural decision

The hard part of a Zig app is the UI for the non-terminal surfaces (browser, editor,
AI, overlays). Three options:

- **A. Full native** (AppKit on macOS, GTK on Linux) — best feel, most work, per-platform.
- **B. Hybrid** — native Zig + GPU for the terminal; an **embedded webview** hosts the
  rich surfaces (browser, editor, AI, overlays) as web UI.
- **C. Immediate-mode GUI** (Dear ImGui / zgui via C bindings) — fast to build, less
  polished.

**Recommendation: B (Hybrid).** The terminal is native Zig + GPU — where Zig genuinely
wins. Everything else is a webview the Zig app hosts — where web tech wins, where the
**built-in browser becomes nearly free**, and where the UI concepts from the current
app (overlays, palette, the retuned Mineral palette) port over directly. This keeps the
Zig effort focused on the terminal core instead of reinventing a GUI toolkit.

## 5. Target architecture (Zig, Hybrid)

```
caldera-console/            (new repo)
├── build.zig
├── build.zig.zon
├── src/
│   ├── main.zig
│   ├── core/               # VT/ANSI parser, grid, scrollback
│   ├── pty/                # PTY spawn, shell integration scripts
│   ├── render/             # GPU text renderer (Metal first), font shaping (HarfBuzz)
│   ├── app/                # window, event loop, surface manager
│   ├── surfaces/           # native terminal surface + webview host
│   ├── ipc/                # native <-> webview bridge (typed messages)
│   └── plugins/            # plugin host
├── ui/                     # web UI for non-terminal surfaces (browser/editor/AI/overlays)
└── docs/
```

## 6. Roadmap / milestones

- **M0** — Zig scaffold: `build.zig`, window + event loop, GPU clears the screen.
- **M1** — Terminal core: VT/ANSI parser, grid, scrollback, PTY, GPU text rendering.
  A genuinely usable single-pane terminal.
- **M2** — Multi-tab, search, shell integration, config/theme (Mineral palette).
- **M3** — Webview host + typed IPC bridge; first web surface (overlays / HUD).
- **M4** — Built-in browser surface.
- **M5** — Editor + file explorer (web surfaces).
- **M6** — Agent orchestration.
- **M7** — Plugin backplane (write the spec before code).

## 7. Open decisions for the new session

1. UI strategy A / B / C (recommend **B**).
2. Renderer: Metal-first (macOS) vs portable GL / WebGPU from day one.
3. Font shaping: HarfBuzz C binding (recommended) vs alternative.
4. Webview library for the Zig host (e.g. the `webview` C library / WKWebView).
5. New repo name; keep `anvil-console` as the interim daily driver (recommended: yes).

## 8. What carries over from the current app

- The retuned **Mineral palette** (light + dark) — see `anvil-console/src/app.css`.
- The **overlay / HUD model** and command-palette logic — concepts, reimplemented.
- PTY and shell-integration know-how.
- Design system: sibling repo `caldera-os` (`BRAND.md`, `docs/design/`). Do not depend
  on `caldera-os`, Linear, or Obsidian at runtime — copy assets locally.

## 9. Current state of `anvil-console` (the interim app)

- Tauri 2 + Rust + Svelte 5 + xterm.js; works.
- Branch `fix/desktop-freeze-palette-polish` (local, not pushed): freeze fix, palette
  rework, polish, GPU (WebGL) terminal rendering. 81 unit + 18 e2e tests green.
- A reactive-loop freeze was fixed — lesson worth keeping: never let a reactive effect
  synchronously write state it depends on.
