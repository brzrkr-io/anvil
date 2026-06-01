---
status: superseded
type: decision
created: 2026-05-22
updated: 2026-05-29
sources: []
confidence: high
---

# 0004 — Zig→Rust Port

## Status

Superseded (2026-05-29). The Rust port was completed through P11 then archived
on tag `rust-port-archive`. The active branch (`zig`) is a ground-up Zig rewrite.
The Zig rewrite is now the primary and only active codebase. The context below
documents the Rust-port rationale for historical reference.

## Context

Two converging forces drove this decision:

1. **Ecosystem unification.** `caldera-os` — the companion workspace that Anvil
   integrates with — is itself being ported to Rust. Keeping Anvil in Zig would
   mean maintaining two languages, two build systems, and two sets of FFI shims
   for the same Apple framework bindings. A Rust port collapses that into one
   ecosystem: shared crates, shared toolchain, shared CI.

2. **AI-native architecture.** Anvil's next-milestone goal is to be a first-class
   AI dev environment — not just a terminal with a webview, but a control plane
   where AI agents can observe and drive the workspace. That requires well-typed
   message schemas, async task orchestration, and a client library for the Caldera
   API. Rust's type system, `serde`, and `tokio` are a better fit for this than
   Zig's current async story.

The Zig implementation (M0–M3) is complete and remains on `main`. The port
begins on the `rust-port` branch as a parallel workspace, leaving Zig untouched
until the Rust implementation reaches parity.

## Decision

### Workspace

A 12-crate Cargo workspace at the repo root (`Cargo.toml`, `crates/`), using:

- `edition = "2024"`, `rust-version = "1.85"`, resolver `"3"`.
- `[profile.release]`: `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`.

### Apple framework bindings

`objc2` (`0.6`) + the `objc2-*` family (`0.3`) replace the hand-written
`capi.zig` extern declarations. These bindings are generated from Apple's SDK
headers and are far more complete and type-safe than the Zig equivalents.

### PTY

`nix` (`0.31`, features: `fs`, `ioctl`, `process`, `signal`, `term`) replaces
the Zig PTY layer. This is the idiomatic Rust approach and matches what
`caldera-os` uses.

### Configuration

TOML (via the `toml` crate) replaces the Zig ZON config format. TOML is
human-readable, widely understood, and has first-class Serde support.

### Error handling

`thiserror` for library-level typed errors; `anyhow` for application-level
context propagation. This matches the established Rust convention and what
`caldera-os` uses.

### AI-native crate split

Three crates form the AI-native control plane:

| Crate | Role |
|---|---|
| `anvil-agent` | Message schema — shared types for agent↔app messages. Kept dependency-free so it can be embedded in agent code without pulling in heavy crates. |
| `anvil-caldera` | Caldera API client — HTTP/WS transport to the Caldera service. Depends only on `anvil-agent`. |
| `anvil-control` | Dual-transport control surface — accepts input from both keyboard (human) and agent (AI). Depends on `anvil-agent` for message types but not on `anvil-caldera`. |

The direction rule: `anvil-agent` depends on nothing internal. No domain crate
(`anvil-term`, `anvil-workspace`, `anvil-theme`) may depend on `anvil-control`.

## Crate Dependency Graph

```
anvil-term ──────────────────────────────────────────────┐
anvil-agent ───────────────────┐                         │
anvil-theme ──────┐            │                         │
                  ├── anvil-config ──────────────────┐   │
                  └── anvil-render ──┐               │   │
anvil-workspace ──┤                  │               │   │
                  ├── anvil-control ─┤               │   │
                  └─────────────────┤               │   │
anvil-caldera ─────────────────────┤               │   │
                                    └── anvil-platform ──┘
                                         └── anvil (bin)
anvil-prompt-core ── anvil-prompt (bin)
```

## Consequences

- P0 (this decision): Cargo workspace scaffolded; `zig build` still works;
  Rust crates are empty stubs.
- P1 onwards: terminal emulator, PTY, config, renderer, and platform layer
  are ported crate by crate.
- The `anvil-agent` schema crate must stay slim — it will be published
  separately so Caldera tooling can depend on it without pulling in the full
  app.
- `0002-tech-stack` is superseded; see pointer there.
