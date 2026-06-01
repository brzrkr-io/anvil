@AGENTS.md

## Claude Code Notes

Claude reads `CLAUDE.md`; other agents may read `AGENTS.md`. Shared rules live in
`AGENTS.md`. Global behavioral guidelines live in `~/.claude/CLAUDE.md` and apply
on top of this file.

## This Project

- Anvil is a native macOS app: Zig, Metal, AppKit (thin Obj-C shim).
- Toolchain: run `./tools/get-zig.sh` once per checkout, then use `.zig/zig`.
- Build and launch: `.zig/zig build run`.
- Run unit tests: `.zig/zig build test`.
- Format: `.zig/zig fmt src build.zig` (`--check` to verify).
- Headless render check: `./zig-out/bin/anvil --dump /tmp/x.png` (catches runtime Metal shader errors).
- Requires macOS with Xcode Command Line Tools.
- See `docs/product/console-rebuild-plan.md` for the full rebuild plan.
