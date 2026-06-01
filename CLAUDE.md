@AGENTS.md

## Claude Code Notes

Claude reads `CLAUDE.md`; other agents may read `AGENTS.md`. Shared rules live in
`AGENTS.md`. Global behavioral guidelines live in `~/.claude/CLAUDE.md` and apply
on top of this file.

## This Project

- Anvil is a macOS desktop app: Tauri v2 backend (Rust, `src-tauri/`) + SvelteKit SPA (Svelte 5 runes, adapter-static, ssr=false). Terminal = xterm.js + WebGL; editor = CodeMirror 6.
- Toolchain: Node 20 + pnpm for the frontend; Rust stable for `src-tauri/`. No Zig.
- Dev app: `pnpm tauri dev`.
- Build frontend: `pnpm build` (Vite).
- Type-check: `pnpm check`.
- Unit tests (JS): `pnpm test` (Vitest); coverage: `pnpm test:coverage`.
- Rust tests: `cd src-tauri && cargo test`; format: `cargo fmt`; lint: `cargo clippy`.
- End-to-end: `pnpm e2e` (Playwright).
