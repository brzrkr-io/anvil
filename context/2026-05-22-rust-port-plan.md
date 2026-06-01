# Anvil Zig → Rust Port — Plan & Status

Date: 2026-05-22. The project pivoted from the Zig feature push (see
`2026-05-22-master-roadmap.md`, now superseded) to a full port of Anvil from
Zig to Rust, at the user's explicit, twice-confirmed decision. caldera-os is
being ported to Rust concurrently, so the ecosystem stays single-language.
Anvil is to be **AI-native** — agents are a first-class architectural concern.

Branch: `rust-port` (off `main` commit `07f002c`, the final Zig baseline).
Decision record: `wiki/decisions/0004-rust-port.md`.

## Architecture — a 12-crate Cargo workspace

Pure-logic crates (no macOS dep, test on any host) + one platform crate that
quarantines all `objc2`/AppKit/Metal/PTY `unsafe` + two thin binaries.

| Crate | Role |
|---|---|
| `anvil-term` | VT/ANSI emulator: cell, grid, scrollback, parser, terminal, search, command blocks |
| `anvil-workspace` | pane tree, tabs, selection, file-tree model, palette |
| `anvil-render` | rasterizer geometry + draw logic (glyph draw via a trait) |
| `anvil-theme` | Mineral palette, color, theme resolution |
| `anvil-config` | TOML config + theme loading (replaces ZON) |
| `anvil-agent` | agent-domain schema (AgentRun, Approval, Finding, Snapshot) — leaf crate |
| `anvil-caldera` | HTTP client to caldera-local (127.0.0.1:4175) |
| `anvil-control` | the Anvil control surface — webview bridge + agent read/drive catalog |
| `anvil-prompt-core` | prompt-renderer logic |
| `anvil-platform` | objc2 / AppKit / Metal / CoreText / WebKit / PTY |
| `anvil` | the app binary |
| `anvil-prompt` | the shell-prompt binary |

Bindings: the `objc2` family (spike-verified sound). PTY: `nix`. Errors:
`thiserror` in libs, `anyhow` in binaries. Config: TOML via `serde`.

## Phase status

- **P0 — workspace scaffold** ✓ done (12 crates, `cargo build`/`cargo test` green; `zig build` still works). Not yet committed.
- **objc2 spike** ✓ done (throwaway in `/tmp/objc2-spike`; verdict: `define_class!` + ivar-held `Rc<RefCell<App>>` + CAMetalLayer + run loop all sound).
- **P1a — anvil-term: cell/grid/scrollback** — in progress.
- **P5 — anvil-prompt-core + anvil-prompt** — in progress (independent subtree).
- P1b — anvil-term: parser/terminal/search.
- P2 — anvil-workspace.
- P3 — anvil-theme, anvil-config, anvil-control (the `anvil-ipc` rename).
- P4 — anvil-render.
- P6 — anvil-platform: PTY (lowest-risk platform piece, first).
- P7 — agent layer: anvil-agent → anvil-caldera → anvil-control read-half → agent-panel render.
- P8 — anvil-platform: Metal + Font.
- P9 — anvil-platform: AppKit + WebView.
- P10 — `anvil` binary wiring + behavior/visual parity vs the Zig app on `main`.
- P11 — delete all Zig (`build.zig`, `*.zig`, `.zig-cache`), rewrite `AGENTS.md`/`CLAUDE.md`/wiki for Rust, merge `rust-port` → `main`.

## Toolchain

Homebrew Rust 1.95.0 (latest stable) — `cargo`, `clippy`, `rustfmt` all work.
rustup is also installed but not on PATH (shell config is nix-managed,
read-only — not edited). Builds use the Homebrew toolchain.

## Known follow-ups for the P10 parity/hardening pass

- **anvil-config:** TOML deserialization silently ignores unknown fields; the
  Zig ZON loader rejected them (catching user typos). Add
  `#[serde(deny_unknown_fields)]` to restore that.
- **anvil-render:** `draw_viewport` snapshots each row via `Vec` per frame — a
  borrow-checker shortcut that breaks the Zig zero-per-frame-allocation
  hardening invariant. Rescope the row borrow (draw row N fully before
  fetching row N+1) so no per-frame allocation occurs; then port the
  `CountingAllocator` zero-alloc test as a global-allocator probe.

## Verify each phase

`cargo build` + `cargo test` green for everything ported so far; `cargo clippy`
clean; `cargo fmt` applied. The Zig app on `main` is the parity oracle until P10.
